//! Looks for newer releases of the installed activities: by itself every
//! [`INTERVAL`], and when the owner asks in Cockpit. What a look finds is kept
//! in `ActivityUpdateChecks` (`updates.sql`), so Cockpit shows it at once and
//! a restart does not look again before it is due.
//!
//! A look reads the repository's releases from GitHub, which allows an
//! instance without a token sixty requests an hour. The worker looks at one
//! installation at a time, holds one of the installer's two download slots
//! while it does, and ends its round at GitHub's request limit; what it did
//! not reach is due at the next round.

use std::collections::HashMap;
use std::time::Duration;

use axum::http::StatusCode;
use rusqlite::{Connection, params};
use serde::Serialize;
use tracing::{info, warn};

use super::{DOWNLOADS, Installed, library, releases};
use crate::background::{Stop, Worker};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

const TARGET: &str = "Stellarr.Api.Activities";

/// How long a look that worked holds before the worker looks again.
pub(super) const INTERVAL: Duration = Duration::from_secs(12 * 60 * 60);
/// How long after a failed look the worker tries again.
const RETRY: Duration = Duration::from_secs(60 * 60);
/// The worker's first round comes a while after the server starts, and the
/// next ones this often; each looks only at what is due.
const FIRST_ROUND: Duration = Duration::from_secs(2 * 60);
const BETWEEN_ROUNDS: Duration = Duration::from_secs(15 * 60);

/// What the last look at an installation's repository found.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Check {
    pub checked_at: Timestamp,
    /// The newer version and its release page; none when the installed version is the newest.
    pub version: Option<String>,
    pub release: Option<String>,
    /// Why the look failed; a failed look says nothing of a newer version.
    pub error: Option<String>,
}

impl Check {
    /// A look that found nothing newer than what is installed.
    fn current(at: Timestamp) -> Check {
        Check {
            checked_at: at,
            version: None,
            release: None,
            error: None,
        }
    }
}

/// One look, kept.
pub(super) struct Looked {
    pub check: Check,
    /// GitHub refused for its request limit, so a look at another repository would fail too.
    pub limited: bool,
}

/// Whether `found` takes precedence over `installed`, as an update must.
fn newer(installed: &str, found: &str) -> ApiResult<bool> {
    let installed = semver::Version::parse(installed).map_err(ApiError::internal)?;
    let found = semver::Version::parse(found).map_err(ApiError::internal)?;
    Ok(found.cmp_precedence(&installed).is_gt())
}

/// The newer version `entry`'s repository offers, and its release page.
async fn look(entry: &Installed) -> ApiResult<Option<(String, String)>> {
    let selection = releases::discover(&entry.source, Some(&entry.manifest.id)).await?;
    let choice = selection.entries.first().ok_or_else(ApiError::not_found)?;
    Ok(newer(&entry.manifest.version, &choice.entry.version)?
        .then(|| (choice.entry.version.clone(), choice.release.clone())))
}

fn limited(error: &ApiError) -> bool {
    matches!(error, ApiError::Problem(problem) if problem.status() == StatusCode::TOO_MANY_REQUESTS)
}

/// What a failed look tells the owner.
fn explain(error: &ApiError) -> String {
    match error {
        ApiError::Problem(problem) => problem
            .message()
            .unwrap_or("GitHub could not supply the releases.")
            .to_owned(),
        ApiError::Status(StatusCode::NOT_FOUND) => {
            "GitHub no longer has this repository or this activity in its releases.".to_owned()
        }
        ApiError::Status(StatusCode::PAYLOAD_TOO_LARGE) => {
            "The repository's releases are larger than the installer reads.".to_owned()
        }
        _ => "The check failed. The server log has the reason.".to_owned(),
    }
}

/// Keeps a finding about the version installed as `digest`. A version
/// replaced or uninstalled while the look ran keeps nothing.
pub(super) fn keep_in(
    connection: &Connection,
    install: Id,
    digest: &str,
    check: &Check,
    next: Timestamp,
) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT OR REPLACE INTO ActivityUpdateChecks(Install,Digest,CheckedAt,NextAt,Version,Release,Error)
         SELECT ?1,?2,?3,?4,?5,?6,?7 WHERE EXISTS (SELECT 1 FROM ActivityInstalls WHERE Id=?1 AND Digest=?2)",
        params![install, digest, check.checked_at, next, check.version, check.release, check.error],
    )?;
    Ok(())
}

/// Carries a finding over to the version it found, as that version replaces
/// the one it was made about: the newest release is then the one installed.
/// A finding that named another version, or failed, stays with the old
/// version and so is no longer listed.
pub(super) fn carry_over(
    connection: &Connection,
    install: Id,
    from: &str,
    to: &str,
    version: &str,
) -> rusqlite::Result<()> {
    connection.execute(
        "UPDATE ActivityUpdateChecks SET Digest=?3,Version=NULL,Release=NULL
         WHERE Install=?1 AND Digest=?2 AND Version=?4 AND Error IS NULL",
        params![install, from, to, version],
    )?;
    Ok(())
}

/// Looks at `entry`'s repository once and keeps what it found, a failure
/// included. The caller holds a download slot.
pub(super) async fn check(state: &AppState, entry: &Installed) -> ApiResult<Looked> {
    let found = look(entry).await;
    let now = Timestamp::now();
    let (check, limited, next) = match found {
        Ok(newer) => {
            let (version, release) = newer.unzip();
            let check = Check {
                version,
                release,
                ..Check::current(now)
            };
            (check, false, now.plus(INTERVAL))
        }
        Err(error) => {
            if let ApiError::Internal(cause) = &error {
                warn!(target: TARGET, install = %entry.install, "An activity update check failed: {cause}");
            }
            let check = Check {
                error: Some(explain(&error)),
                ..Check::current(now)
            };
            (check, limited(&error), now.plus(RETRY))
        }
    };
    let (install, digest, kept) = (entry.install, entry.digest.clone(), check.clone());
    state
        .database()
        .write(move |tx| -> ApiResult<()> { Ok(keep_in(tx, install, &digest, &kept, next)?) })
        .await?;
    Ok(Looked { check, limited })
}

/// When each installation is next due, for those whose installed version has been looked at.
async fn due(state: &AppState) -> ApiResult<HashMap<Id, Timestamp>> {
    state
        .database()
        .read(|db| -> ApiResult<_> {
            Ok(db
                .prepare(
                    "SELECT c.Install,c.NextAt FROM ActivityUpdateChecks c
                     JOIN ActivityInstalls i ON i.Id=c.Install AND i.Digest=c.Digest",
                )?
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<_, _>>()?)
        })
        .await
}

/// Looks at every installation that is due, one after another.
async fn round(state: &AppState, stop: &Stop) -> ApiResult<()> {
    let now = Timestamp::now();
    let due = due(state).await?;
    for entry in library(state).await? {
        if due.get(&entry.install).is_some_and(|at| *at > now) {
            continue;
        }
        let slot = tokio::select! {
            slot = DOWNLOADS.acquire() => slot.map_err(ApiError::internal)?,
            () = stop.stopped() => return Ok(()),
        };
        let looked = check(state, &entry).await?;
        drop(slot);
        if looked.limited {
            info!(target: TARGET, "GitHub's request limit ended this round of activity update checks");
            break;
        }
    }
    Ok(())
}

pub fn workers() -> Vec<Worker> {
    vec![Worker::new("activity-updates", |state, stop| async move {
        let mut wait = FIRST_ROUND;
        while stop.sleep(wait).await {
            if let Err(error) = round(&state, &stop).await {
                warn!(target: TARGET, "A round of activity update checks failed: {error:?}");
            }
            wait = BETWEEN_ROUNDS;
        }
    })]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Problem;

    #[test]
    fn only_a_version_of_higher_precedence_is_newer() {
        assert!(newer("1.2.3", "1.3.0").unwrap());
        assert!(newer("1.0.0-beta.2", "1.0.0").unwrap());
        assert!(!newer("1.2.3", "1.2.3").unwrap());
        assert!(!newer("1.2.3", "1.2.3+build.7").unwrap());
        assert!(!newer("2.0.0", "1.9.9").unwrap());
        assert!(newer("1.0.0", "not a version").is_err());
    }

    #[test]
    fn a_failed_look_names_its_reason_and_only_the_request_limit_ends_a_round() {
        let quota = ApiError::from(
            Problem::new(StatusCode::TOO_MANY_REQUESTS).detail("GitHub's release request limit has been reached."),
        );
        assert!(limited(&quota));
        assert_eq!(explain(&quota), "GitHub's release request limit has been reached.");

        let refused = super::super::invalid("No matching stable activity release.");
        assert!(!limited(&refused));
        assert_eq!(explain(&refused), "No matching stable activity release.");

        let gone = ApiError::not_found();
        assert!(!limited(&gone));
        assert!(explain(&gone).contains("no longer has"));

        let broken = ApiError::internal("disk on fire");
        assert!(!explain(&broken).contains("disk"));
    }

    #[test]
    fn a_finding_is_kept_only_for_the_version_still_installed_and_replaces_the_last() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(concat!(
            include_str!("schema.sql"),
            include_str!("updates.sql")
        ))
        .unwrap();
        let install = Id::random();
        db.execute(
            "INSERT INTO ActivityInstalls(Id,Source,Activity,Digest,Enabled) VALUES(?1,'https://github.com/o/r','cards','new',1)",
            [install],
        )
        .unwrap();
        let at = Timestamp::from_unix_seconds(1_795_000_000).unwrap();
        let found = |db: &Connection| {
            db.query_row("SELECT Digest,Version,Error FROM ActivityUpdateChecks", [], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?))
            })
            .ok()
        };

        // The look began before an update: what it found is about the old version.
        keep_in(&db, install, "old", &Check::current(at), at.plus(INTERVAL)).unwrap();
        assert_eq!(found(&db), None);

        let newer = Check {
            version: Some("2.0.0".into()),
            release: Some("https://github.com/o/r/releases/tag/cards/v2.0.0".into()),
            ..Check::current(at)
        };
        keep_in(&db, install, "new", &newer, at.plus(INTERVAL)).unwrap();
        assert_eq!(found(&db), Some(("new".into(), Some("2.0.0".into()), None)));

        let failed = Check {
            error: Some("GitHub could not supply the releases.".into()),
            ..Check::current(at.plus(INTERVAL))
        };
        keep_in(&db, install, "new", &failed, at.plus(INTERVAL + RETRY)).unwrap();
        assert_eq!(
            found(&db),
            Some(("new".into(), None, Some("GitHub could not supply the releases.".into())))
        );
    }

    #[test]
    fn an_update_to_the_version_a_look_found_leaves_the_activity_current() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(concat!(include_str!("schema.sql"), include_str!("updates.sql")))
            .unwrap();
        let install = Id::random();
        db.execute(
            "INSERT INTO ActivityInstalls(Id,Source,Activity,Digest,Enabled) VALUES(?1,'https://github.com/o/r','cards','v1',1)",
            [install],
        )
        .unwrap();
        let at = Timestamp::from_unix_seconds(1_795_000_000).unwrap();
        let found = |version: &str| Check {
            version: Some(version.into()),
            release: Some(format!("https://github.com/o/r/releases/tag/cards/v{version}")),
            ..Check::current(at)
        };
        let listed = |db: &Connection| {
            db.query_row(
                "SELECT c.Version,c.Release,c.CheckedAt FROM ActivityUpdateChecks c
                 JOIN ActivityInstalls i ON i.Id=c.Install AND i.Digest=c.Digest",
                [],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Timestamp>(2)?)),
            )
            .ok()
        };
        let update = |db: &Connection, from: &str, to: &str, version: &str| {
            db.execute("UPDATE ActivityInstalls SET Digest=?1 WHERE Id=?2", params![to, install])
                .unwrap();
            carry_over(db, install, from, to, version).unwrap();
        };

        // The look found 2.0.0, and 2.0.0 is installed: current as of that look.
        keep_in(&db, install, "v1", &found("2.0.0"), at.plus(INTERVAL)).unwrap();
        update(&db, "v1", "v2", "2.0.0");
        assert_eq!(listed(&db), Some((None, None, at)));

        // The look found 3.0.0, but 2.5.0 is installed from its own link: unknown again.
        keep_in(&db, install, "v2", &found("3.0.0"), at.plus(INTERVAL)).unwrap();
        update(&db, "v2", "v25", "2.5.0");
        assert_eq!(listed(&db), None);
    }
}
