//! Stellarr Init: the process manager that runs as PID 1 in the Stellarr
//! container.
//!
//! [`service`] describes a service and checks a table of them; [`services`]
//! is the table compiled into the binary. [`core`] makes every decision, free
//! of processes and clocks. [`manager`] runs it: it spawns and reaps the
//! processes, checks readiness through [`probe`], and answers the control
//! socket of [`control`], whose wire format is [`protocol`]. [`client`] is the
//! other end of that socket, behind the subcommands. All `unsafe` code is in
//! [`sys`].

pub mod accounts;
pub mod client;
pub mod control;
pub mod core;
pub mod log;
pub mod manager;
pub mod probe;
pub mod protocol;
pub mod service;
pub mod services;
pub mod sys;
