//! What the delivery module is held to. The pieces share an instance to
//! arrange rows in and servers of the test's own that stand in for a push
//! service and a mail server.

mod dispatch;
mod outbox;
mod push;
mod support;
