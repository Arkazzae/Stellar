//! What the instance is made of: its accounts, its servers and who may see
//! what on them, as far as signing in and registering need to read and write
//! them. The groups of endpoints that manage servers and roles own the rest.

pub mod access;
pub mod invites;
pub mod names;
pub mod starter;
pub mod users;
pub mod welcome;
