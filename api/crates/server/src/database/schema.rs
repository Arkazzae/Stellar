//! The schema a new database starts with.
//!
//! `schema.sql` is the schema the first server creates on an empty folder,
//! dumped from a real run and kept as it is: nothing in it is renamed,
//! retyped or tidied, so that both servers can run on copies of one file.
//! Ordered migrations bring that baseline and existing files up to date.

use rusqlite::{Connection, TransactionBehavior};

use crate::values::Timestamp;

use super::DbError;

const SCHEMA: &str = include_str!("schema.sql");

/// The one statement of the file that SQLite refuses: the dump lists the
/// bookkeeping table of `AUTOINCREMENT`, which SQLite has already created by
/// the time it reaches it, and a name starting with `sqlite_` is reserved.
const IMPLICIT_TABLE: &str = "CREATE TABLE sqlite_sequence(name,seq);\n";

/// Work that happens once in the life of a database. A database that starts
/// empty has no data for them to change, so it only records that they have
/// run; whatever writes data later has to write what they would have.
const ONE_TIME_STEPS: [&str; 2] = ["ordinary-permissions", "view-channel"];

/// One schema change. Every step has to be able to run again on a file that
/// already has it: a file at version zero may have been brought up before, so
/// statements say `IF NOT EXISTS` or change nothing the second time. SQLite
/// cannot add a column only when it is missing, so a column is looked for
/// first.
enum Step {
    Sql(&'static str),
    /// A column of a table the file already has, and what goes with it.
    Column {
        table: &'static str,
        column: &'static str,
        definition: &'static str,
        then: &'static str,
    },
}

impl Step {
    fn run(&self, connection: &Connection) -> rusqlite::Result<()> {
        match self {
            Step::Sql(sql) => connection.execute_batch(sql),
            Step::Column {
                table,
                column,
                definition,
                then,
            } => {
                let present: bool = connection.query_row(
                    "SELECT EXISTS (SELECT 1 FROM pragma_table_info(?1) WHERE \"name\" = ?2)",
                    [table, column],
                    |row| row.get(0),
                )?;
                if !present {
                    connection.execute_batch(&format!(
                        "ALTER TABLE \"{table}\" ADD COLUMN \"{column}\" {definition};"
                    ))?;
                }
                connection.execute_batch(then)
            }
        }
    }
}

/// Append each new schema change here. Version zero includes installations
/// that already have some or all of the original additive tables.
const MIGRATIONS: &[Step] = &[
    Step::Sql(concat!(
        include_str!("../discord_import/jobs.sql"),
        "\n",
        include_str!("../link_previews/imported.sql"),
        "\n",
        include_str!("upload_limits.sql"),
        "\n",
        include_str!("../activities/schema.sql"),
    )),
    Step::Sql(include_str!("../encryption/key_backups.sql")),
    Step::Sql(
        "CREATE TABLE IF NOT EXISTS AccountBoundEpochs (
        EpochId TEXT PRIMARY KEY REFERENCES EncryptionEpochs(Id) ON DELETE CASCADE
    );",
    ),
    Step::Sql(
        "CREATE TABLE IF NOT EXISTS CodeRecoveryAccounts (
        UserId TEXT PRIMARY KEY REFERENCES Users(Id) ON DELETE CASCADE,
        ArchivedKeyId TEXT
    ); CREATE TABLE IF NOT EXISTS RetiredPinAccounts (UserId TEXT PRIMARY KEY);
    CREATE TABLE IF NOT EXISTS RecoveryCodeUpgradeScrub (Pending INTEGER);",
    ),
    Step::Sql(include_str!("realtime_changes.sql")),
    Step::Sql(include_str!("../scheduled/schema.sql")),
    Step::Sql(include_str!("../polls/schema.sql")),
    Step::Sql(include_str!("../server_events/schema.sql")),
    // The groups a message calls on, as bits (1 everyone, 2 those online),
    // stamped by the server when an author who may call on them posts it;
    // see `messages::mentions`.
    Step::Column {
        table: "Messages",
        column: "GroupMention",
        definition: "INTEGER NOT NULL DEFAULT 0",
        then: include_str!("../messages/group_mentions.sql"),
    },
    // When an activity was first installed and when each of its versions
    // was; null for those installed before either was kept.
    Step::Column {
        table: "ActivityInstalls",
        column: "InstalledAt",
        definition: "TEXT",
        then: "",
    },
    Step::Column {
        table: "ActivityVersions",
        column: "InstalledAt",
        definition: "TEXT",
        then: "",
    },
    Step::Sql(include_str!("../activities/updates.sql")),
];

/// Sets the journal mode, which the database file remembers, and gives an
/// empty file its schema.
pub fn prepare(connection: &mut Connection) -> Result<(), DbError> {
    let mode: String = connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(DbError::Task(format!(
            "the database would not go into write-ahead logging; it answered \"{mode}\""
        )));
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: i64 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let pending = usize::try_from(version)
        .ok()
        .and_then(|v| MIGRATIONS.get(v..))
        .ok_or_else(|| {
            DbError::Task(format!(
                "unsupported database schema version {version} (this server supports 0 through {})",
                MIGRATIONS.len()
            ))
        })?;
    // Decided inside the write lock, so that two processes starting on one
    // empty file do not both create the tables.
    if version == 0
        && !transaction.query_row("SELECT EXISTS (SELECT 1 FROM sqlite_master)", [], |row| {
            row.get::<_, bool>(0)
        })?
    {
        // `IMPLICIT_TABLE` is dropped because SQLite refuses to create a table
        // whose name starts with `sqlite_`, and it has made this one itself
        // by the time the statement would run.
        transaction.execute_batch(&SCHEMA.replacen(IMPLICIT_TABLE, "", 1))?;
        // Older instances keep this baseline table until their PIN backups are sealed.
        transaction.execute("DROP TABLE AccountKeyBackups", [])?;
        let now = Timestamp::now();
        for name in ONE_TIME_STEPS {
            transaction.execute(
                "INSERT OR IGNORE INTO \"OneTimeSteps\" (\"Name\", \"AppliedAt\") VALUES (?1, ?2)",
                rusqlite::params![name, now],
            )?;
        }
    }
    if !pending.is_empty() {
        for step in pending {
            step.run(&transaction)?;
        }
        transaction.pragma_update(None, "user_version", MIGRATIONS.len() as i64)?;
    }
    transaction.commit()?;
    Ok(())
}
