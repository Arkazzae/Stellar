//! Where uploaded files are kept and how link previews are fetched: the two
//! sections that name an outside system.

use stellarr_embeds::EmbedSettings;
use stellarr_storage::{StorageOptions, StorageProvider};

use super::{Config, SettingError};

/// Where uploads are kept: files on the data volume, unless an S3-compatible
/// store is named. The fields after the provider are that store's.
#[derive(Clone, PartialEq, Eq)]
pub struct StorageSettings {
    pub provider: Provider,
    pub endpoint: String,
    pub access_key: String,
    pub secret_key: String,
    pub bucket: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    FileSystem,
    S3,
}

/// The secret is left out of what is printed.
impl std::fmt::Debug for StorageSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageSettings")
            .field("provider", &self.provider)
            .field("endpoint", &self.endpoint)
            .field("access_key", &self.access_key)
            .field("secret_key", &"<hidden>")
            .field("bucket", &self.bucket)
            .finish()
    }
}

impl StorageSettings {
    pub(super) fn load(config: &Config) -> Result<StorageSettings, SettingError> {
        let provider = match config.text("Storage:Provider") {
            None => Provider::FileSystem,
            Some(name) if name.eq_ignore_ascii_case("FileSystem") => Provider::FileSystem,
            Some(name) if name.eq_ignore_ascii_case("S3") => Provider::S3,
            Some(name) => {
                return Err(SettingError::new(format!(
                    "Storage:Provider must be \"FileSystem\" or \"S3\", but is \"{name}\"."
                )));
            }
        };
        let text = |key: &str| config.text(key).unwrap_or("").to_string();
        let settings = StorageSettings {
            provider,
            endpoint: text("Storage:Endpoint"),
            access_key: text("Storage:AccessKey"),
            secret_key: text("Storage:SecretKey"),
            bucket: text("Storage:Bucket"),
        };
        if provider == Provider::S3 && (settings.endpoint.is_empty() || settings.bucket.is_empty())
        {
            return Err(SettingError::new(
                "Storage:Provider is S3, so Storage:Endpoint and Storage:Bucket are required.",
            ));
        }
        Ok(settings)
    }

    /// The settings as the object store takes them.
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

/// Reads the link preview limits; what is not set keeps the default of the
/// crate that fetches.
pub(super) fn load_embeds(config: &Config) -> Result<EmbedSettings, SettingError> {
    let mut settings = EmbedSettings::default();
    if let Some(enabled) = config.flag("Stellarr:Embeds:Enabled")? {
        settings.enabled = enabled;
    }
    if let Some(allowed) = config.flag("Stellarr:Embeds:AllowPrivateNetworks")? {
        settings.allow_private_networks = allowed;
    }
    let number = |key: &str| config.number(key);
    if let Some(value) = number("Stellarr:Embeds:MaxPerMessage")? {
        settings.max_per_message = usize::try_from(value.max(0)).unwrap_or(usize::MAX);
    }
    if let Some(value) = number("Stellarr:Embeds:TimeoutSeconds")? {
        settings.timeout_seconds = u64::try_from(value.max(0)).unwrap_or(0);
    }
    if let Some(value) = number("Stellarr:Embeds:MaxHtmlBytes")? {
        settings.max_html_bytes = usize::try_from(value.max(0)).unwrap_or(usize::MAX);
    }
    if let Some(value) = number("Stellarr:Embeds:MaxImageBytes")? {
        settings.max_image_bytes = usize::try_from(value.max(0)).unwrap_or(usize::MAX);
    }
    if let Some(value) = number("Stellarr:Embeds:MaxRedirects")? {
        settings.max_redirects = u32::try_from(value.max(0)).unwrap_or(u32::MAX);
    }
    if let Some(agent) = config.text("Stellarr:Embeds:UserAgent") {
        settings.user_agent = agent.to_string();
    }
    Ok(settings)
}
