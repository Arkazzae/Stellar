//! An instance on disk that the tool is run against, as `docker exec` would
//! run it: a data root with the database the server makes, a keys folder, and
//! rows the tests put in.
//!
//! The database is made by the server's own library, so its schema is the one
//! the server runs on. Nothing here talks to a server: the tool needs none.

#![allow(dead_code)]

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::{Connection, params};
use serde_json::Value;
use stellarr_api::database::Database;
use stellarr_api::seal::Sealer;
use stellarr_api::settings::DatabaseSettings;
use stellarr_api::values::{Id, Timestamp};
use stellarr_cli::terminal::PromptFailure;
use stellarr_cli::{Io, run};

/// What stands in for the hidden prompt of a terminal.
pub type Prompt = Box<dyn FnMut() -> Result<String, PromptFailure>>;

pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> TempDir {
        let path = std::env::temp_dir().join(format!("stellarr-cli-test-{}", Id::random()));
        std::fs::create_dir_all(&path).expect("a temporary folder");
        TempDir(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What a run printed and how it ended.
#[derive(Debug)]
pub struct Run {
    pub exit: i32,
    pub output: String,
    pub error: String,
}

impl Run {
    /// The standard output as JSON, which a `--json` run always wrote.
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.output)
            .unwrap_or_else(|error| panic!("not JSON ({error}): {}", self.output))
    }

    pub fn code(&self) -> String {
        self.json()["error"]["code"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }
}

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub struct Instance {
    dir: TempDir,
}

impl Instance {
    /// A data root that has nothing in it.
    pub fn unstarted() -> Instance {
        Instance {
            dir: TempDir::new(),
        }
    }

    /// A data root of an instance that has run: its database, with the
    /// instance server and an owner, and the master key.
    pub fn started() -> Instance {
        let instance = Instance::unstarted();
        std::fs::create_dir_all(instance.root().join("db")).unwrap();
        Database::open(&DatabaseSettings {
            path: instance.database_path(),
            busy_timeout: Duration::from_secs(5),
        })
        .expect("the database opens");
        Sealer::load_or_create(&instance.root().join("keys")).expect("a master key");
        let connection = instance.db();
        connection
            .execute(
                "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\")
                 VALUES (?1, 'Orbit', '2026-11-01 10:00:00', 1, 1)",
                [Id::random()],
            )
            .unwrap();
        instance.user("jo", "Jo", Some("jo@example.test"));
        instance
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    pub fn database_path(&self) -> PathBuf {
        self.root().join("db").join("stellarr.db")
    }

    pub fn keys(&self) -> PathBuf {
        self.root().join("keys")
    }

    pub fn db(&self) -> Connection {
        let connection = Connection::open(self.database_path()).expect("the database");
        connection.busy_timeout(Duration::from_secs(5)).unwrap();
        connection
    }

    /// An account, a member of the instance server, with a password hash.
    pub fn user(&self, username: &str, display_name: &str, email: Option<&str>) -> Id {
        let connection = self.db();
        let id = Id::random();
        connection
            .execute(
                "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"PasswordHash\", \"Email\",
                     \"NormalizedEmail\", \"IsDisabled\", \"SessionVersion\", \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\")
                 VALUES (?1, ?2, ?3, ?4, 'pbkdf2-sha256$600000$salt$hash', ?5, ?6, 0, 1, 0, 'c', '2026-11-02 10:00:00')",
                params![
                    id,
                    username,
                    username.to_uppercase(),
                    display_name,
                    email,
                    email.map(str::to_uppercase)
                ],
            )
            .unwrap();
        let (server, role): (Id, Option<Id>) = {
            let server: Id = connection
                .query_row("SELECT \"Id\" FROM \"Servers\" LIMIT 1", [], |row| {
                    row.get(0)
                })
                .unwrap();
            let role: Option<Id> = connection
                .query_row(
                    "SELECT \"Id\" FROM \"ServerRoles\" WHERE \"ServerId\" = ?1 LIMIT 1",
                    [server],
                    |row| row.get(0),
                )
                .ok();
            (server, role)
        };
        let role = role.unwrap_or_else(|| {
            let role = Id::random();
            connection
                .execute(
                    "INSERT INTO \"ServerRoles\" (\"Id\", \"ServerId\", \"Name\", \"Color\", \"Permissions\", \"Position\", \"IsSystem\", \"IsOwner\")
                     VALUES (?1, ?2, 'Member', '', 0, 1, 1, 0)",
                    params![role, server],
                )
                .unwrap();
            role
        });
        connection
            .execute(
                "INSERT INTO \"ServerMembers\" (\"ServerId\", \"UserId\", \"RoleId\", \"JoinedAt\", \"OwnerSlot\") VALUES (?1, ?2, ?3, '2026-11-02 10:00:00', NULL)",
                params![server, id, role],
            )
            .unwrap();
        id
    }

    pub fn session(&self, user: Id, device: &str) -> Id {
        let id = Id::random();
        let now = Timestamp::now();
        let ends = now.plus(Duration::from_secs(86_400));
        self.db()
            .execute(
                "INSERT INTO \"UserSessions\" (\"Id\", \"UserId\", \"Device\", \"IpAddress\", \"CreatedAt\", \"LastSeenAt\", \"ExpiresAt\",
                     \"AbsoluteExpiresAt\", \"PrimarySource\", \"AuthenticatedAt\", \"MfaCompletedAt\", \"MfaEvidence\", \"SecurityVersion\", \"ShortLived\")
                 VALUES (?1, ?2, ?3, '192.0.2.10', ?4, ?4, ?5, ?5, 'password', ?4, NULL, NULL, 1, 0)",
                params![id, user, device, now, ends],
            )
            .unwrap();
        id
    }

    pub fn run(&self, args: &[&str]) -> Run {
        self.run_with(args, "", false, &[])
    }

    pub fn run_with_stdin(&self, stdin: &str, args: &[&str]) -> Run {
        self.run_with(args, stdin, false, &[])
    }

    pub fn run_interactive(&self, stdin: &str, args: &[&str]) -> Run {
        self.run_with(args, stdin, true, &[])
    }

    pub fn run_with_environment(&self, environment: &[(&str, &str)], args: &[&str]) -> Run {
        self.run_with(args, "", false, environment)
    }

    /// Asked, with something done while the question is waiting for its answer:
    /// what another person might do to the instance in the meantime.
    pub fn run_asked(
        &self,
        args: &[&str],
        answer: &str,
        meanwhile: impl FnOnce() + 'static,
    ) -> Run {
        self.run_io(
            args,
            Box::new(Asked(
                Some(Box::new(meanwhile)),
                Cursor::new(answer.as_bytes().to_vec()),
            )),
            true,
            &[],
        )
    }

    pub fn run_with(
        &self,
        args: &[&str],
        stdin: &str,
        interactive: bool,
        environment: &[(&str, &str)],
    ) -> Run {
        self.run_io(
            args,
            Box::new(Cursor::new(stdin.as_bytes().to_vec())),
            interactive,
            environment,
        )
    }

    /// With an output that has gone away, as a pipe does when what reads it
    /// is closed.
    pub fn run_into_a_closed_pipe(&self, args: &[&str]) -> Run {
        let mut words = vec!["--data-root".to_string(), self.root().display().to_string()];
        words.extend(args.iter().map(|word| word.to_string()));
        let error = Sink::default();
        let io = Io {
            input: Box::new(Cursor::new(Vec::new())),
            output: Box::new(ClosedPipe),
            error: Box::new(error.clone()),
            interactive: false,
            secret_prompt: None,
        };
        let variables = vec![("ASPNETCORE_ENVIRONMENT".to_string(), "Testing".to_string())];
        let exit = run(&words, io, variables, Arc::new(AtomicBool::new(false)));
        Run {
            exit,
            output: String::new(),
            error: String::from_utf8_lossy(&error.0.lock().unwrap()).into_owned(),
        }
    }

    /// At a terminal that answers the hidden prompt with `prompt`; `cancelled`
    /// is the flag Ctrl+C sets.
    pub fn run_prompted(&self, args: &[&str], prompt: Prompt, cancelled: Arc<AtomicBool>) -> Run {
        self.run_full(
            args,
            Box::new(Cursor::new(Vec::new())),
            true,
            &[],
            Some(prompt),
            cancelled,
        )
    }

    fn run_io(
        &self,
        args: &[&str],
        input: Box<dyn std::io::BufRead>,
        interactive: bool,
        environment: &[(&str, &str)],
    ) -> Run {
        self.run_full(
            args,
            input,
            interactive,
            environment,
            None,
            Arc::new(AtomicBool::new(false)),
        )
    }

    fn run_full(
        &self,
        args: &[&str],
        input: Box<dyn std::io::BufRead>,
        interactive: bool,
        environment: &[(&str, &str)],
        prompt: Option<Prompt>,
        cancelled: Arc<AtomicBool>,
    ) -> Run {
        let mut words = vec!["--data-root".to_string(), self.root().display().to_string()];
        words.extend(args.iter().map(|word| word.to_string()));
        let output = Sink::default();
        let error = Sink::default();
        let io = Io {
            input,
            output: Box::new(output.clone()),
            error: Box::new(error.clone()),
            interactive,
            secret_prompt: prompt,
        };
        // The environment of a run is what it is given, and a test names the
        // environment the host runs in unless it names another.
        let mut variables: Vec<(String, String)> =
            vec![("ASPNETCORE_ENVIRONMENT".into(), "Testing".into())];
        for (name, value) in environment {
            variables.retain(|(have, _)| have != name);
            variables.push((name.to_string(), value.to_string()));
        }
        let exit = run(&words, io, variables, cancelled);
        let text = |sink: &Sink| String::from_utf8_lossy(&sink.0.lock().unwrap()).into_owned();
        Run {
            exit,
            output: text(&output),
            error: text(&error),
        }
    }
}

/// An input that is empty and an output that goes nowhere, for a test that
/// only builds a runtime.
pub fn silent_io() -> Io {
    Io {
        input: Box::new(Cursor::new(Vec::new())),
        output: Box::new(std::io::sink()),
        error: Box::new(std::io::sink()),
        interactive: false,
        secret_prompt: None,
    }
}

/// An output whose reader has gone: every write fails.
struct ClosedPipe;

impl Write for ClosedPipe {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }
}

/// An input that runs something the first time it is read, then answers.
struct Asked(Option<Box<dyn FnOnce()>>, Cursor<Vec<u8>>);

impl std::io::Read for Asked {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if let Some(meanwhile) = self.0.take() {
            meanwhile();
        }
        self.1.read(buffer)
    }
}

impl std::io::BufRead for Asked {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if let Some(meanwhile) = self.0.take() {
            meanwhile();
        }
        self.1.fill_buf()
    }

    fn consume(&mut self, amount: usize) {
        self.1.consume(amount);
    }
}
