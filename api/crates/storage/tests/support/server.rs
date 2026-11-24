//! A real S3-compatible server named by the environment, for the tests that
//! are ignored unless one is running. `tests/minio.sh` starts one.
//!
//! * `STELLARR_TEST_S3_ENDPOINT`: `http://127.0.0.1:19000` or `https://...`
//! * `STELLARR_TEST_S3_ACCESS_KEY`, `STELLARR_TEST_S3_SECRET_KEY`
//! * `STELLARR_TEST_S3_CA`: for an https endpoint, the PEM file of the root
//!   that signed its certificate.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use stellarr_outbound::{Client, tls};
use stellarr_storage::{BucketStore, BucketTuning, ObjectStorage, StorageOptions, StorageProvider};

#[derive(Clone)]
pub struct Server {
    pub endpoint: String,
    pub access_key: String,
    pub secret_key: String,
    pub ca: Option<PathBuf>,
}

fn variable(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!(
            "{name} is not set: these tests need a real S3-compatible server; see tests/minio.sh"
        )
    })
}

impl Server {
    pub fn from_env() -> Self {
        Self {
            endpoint: variable("STELLARR_TEST_S3_ENDPOINT"),
            access_key: variable("STELLARR_TEST_S3_ACCESS_KEY"),
            secret_key: variable("STELLARR_TEST_S3_SECRET_KEY"),
            ca: std::env::var_os("STELLARR_TEST_S3_CA").map(PathBuf::from),
        }
    }

    pub fn https(&self) -> bool {
        self.endpoint.starts_with("https://")
    }

    /// An HTTP client that trusts the test root when there is one.
    pub fn http(&self) -> Client {
        match &self.ca {
            Some(path) => {
                let roots = tls::roots_from_file(path).expect("the test root is a PEM certificate");
                Client::with_tls_config(tls::client_config(roots))
            }
            None => Client::new(),
        }
    }

    pub fn options(&self, bucket: &str) -> StorageOptions {
        StorageOptions {
            provider: StorageProvider::S3,
            endpoint: self.endpoint.clone(),
            access_key: self.access_key.clone(),
            secret_key: self.secret_key.clone(),
            bucket: bucket.to_owned(),
        }
    }
}

/// A store on a bucket nobody has used: a new name every call.
pub async fn store_from_env(label: &str) -> BucketStore {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let server = Server::from_env();
    let bucket = format!(
        "stellarr-{label}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let tuning = BucketTuning {
        part_size: 5 * 1024 * 1024,
        ..BucketTuning::default()
    };
    // Never the default: a real store may be slow to its first answer.
    let tuning = BucketTuning {
        request_timeout: std::time::Duration::from_secs(60),
        ..tuning
    };
    let store = BucketStore::with_tuning(&server.options(&bucket), server.http(), tuning).unwrap();
    store
        .prepare()
        .await
        .expect("the store prepares the bucket");
    store
}
