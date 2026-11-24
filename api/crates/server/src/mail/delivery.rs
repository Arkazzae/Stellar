//! Sending one built message by the transport the settings and secrets
//! resolve to: SMTP with the SMTP client, Resend with the outbound HTTP client.
//!
//! What a caller learns is an [`Outcome`]: the message was taken, it was
//! refused in a way another try will not change, or it is worth trying again.
//! Nothing here logs a message, an address, a password or a key.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::{ClientConfig, RootCertStore};
use stellarr_mail::{
    MailTransport, Message, Provider, ResendError, ResendRequest, read_resend_answer,
};
use stellarr_outbound::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use stellarr_outbound::{
    AddressPolicy, Body, Client, Error as HttpError, HeaderName, HeaderValue, Request, Resolve, tls,
};
use stellarr_smtp::{Connection, Options, Security};

/// How long Resend has, from connecting to the last byte of its answer. A
/// request that waits on mail must not be held by a service that hangs.
const RESEND_TIMEOUT: Duration = Duration::from_secs(15);
const RESEND_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// An answer of Resend is a line of JSON; more than this is not one.
const RESEND_ANSWER_LIMIT: u64 = 64 * 1024;
const USER_AGENT_VALUE: &str = "Stellarr/2.0";

/// What became of a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The provider took it; `id` is the provider's name for it.
    Sent { id: String },
    /// The provider will not take this message, whatever the number of tries:
    /// an address it rejects, a login that is wrong, a message that cannot be
    /// written.
    Refused { reason: String },
    /// The provider could not be reached or asked for a later try.
    TryAgain { reason: String },
    /// The operator has switched mail off. Only [`deliver`](super::deliver)
    /// says this; sending by a transport already found does not ask.
    Off,
    /// Mail is on, but something it needs is not set: a sender, the public
    /// address of the instance, an SMTP host or the Resend key.
    NotConfigured,
}

impl Outcome {
    pub fn is_sent(&self) -> bool {
        matches!(self, Outcome::Sent { .. })
    }

    /// Why a message was not taken, for the row that records it.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Outcome::Refused { reason } | Outcome::TryAgain { reason } => Some(reason),
            _ => None,
        }
    }
}

/// Where the trusted roots of SMTP over TLS come from.
enum SmtpTrust {
    /// The system's bundle, read when the first TLS connection needs it, so
    /// that a host without one can still use a relay without TLS, and a
    /// bundle that turns up later is found.
    System(Mutex<Option<Arc<ClientConfig>>>),
    Fixed(Arc<ClientConfig>),
}

struct Inner {
    http: Client,
    resend_url: String,
    resend_policy: AddressPolicy,
    smtp_trust: SmtpTrust,
    smtp_options: Options,
    /// For a connection that does not use TLS: no root is ever asked.
    smtp_plain: Arc<ClientConfig>,
}

/// The delivery service. Cheap to clone; the process has one.
#[derive(Clone)]
pub struct MailDelivery {
    inner: Arc<Inner>,
}

/// Where the provider's side is, for a test that stands in for it.
pub struct Wiring {
    /// The roots both the HTTP client and SMTP over TLS trust; the system's
    /// when none.
    pub roots: Option<RootCertStore>,
    pub resend_url: String,
    /// Which addresses Resend may be at.
    pub resend_policy: AddressPolicy,
    /// How names are resolved for Resend; the system's when none.
    pub resolver: Option<Arc<dyn Resolve>>,
    /// How long a step of the SMTP exchange, and the whole of it, may take;
    /// fifteen seconds when none.
    pub smtp_timeout: Option<Duration>,
}

impl Default for MailDelivery {
    fn default() -> MailDelivery {
        MailDelivery::new()
    }
}

impl MailDelivery {
    /// Trusts the system's certificates; Resend is at its own address, and
    /// only a public one.
    pub fn new() -> MailDelivery {
        MailDelivery::build(
            Client::new(),
            SmtpTrust::System(Mutex::new(None)),
            ResendRequest::URL.to_string(),
            AddressPolicy::PublicOnly,
            Options::default(),
        )
    }

    /// A delivery whose provider is somewhere else, for a test.
    pub fn wired(wiring: Wiring) -> MailDelivery {
        let (mut http, trust) = match wiring.roots {
            Some(roots) => {
                let http = Client::with_tls_config(tls::client_config(roots.clone()));
                (http, SmtpTrust::Fixed(smtp_config(roots)))
            }
            None => (Client::new(), SmtpTrust::System(Mutex::new(None))),
        };
        if let Some(resolver) = wiring.resolver {
            http = http.with_resolver(resolver);
        }
        let mut options = Options::default();
        if let Some(timeout) = wiring.smtp_timeout {
            options.step_timeout = timeout;
            options.total_timeout = timeout;
        }
        MailDelivery::build(
            http,
            trust,
            wiring.resend_url,
            wiring.resend_policy,
            options,
        )
    }

    fn build(
        http: Client,
        smtp_trust: SmtpTrust,
        resend_url: String,
        resend_policy: AddressPolicy,
        smtp_options: Options,
    ) -> MailDelivery {
        MailDelivery {
            inner: Arc::new(Inner {
                http,
                resend_url,
                resend_policy,
                smtp_trust,
                smtp_options,
                smtp_plain: smtp_config(RootCertStore::empty()),
            }),
        }
    }

    /// Sends `message` by `transport`. A transport that is not ready is
    /// [`Outcome::NotConfigured`] and nothing is sent.
    pub async fn send(&self, message: &Message, transport: &MailTransport) -> Outcome {
        if !transport.ready {
            return Outcome::NotConfigured;
        }
        match (transport.provider, &transport.smtp) {
            (Provider::Smtp, Some(connection)) => self.send_smtp(message, connection).await,
            (Provider::Smtp, None) => Outcome::NotConfigured,
            (Provider::Resend, _) => self.send_resend(message, &transport.resend_api_key).await,
        }
    }

    async fn send_smtp(&self, message: &Message, connection: &Connection) -> Outcome {
        let tls = match self.smtp_tls(connection.security).await {
            Ok(config) => config,
            Err(reason) => return Outcome::TryAgain { reason },
        };
        // Fifteen seconds for a step and for the whole exchange, as the first
        // server had.
        match stellarr_smtp::send(connection, tls, &self.inner.smtp_options, message).await {
            Ok(delivery) => Outcome::Sent {
                id: delivery.message_id,
            },
            Err(error) if error.is_transient() => Outcome::TryAgain {
                reason: error.to_string(),
            },
            Err(error) => Outcome::Refused {
                reason: error.to_string(),
            },
        }
    }

    async fn smtp_tls(&self, security: Security) -> Result<Arc<ClientConfig>, String> {
        if security == Security::None {
            return Ok(self.inner.smtp_plain.clone());
        }
        match &self.inner.smtp_trust {
            SmtpTrust::Fixed(config) => Ok(config.clone()),
            SmtpTrust::System(loaded) => {
                if let Some(config) = loaded.lock().unwrap().clone() {
                    return Ok(config);
                }
                // A file of a few hundred kilobytes; read off the executor.
                let roots = tokio::task::spawn_blocking(tls::system_roots)
                    .await
                    .map_err(|_| "reading the trusted certificates was interrupted".to_string())?
                    .map_err(|error| error.to_string())?;
                let config = smtp_config(roots);
                *loaded.lock().unwrap() = Some(config.clone());
                Ok(config)
            }
        }
    }

    async fn send_resend(&self, message: &Message, api_key: &str) -> Outcome {
        let request = match ResendRequest::new(message, api_key) {
            Ok(request) => request,
            Err(ResendError::NoApiKey) => return Outcome::NotConfigured,
            Err(error) => {
                return Outcome::Refused {
                    reason: error.to_string(),
                };
            }
        };
        let refused = |reason: &str| Outcome::Refused {
            reason: reason.to_string(),
        };
        // The key travels in a header that is marked so that no log or debug
        // output of the client shows it.
        let Ok(mut authorization) = HeaderValue::from_str(&request.authorization) else {
            return refused("the Resend API key holds characters a header cannot carry");
        };
        authorization.set_sensitive(true);
        let Ok(idempotency) = HeaderValue::from_str(&request.idempotency_key) else {
            return refused("the message's key is not usable as a header");
        };
        let built = Request::post(&self.inner.resend_url).map(|post| {
            post.header(AUTHORIZATION, authorization)
                .header(HeaderName::from_static("idempotency-key"), idempotency)
                .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                .header(ACCEPT, HeaderValue::from_static("application/json"))
                .header(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE))
                .body(Body::bytes(request.json.into_bytes()))
                .connect_timeout(RESEND_CONNECT_TIMEOUT)
                .timeout(RESEND_TIMEOUT)
                .max_response_bytes(RESEND_ANSWER_LIMIT)
                .policy(self.inner.resend_policy)
        });
        let post = match built {
            Ok(post) => post,
            Err(error) => {
                return Outcome::Refused {
                    reason: error.to_string(),
                };
            }
        };
        let answer = match self.inner.http.send_collected(post).await {
            Ok(answer) => answer,
            Err(error) => {
                let reason = format!("Resend could not be reached: {error}");
                return if is_permanent(&error) {
                    Outcome::Refused { reason }
                } else {
                    Outcome::TryAgain { reason }
                };
            }
        };
        let status = answer.status.as_u16();
        match read_resend_answer(status, &answer.body) {
            Ok(id) => Outcome::Sent { id },
            Err(error @ ResendError::Rejected { .. }) => {
                let reason = error.to_string();
                // Asked to slow down, asked to come back, or the service is
                // down: another try can work. Anything else is the service
                // saying no to this message or this key.
                if matches!(status, 408 | 409 | 425 | 429) || status >= 500 {
                    Outcome::TryAgain { reason }
                } else {
                    Outcome::Refused { reason }
                }
            }
            // A success that names no message: the service may have it.
            // The idempotency key keeps a second try from sending it twice.
            Err(error @ ResendError::NoMessageId) => Outcome::TryAgain {
                reason: error.to_string(),
            },
            Err(ResendError::NoApiKey) => Outcome::NotConfigured,
            Err(error) => Outcome::Refused {
                reason: error.to_string(),
            },
        }
    }
}

/// Whether another try at the same request would only be refused again: the
/// address policy or the request itself, not the network.
fn is_permanent(error: &HttpError) -> bool {
    matches!(
        error,
        HttpError::InvalidUrl(_) | HttpError::ForbiddenHeader(_) | HttpError::AddressRefused { .. }
    )
}

/// The TLS configuration of SMTP: the roots given, no client certificate and
/// no application protocol (an SMTP server has none to choose).
fn smtp_config(roots: RootCertStore) -> Arc<ClientConfig> {
    Arc::new(
        ClientConfig::builder_with_provider(
            Arc::new(rustls::crypto::aws_lc_rs::default_provider()),
        )
        .with_safe_default_protocol_versions()
        .expect("the provider supports the default protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth(),
    )
}
