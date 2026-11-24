//! How link previews are fetched: the section that names outside hosts.

use stellarr_embeds::EmbedSettings;

use super::{Config, SettingError};

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
