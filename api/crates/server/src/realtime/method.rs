//! How a group adds to what a connection can do: methods the client calls,
//! and work to do when a connection starts or ends.
//!
//! A group describes what it adds in one function and writes the path of its
//! module on a line of `methods.list.rs`, the one line it shares with the
//! others; the file merges by itself, as the list of groups does.
//!
//! ```ignore
//! // src/channels/hub.rs
//! pub fn registration() -> Registration {
//!     Registration::new()
//!         .method(Method::new("SendTyping", |cx, arguments| async move {
//!             let [channel] = arguments::exactly(arguments)?;
//!             // ...
//!             Ok(Value::Null)
//!         }))
//!         .on_disconnected(|cx, _error| async move { /* free what the connection held */ })
//! }
//! ```
//!
//! ```text
//! // src/realtime/methods.list.rs
//! methods! {
//!     crate::channels::hub,
//! }
//! ```
//!
//! A call that names a method nobody has registered is answered with the
//! completion error of an unknown method. Names are compared exactly, as the
//! web client writes them.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use stellarr_hub::{Call, Hub, HubError, Outbound};

use super::Realtime;
use crate::state::AppState;
use crate::values::Id;

/// Who a connection belongs to, as the hub let it in.
#[derive(Debug, Clone)]
pub struct Who {
    pub user: Id,
    pub session: Id,
    /// The account's session version the connection was let in with.
    pub version: i64,
    /// The server the client asked to follow, as it wrote it in the address.
    pub requested_server: Option<String>,
}

/// What a method knows about the call it serves.
#[derive(Clone)]
pub struct Context {
    state: AppState,
    realtime: Arc<Realtime>,
    call: Call<Who>,
}

impl Context {
    pub(super) fn new(state: AppState, realtime: Arc<Realtime>, call: Call<Who>) -> Context {
        Context {
            state,
            realtime,
            call,
        }
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn realtime(&self) -> &Arc<Realtime> {
        &self.realtime
    }

    pub fn hub(&self) -> &Hub {
        self.realtime.hub()
    }

    pub fn user(&self) -> Id {
        self.call.identity().user
    }

    pub fn session(&self) -> Id {
        self.call.identity().session
    }

    pub fn connection_id(&self) -> &str {
        self.call.connection_id()
    }

    pub fn who(&self) -> &Who {
        self.call.identity()
    }

    /// Tells the caller's own connection something. False when it is gone.
    pub fn send_to_caller<T: Serialize + ?Sized>(&self, event: &str, payload: &T) -> bool {
        match Outbound::single(event, payload) {
            Ok(message) => self.call.send(&message),
            Err(error) => {
                tracing::error!(%error, event, "an event could not be written");
                false
            }
        }
    }
}

pub type MethodFuture = Pin<Box<dyn Future<Output = Result<Value, HubError>> + Send>>;

type Run = Arc<dyn Fn(Context, Vec<Value>) -> MethodFuture + Send + Sync>;
type ConnectHook = Arc<
    dyn Fn(Context) -> Pin<Box<dyn Future<Output = Result<(), HubError>> + Send>> + Send + Sync,
>;
type DisconnectHook =
    Arc<dyn Fn(Context, Option<String>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// One method a client may call.
#[derive(Clone)]
pub struct Method {
    name: &'static str,
    run: Run,
}

impl Method {
    /// `run` gets the call and the arguments the client sent, and answers with
    /// what the client receives (`Value::Null` for a method with no result) or
    /// the error it is told. See [`arguments`] for taking them apart.
    pub fn new<F, Fut>(name: &'static str, run: F) -> Method
    where
        F: Fn(Context, Vec<Value>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value, HubError>> + Send + 'static,
    {
        Method {
            name,
            run: Arc::new(move |context, arguments| Box::pin(run(context, arguments))),
        }
    }
}

/// What one group adds to the hub.
#[derive(Clone, Default)]
pub struct Registration {
    methods: Vec<Method>,
    connected: Vec<ConnectHook>,
    disconnected: Vec<DisconnectHook>,
}

impl Registration {
    pub fn new() -> Registration {
        Registration::default()
    }

    pub fn method(mut self, method: Method) -> Registration {
        self.methods.push(method);
        self
    }

    /// Work for a connection that has just been let in, after the hub has put
    /// it in its groups and told it its own status. An error ends the
    /// connection.
    pub fn on_connected<F, Fut>(mut self, hook: F) -> Registration
    where
        F: Fn(Context) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), HubError>> + Send + 'static,
    {
        self.connected
            .push(Arc::new(move |context| Box::pin(hook(context))));
        self
    }

    /// Work for a connection that has ended, before the account is told to be
    /// offline when this was its last one: voice seats are freed here. `error`
    /// says why when it did not end in an orderly way.
    pub fn on_disconnected<F, Fut>(mut self, hook: F) -> Registration
    where
        F: Fn(Context, Option<String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.disconnected.push(Arc::new(move |context, error| {
            Box::pin(hook(context, error))
        }));
        self
    }
}

/// Every registration of the process, joined.
#[derive(Default)]
pub struct Table {
    methods: HashMap<String, Method>,
    connected: Vec<ConnectHook>,
    disconnected: Vec<DisconnectHook>,
}

impl Table {
    /// Joins registrations. Two methods of one name are a mistake of the code
    /// that registered them, found when the server starts.
    pub fn new(registrations: Vec<Registration>) -> Table {
        let mut table = Table::default();
        for registration in registrations {
            for method in registration.methods {
                let key = method.name.to_string();
                assert!(
                    !table.methods.contains_key(&key),
                    "the hub method {} is registered twice",
                    method.name
                );
                table.methods.insert(key, method);
            }
            table.connected.extend(registration.connected);
            table.disconnected.extend(registration.disconnected);
        }
        table
    }

    pub fn has_method(&self, name: &str) -> bool {
        self.methods.contains_key(name)
    }

    pub(super) async fn invoke(
        &self,
        context: Context,
        name: &str,
        arguments: Vec<Value>,
    ) -> Result<Value, HubError> {
        match self.methods.get(name) {
            Some(method) => (method.run)(context, arguments).await,
            None => Err(HubError::UnknownMethod),
        }
    }

    pub(super) async fn run_connected(&self, context: &Context) -> Result<(), HubError> {
        for hook in &self.connected {
            hook(context.clone()).await?;
        }
        Ok(())
    }

    pub(super) async fn run_disconnected(&self, context: &Context, error: Option<&str>) {
        for hook in &self.disconnected {
            hook(context.clone(), error.map(str::to_owned)).await;
        }
    }
}

/// Taking the arguments of a call apart the way the client library's server
/// does: a call with too few or too many, or one of the wrong kind, fails as
/// a call whose arguments do not fit.
pub mod arguments {
    use serde_json::Value;
    use stellarr_hub::HubError;

    /// Exactly `N` arguments.
    pub fn exactly<const N: usize>(arguments: Vec<Value>) -> Result<[Value; N], HubError> {
        arguments
            .try_into()
            .map_err(|_: Vec<Value>| HubError::InvalidArguments)
    }

    /// A text, or nothing where the client sent `null`.
    pub fn text(value: Value) -> Result<Option<String>, HubError> {
        match value {
            Value::String(text) => Ok(Some(text)),
            Value::Null => Ok(None),
            _ => Err(HubError::InvalidArguments),
        }
    }
}
