//! What the instance needs before it can write to anybody: whether mail is on,
//! who it writes as, where the instance is, and the transport the operator's
//! settings and secrets resolve to.
//!
//! The settings live in two places. What the operator saved in the database
//! (the switch, the sender address, the SMTP server and the choice of
//! provider) wins; the environment gives the starting values, the public
//! address of the instance and the name shown beside the sender. A caller
//! that sends a kind of mail asks [`find`] and, on [`MailSetup::Ready`],
//! builds its message with the sender, the address and the instance's name and
//! hands it to [`crate::mail::MailDelivery`]. Each kind of mail has its own
//! switch among the settings; they are in [`Mailing::switches`].

use rusqlite::{Connection, OptionalExtension};
use stellarr_mail::{MailSettings as StoredSmtp, MailTransport};

use crate::instance::access;
use crate::seal::Sealer;
use crate::secrets;
use crate::settings::Settings;
use crate::values::Id;

/// The operator's switches for the kinds of mail. Without a saved row of
/// settings invitations and recovery are on and alerts are off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Switches {
    pub invitations: bool,
    pub recovery: bool,
    pub alerts: bool,
}

/// Everything a message needs and a transport to send it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mailing {
    /// The `From` of every message: `Name <address>`, or the bare address
    /// when no name is set.
    pub sender: String,
    /// The address of the instance, without a trailing slash.
    pub public_url: String,
    pub instance_name: String,
    pub transport: MailTransport,
    pub switches: Switches,
    /// The instance's time zone as the operator saved it; the name of one
    /// the host may not know.
    pub time_zone: String,
}

/// Whether the instance can send, and with what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailSetup {
    /// The operator has switched mail off.
    Off,
    /// Mail is on and something it needs is missing: the instance server, a
    /// sender address, the public address, an SMTP host or the Resend key.
    NotConfigured,
    Ready(Box<Mailing>),
}

struct Row {
    mailing_enabled: bool,
    from_address: String,
    stored: StoredSmtp,
    switches: Switches,
    time_zone: String,
}

fn read_row(connection: &Connection, server: Id) -> rusqlite::Result<Option<Row>> {
    connection
        .query_row(
            "SELECT \"MailingEnabled\", \"MailFromAddress\", \"MailSmtpHost\", \"MailSmtpPort\", \"MailSmtpUsername\",
                    \"MailSmtpSecurity\", \"MailResendEnabled\", \"InviteMailEnabled\", \"RecoveryMailEnabled\",
                    \"AlertMailEnabled\", \"TimeZone\"
             FROM \"OperationsSettings\" WHERE \"ServerId\" = ?1",
            [server],
            |row| {
                let port: i64 = row.get(3)?;
                Ok(Row {
                    mailing_enabled: row.get(0)?,
                    from_address: row.get(1)?,
                    stored: StoredSmtp {
                        smtp_host: row.get(2)?,
                        // A port outside what a port can be is as good as none.
                        smtp_port: u16::try_from(port).unwrap_or(stellarr_mail::DEFAULT_SMTP_PORT),
                        smtp_username: row.get(4)?,
                        smtp_security: row.get(5)?,
                        resend_enabled: row.get(6)?,
                    },
                    switches: Switches {
                        invitations: row.get(7)?,
                        recovery: row.get(8)?,
                        alerts: row.get(9)?,
                    },
                    time_zone: row.get(10)?,
                })
            },
        )
        .optional()
}

/// The switches as saved, or what an instance that never saved settings has.
pub fn switches(connection: &Connection) -> rusqlite::Result<Switches> {
    let row = match access::instance_server(connection)? {
        Some(server) => read_row(connection, server)?,
        None => None,
    };
    Ok(row.map_or(DEFAULTS, |row| row.switches))
}

const DEFAULTS: Switches = Switches {
    invitations: true,
    recovery: true,
    alerts: false,
};

/// Looks at the operator's settings and secrets and says whether mail can be
/// sent, and how.
pub fn find(
    connection: &Connection,
    settings: &Settings,
    sealer: &Sealer,
) -> rusqlite::Result<MailSetup> {
    let Some(server) = access::instance_server(connection)? else {
        return Ok(MailSetup::NotConfigured);
    };
    let instance_name: String = connection.query_row(
        "SELECT \"Name\" FROM \"Servers\" WHERE \"Id\" = ?1",
        [server],
        |row| row.get(0),
    )?;
    let row = read_row(connection, server)?;
    let enabled = row
        .as_ref()
        .map_or(settings.mail.enabled, |row| row.mailing_enabled);
    // The address the operator saved, else the one the environment gave.
    let from_address = row
        .as_ref()
        .map(|row| row.from_address.trim())
        .filter(|address| !address.is_empty())
        .unwrap_or(settings.mail.from_address.trim());
    let public_url = settings.operations.public_url.clone();
    let resolved = secrets::read(connection, &settings.secrets, sealer)?;
    let transport = MailTransport::resolve(
        row.as_ref().map(|row| &row.stored),
        resolved.resend_api_key().expose(),
        resolved.smtp_password().expose(),
    );
    if !enabled {
        return Ok(MailSetup::Off);
    }
    if from_address.is_empty() || public_url.is_empty() || !transport.ready {
        return Ok(MailSetup::NotConfigured);
    }
    let sender = if settings.mail.from_name.is_empty() {
        from_address.to_string()
    } else {
        format!("{} <{from_address}>", settings.mail.from_name)
    };
    Ok(MailSetup::Ready(Box::new(Mailing {
        sender,
        public_url,
        instance_name,
        transport,
        switches: row.as_ref().map_or(DEFAULTS, |row| row.switches),
        time_zone: row.map_or_else(String::new, |row| row.time_zone),
    })))
}
