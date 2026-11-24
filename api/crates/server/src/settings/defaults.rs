//! What an instance starts with when nothing sets a key: the shipped
//! configuration files, one list for every environment and one overlay for
//! each of the two that differ.
//!
//! A key belongs here as soon as some part of the API reads it, whether or not
//! that part is written yet, so that the defaults stay the ones the operator
//! documentation promises.

pub const BASE: &[(&str, &str)] = &[
    (
        "ConnectionStrings:Database",
        "Data Source=data/stellarr.db;Pooling=True",
    ),
    // What the API logs: warnings and errors from everything, and the
    // information of its own code, one line per request among it.
    ("Logging:LogLevel:Default", "Information"),
    ("Logging:LogLevel:Microsoft.AspNetCore", "Warning"),
    (
        "Logging:LogLevel:Microsoft.EntityFrameworkCore.Database.Command",
        "Warning",
    ),
    ("Logging:LogLevel:System.Net.Http.HttpClient", "Warning"),
    ("Uploads:MaxImageBytes", "10485760"),
    ("Uploads:MaxAudioBytes", "26214400"),
    ("Uploads:MaxVideoBytes", "104857600"),
    ("Uploads:MaxFileBytes", "26214400"),
    ("Uploads:MaxStoredBytesPerUser", "2147483648"),
    ("Uploads:MaxStoredBytes", "21474836480"),
    ("Uploads:MinimumFreeDataRootBytes", "1073741824"),
    ("Uploads:MaxConcurrentUploads", "4"),
    ("Mail:Enabled", "false"),
    ("Mail:ResendApiKey", ""),
    ("Mail:FromAddress", ""),
    ("Mail:FromName", "Stellarr"),
    ("Maintenance:Enabled", "true"),
    ("Maintenance:InitialDelayMinutes", "5"),
    ("Maintenance:IntervalHours", "1"),
    ("Maintenance:BatchSize", "1000"),
    ("Maintenance:MessageChangeRetentionDays", "30"),
    ("Desktop:CanonicalOrigin", ""),
    ("Desktop:MinimumVersion", "0.1.0"),
    ("Desktop:RecommendedVersion", "0.1.3"),
    ("Stellarr:Language", "en"),
    ("Stellarr:Embeds:Enabled", "true"),
    ("Stellarr:Embeds:MaxPerMessage", "2"),
    ("Stellarr:Embeds:TimeoutSeconds", "6"),
    ("Stellarr:Embeds:AllowPrivateNetworks", "false"),
];

/// What an installation starts with and a developer's run does not.
pub const PRODUCTION: &[(&str, &str)] = &[("Mail:Enabled", "true"), ("Backups:Enabled", "true")];

/// The development stack: its own database file, and the published
/// development secret for the TURN relay.
pub const DEVELOPMENT: &[(&str, &str)] = &[
    (
        "ConnectionStrings:Database",
        "Data Source=data/stellarr-dev.db;Pooling=True",
    ),
    ("Rtc:PublicHost", "localhost"),
    ("Rtc:TurnSharedSecret", "stellarr-turn-development-secret"),
    ("Mail:Enabled", "false"),
    ("Mail:ResendApiKey", ""),
    ("Mail:FromAddress", "onboarding@resend.dev"),
    ("Mail:FromName", "Stellarr"),
];
