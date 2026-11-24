//! Tests that talk to the real server over HTTP. See `support` for how.

mod support;

// The test modules are listed in `modules.list.rs`, one to a line, so that
// tests added on different branches merge without a conflict.
include!("modules.list.rs");
