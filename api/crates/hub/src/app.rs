use std::future::Future;
use std::sync::Arc;

use serde_json::Value;

use crate::hub::{Hub, Outbound, Shared, close_requested};

/// Why a call failed. Only a deliberate [`HubError::Visible`] message is
/// sent to the client along with its error code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubError {
    /// A message written for the person: the client is told the text.
    Visible(String),
    /// No such method.
    UnknownMethod,
    /// The arguments do not fit the method.
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
        self.shared
            .request_close(stellarr_websocket::status::NORMAL, None);
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

    /// A WebSocket has been upgraded. The connection already exists, so
    /// this is where it joins groups and queues the first events. The client
    /// receives readiness before those events and its calls are read only
    /// after this hook succeeds. An error closes the connection.
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

    /// Whether `target` names a method. Must agree with
    /// [`invoke`](Self::invoke).
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
