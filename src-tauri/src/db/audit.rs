//! Append-only audit trail for accounting definitions and destructive actions.

use rusqlite::{params, Connection};

use crate::db::common::db_error;
use crate::db::model::{AuditLogEntry, AuditLogList, Database};

/// Records one change. The caller decides how much detail to store in the
/// before/after JSON snapshots; the audit row itself is never updated.
pub(crate) fn log(
    conn: &Connection,
    action: &str,
    entity_type: &str,
    entity_id: Option<i64>,
    before_value: Option<&str>,
    after_value: Option<&str>,
    details: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO audit_logs
            (action, entity_type, entity_id, before_value, after_value, details, app_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            action,
            entity_type,
            entity_id,
            before_value,
            after_value,
            details,
            env!("CARGO_PKG_VERSION")
        ],
    )
    .map_err(db_error)?;
    Ok(())
}

impl Database {
    pub fn list_audit_logs(
        &self,
        limit: Option<i64>,
        offset: Option<i64>,
        entity_type: Option<&str>,
    ) -> Result<AuditLogList, String> {
        let limit = limit.unwrap_or(100).clamp(1, 1000);
        let offset = offset.unwrap_or(0).max(0);
        let entity_type = entity_type.unwrap_or("").trim().to_string();
        let database_path = self.path_string();

        self.with_conn(move |conn| {
            let total: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM audit_logs
                      WHERE (?1 = '' OR entity_type = ?1)",
                    params![entity_type],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            let mut stmt = conn
                .prepare(
                    "SELECT id, action, entity_type, entity_id, before_value,
                            after_value, details, app_version, created_at
                       FROM audit_logs
                      WHERE (?1 = '' OR entity_type = ?1)
                      ORDER BY id DESC
                      LIMIT ?2 OFFSET ?3",
                )
                .map_err(db_error)?;
            let entries = stmt
                .query_map(params![entity_type, limit, offset], |row| {
                    Ok(AuditLogEntry {
                        id: row.get(0)?,
                        action: row.get(1)?,
                        entity_type: row.get(2)?,
                        entity_id: row.get(3)?,
                        before_value: row.get(4)?,
                        after_value: row.get(5)?,
                        details: row.get(6)?,
                        app_version: row.get(7)?,
                        created_at: row.get(8)?,
                    })
                })
                .map_err(db_error)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_error)?;
            Ok(AuditLogList {
                entries,
                total,
                database_path,
            })
        })
    }
}
