-- Migration 001: Initial schema for Maintainer persistence layer.
-- Tables: schema_migrations, settings, operation_history, scan_metadata, diagnostic_history.

CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY NOT NULL,
    applied_at INTEGER NOT NULL  -- UTC Unix seconds
);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS operation_history (
    id TEXT PRIMARY KEY NOT NULL,          -- UUID v4
    operation TEXT NOT NULL,               -- e.g. "cleanup", "scan", "update"
    started_at INTEGER NOT NULL,           -- UTC Unix seconds
    completed_at INTEGER,                  -- UTC Unix seconds, NULL if still running
    status TEXT NOT NULL CHECK(status IN ('success', 'partial_failure', 'failure', 'cancelled')),
    recovered_bytes INTEGER DEFAULT 0 CHECK(recovered_bytes >= 0),
    errors TEXT                            -- JSON array of error strings, NULL if none
);

CREATE TABLE IF NOT EXISTS scan_metadata (
    id TEXT PRIMARY KEY NOT NULL,          -- UUID v4
    scan_type TEXT NOT NULL,               -- e.g. "large_files", "duplicates"
    started_at INTEGER NOT NULL,           -- UTC Unix seconds
    completed_at INTEGER,                  -- UTC Unix seconds
    status TEXT NOT NULL CHECK(status IN ('success', 'partial_failure', 'failure', 'cancelled')),
    items_found INTEGER DEFAULT 0 CHECK(items_found >= 0),
    total_bytes INTEGER DEFAULT 0 CHECK(total_bytes >= 0)
);

CREATE TABLE IF NOT EXISTS diagnostic_history (
    id TEXT PRIMARY KEY NOT NULL,          -- UUID v4
    diagnostic_type TEXT NOT NULL,         -- e.g. "failed_services", "journal_errors"
    created_at INTEGER NOT NULL,           -- UTC Unix seconds
    summary TEXT NOT NULL,
    details TEXT                           -- JSON, NULL if none
);

CREATE INDEX IF NOT EXISTS idx_operation_history_started_at ON operation_history(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_scan_metadata_started_at ON scan_metadata(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_diagnostic_history_created_at ON diagnostic_history(created_at DESC);
