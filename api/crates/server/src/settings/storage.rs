//! Where uploaded files are kept and how much of them an instance takes: the
//! `Storage` and `Uploads` sections.

use std::fmt;

use stellarr_storage::{StorageOptions, StorageProvider, UploadOptions};

use super::SettingError;
use super::config::Config;

/// The `Storage` section: the folder of the data volume, or a bucket.
#[derive(Clone, PartialEq, Eq)]
pub struct StorageSettings {
    pub provider: Provider,
    pub endpoint: String,
    pub access_key: String,
    pub secret_key: String,
    pub bucket: String,
    /// Set by tests that stand in for the store: the start does not wait for
    /// the store to be ready.
    pub skip_initialization: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    FileSystem,
    S3,
}

impl StorageSettings {
    pub(super) fn load(config: &Config) -> Result<StorageSettings, SettingError> {
        let provider = match config.text("Storage:Provider") {
            None => Provider::FileSystem,
            Some(name) if name.eq_ignore_ascii_case("FileSystem") || name == "0" => {
                Provider::FileSystem
            }
            Some(name) if name.eq_ignore_ascii_case("S3") || name == "1" => Provider::S3,
            Some(name) => {
                return Err(SettingError::new(format!(
                    "Storage:Provider must be \"FileSystem\" or \"S3\", but is \"{name}\"."
                )));
            }
        };
        let text = |key: &str| config.raw(key).unwrap_or("").to_string();
        let settings = StorageSettings {
            provider,
            endpoint: text("Storage:Endpoint"),
            access_key: text("Storage:AccessKey"),
            secret_key: text("Storage:SecretKey"),
            bucket: text("Storage:Bucket"),
            skip_initialization: config.flag("Storage:SkipInitialization")?.unwrap_or(false),
        };
        if provider == Provider::S3
            && (settings.endpoint.trim().is_empty() || settings.bucket.trim().is_empty())
        {
            return Err(SettingError::new(
                "Storage:Provider is S3, so Storage:Endpoint and Storage:Bucket are required.",
            ));
        }
        Ok(settings)
    }

    /// The settings as the storage crate takes them.
    pub fn options(&self) -> StorageOptions {
        StorageOptions {
            provider: match self.provider {
                Provider::FileSystem => StorageProvider::FileSystem,
                Provider::S3 => StorageProvider::S3,
            },
            endpoint: self.endpoint.clone(),
            access_key: self.access_key.clone(),
            secret_key: self.secret_key.clone(),
            bucket: self.bucket.clone(),
        }
    }
}

impl fmt::Debug for StorageSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StorageSettings")
            .field("provider", &self.provider)
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .field("skip_initialization", &self.skip_initialization)
            .finish_non_exhaustive()
    }
}

/// The `Uploads` section: size limits and quotas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadSettings {
    pub max_image_bytes: i64,
    pub max_audio_bytes: i64,
    pub max_video_bytes: i64,
    pub max_file_bytes: i64,
    pub max_stored_bytes_per_user: i64,
    pub max_stored_bytes: i64,
    pub minimum_free_data_root_bytes: i64,
    pub max_concurrent_uploads: i32,
}

impl UploadSettings {
    pub(super) fn load(config: &Config) -> Result<UploadSettings, SettingError> {
        let defaults = UploadOptions::default();
        let number = |key: &str, fallback: i64| -> Result<i64, SettingError> {
            Ok(config.number(key)?.unwrap_or(fallback))
        };
        Ok(UploadSettings {
            max_image_bytes: number("Uploads:MaxImageBytes", defaults.max_image_bytes)?,
            max_audio_bytes: number("Uploads:MaxAudioBytes", defaults.max_audio_bytes)?,
            max_video_bytes: number("Uploads:MaxVideoBytes", defaults.max_video_bytes)?,
            max_file_bytes: number("Uploads:MaxFileBytes", defaults.max_file_bytes)?,
            max_stored_bytes_per_user: number(
                "Uploads:MaxStoredBytesPerUser",
                defaults.max_stored_bytes_per_user,
            )?,
            max_stored_bytes: number("Uploads:MaxStoredBytes", defaults.max_stored_bytes)?,
            minimum_free_data_root_bytes: number(
                "Uploads:MinimumFreeDataRootBytes",
                defaults.minimum_free_data_root_bytes,
            )?,
            max_concurrent_uploads: i32::try_from(number(
                "Uploads:MaxConcurrentUploads",
                i64::from(defaults.max_concurrent_uploads),
            )?)
            .map_err(|_| {
                SettingError::new("Uploads:MaxConcurrentUploads must be a 32-bit whole number.")
            })?,
        })
    }

    /// The settings as the storage crate's checks take them.
    pub fn options(&self) -> UploadOptions {
        UploadOptions {
            max_image_bytes: self.max_image_bytes,
            max_audio_bytes: self.max_audio_bytes,
            max_video_bytes: self.max_video_bytes,
            max_file_bytes: self.max_file_bytes,
            max_stored_bytes_per_user: self.max_stored_bytes_per_user,
            max_stored_bytes: self.max_stored_bytes,
            minimum_free_data_root_bytes: self.minimum_free_data_root_bytes,
            max_concurrent_uploads: self.max_concurrent_uploads,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(pairs: &[(&str, &str)]) -> Config {
        let variables: Vec<(String, String)> = pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        Config::layered("Production", &variables)
    }

    #[test]
    fn the_folder_and_the_shipped_limits_are_the_default() {
        let config = config(&[]);

        let storage = StorageSettings::load(&config).unwrap();
        let uploads = UploadSettings::load(&config).unwrap();

        assert_eq!(storage.provider, Provider::FileSystem);
        assert!(!storage.skip_initialization);
        assert_eq!(uploads.options().max_video_bytes, 100 * 1024 * 1024);
        assert_eq!(uploads.max_concurrent_uploads, 4);
    }

    #[test]
    fn a_bucket_needs_an_endpoint_and_a_bucket() {
        let missing = StorageSettings::load(&config(&[("Storage__Provider", "s3")])).unwrap_err();
        assert!(
            missing.to_string().contains("Storage:Endpoint"),
            "{missing}"
        );

        let named = StorageSettings::load(&config(&[
            ("Storage__Provider", "S3"),
            ("Storage__Endpoint", "http://bucket.test:9000"),
            ("Storage__Bucket", "stellarr"),
        ]))
        .unwrap();
        assert_eq!(named.options().provider, StorageProvider::S3);
        assert!(!format!("{named:?}").contains("secret"));
    }

    #[test]
    fn an_unknown_provider_is_refused_by_name() {
        let refused = StorageSettings::load(&config(&[("Storage__Provider", "tape")])).unwrap_err();

        assert!(refused.to_string().contains("tape"), "{refused}");
    }

    #[test]
    fn a_limit_is_read_from_its_key() {
        let uploads = UploadSettings::load(&config(&[
            ("Uploads__MaxImageBytes", "12"),
            ("STELLARR_MIN_FREE_BYTES", "0"),
        ]))
        .unwrap();

        assert_eq!(uploads.max_image_bytes, 12);
        assert_eq!(uploads.minimum_free_data_root_bytes, 0);
    }
}
