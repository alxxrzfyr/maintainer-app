//! Maintainer privileged helper.
//!
//! Provides a fixed set of system operations accessible over D-Bus with
//! Polkit authorization. No method accepts arbitrary commands, paths, or
//! shell text.

pub mod api;
pub mod authorize;
pub mod operations;
pub mod server;
pub mod validate;
