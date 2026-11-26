//! End-to-end encryption of direct conversations: the account's keys, its
//! PIN backup and devices, and the switch and epochs of a conversation.

mod account;
mod backup_restore;
mod direct;
mod kit;
mod lifecycle;
pub mod limits;
mod races;
