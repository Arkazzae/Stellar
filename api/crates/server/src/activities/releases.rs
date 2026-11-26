//! Resolve published GitHub release assets. Only the archive whose bytes the
//! owner reviewed may become current; branch content is never executable input.
use std::{
    collections::{BTreeMap, HashSet},
    io::Cursor,
    sync::LazyLock,
    time::Duration,
};

use aws_lc_rs::digest::{SHA256, digest};
use axum::http::{HeaderValue, StatusCode, header};
use serde::{Deserialize, Serialize};
use stellarr_archive::{Limits, ZipArchive};
use stellarr_outbound::{AddressPolicy, Client, Request};

use super::{Manifest, invalid};
use crate::encoding::{hex_digit, hex_lower, percent_encode};
use crate::http::{ApiError, ApiResult};

const ARCHIVE_LIMIT: u64 = 256 * 1024 * 1024;
static HTTP: LazyLock<Client> = LazyLock::new(Client::new);

#[derive(Deserialize)]
struct GithubRelease {
    id: u64,
    tag_name: String,
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    id: u64,
    name: String,
    size: u64,
    browser_download_url: String,
}
#[derive(Deserialize)]
struct Commit {
    sha: String,
}
#[derive(Deserialize)]
struct Index {
    protocol: u16,
    activities: Vec<Entry>,
}
#[derive(Clone, Deserialize, Serialize)]
pub(super) struct Entry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub asset: String,
    pub sha256: String,
    pub bytes: u64,
}
/// A repository can offer choices from different GitHub releases.
#[derive(Serialize)]
pub(super) struct Choice {
    #[serde(flatten)]
    pub entry: Entry,
    pub release: String,
    #[serde(skip)]
    tag: String,
    #[serde(skip)]
    download: Download,
}
#[derive(Serialize)]
pub(super) struct Selection {
    pub source: String,
    pub entries: Vec<Choice>,
    #[serde(skip)]
    repository: String,
}
struct Source {
    repository: String,
    tag: Option<String>,
}

fn component(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
fn tag_component(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b))
}
fn decode_tag(encoded: &str) -> Option<String> {
    if encoded.len() > 768 {
        return None;
    }
    let mut decoded = Vec::new();
    let mut bytes = encoded.bytes();
    while let Some(byte) = bytes.next() {
        decoded.push(if byte == b'%' {
            hex_digit(bytes.next()?)? * 16 + hex_digit(bytes.next()?)?
        } else {
            byte
        });
    }
    let tag = String::from_utf8(decoded).ok()?;
    (tag.len() <= 255 && tag.split('/').all(tag_component)).then_some(tag)
}
fn source(url: &str) -> ApiResult<Source> {
    let parts: Vec<_> = url
        .trim_end_matches('/')
        .strip_prefix("https://github.com/")
        .ok_or_else(|| invalid("Paste a public GitHub repository or release URL."))?
        .split('/')
        .collect();
    let [owner, repository, tail @ ..] = parts.as_slice() else {
        return Err(invalid("Paste a repository link or a release tag."));
    };
    let repo = repository.strip_suffix(".git").unwrap_or(repository);
    if !component(owner) || !component(repo) {
        return Err(invalid("Invalid GitHub repository URL."));
    }
    let tag = match tail {
        [] => None,
        ["releases", "tag", parts @ ..] if !parts.is_empty() => Some(
            decode_tag(&parts.join("/")).ok_or_else(|| invalid("Invalid GitHub release tag."))?,
        ),
        _ => return Err(invalid("Paste a repository link or a release tag.")),
    };
    Ok(Source {
        repository: format!("{owner}/{repo}").to_ascii_lowercase(),
        tag,
    })
}

fn scoped(tag: &str) -> Option<(&str, semver::Version)> {
    let (id, version) = tag.split_once("/v")?;
    super::activity_id(id).then_some((id, semver::Version::parse(version).ok()?))
}
fn tag_link(repository: &str, tag: &str) -> String {
    let path = tag
        .split('/')
        .map(percent_encode)
        .collect::<Vec<_>>()
        .join("/");
    format!("https://github.com/{repository}/releases/tag/{path}")
}
async fn fetch(url: String, limit: u64, asset: bool) -> ApiResult<Vec<u8>> {
    let answer = HTTP
        .send_collected(
            Request::get(&url)
                .map_err(ApiError::internal)?
                .header(
                    header::USER_AGENT,
                    HeaderValue::from_static("Stellarr-Activities/1"),
                )
                .header(
                    header::ACCEPT,
                    HeaderValue::from_static(if asset {
                        "application/octet-stream"
                    } else {
                        "application/vnd.github+json"
                    }),
                )
                .policy(AddressPolicy::PublicOnly)
                .timeout(Duration::from_secs(45))
                .max_redirects(3)
                // The outbound ceiling counts response headers as well as the body.
                .max_response_bytes(limit + 64 * 1024),
        )
        .await
        .map_err(|error| {
            tracing::warn!(url, reason = %error, "activity release download failed");
            invalid("GitHub could not deliver the release within the download limits.")
        })?;
    if answer.status == axum::http::StatusCode::TOO_MANY_REQUESTS
        || (answer.status == axum::http::StatusCode::FORBIDDEN
            && answer
                .headers
                .get("x-ratelimit-remaining")
                .is_some_and(|v| v == "0"))
    {
        let reset = answer
            .headers
            .get("x-ratelimit-reset")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .and_then(crate::values::Timestamp::from_unix_seconds);
        let retry = if let Some(at) = reset {
            format!("Retry after {at} (UTC).")
        } else if let Some(seconds) = answer
            .headers
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
        {
            format!("Retry in {seconds} seconds.")
        } else {
            "Try again when the request quota resets.".into()
        };
        return Err(
            crate::http::Problem::new(axum::http::StatusCode::TOO_MANY_REQUESTS)
                .detail(format!(
                    "GitHub's release request limit for this instance has been reached. {retry}"
                ))
                .into(),
        );
    }
    if matches!(answer.status, StatusCode::NOT_FOUND | StatusCode::GONE) {
        return Err(ApiError::not_found());
    }
    if !answer.status.is_success()
        || !answer.url.is_https()
        || !(answer.url.host() == "api.github.com"
            || (asset
                && (answer.url.host() == "github.com"
                    || answer.url.host().ends_with(".githubusercontent.com"))))
    {
        return Err(invalid(
            "GitHub could not supply this published release or asset.",
        ));
    }
    if answer.body.len() as u64 > limit {
        return Err(invalid("The downloaded asset exceeds its declared size."));
    }
    Ok(answer.body.to_vec())
}

struct Download {
    browser: String,
    api: String,
}
impl Download {
    fn new(repository: &str, asset: &Asset) -> ApiResult<Self> {
        let browser = &asset.browser_download_url;
        let prefix = format!("https://github.com/{repository}/releases/download/");
        if asset.id == 0 || !browser.to_ascii_lowercase().starts_with(&prefix) {
            return Err(invalid("Unexpected GitHub release download URL."));
        }
        Ok(Self {
            browser: browser.clone(),
            api: format!(
                "https://api.github.com/repos/{repository}/releases/assets/{}",
                asset.id
            ),
        })
    }

    async fn bytes<F, Fut>(&self, limit: u64, get: &F) -> ApiResult<Vec<u8>>
    where
        F: Fn(String, u64, bool) -> Fut,
        Fut: std::future::Future<Output = ApiResult<Vec<u8>>>,
    {
        // Public browser downloads preserve the REST quota. A release retag
        // can invalidate its old browser URL while the asset ID stays valid.
        match get(self.browser.clone(), limit, true).await {
            Err(ApiError::Status(StatusCode::NOT_FOUND)) => {
                get(self.api.clone(), limit, true).await
            }
            result => result,
        }
    }
}

fn choices(repository: &str, release: &GithubRelease, bytes: &[u8]) -> ApiResult<Vec<Choice>> {
    let index: Index =
        serde_json::from_slice(bytes).map_err(|_| invalid("Invalid stellarr-release.json."))?;
    if index.protocol != 1 || index.activities.is_empty() || index.activities.len() > 64 {
        return Err(invalid("Unsupported or empty release index."));
    }
    if let Some((id, version)) = scoped(&release.tag_name) {
        let [one] = index.activities.as_slice() else {
            return Err(invalid(
                "A scoped release must contain exactly its own activity.",
            ));
        };
        if one.id != id || one.version != version.to_string() {
            return Err(invalid(
                "The release tag must match the index activity ID and version.",
            ));
        }
    }
    let mut unique = HashSet::new();
    let mut entries = Vec::new();
    for entry in index.activities {
        let asset = release
            .assets
            .iter()
            .find(|a| a.name == entry.asset && a.size == entry.bytes)
            .ok_or_else(|| {
                invalid("The ZIP listed in the release index is missing or has a different size.")
            })?;
        if !super::activity_id(&entry.id)
            || !unique.insert(entry.id.clone())
            || entry.name.trim().is_empty()
            || semver::Version::parse(&entry.version).is_err()
            || entry.bytes == 0
            || entry.bytes > ARCHIVE_LIMIT
            || entry.asset.contains('/')
            || !entry.asset.ends_with(".zip")
            || entry.sha256.len() != 64
            || !entry
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid(
                "Invalid activity, digest, version or ZIP size in the release index.",
            ));
        }
        entries.push(Choice {
            entry,
            release: tag_link(repository, &release.tag_name),
            tag: release.tag_name.clone(),
            download: Download::new(repository, asset)?,
        });
    }
    Ok(entries)
}

pub(super) async fn discover(url: &str, wanted: Option<&str>) -> ApiResult<Selection> {
    discover_with(url, wanted, &fetch).await
}

async fn discover_with<F, Fut>(url: &str, wanted: Option<&str>, get: &F) -> ApiResult<Selection>
where
    F: Fn(String, u64, bool) -> Fut,
    Fut: std::future::Future<Output = ApiResult<Vec<u8>>>,
{
    let Source { repository, tag } = source(url)?;
    let base = format!("https://api.github.com/repos/{repository}");
    let mut releases = Vec::new();
    if let Some(exact) = &tag {
        let bytes = get(
            format!("{base}/releases/tags/{}", percent_encode(exact)),
            2 << 20,
            false,
        )
        .await?;
        let release: GithubRelease = serde_json::from_slice(&bytes)
            .map_err(|_| invalid("Invalid GitHub release metadata."))?;
        if release.draft || release.id == 0 || &release.tag_name != exact {
            return Err(invalid("This tag does not name a published release."));
        }
        releases.push(release);
    } else {
        let mut seen = HashSet::new();
        for page in 1.. {
            let bytes = get(
                format!("{base}/releases?per_page=100&page={page}"),
                16 << 20,
                false,
            )
            .await?;
            let batch: Vec<GithubRelease> = serde_json::from_slice(&bytes)
                .map_err(|_| invalid("Invalid GitHub release list."))?;
            let last = batch.len() < 100;
            releases.extend(batch.into_iter().filter(|r| {
                !r.draft
                    && !r.prerelease
                    && r.id != 0
                    && seen.insert(r.id)
                    && decode_tag(&r.tag_name).as_deref() == Some(r.tag_name.as_str())
                    && r.assets.iter().any(|a| a.name == "stellarr-release.json")
                    && (!r.tag_name.contains('/')
                        || scoped(&r.tag_name).is_some_and(|(id, v)| {
                            v.pre.is_empty() && wanted.is_none_or(|w| w == id)
                        }))
            }));
            if last {
                break;
            }
        }
        // Legacy bundle tag versions do not describe the individual entries;
        // consult their indexes first. Then visit scoped versions newest first.
        releases.sort_by(|a, b| match (scoped(&a.tag_name), scoped(&b.tag_name)) {
            (Some((_, av)), Some((_, bv))) => bv.cmp_precedence(&av).then(a.id.cmp(&b.id)),
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, None) => a.id.cmp(&b.id),
        });
    }
    let mut best = BTreeMap::<String, (semver::Version, bool, Choice)>::new();
    let mut diagnostic = None;
    for release in releases {
        let scope = scoped(&release.tag_name);
        if let Some((id, version)) = &scope
            && let Some((current, is_scoped, _)) = best.get(*id)
        {
            let cmp = current.cmp_precedence(version);
            if cmp.is_gt() || (cmp.is_eq() && *is_scoped) {
                continue;
            }
        }
        let Some(index_asset) = release
            .assets
            .iter()
            .find(|a| a.name == "stellarr-release.json")
        else {
            return Err(invalid(format!(
                "Release {} has no stellarr-release.json. Attach the built activity ZIP and its release index.",
                release.tag_name
            )));
        };
        let bytes = Download::new(&repository, index_asset)?
            .bytes(256 << 10, get)
            .await?;
        let entries = match choices(&repository, &release, &bytes) {
            Ok(items) => items,
            Err(error) if tag.is_some() => return Err(error),
            Err(error) => {
                diagnostic = Some(error);
                continue;
            }
        };
        for entry in entries {
            let version =
                semver::Version::parse(&entry.entry.version).map_err(ApiError::internal)?;
            let id = &entry.entry.id;
            if wanted.is_some_and(|w| w != id) || (tag.is_none() && !version.pre.is_empty()) {
                continue;
            }
            let takes_precedence = best.get(id).is_none_or(|(old, was_scoped, _)| {
                let cmp = version.cmp_precedence(old);
                cmp.is_gt() || (cmp.is_eq() && scope.is_some() && !was_scoped)
            });
            if takes_precedence {
                best.insert(id.clone(), (version, scope.is_some(), entry));
            }
        }
    }
    if best.is_empty() {
        return Err(diagnostic.unwrap_or_else(|| {
            invalid("No matching stable activity release. Publish the built ZIP and stellarr-release.json or paste a specific release link.")
        }));
    }
    Ok(Selection {
        source: format!("https://github.com/{repository}"),
        repository,
        entries: best.into_values().map(|(_, _, choice)| choice).collect(),
    })
}

pub(super) async fn revision(selection: &Selection, choice: &Choice) -> ApiResult<String> {
    let url = format!(
        "https://api.github.com/repos/{}/commits/{}",
        selection.repository,
        percent_encode(&choice.tag)
    );
    let commit: Commit = serde_json::from_slice(&fetch(url, 4 << 20, false).await?)
        .map_err(|_| invalid("Could not resolve this release tag to its source revision."))?;
    if commit.sha.len() != 40 || !commit.sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("Invalid source revision."));
    }
    Ok(commit.sha)
}

pub(super) struct Package {
    pub manifest: Manifest,
    pub files: BTreeMap<String, Vec<u8>>,
}
pub(super) async fn package(choice: &Choice) -> ApiResult<Package> {
    let entry = choice.entry.clone();
    let data = choice.download.bytes(entry.bytes, &fetch).await?;
    tokio::task::spawn_blocking(move || unpack(data, &entry))
        .await
        .map_err(ApiError::internal)?
}

fn unpack(data: Vec<u8>, entry: &Entry) -> ApiResult<Package> {
    if data.len() as u64 != entry.bytes
        || hex_lower(digest(&SHA256, &data).as_ref()) != entry.sha256
    {
        return Err(invalid(
            "The release size or SHA-256 checksum changed. Review the release again.",
        ));
    }
    let mut zip = ZipArchive::with_limits(
        Cursor::new(data),
        Limits {
            max_entries: 8192,
            max_directory_bytes: 2 << 20,
            max_entry_bytes: ARCHIVE_LIMIT,
            max_total_bytes: ARCHIVE_LIMIT,
        },
    )
    .map_err(|_| invalid("Invalid or oversized activity ZIP."))?;
    let mut files = BTreeMap::new();
    for i in 0..zip.entries().len() {
        let item = &zip.entries()[i];
        let path = item
            .name()
            .ok_or_else(|| invalid("Activity filenames must be UTF-8."))?;
        let directory = item.is_directory();
        if item.is_symlink()
            || path.len() > 512
            || path.contains('\\')
            || path.chars().any(char::is_control)
            || path.trim_end_matches('/').split('/').any(|s| {
                s.is_empty() || s.starts_with('.') || s == "_stellarr" || s == "node_modules"
            })
        {
            return Err(invalid("The ZIP contains a reserved or unsafe path."));
        }
        if directory {
            continue;
        }
        let path = path.to_owned();
        let bytes = zip
            .read_to_vec(i, ARCHIVE_LIMIT)
            .map_err(|_| invalid("Activity ZIP verification failed."))?;
        if files.insert(path, bytes).is_some() {
            return Err(invalid("Duplicate file in the activity ZIP."));
        }
    }
    let manifest: Manifest = serde_json::from_slice(
        files
            .get("stellarr.json")
            .ok_or_else(|| invalid("Missing stellarr.json at ZIP root."))?,
    )
    .map_err(|_| invalid("Invalid stellarr.json."))?;
    manifest.check()?;
    if !files.contains_key("index.html")
        || manifest.id != entry.id
        || manifest.version != entry.version
        || manifest.name != entry.name
    {
        return Err(invalid(
            "The built activity must include index.html and match its release index.",
        ));
    }
    Ok(Package { manifest, files })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use stellarr_archive::{EntryOptions, ZipWriter};

    fn archive(permission: &str, extra: &str) -> (Vec<u8>, Entry) {
        let out = Cursor::new(Vec::new());
        let mut zip = ZipWriter::new(out);
        let manifest = json!({"id":"demo","protocol":1,"name":"Demo","version":"1.0.0+1","description":"A board","permissions":[permission]});
        for (name, data) in [
            ("index.html", String::from("<title>Board</title>")),
            ("stellarr.json", manifest.to_string()),
            (extra, String::from("content")),
        ] {
            zip.add_bytes(name, EntryOptions::stored(), data.as_bytes())
                .unwrap();
        }
        let data = zip.finish().unwrap().into_inner();
        let entry = Entry {
            id: "demo".into(),
            name: "Demo".into(),
            version: "1.0.0+1".into(),
            asset: "demo.zip".into(),
            bytes: data.len() as u64,
            sha256: hex_lower(digest(&SHA256, &data).as_ref()),
        };
        (data, entry)
    }
    #[test]
    fn validates_release_before_admitting_any_files_or_device_grants() {
        let (data, mut entry) = archive("media.cameraTile", "art/board.png");
        let package = unpack(data.clone(), &entry).unwrap();
        assert_eq!(package.manifest.permissions, ["media.cameraTile"]);
        assert!(package.files.contains_key("art/board.png"));
        entry.sha256 = "0".repeat(64);
        assert!(unpack(data, &entry).is_err());
        let (data, entry) = archive("media.cameraTile", "../outside");
        assert!(unpack(data, &entry).is_err());
        let (data, entry) = archive("unknown", "art/board.png");
        assert!(unpack(data, &entry).is_err());
    }
    #[test]
    fn accepts_repository_and_published_tag_links_only() {
        assert_eq!(
            source("https://github.com/Owner/demo").unwrap().repository,
            "owner/demo"
        );
        assert_eq!(
            source("https://github.com/owner/demo/releases/tag/v1.2.0")
                .unwrap()
                .tag
                .as_deref(),
            Some("v1.2.0")
        );
        for bad in [
            "http://github.com/owner/demo",
            "https://github.com.example/owner/demo",
            "https://github.com/a/b/tree/main",
            "https://github.com/a/..",
        ] {
            assert!(source(bad).is_err());
        }
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    use serde_json::{Value, json};
    use std::cell::RefCell;
    const REPO: &str = "https://github.com/example/games";
    const API: &str = "https://api.github.com/repos/example/games";
    const INDEX: &str = "stellarr-release.json";

    struct Published {
        meta: Value,
        url: String,
        index: Value,
    }
    fn published(id: u64, tag: &str, items: &[(&str, &str)]) -> Published {
        // GitHub can return percent-encoded slashes in an asset's download URL,
        // independently of the spelling in the human-facing release URL.
        let prefix = format!("{REPO}/releases/download/{}", percent_encode(tag));
        let url = format!("{prefix}/{INDEX}");
        let mut assets =
            vec![json!({"id":id*100,"name":INDEX,"size":200,"browser_download_url":url})];
        let entries: Vec<_> = items
            .iter()
            .enumerate()
            .map(|(n, (name, version))| {
                let file = format!("{name}.zip");
                assets.push(json!({
                    "id": id * 100 + 1 + n as u64, "name": file, "size": 502,
                    "browser_download_url": format!("{prefix}/{file}")
                }));
                json!({
                    "id": name, "name": name, "version": version,
                    "sha256": "e".repeat(64), "bytes": 502, "asset": file
                })
            })
            .collect();
        Published {
            meta: json!({"id":id,"name":"A human-readable title","tag_name":tag,"draft":false,"prerelease":false,"assets":assets}),
            url,
            index: json!({"protocol":1,"activities":entries}),
        }
    }
    #[tokio::test]
    async fn repository_paginates_and_picks_each_highest_stable_compatible_index() {
        let mut cards_future = published(8, "cards/v11.0.0", &[("cards", "11.0.0")]);
        cards_future.index["protocol"] = json!(2);
        let mut draft = published(9, "cards/v12.0.0", &[("cards", "12.0.0")]);
        draft.meta["draft"] = json!(true);
        let mut pre = published(10, "music/v8.0.0", &[("music", "8.0.0")]);
        pre.meta["prerelease"] = json!(true);
        let releases = vec![
            published(3, "music/v2.0.0", &[("music", "2.0.0")]),
            published(1, "v1.0", &[("legacy", "1.2.0"), ("cards", "8.0.0")]),
            published(4, "cards/v9.0.0", &[("cards", "9.0.0")]),
            published(5, "cards/v10.0.0", &[("cards", "10.0.0")]),
            published(2, "v2.0", &[("music", "2.0.0")]),
            published(6, "music/v3.0.0-beta.1", &[("music", "3.0.0-beta.1")]),
            cards_future,
            draft,
            pre,
        ];
        let mut first: Vec<_> = (100..194)
            .map(|id| json!({"id":id,"tag_name":format!("other-{id}"),"draft":false,"assets":[]}))
            .collect();
        first.extend(releases[..6].iter().map(|r| r.meta.clone()));
        assert_eq!(first.len(), 100);
        let mut answers = BTreeMap::from([
            (format!("{API}/releases?per_page=100&page=1"), json!(first)),
            (
                format!("{API}/releases?per_page=100&page=2"),
                json!(
                    releases[6..]
                        .iter()
                        .map(|r| r.meta.clone())
                        .collect::<Vec<_>>()
                ),
            ),
        ]);
        for r in &releases {
            answers.insert(r.url.clone(), r.index.clone());
        }
        let asked = RefCell::new(Vec::new());
        let get = |url: String, _limit: u64, _asset: bool| {
            asked.borrow_mut().push(url.clone());
            std::future::ready(Ok(serde_json::to_vec(
                answers
                    .get(&url)
                    .unwrap_or_else(|| panic!("unexpected fetch {url}")),
            )
            .unwrap()))
        };
        let catalog = discover_with(REPO, None, &get).await.unwrap();
        assert_eq!(
            catalog
                .entries
                .iter()
                .map(|c| (
                    c.entry.id.as_str(),
                    c.entry.version.as_str(),
                    c.tag.as_str()
                ))
                .collect::<Vec<_>>(),
            [
                ("cards", "10.0.0", "cards/v10.0.0"),
                ("legacy", "1.2.0", "v1.0"),
                ("music", "2.0.0", "music/v2.0.0")
            ]
        );
        assert_eq!(
            catalog.entries[0].download.browser,
            format!("{REPO}/releases/download/cards%2Fv10.0.0/cards.zip")
        );
        assert!(
            !asked.borrow().contains(&releases[2].url),
            "Downloaded an obsolete scoped index"
        );
        for r in [&releases[5], &releases[7], &releases[8]] {
            assert!(!asked.borrow().contains(&r.url));
        }
        asked.borrow_mut().clear();
        let music = discover_with(REPO, Some("music"), &get).await.unwrap();
        assert_eq!(music.entries.len(), 1);
        assert_eq!(music.entries[0].tag, "music/v2.0.0");
        assert!(!asked.borrow().iter().any(|u| u.contains("/download/cards")));
    }
    #[tokio::test]
    async fn renamed_tag_downloads_its_original_asset_by_numeric_id() {
        let download = Download::new(
            "example/games",
            &Asset {
                id: 123,
                name: "cards.zip".into(),
                size: 3,
                browser_download_url: format!("{REPO}/releases/download/old-tag/cards.zip"),
            },
        )
        .unwrap();
        let asked = RefCell::new(Vec::new());
        let get = |url: String, limit, asset: bool| {
            assert_eq!(limit, 3);
            assert!(asset);
            asked.borrow_mut().push(url.clone());
            std::future::ready(if url == download.browser {
                Err(ApiError::not_found())
            } else {
                assert_eq!(url, format!("{API}/releases/assets/123"));
                Ok(vec![1, 2, 3])
            })
        };
        assert_eq!(download.bytes(3, &get).await.unwrap(), [1, 2, 3]);
        assert_eq!(asked.borrow().len(), 2);
    }
    #[tokio::test]
    async fn exact_links_preserve_historical_tags_and_validate_scoped_identity() {
        for (spelling, tag) in [
            ("v1.0.0", "v1.0.0"),
            ("cards/v1.0.0%2Bfirst", "cards/v1.0.0+first"),
            ("cards%2Fv1.0.0%2Bfirst", "cards/v1.0.0+first"),
        ] {
            let version = if tag.contains('+') {
                "1.0.0+first"
            } else {
                "1.0.0"
            };
            let release = published(1, tag, &[("cards", version)]);
            let exact_api = format!("{API}/releases/tags/{}", percent_encode(tag));
            let get = |url: String, _limit: u64, _asset: bool| {
                let val = if url == exact_api {
                    &release.meta
                } else {
                    assert_eq!(url, release.url);
                    &release.index
                };
                std::future::ready(Ok(serde_json::to_vec(val).unwrap()))
            };
            let selection = discover_with(&format!("{REPO}/releases/tag/{spelling}"), None, &get)
                .await
                .unwrap();
            assert_eq!(selection.entries[0].tag, tag);
            assert_eq!(selection.entries[0].release, tag_link("example/games", tag));
        }
        let bad = published(9, "cards/v9.0.0", &[("music", "9.0.0")]);
        let release: GithubRelease = serde_json::from_value(bad.meta).unwrap();
        assert!(
            choices(
                "example/games",
                &release,
                &serde_json::to_vec(&bad.index).unwrap()
            )
            .is_err()
        );
        let bundle = published(
            10,
            "cards/v9.0.0",
            &[("cards", "9.0.0"), ("music", "1.0.0")],
        );
        assert!(
            choices(
                "example/games",
                &serde_json::from_value(bundle.meta).unwrap(),
                &serde_json::to_vec(&bundle.index).unwrap()
            )
            .is_err()
        );
        for invalid in ["cards%2F..%2Fv1", "cards%00", "cards%252Fv1", "cards%3Fq"] {
            assert!(source(&format!("{REPO}/releases/tag/{invalid}")).is_err());
        }
    }
}
