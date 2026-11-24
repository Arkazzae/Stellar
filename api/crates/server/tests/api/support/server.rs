use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use stellarr_api::http::Endpoints;
use stellarr_api::settings::Settings;
use stellarr_api::values::Id;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::client::Client;

/// A folder of its own, removed when it goes out of scope.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> TempDir {
        let path = std::env::temp_dir().join(format!("stellarr-it-{}", Id::random()));
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

enum Running {
    Process(Child),
    InProcess {
        stop: Option<oneshot::Sender<()>>,
        task: JoinHandle<()>,
    },
}

pub struct Server {
    address: SocketAddr,
    folder_path: PathBuf,
    log: Arc<Mutex<Vec<String>>>,
    running: Option<Running>,
    /// Last, so that it is removed after the process has been told to stop.
    folder: Option<TempDir>,
}

fn environment(folder: &Path, extra: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut variables: Vec<(String, String)> = vec![
        ("ASPNETCORE_ENVIRONMENT".into(), "Testing".into()),
        ("ASPNETCORE_URLS".into(), "http://127.0.0.1:0".into()),
        ("STELLARR_DATA_ROOT".into(), folder.display().to_string()),
        (
            "ConnectionStrings__Database".into(),
            format!(
                "Data Source={};Pooling=True",
                folder.join("stellarr.db").display()
            ),
        ),
        (
            "Testing__EventLog".into(),
            folder.join("events.jsonl").display().to_string(),
        ),
    ];
    for (name, value) in extra {
        variables.retain(|(have, _)| have != name);
        variables.push((name.to_string(), value.to_string()));
    }
    variables
}

impl Server {
    /// The real binary, in the Testing environment.
    pub async fn start() -> Server {
        Server::start_with(&[]).await
    }

    /// The real binary with more variables in its environment, or other
    /// values for the ones the harness sets.
    pub async fn start_with(extra: &[(&str, &str)]) -> Server {
        let folder = TempDir::new();
        let path = folder.path().to_path_buf();
        let mut server = Server::spawn(&path, extra).await;
        server.folder = Some(folder);
        server
    }

    /// The real binary on a data folder that is not this server's own: it is
    /// not removed when the server goes. For a second start on the same files.
    pub async fn start_in(folder: &Path, extra: &[(&str, &str)]) -> Server {
        Server::spawn(folder, extra).await
    }

    /// The real binary, which is expected not to start. Returns what it said.
    pub async fn refused_to_start(extra: &[(&str, &str)]) -> String {
        let folder = TempDir::new();
        let output = Command::new(env!("CARGO_BIN_EXE_stellarr-api"))
            .env_clear()
            .envs(environment(folder.path(), extra))
            .stdin(Stdio::null())
            .output()
            .await
            .expect("the binary runs");
        assert!(!output.status.success(), "it should not have started");
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    async fn spawn(folder: &Path, extra: &[(&str, &str)]) -> Server {
        let mut child = Command::new(env!("CARGO_BIN_EXE_stellarr-api"))
            .env_clear()
            .envs(environment(folder, extra))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .expect("the binary starts");
        let mut lines = BufReader::new(child.stdout.take().expect("a piped output")).lines();
        let log = Arc::new(Mutex::new(Vec::new()));

        // The server says where it listens once it is bound.
        let address = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let line = lines
                    .next_line()
                    .await
                    .expect("the server's output")
                    .expect("the server stopped before it listened");
                log.lock().unwrap().push(line.clone());
                if let Some((_, address)) = line.split_once("Now listening on: http://") {
                    return address.trim().parse::<SocketAddr>().expect("an address");
                }
            }
        })
        .await
        .expect("the server listens within 30 seconds");
        let sink = Arc::clone(&log);
        tokio::spawn(async move {
            while let Ok(Some(line)) = lines.next_line().await {
                sink.lock().unwrap().push(line);
            }
        });

        let server = Server {
            address,
            folder: None,
            folder_path: folder.to_path_buf(),
            running: Some(Running::Process(child)),
            log,
        };
        server.wait_until_ready().await;
        server
    }

    /// The library's server, run in this process with `routes` instead of the
    /// real groups, plus the health group.
    pub async fn in_process(routes: Endpoints, extra: &[(&str, &str)]) -> Server {
        let folder = TempDir::new();
        let variables = environment(folder.path(), extra);
        let settings = Settings::load(&variables).expect("usable settings");
        let (told, listening) = oneshot::channel();
        let (stop, stopped) = oneshot::channel::<()>();
        let mut groups = stellarr_api::groups();
        groups.push(routes);
        let task = tokio::spawn(async move {
            let result = stellarr_api::serve(
                settings,
                groups,
                move |addresses| {
                    let _ = told.send(addresses[0]);
                },
                async move {
                    let _ = stopped.await;
                },
            )
            .await;
            result.expect("the server ran");
        });
        let address = listening.await.expect("the server listens");
        let server = Server {
            address,
            folder_path: folder.path().to_path_buf(),
            folder: Some(folder),
            running: Some(Running::InProcess {
                stop: Some(stop),
                task,
            }),
            log: Arc::new(Mutex::new(Vec::new())),
        };
        server.wait_until_ready().await;
        server
    }

    async fn wait_until_ready(&self) {
        let client = Client::anonymous(self.address);
        for _ in 0..300 {
            let asked = client
                .try_send(super::client::Request::new(
                    http::Method::GET,
                    "/health/ready",
                ))
                .await;
            if asked.is_ok_and(|response| response.status == 200) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("the server did not become ready");
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// A client that says it is the browser build.
    pub fn client(&self) -> Client {
        Client::browser(self.address)
    }

    /// A client that sends nothing to say what it is.
    pub fn anonymous(&self) -> Client {
        Client::anonymous(self.address)
    }

    /// A client that identifies itself as `kind` at `version`.
    pub fn client_of(&self, kind: &str, version: &str) -> Client {
        Client::anonymous(self.address)
            .with_header("X-Stellarr-Client", kind)
            .with_header("X-Stellarr-Client-Version", version)
    }

    pub fn folder(&self) -> &Path {
        &self.folder_path
    }

    pub fn database_path(&self) -> PathBuf {
        self.folder_path.join("stellarr.db")
    }

    /// The lines the server has written to its log so far (the binary only).
    pub fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    /// The events handlers have published so far, oldest first, as the JSON
    /// lines the Testing environment records.
    pub fn events(&self) -> Vec<serde_json::Value> {
        let text =
            std::fs::read_to_string(self.folder_path.join("events.jsonl")).unwrap_or_default();
        text.lines()
            .map(|line| serde_json::from_str(line).expect("a line of JSON"))
            .collect()
    }

    /// Stops the server the way the container's manager does, with SIGTERM,
    /// and says whether it left cleanly.
    pub async fn stop(mut self) -> bool {
        match self.running.take() {
            Some(Running::Process(mut child)) => {
                let pid = child.id().expect("a running process");
                let sent = std::process::Command::new("kill")
                    .arg("-TERM")
                    .arg(pid.to_string())
                    .status();
                if !sent.is_ok_and(|status| status.success()) {
                    return false;
                }
                match tokio::time::timeout(Duration::from_secs(30), child.wait()).await {
                    Ok(Ok(status)) => status.success(),
                    _ => false,
                }
            }
            Some(Running::InProcess { mut stop, task }) => {
                let _ = stop.take().map(|stop| stop.send(()));
                task.await.is_ok()
            }
            None => true,
        }
    }
}

impl Drop for Server {
    /// The process is killed before the folder is removed, which happens when
    /// the fields go, after this.
    fn drop(&mut self) {
        match &mut self.running {
            Some(Running::InProcess { stop, task }) => {
                let _ = stop.take().map(|stop| stop.send(()));
                task.abort();
            }
            Some(Running::Process(child)) => {
                let _ = child.start_kill();
            }
            None => {}
        }
    }
}
