use std::future::Future;
use std::sync::Arc;

use serde_json::Value;

use crate::hub::{CloseRequest, Hub, Outbound, Shared, close_requested};

/// Why a call failed, and so what the client is told. The client library
/// raises the text as an error in the caller, so what it shows matters: only
/// a deliberate [`HubError::Visible`] text reaches it, anything else is
/// hidden behind a general sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubError {
    /// A message written for the person: the client is told the text.
    Visible(String),
    /// No such method: the name is matched exactly.
    UnknownMethod,
    /// The arguments do not fit the method: too few, too many, or of the
    /// wrong kind. The client is told the call failed, not why.
    InvalidArguments,
    /// Anything else that went wrong. The text is for the log only.
    Internal(String),
}

impl HubError {
    pub fn visible(text: impl Into<String>) -> Self {
        Self::Visible(text.into())
    }

    pub fn internal(text: impl Into<String>) -> Self {
        Self::Internal(text.into())
    }

    /// What the completion carries as its error for a call to `target`.
    pub fn client_text(&self, target: &str) -> String {
        match self {
            Self::Visible(text) => {
                format!(
                    "An unexpected error occurred invoking '{target}' on the server. HubException: {text}"
                )
            }
            Self::UnknownMethod => {
                format!("Unknown method '{target}'.")
            }
            Self::InvalidArguments => {
                format!("Failed to invoke '{target}' due to an error on the server.")
            }
            Self::Internal(_) => {
                format!("An unexpected error occurred invoking '{target}' on the server.")
            }
        }
    }
}

/// What a method of the application knows about the call it is serving.
pub struct Call<I> {
    connection_id: Arc<str>,
    identity: Arc<I>,
    hub: Hub,
    shared: Arc<Shared>,
}

impl<I> Clone for Call<I> {
    fn clone(&self) -> Self {
        Self {
            connection_id: self.connection_id.clone(),
            identity: self.identity.clone(),
            hub: self.hub.clone(),
            shared: self.shared.clone(),
        }
    }
}

impl<I> Call<I> {
    pub(crate) fn new(
        connection_id: Arc<str>,
        identity: Arc<I>,
        hub: Hub,
        shared: Arc<Shared>,
    ) -> Self {
        Self {
            connection_id,
            identity,
            hub,
            shared,
        }
    }

    /// The id of the connection the call came in on.
    pub fn connection_id(&self) -> &str {
        &self.connection_id
    }

    /// Who is connected, as the function that let the connection in said.
    pub fn identity(&self) -> &I {
        &self.identity
    }

    pub fn hub(&self) -> &Hub {
        &self.hub
    }

    /// Sends to the caller's own connection.
    pub fn send(&self, message: &Outbound) -> bool {
        self.hub.send_to_connection(&self.connection_id, message)
    }

    /// Ends the caller's connection without letting it start over.
    pub fn abort(&self) {
        self.shared.request_close(CloseRequest {
            error: None,
            allow_reconnect: false,
        });
    }

    /// Completes when the connection is ending, for work that should stop
    /// with it.
    pub async fn aborted(&self) {
        let mut closing = self.shared.subscribe_close();
        close_requested(&mut closing).await;
    }
}

/// The application behind the hub. One value serves every connection, and
/// the hub calls it from many tasks at once; calls from the same connection
/// come one at a time, in the order they were sent.
pub trait Application: Send + Sync + 'static {
    /// Who a connection belongs to; made by the function that admits it.
    type Identity: Send + Sync + 'static;

    /// A client has finished its handshake. The connection already exists,
    /// so this is where it joins groups and where the first messages for it
    /// are sent. Nothing the client sends is read until this returns. An
    /// error closes the connection.
    fn connected(
        &self,
        call: &Call<Self::Identity>,
    ) -> impl Future<Output = Result<(), HubError>> + Send;

    /// The connection is gone. `error` says why when it was not an orderly
    /// close. The connection is no longer in any group, and sending to it
    /// does nothing.
    fn disconnected(
        &self,
        call: &Call<Self::Identity>,
        error: Option<&str>,
    ) -> impl Future<Output = ()> + Send;

    /// Whether `target` names a method. Names are compared without regard to
    /// case. This must agree with [`invoke`](Self::invoke), which answers an
    /// unknown method with [`HubError::UnknownMethod`].
    fn has_method(&self, target: &str) -> bool;

    /// Runs a method. `Ok(Value::Null)` is the answer of a method with no
    /// result.
    fn invoke(
        &self,
        call: &Call<Self::Identity>,
        target: &str,
        arguments: Vec<Value>,
    ) -> impl Future<Output = Result<Value, HubError>> + Send;
}
