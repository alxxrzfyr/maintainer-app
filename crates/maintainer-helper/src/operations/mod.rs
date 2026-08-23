//! Fixed privileged operations.
//!
//! Each operation performs exactly one validated system action.
//! No operation accepts arbitrary commands, paths, or shell text.

pub mod journal;
pub mod kernel;
pub mod package_cache;
pub mod process;
pub mod smart;
pub mod updates;
