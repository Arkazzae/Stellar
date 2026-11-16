//! Stellarr Init: the process manager that runs as PID 1 in the Stellarr
//! container.
//!
//! [`service`] describes a service and checks a table of them. [`core`]
//! makes every decision, free of processes and clocks.

pub mod core;
pub mod service;
