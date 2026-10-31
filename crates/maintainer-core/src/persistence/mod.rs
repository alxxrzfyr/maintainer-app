//! SQLite persistence layer for settings and operation history.
//!
//! All database access goes through the [`Database`] struct, which owns
//! the connection and applies migrations on first open. Repositories
//! provide typed access to specific tables.

pub mod database;
pub mod history;
pub mod migrations;
mod sqlite_snapshot;
