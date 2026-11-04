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
mod backup;
mod common;
mod connection;
mod model;
mod schema;
mod settings;
mod validation;

pub use common::{normalize_digits, DB_FILE_NAME};
pub use connection::initialize;
pub use model::{
    Account, AccountGroup, AccountGroupList, AccountList, BulkAccountDeleteFailure,
    BulkAccountDeleteResult, Database, Limits, NewAccount, Settings, ACCOUNT_GROUP_SECTIONS,
    ACCOUNT_GROUP_TYPES, ACCOUNT_NATURES, MAX_ACCOUNT_LEVEL,
};

pub(crate) use common::{
    db_error, normalize_code, to_fa_digits, OTHER_NON_OPERATING_GROUP_CODE,
    OTHER_OPERATING_GROUP_CODE,
};
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

#[cfg(test)]
mod tests;
