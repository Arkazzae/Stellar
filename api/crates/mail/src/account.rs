//! The messages the instance sends about an account or about itself.

use stellarr_messages::{Language, MessageKey, Messages};
use stellarr_timezone::TimeZone;

use crate::device::describe_device;
use crate::error::Error;
use crate::html::origin_of;
use crate::layout::MailLayout;
use crate::transport::{MailTransport, Provider};
use crate::{guid, signature};
use stellarr_smtp::Message;

/// The kinds of security event an alert can be about.
pub mod event_kinds {
    pub const LOGIN_SUCCEEDED: &str = "login-succeeded";
    pub const LOGIN_FAILED: &str = "login-failed";
    pub const MFA_SUCCEEDED: &str = "mfa-succeeded";
    pub const MFA_FAILED: &str = "mfa-failed";
    pub const METHOD_ADDED: &str = "method-added";
    pub const METHOD_REMOVED: &str = "method-removed";
    pub const RECOVERY_CODES_REGENERATED: &str = "recovery-codes-regenerated";
    pub const RECOVERY_CODE_USED: &str = "recovery-code-used";
    pub const TRUSTED_DEVICE_ADDED: &str = "trusted-device-added";
    pub const TRUSTED_DEVICE_REMOVED: &str = "trusted-device-removed";
    pub const SESSION_REVOKED: &str = "session-revoked";
    pub const PASSWORD_CHANGED: &str = "password-changed";
    pub const CLI_RECOVERY: &str = "cli-recovery";
    pub const POLICY_CHANGED: &str = "policy-changed";
    pub const DESKTOP_APPROVED: &str = "desktop-approved";
    pub const IDENTITY_LINKED: &str = "identity-linked";
    pub const IDENTITY_UNLINKED: &str = "identity-unlinked";
    pub const EMAIL_CHANGED: &str = "email-changed";
}

/// What an alert tells about an event.
#[derive(Debug, Clone)]
pub struct SecurityEvent<'a> {
    /// The 128 bits of the event's id.
    pub id: u128,
    /// One of [`event_kinds`], or any other text, which is shown as it is.
    pub kind: &'a str,
    /// Empty when unknown.
    pub ip_address: &'a str,
    /// The user-agent string; empty when unknown.
    pub device: &'a str,
    /// When it happened, seconds since 1970 in UTC.
    pub created_at: i64,
}

/// A link to set a new password, sent to the address of the account.
#[allow(clippy::too_many_arguments)]
pub fn password_reset(
    from: &str,
    to: &str,
    display_name: &str,
    instance_name: &str,
    reset_url: &str,
    token_id: u128,
    messages: &Messages,
) -> Result<Message, Error> {
    let name = [("name", &display_name as &(dyn std::fmt::Display + Sync))];
    let instance = [(
        "instance",
        &instance_name as &(dyn std::fmt::Display + Sync),
    )];
    let intro = messages.get(MessageKey::PasswordResetIntro, &name)?;
    let ignore = messages.get(MessageKey::PasswordResetIgnore, &[])?;
    let origin = origin_of(reset_url);
    let text = lines(&[
        &messages.get(MessageKey::PasswordResetGreeting, &name)?,
        "",
        &messages.get(MessageKey::PasswordResetInstruction, &instance)?,
        reset_url,
        "",
        &messages.get(MessageKey::PasswordResetValidity, &[])?,
        &signature(instance_name, &origin, messages)?,
    ]);
    let subject = messages.get(MessageKey::PasswordResetSubject, &instance)?;

    let layout = MailLayout::new(messages.language().code())
        .instance(instance_name)
        .preheader(&intro)
        .heading(&messages.get(MessageKey::PasswordResetHeading, &[])?)
        .paragraph(&intro)
        .button(
            reset_url,
            &messages.get(MessageKey::PasswordResetButton, &[])?,
        )
        .link_fallback(reset_url, &messages.get(MessageKey::MailLinkFallback, &[])?)
        .divider()
        .note(&ignore)
        .footer(&messages.get(MessageKey::MailSentFromInstance, &instance)?)
        .footer_link(&origin);
    crate::assemble(
        from,
        to,
        subject,
        text,
        layout,
        format!("password-reset/{}", guid(token_id)),
    )
}

/// Sent to the new address of an account, which proves it receives mail by
/// opening the link.
#[allow(clippy::too_many_arguments)]
pub fn email_change(
    from: &str,
    to: &str,
    display_name: &str,
    instance_name: &str,
    confirm_url: &str,
    request_id: u128,
    messages: &Messages,
) -> Result<Message, Error> {
    let name = [("name", &display_name as &(dyn std::fmt::Display + Sync))];
    let instance = [(
        "instance",
        &instance_name as &(dyn std::fmt::Display + Sync),
    )];
    let intro = messages.get(MessageKey::EmailChangeIntro, &name)?;
    let origin = origin_of(confirm_url);
    let text = lines(&[
        &messages.get(MessageKey::EmailChangeGreeting, &name)?,
        "",
        &messages.get(MessageKey::EmailChangeInstruction, &instance)?,
        confirm_url,
        "",
        &messages.get(MessageKey::EmailChangeValidity, &[])?,
        &signature(instance_name, &origin, messages)?,
    ]);
    let subject = messages.get(MessageKey::EmailChangeSubject, &instance)?;

    let layout = MailLayout::new(messages.language().code())
        .instance(instance_name)
        .preheader(&intro)
        .heading(&messages.get(MessageKey::EmailChangeHeading, &[])?)
        .paragraph(&intro)
        .button(
            confirm_url,
            &messages.get(MessageKey::EmailChangeButton, &[])?,
        )
        .link_fallback(
            confirm_url,
            &messages.get(MessageKey::MailLinkFallback, &[])?,
        )
        .divider()
        .note(&messages.get(MessageKey::EmailChangeIgnore, &[])?)
        .footer(&messages.get(MessageKey::MailSentFromInstance, &instance)?)
        .footer_link(&origin);
    crate::assemble(
        from,
        to,
        subject,
        text,
        layout,
        format!("email-change/{}", guid(request_id)),
    )
}

/// What the operator panel sends to check that the provider accepts mail. It
/// names the way it went, so a test that arrives also says which settings
/// sent it. `test_id` makes the message its own: a fresh random id each time.
pub fn delivery_test(
    from: &str,
    to: &str,
    instance_name: &str,
    public_url: &str,
    transport: &MailTransport,
    test_id: u128,
    messages: &Messages,
) -> Result<Message, Error> {
    let instance = [(
        "instance",
        &instance_name as &(dyn std::fmt::Display + Sync),
    )];
    let body = messages.get(MessageKey::DeliveryTestBody, &instance)?;
    let route = match (&transport.provider, &transport.smtp) {
        (Provider::Smtp, Some(smtp)) => {
            let server = format!("{}:{}", smtp.host, smtp.port);
            messages.get(MessageKey::DeliveryTestSmtp, &[("server", &server)])?
        }
        _ => messages.get(MessageKey::DeliveryTestResend, &[])?,
    };
    let route_label = messages.get(MessageKey::DeliveryTestTransportLabel, &[])?;
    let text = lines(&[
        &body,
        "",
        &format!("{route_label}: {route}"),
        &signature(instance_name, public_url, messages)?,
    ]);
    let subject = messages.get(MessageKey::DeliveryTestSubject, &instance)?;

    let layout = MailLayout::new(messages.language().code())
        .instance(instance_name)
        .preheader(&body)
        .heading(&messages.get(MessageKey::DeliveryTestHeading, &[])?)
        .paragraph(&body)
        .details(&[(route_label, route)])
        .footer(&messages.get(MessageKey::MailSentFromInstance, &instance)?)
        .footer_link(public_url);
    crate::assemble(
        from,
        to,
        subject,
        text,
        layout,
        format!("delivery-test/{}", guid(test_id)),
    )
}

/// A one-time code for the second step or for setting up mail as one.
#[allow(clippy::too_many_arguments)]
pub fn email_code(
    from: &str,
    to: &str,
    display_name: &str,
    instance_name: &str,
    public_url: &str,
    code: &str,
    valid_minutes: u32,
    code_id: u128,
    messages: &Messages,
) -> Result<Message, Error> {
    let name = [("name", &display_name as &(dyn std::fmt::Display + Sync))];
    let instance = [(
        "instance",
        &instance_name as &(dyn std::fmt::Display + Sync),
    )];
    let minutes = [("minutes", &valid_minutes as &(dyn std::fmt::Display + Sync))];
    let intro = messages.get(
        MessageKey::EmailCodeIntro,
        &[("name", &display_name), ("minutes", &valid_minutes)],
    )?;
    let ignore = messages.get(MessageKey::SecurityAlertIgnore, &[])?;
    let text = lines(&[
        &messages.get(MessageKey::SecurityAlertGreeting, &name)?,
        "",
        &messages.get(MessageKey::EmailCodeInstruction, &minutes)?,
        code,
        "",
        &ignore,
        &signature(instance_name, public_url, messages)?,
    ]);
    let subject = messages.get(MessageKey::EmailCodeSubject, &instance)?;

    let layout = MailLayout::new(messages.language().code())
        .instance(instance_name)
        .preheader(&intro)
        .heading(&messages.get(MessageKey::EmailCodeHeading, &[])?)
        .paragraph(&intro)
        .code(code)
        .note(&ignore)
        .footer(&messages.get(MessageKey::MailSentFromInstance, &instance)?)
        .footer_link(public_url);
    crate::assemble(
        from,
        to,
        subject,
        text,
        layout,
        format!("email-code/{}", guid(code_id)),
    )
}

/// Tells the owner of an account what just happened to it, when and from
/// where, in the instance's time zone.
#[allow(clippy::too_many_arguments)]
pub fn security_alert(
    from: &str,
    to: &str,
    display_name: &str,
    instance_name: &str,
    public_url: &str,
    event: &SecurityEvent<'_>,
    time_zone: &TimeZone,
    messages: &Messages,
) -> Result<Message, Error> {
    let lead = messages.get(
        MessageKey::SecurityAlertLead,
        &[("name", &display_name), ("instance", &instance_name)],
    )?;
    let activity = describe_event(event.kind, messages)?;
    let time = format_time(event.created_at, time_zone, messages);
    let ignore = messages.get(MessageKey::SecurityAlertIgnore, &[])?;

    let mut facts = vec![
        (
            messages.get(MessageKey::SecurityAlertActivityLabel, &[])?,
            activity.clone(),
        ),
        (
            messages.get(MessageKey::SecurityAlertTimeLabel, &[])?,
            time.clone(),
        ),
    ];
    if let Some(device) = describe_device(event.device, messages)? {
        facts.push((
            messages.get(MessageKey::SecurityAlertDeviceLabel, &[])?,
            device,
        ));
    }
    if !event.ip_address.is_empty() {
        facts.push((
            messages.get(MessageKey::SecurityAlertAddressLabel, &[])?,
            event.ip_address.to_owned(),
        ));
    }

    let mut text_lines = vec![lead.clone(), String::new()];
    text_lines.extend(
        facts
            .iter()
            .map(|(label, value)| format!("{label}: {value}")),
    );
    text_lines.push(String::new());
    text_lines.push(ignore.clone());
    text_lines.push(signature(instance_name, public_url, messages)?);
    let text = text_lines.join("\n");

    let instance = [(
        "instance",
        &instance_name as &(dyn std::fmt::Display + Sync),
    )];
    let subject = messages.get(MessageKey::SecurityAlertSubject, &instance)?;
    let open = format!("{}/", public_url.trim_end_matches('/'));
    let layout = MailLayout::new(messages.language().code())
        .instance(instance_name)
        .preheader(&format!("{activity} · {time}"))
        .heading(&messages.get(MessageKey::SecurityAlertHeading, &[])?)
        .paragraph(&lead)
        .details(&facts)
        .paragraph(&ignore)
        .button(
            &open,
            &messages.get(MessageKey::NotificationOpenButton, &[])?,
        )
        .footer(&messages.get(MessageKey::MailSentFromInstance, &instance)?)
        .footer_link(public_url);
    crate::assemble(
        from,
        to,
        subject,
        text,
        layout,
        format!("security-event/{}", guid(event.id)),
    )
}

/// The name of an event as the account's activity list shows it.
pub fn describe_event(kind: &str, messages: &Messages) -> Result<String, Error> {
    use event_kinds::*;
    let key = match kind {
        LOGIN_SUCCEEDED => MessageKey::SecurityEventLoginSucceeded,
        LOGIN_FAILED => MessageKey::SecurityEventLoginFailed,
        MFA_SUCCEEDED => MessageKey::SecurityEventMfaSucceeded,
        MFA_FAILED => MessageKey::SecurityEventMfaFailed,
        METHOD_ADDED => MessageKey::SecurityEventMethodAdded,
        METHOD_REMOVED => MessageKey::SecurityEventMethodRemoved,
        RECOVERY_CODES_REGENERATED => MessageKey::SecurityEventRecoveryCodesRegenerated,
        RECOVERY_CODE_USED => MessageKey::SecurityEventRecoveryCodeUsed,
        TRUSTED_DEVICE_ADDED => MessageKey::SecurityEventTrustedDeviceAdded,
        TRUSTED_DEVICE_REMOVED => MessageKey::SecurityEventTrustedDeviceRemoved,
        SESSION_REVOKED => MessageKey::SecurityEventSessionRevoked,
        PASSWORD_CHANGED => MessageKey::SecurityEventPasswordChanged,
        CLI_RECOVERY => MessageKey::SecurityEventCliRecovery,
        POLICY_CHANGED => MessageKey::SecurityEventPolicyChanged,
        DESKTOP_APPROVED => MessageKey::SecurityEventDesktopApproved,
        IDENTITY_LINKED => MessageKey::SecurityEventIdentityLinked,
        IDENTITY_UNLINKED => MessageKey::SecurityEventIdentityUnlinked,
        EMAIL_CHANGED => MessageKey::SecurityEventEmailChanged,
        _ => return Ok(messages.get(MessageKey::SecurityEventOther, &[("kind", &kind)])?),
    };
    Ok(messages.get(key, &[])?)
}

const POLISH_MONTHS: [&str; 12] = [
    "stycznia",
    "lutego",
    "marca",
    "kwietnia",
    "maja",
    "czerwca",
    "lipca",
    "sierpnia",
    "września",
    "października",
    "listopada",
    "grudnia",
];
const ENGLISH_MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// "20 listopada 2026, 14:03 (Europe/Warsaw)": the date in the instance's
/// language, the zone named.
fn format_time(moment: i64, zone: &TimeZone, messages: &Messages) -> String {
    let local = zone.local_time(moment);
    let months = match messages.language() {
        Language::Polish => &POLISH_MONTHS,
        Language::English => &ENGLISH_MONTHS,
    };
    format!(
        "{} {} {:04}, {:02}:{:02} ({})",
        local.day,
        months[(local.month - 1) as usize],
        local.year,
        local.hour,
        local.minute,
        zone.id()
    )
}

fn lines(lines: &[&str]) -> String {
    lines.join("\n")
}
