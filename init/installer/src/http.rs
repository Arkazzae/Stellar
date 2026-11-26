//! HTTPS, and nothing else: the release files from GitHub, Docker's install
//! script, and one question to the instance on the loopback address.

use std::time::Duration;

/// What a request came back with.
pub enum Answer {
    Body(Vec<u8>),
    /// A redirect that was not followed, and where it leads.
    Redirect(String),
    Status(u16),
    /// No answer at all: the error, as one line.
    Failed(String),
}

pub struct Http {
    #[cfg(feature = "test-seams")]
    fake: Option<fake::Fake>,
}

impl Http {
    pub fn new() -> Http {
        Http {
            #[cfg(feature = "test-seams")]
            fake: fake::Fake::from_env(),
        }
    }

    fn request(&self, url: &str, follow: bool, insecure: bool, timeout: u64) -> Answer {
        #[cfg(feature = "test-seams")]
        if let Some(fake) = &self.fake {
            return fake.request(url, follow);
        }
        real::request(url, follow, insecure, timeout)
    }

    /// Downloads `url`, following redirects to HTTPS only and trying again
    /// after a failure that may pass. The error is one line to show.
    pub fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        self.get_optional(url)?
            .ok_or_else(|| "The requested URL returned error: 404".to_string())
    }

    /// The old release layout is tried only when the requested path is absent.
    pub fn get_optional(&self, url: &str) -> Result<Option<Vec<u8>>, String> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let retry = match self.request(url, true, false, 120) {
                Answer::Body(body) => return Ok(Some(body)),
                Answer::Status(404) => return Ok(None),
                Answer::Redirect(to) => {
                    return Err(format!("Too many redirects, the last to {to}"));
                }
                Answer::Status(code) if !matches!(code, 408 | 429 | 500..=599) => {
                    return Err(format!("The requested URL returned error: {code}"));
                }
                Answer::Status(code) => format!("The requested URL returned error: {code}"),
                Answer::Failed(error) => error,
            };
            if attempt > 3 {
                return Err(retry);
            }
            self.pause(2);
        }
    }

    /// One request to the instance on this machine. Its certificate is not
    /// checked: on the loopback address it is self-signed and names another
    /// host.
    pub fn loopback(&self, url: &str) -> Option<String> {
        match self.request(url, false, true, 10) {
            Answer::Body(body) => Some(String::from_utf8_lossy(&body).into_owned()),
            _ => None,
        }
    }

    fn pause(&self, secs: u64) {
        #[cfg(feature = "test-seams")]
        if self.fake.is_some() {
            return;
        }
        std::thread::sleep(Duration::from_secs(secs));
    }
}

mod real {
    use super::{Answer, Duration};
    use ureq::Agent;
    use ureq::tls::TlsConfig;

    pub fn request(url: &str, follow: bool, insecure: bool, timeout: u64) -> Answer {
        // A plain address is refused before any connection, and a redirect
        // to one is never followed.
        if !url.starts_with("https://") {
            return Answer::Failed(format!("Not an HTTPS address: {url}"));
        }
        let tls = TlsConfig::builder().disable_verification(insecure).build();
        let agent: Agent = Agent::config_builder()
            .https_only(true)
            .http_status_as_error(false)
            .max_redirects(if follow { 10 } else { 0 })
            .max_redirects_will_error(false)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_global(Some(Duration::from_secs(timeout)))
            .user_agent(concat!("stellarr-installer/", env!("CARGO_PKG_VERSION")))
            .tls_config(tls)
            .build()
            .into();
        let mut response = match agent.get(url).call() {
            Ok(response) => response,
            Err(error) => return Answer::Failed(error.to_string()),
        };
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default();
            return Answer::Redirect(location.to_string());
        }
        if status != 200 {
            return Answer::Status(status);
        }
        match response
            .body_mut()
            .with_config()
            .limit(16 * 1024 * 1024)
            .read_to_vec()
        {
            Ok(body) => Answer::Body(body),
            Err(error) => Answer::Failed(error.to_string()),
        }
    }
}

/// Answers from files, for the tests. STELLARR_INSTALLER_TEST_HTTP names a
/// directory where `https://host/path` is the file `host/path` (`index` for
/// the bare host, the port left out), `host/path.redirect` holds where it
/// redirects, and `host.down` makes the host unreachable. Each request is
/// logged to STELLARR_INSTALLER_TEST_HTTP_LOG.
#[cfg(feature = "test-seams")]
mod fake {
    use super::Answer;
    use std::io::Write;
    use std::path::PathBuf;

    pub struct Fake {
        root: PathBuf,
        log: Option<PathBuf>,
    }

    impl Fake {
        pub fn from_env() -> Option<Fake> {
            let root = std::env::var_os("STELLARR_INSTALLER_TEST_HTTP")?;
            Some(Fake {
                root: root.into(),
                log: std::env::var_os("STELLARR_INSTALLER_TEST_HTTP_LOG").map(Into::into),
            })
        }

        pub fn request(&self, url: &str, follow: bool) -> Answer {
            if let Some(log) = &self.log
                && let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(log)
            {
                let _ = writeln!(file, "https GET {url}");
            }
            let Some(rest) = url.strip_prefix("https://") else {
                return Answer::Failed(format!("Not an HTTPS address: {url}"));
            };
            let rest = rest.split(['?', '#']).next().unwrap_or_default();
            let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
            let host = host.split(':').next().unwrap_or_default();
            if self.root.join(format!("{host}.down")).exists() {
                return Answer::Failed(format!("Could not resolve host: {host}"));
            }
            let path = if path.is_empty() { "index" } else { path };
            let file = self.root.join(host).join(path);
            let redirect = PathBuf::from(format!("{}.redirect", file.display()));
            if let Ok(to) = std::fs::read_to_string(&redirect) {
                let to = to.trim().to_string();
                return if follow {
                    self.request(&to, follow)
                } else {
                    Answer::Redirect(to)
                };
            }
            match std::fs::read(&file) {
                Ok(body) if file.is_file() => Answer::Body(body),
                _ => Answer::Status(404),
            }
        }
    }
}
