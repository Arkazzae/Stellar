//! Recovery codes: ten single-use codes an account keeps for the day its other
//! second steps are out of reach. Only the SHA-256 of a code is stored, so the
//! codes themselves are shown once, when they are made.

use rusqlite::{Connection, Transaction, params};

use super::tokens;
use crate::database::DbError;
use crate::values::{Id, Timestamp};

/// How many codes a set holds.
pub const COUNT: usize = 10;
const GROUPS: usize = 4;
const GROUP_LENGTH: usize = 7;

/// What a code is worked out from, whatever the person typed: its letters and
/// digits in capitals. Dashes, spaces and lower case do not matter.
pub fn normalize(code: &str) -> String {
    code.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// A new set, as the person is shown it: the old unused codes stop working at
/// `now` and ten new ones are stored as digests.
pub fn regenerate(tx: &Transaction, user: Id, now: Timestamp) -> Result<Vec<String>, DbError> {
    spend_all(tx, user, now)?;
    let batch = Id::random();
    let mut codes = Vec::with_capacity(COUNT);
    for _ in 0..COUNT {
        let code = (0..GROUPS)
            .map(|_| tokens::random_text(GROUP_LENGTH))
            .collect::<Vec<_>>()
            .join("-");
        tx.execute(
            "INSERT INTO \"RecoveryCodes\" (\"Id\", \"UserId\", \"BatchId\", \"CodeHash\", \"CreatedAt\", \"UsedAt\")
             VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
            params![Id::random(), user, batch, tokens::hash(&normalize(&code)), now],
        )?;
        codes.push(code);
    }
    Ok(codes)
}

/// Retires every unused code of the account.
pub fn spend_all(tx: &Transaction, user: Id, now: Timestamp) -> Result<usize, DbError> {
    Ok(tx.execute(
        "UPDATE \"RecoveryCodes\" SET \"UsedAt\" = ?2 WHERE \"UserId\" = ?1 AND \"UsedAt\" IS NULL",
        params![user, now],
    )?)
}

/// Spends the code, if it is one of the account's unused ones. True for one
/// request only: the row is claimed with a conditional update, so two at once
/// cannot both spend it.
///
/// The digest is compared with every unused digest of the account in constant
/// time, rather than by a lookup on it, so that how long the answer takes
/// says nothing about the code.
pub(super) fn consume(
    tx: &Transaction,
    user: Id,
    code: &str,
    now: Timestamp,
) -> Result<bool, DbError> {
    let normalized = normalize(code);
    if normalized.is_empty() {
        return Ok(false);
    }
    let supplied = tokens::hash(&normalized);
    let mut statement = tx.prepare(
        "SELECT \"Id\", \"CodeHash\" FROM \"RecoveryCodes\" WHERE \"UserId\" = ?1 AND \"UsedAt\" IS NULL",
    )?;
    let rows = statement
        .query_map([user], |row| {
            Ok((row.get::<_, Id>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut found = None;
    for (id, stored) in rows {
        if tokens::fixed_time_equals(supplied.as_bytes(), stored.as_bytes()) {
            found = Some(id);
        }
    }
    let Some(id) = found else {
        return Ok(false);
    };
    let changed = tx.execute(
        "UPDATE \"RecoveryCodes\" SET \"UsedAt\" = ?2 WHERE \"Id\" = ?1 AND \"UsedAt\" IS NULL",
        params![id, now],
    )?;
    Ok(changed == 1)
}

/// How many codes the account can still spend.
pub fn remaining(connection: &Connection, user: Id) -> rusqlite::Result<i64> {
    connection.query_row(
        "SELECT COUNT(*) FROM \"RecoveryCodes\" WHERE \"UserId\" = ?1 AND \"UsedAt\" IS NULL",
        [user],
        |row| row.get(0),
    )
}

/// A first set for an account that has none left, and nothing for one that
/// has: adding another second step does not replace codes already written
/// down.
pub fn regenerate_if_missing(
    tx: &Transaction,
    user: Id,
    now: Timestamp,
) -> Result<Option<Vec<String>>, DbError> {
    if remaining(tx, user)? > 0 {
        return Ok(None);
    }
    regenerate(tx, user, now).map(Some)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::time::Duration;

    use super::*;
    use crate::auth::fixtures::{add_user, open};

    fn used_at(connection: &Connection, user: Id) -> Vec<Option<Timestamp>> {
        let mut statement = connection
            .prepare("SELECT \"UsedAt\" FROM \"RecoveryCodes\" WHERE \"UserId\" = ?1 ORDER BY \"CreatedAt\", \"Id\"")
            .unwrap();
        statement
            .query_map([user], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn a_set_is_ten_codes_of_four_groups_and_only_their_digests_are_kept() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let tx = connection.transaction().unwrap();

        let codes = regenerate(&tx, user, Timestamp::now()).unwrap();
        tx.commit().unwrap();

        assert_eq!(codes.len(), COUNT);
        assert_eq!(codes.iter().collect::<HashSet<_>>().len(), COUNT);
        for code in &codes {
            let groups: Vec<&str> = code.split('-').collect();
            assert_eq!(groups.len(), 4, "{code}");
            assert!(groups.iter().all(|g| g.len() == 7), "{code}");
        }
        let mut statement = connection
            .prepare("SELECT \"CodeHash\" FROM \"RecoveryCodes\" WHERE \"UserId\" = ?1")
            .unwrap();
        let stored: HashSet<String> = statement
            .query_map([user], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let expected: HashSet<String> = codes.iter().map(|c| tokens::hash(&normalize(c))).collect();
        assert_eq!(stored, expected);
        assert!(stored.iter().all(|hash| hash.len() == 64));
        assert_eq!(remaining(&connection, user).unwrap(), 10);
    }

    #[test]
    fn a_code_is_taken_as_typed_in_any_case_with_or_without_its_dashes() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let tx = connection.transaction().unwrap();
        let codes = regenerate(&tx, user, Timestamp::now()).unwrap();
        let now = Timestamp::now();

        assert!(consume(&tx, user, &codes[0].to_lowercase(), now).unwrap());
        assert!(consume(&tx, user, &format!(" {} ", codes[1].replace('-', " ")), now).unwrap());
        assert!(consume(&tx, user, &codes[2].replace('-', ""), now).unwrap());
        assert_eq!(remaining(&tx, user).unwrap(), 7);
    }

    #[test]
    fn a_code_is_spent_once() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let tx = connection.transaction().unwrap();
        let codes = regenerate(&tx, user, Timestamp::now()).unwrap();
        let now = Timestamp::now();

        assert!(consume(&tx, user, &codes[0], now).unwrap());
        assert!(!consume(&tx, user, &codes[0], now).unwrap());
        assert_eq!(remaining(&tx, user).unwrap(), 9);
        assert!(consume(&tx, user, &codes[1], now).unwrap());
    }

    #[test]
    fn a_code_of_another_account_or_a_wrong_or_empty_one_spends_nothing() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let other = add_user(&connection, None);
        let tx = connection.transaction().unwrap();
        let codes = regenerate(&tx, user, Timestamp::now()).unwrap();
        regenerate(&tx, other, Timestamp::now()).unwrap();
        let now = Timestamp::now();

        assert!(!consume(&tx, other, &codes[0], now).unwrap());
        assert!(!consume(&tx, user, "AAAAAAA-BBBBBBB-CCCCCCC-DDDDDDD", now).unwrap());
        assert!(!consume(&tx, user, "", now).unwrap());
        assert!(!consume(&tx, user, " - - ", now).unwrap());
        assert!(!consume(&tx, user, &codes[0][..27], now).unwrap());
        assert_eq!(remaining(&tx, user).unwrap(), 10);
        assert_eq!(remaining(&tx, other).unwrap(), 10);
    }

    #[test]
    fn a_new_set_retires_the_old_one_at_the_moment_it_is_made() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let tx = connection.transaction().unwrap();
        let old = regenerate(&tx, user, Timestamp::now()).unwrap();
        let later = Timestamp::now().plus(Duration::from_secs(60));

        let new = regenerate(&tx, user, later).unwrap();

        assert_eq!(remaining(&tx, user).unwrap(), 10);
        let times = used_at(&tx, user);
        assert_eq!(times.iter().filter(|t| **t == Some(later)).count(), 10);
        assert_eq!(times.iter().filter(|t| t.is_none()).count(), 10);
        assert!(!consume(&tx, user, &old[0], later).unwrap());
        assert!(consume(&tx, user, &new[0], later).unwrap());
    }

    #[test]
    fn a_first_set_is_made_only_for_an_account_with_no_code_left() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let tx = connection.transaction().unwrap();
        let now = Timestamp::now();

        let first = regenerate_if_missing(&tx, user, now).unwrap().unwrap();
        assert_eq!(regenerate_if_missing(&tx, user, now).unwrap(), None);
        // With nine left the set stays; with none left, a new one is made.
        assert!(consume(&tx, user, &first[0], now).unwrap());
        assert_eq!(regenerate_if_missing(&tx, user, now).unwrap(), None);
        for code in &first[1..] {
            assert!(consume(&tx, user, code, now).unwrap());
        }
        assert_eq!(remaining(&tx, user).unwrap(), 0);
        assert_eq!(
            regenerate_if_missing(&tx, user, now)
                .unwrap()
                .unwrap()
                .len(),
            10
        );
    }

    #[test]
    fn spending_every_code_leaves_none() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let tx = connection.transaction().unwrap();
        regenerate(&tx, user, Timestamp::now()).unwrap();

        assert_eq!(spend_all(&tx, user, Timestamp::now()).unwrap(), 10);
        assert_eq!(spend_all(&tx, user, Timestamp::now()).unwrap(), 0);
        assert_eq!(remaining(&tx, user).unwrap(), 0);
    }

    #[test]
    fn normalizing_keeps_ascii_letters_and_digits_in_capitals_and_nothing_else() {
        assert_eq!(normalize("ab-C d_9\u{e9}\u{661}\u{ff21}\t"), "ABCD9");
        assert_eq!(normalize("ı ſ \u{212a}"), "");
        assert_eq!(normalize(""), "");
    }
}
