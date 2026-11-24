//! How a group of endpoints describes its routes.
//!
//! ```ignore
//! pub fn routes() -> Endpoints {
//!     Endpoints::new()
//!         .get("/api/things", list_things)
//!         .post("/api/things", create_thing).json_body().rate_limit(RateLimitPolicy::AuthWrite)
//! }
//! ```
//!
//! `.json_body()`, `.authorized()`, `.fresh_second_step()` and `.rate_limit(..)` apply to the route added just before
//! them. They are data the pipeline reads, not layers of the route: the limit
//! has to count a request before the client gate looks at it, which a layer
//! around the handler could not do.
//!
//! Only the methods named are served: a `GET` route does not answer `HEAD`,
//! it is a 405 with an `Allow` header.

use std::collections::HashMap;

use axum::Router;
use axum::handler::Handler;
use axum::http::Method;
use axum::routing::{MethodFilter, on};

use super::rate_limit::RateLimitPolicy;
use crate::state::AppState;

/// How much of a JSON body a route reads unless it asks for more: 1 MiB. A
/// route that needs no session, such as signing in, would otherwise let anybody
/// make the server hold 128 MiB.
pub const DEFAULT_JSON_BODY_BYTES: usize = 1024 * 1024;

/// What the pipeline does around one route.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rules {
    /// The limit the route counts its requests against.
    pub rate_limit: Option<RateLimitPolicy>,
    /// The route reads a JSON body: a request that names another content type
    /// is answered 415 before the limit counts it.
    pub json_body: bool,
    /// The most a JSON body of the route may be; none means
    /// [`DEFAULT_JSON_BODY_BYTES`].
    pub body_limit: Option<usize>,
    /// The route needs a signed-in person: a request without a valid session
    /// is answered 401 with no body.
    pub authorize: bool,
    /// The route needs a session that passed a second step lately, when the
    /// account has one (see `auth::fresh`): a session that did not is answered
    /// 403 with a problem the clients answer by asking for the step.
    pub fresh_second_step: bool,
}

/// The routes of one group, with the rules of each.
pub struct Endpoints {
    router: Router<AppState>,
    rules: HashMap<(Method, String), Rules>,
    last: Option<(Method, String)>,
}

impl Default for Endpoints {
    fn default() -> Endpoints {
        Endpoints::new()
    }
}

impl Endpoints {
    pub fn new() -> Endpoints {
        Endpoints {
            router: Router::new(),
            rules: HashMap::new(),
            last: None,
        }
    }

    pub fn get<H, T>(self, path: &str, handler: H) -> Endpoints
    where
        H: Handler<T, AppState>,
        T: 'static,
    {
        self.add(Method::GET, MethodFilter::GET, path, handler)
    }

    pub fn post<H, T>(self, path: &str, handler: H) -> Endpoints
    where
        H: Handler<T, AppState>,
        T: 'static,
    {
        self.add(Method::POST, MethodFilter::POST, path, handler)
    }

    pub fn put<H, T>(self, path: &str, handler: H) -> Endpoints
    where
        H: Handler<T, AppState>,
        T: 'static,
    {
        self.add(Method::PUT, MethodFilter::PUT, path, handler)
    }

    pub fn patch<H, T>(self, path: &str, handler: H) -> Endpoints
    where
        H: Handler<T, AppState>,
        T: 'static,
    {
        self.add(Method::PATCH, MethodFilter::PATCH, path, handler)
    }

    pub fn delete<H, T>(self, path: &str, handler: H) -> Endpoints
    where
        H: Handler<T, AppState>,
        T: 'static,
    {
        self.add(Method::DELETE, MethodFilter::DELETE, path, handler)
    }

    fn add<H, T>(
        mut self,
        method: Method,
        filter: MethodFilter,
        path: &str,
        handler: H,
    ) -> Endpoints
    where
        H: Handler<T, AppState>,
        T: 'static,
    {
        self.router = self.router.route(path, on(filter, handler));
        let key = (method, path.to_string());
        self.rules.entry(key.clone()).or_default();
        self.last = Some(key);
        self
    }

    /// Serves everything under `path` (and `path` itself) with a router that
    /// does its own routing, such as the hub's. The pipeline wraps it like any
    /// route; the router decides who may enter, as it has no rules here.
    pub fn service(mut self, path: &str, router: Router) -> Endpoints {
        self.router = self.router.nest_service(path, router);
        self.last = None;
        self
    }

    /// Counts the requests of the route added last against `policy`.
    pub fn rate_limit(mut self, policy: RateLimitPolicy) -> Endpoints {
        self.rules_of_last().rate_limit = Some(policy);
        self
    }

    /// Marks the route added last as one that needs a signed-in person.
    pub fn authorized(mut self) -> Endpoints {
        self.rules_of_last().authorize = true;
        self
    }

    /// Marks the route added last as one that needs a signed-in person who
    /// passed a second step in the last few minutes, if their account has one.
    /// It needs a session, as `.authorized()` does.
    pub fn fresh_second_step(mut self) -> Endpoints {
        let rules = self.rules_of_last();
        rules.authorize = true;
        rules.fresh_second_step = true;
        self
    }

    /// Lets the JSON body of the route added last be up to `bytes`, where the
    /// ordinary limit is [`DEFAULT_JSON_BODY_BYTES`]; never more than the
    /// ceiling of the whole server.
    pub fn json_body_up_to(mut self, bytes: usize) -> Endpoints {
        let rules = self.rules_of_last();
        rules.json_body = true;
        rules.body_limit = Some(bytes.min(crate::http::MAX_REQUEST_BODY_BYTES));
        self
    }

    /// Marks the route added last as one that reads a JSON body.
    pub fn json_body(mut self) -> Endpoints {
        self.rules_of_last().json_body = true;
        self
    }

    fn rules_of_last(&mut self) -> &mut Rules {
        let key = self
            .last
            .clone()
            .expect("rules apply to a route, so one must be added first");
        self.rules.entry(key).or_default()
    }

    /// Joins the routes of two groups.
    pub fn merge(mut self, other: Endpoints) -> Endpoints {
        self.router = self.router.merge(other.router);
        self.rules.extend(other.rules);
        self.last = None;
        self
    }

    pub(crate) fn into_parts(self) -> (Router<AppState>, HashMap<(Method, String), Rules>) {
        (self.router, self.rules)
    }
}
