//! SQLite storage for the accounting application.
//!
//! The implementation is grouped by responsibility:
//!
//! - [`model`] holds the wire types and the shared connection holder.
//! - [`schema`] creates and migrates the schema and seeds the starter chart.
//! - [`connection`] opens the database file and owns the SQLite connection.
//! - [`accounts`] persists account groups and the chart of accounts.
//! - [`settings`] reads and writes company settings.
//! - [`backup`] validates, creates and restores database snapshots.
//! - [`validation`] normalizes codes and validates form input.
//!
//! SQLite schema migrations are kept versioned; the public `crate::db::*` API
//! keeps stable command-facing names while the persisted voucher model evolves.

mod accounts;
pub(crate) mod audit;
mod backup;
pub(crate) mod classification;
mod closing;
mod common;
mod connection;
mod fiscal_years;
pub(crate) mod fs_catalog;
mod legacy;
mod model;
mod portable;
mod schema;
mod settings;
mod validation;

pub(crate) use accounts::ClassificationUpdate;
pub(crate) use backup::{
    cleanup_stale_plaintext_temps, prune_backup_files, prune_restore_safety_files,
};
pub use common::{normalize_digits, DB_FILE_NAME};
pub use connection::initialize;
pub use model::{
    Account, AccountGroup, AccountGroupList, AccountList, AuditLogEntry, AuditLogList,
    BulkAccountDeleteFailure, BulkAccountDeleteResult, CarryForwardExecutionResult,
    CarryForwardLine, CarryForwardPlan, ClosingBlocker, ClosingCounts, ClosingExecutionResult,
    ClosingLine, ClosingLineTotals, ClosingPreview, ClosingReversalResult, ClosingRoleAccount,
    ClosingRoles, ClosingRunRecord, ClosingTotals, ClosingVoucherRef, Database, FiscalYear,
    FiscalYearList, FsCatalog, FsCatalogOption, FsLineInfo, LegacyLevel4Account,
    LegacyLevel4Analysis, LegacyLevel4MigrationResult, LegacyLevel4ResolutionPreview, Limits,
    NewAccount, Settings, ACCOUNT_GROUP_SECTIONS, ACCOUNT_GROUP_TYPES, ACCOUNT_NATURES,
    MAX_ACCOUNT_LEVEL,
};
pub(crate) use portable::{is_encrypted_backup, validate_portable_password, PORTABLE_EXTENSION};

pub(crate) use fiscal_years::{ensure_open_period, ensure_year_accepts_date};
pub(crate) use legacy::legacy_migration_backup_path;

pub(crate) use common::{db_error, normalize_code, random_hex, sha256_file, to_fa_digits};
#[cfg(test)]
pub(crate) use schema::{
    apply_base_schema, apply_schema_migrations, seed_default_accounts, seed_default_groups,
    seed_default_settings,
};

#[cfg(test)]
pub(crate) fn memory_database() -> Database {
    let mut conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    apply_base_schema(&conn).unwrap();
    apply_schema_migrations(&mut conn).unwrap();
    seed_default_groups(&mut conn).unwrap();
    seed_default_accounts(&mut conn).unwrap();
    seed_default_settings(&conn).unwrap();
    let db = Database::default();
    db.attach(conn, std::path::PathBuf::from(":memory:"));
    db
}

/// Opens a real on-disk SQLite database with the full schema and starter
/// chart. Tests that cross the file boundary (backup, restore, Excel, two
/// connections) use this instead of an in-memory database.
#[cfg(test)]
pub(crate) fn file_database(path: &std::path::Path) -> Database {
    let mut conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    apply_base_schema(&conn).unwrap();
    apply_schema_migrations(&mut conn).unwrap();
    seed_default_groups(&mut conn).unwrap();
    seed_default_accounts(&mut conn).unwrap();
    seed_default_settings(&conn).unwrap();
    let db = Database::default();
    db.attach(conn, path.to_path_buf());
    db
}

/// Creates a fresh temporary directory for a real-file test.
#[cfg(test)]
pub(crate) fn test_temp_dir(prefix: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    let directory = std::env::temp_dir().join(format!("hesabdari-{prefix}-{stamp}"));
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

#[cfg(test)]
mod integration_tests;
#[cfg(test)]
mod tests;
