//! Explicit migration for legacy level-4 detail accounts.
//!
//! Opening a database never deletes or merges legacy level-4 rows. They stay
//! linked to their original account with `legacy_level4_pending = 1` until an
//! operator reviews the preflight report, takes a backup and applies the
//! conversion.

use std::path::Path;

use rusqlite::Connection;

use crate::db::audit;
use crate::db::common::{db_error, to_fa_digits};
use crate::db::model::{
    Database, LegacyLevel4Account, LegacyLevel4Analysis, LegacyLevel4MigrationResult,
};

fn analyze(conn: &Connection) -> Result<LegacyLevel4Analysis, String> {
    let mut accounts = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT a.code, a.name, COALESCE(a.parent_code, ''),
                        (SELECT COUNT(*) FROM voucher_lines l
                          WHERE l.account_code = a.code AND l.legacy_level4_pending = 1),
                        CASE WHEN p.level = 3 AND b.level = 2
                                   AND p.parent_code = b.code
                                   AND p.group_code = b.group_code
                                   AND a.group_code = b.group_code
                             THEN 1 ELSE 0 END
                   FROM accounts a
                   LEFT JOIN accounts p ON p.code = a.parent_code
                   LEFT JOIN accounts b ON b.code = p.parent_code
                  WHERE a.level = 4
                  ORDER BY a.code",
            )
            .map_err(db_error)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(LegacyLevel4Account {
                    code: row.get(0)?,
                    name: row.get(1)?,
                    parent_code: row.get(2)?,
                    line_count: row.get(3)?,
                    convertible: row.get::<_, i64>(4)? != 0,
                })
            })
            .map_err(db_error)?;
        for row in rows {
            accounts.push(row.map_err(db_error)?);
        }
    }

    let (line_count, voucher_count): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), COUNT(DISTINCT l.voucher_id)
               FROM voucher_lines l
               JOIN accounts a ON a.code = l.account_code
              WHERE a.level = 4 AND l.legacy_level4_pending = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(db_error)?;

    let convertible_line_count: i64 = accounts
        .iter()
        .filter(|account| account.convertible)
        .map(|account| account.line_count)
        .sum();
    Ok(LegacyLevel4Analysis {
        account_count: accounts.len() as i64,
        line_count,
        voucher_count,
        convertible_line_count,
        ambiguous_line_count: line_count - convertible_line_count,
        accounts,
    })
}

impl Database {
    /// Read-only preflight. Safe to call repeatedly and never changes data.
    pub fn analyze_legacy_level4(&self) -> Result<LegacyLevel4Analysis, String> {
        self.with_conn(|conn| analyze(conn))
    }

    /// Converts every convertible level-4 line to its level-3 detail and
    /// level-2 base account, then removes level-4 accounts that are no longer
    /// referenced. Ambiguous rows stay pending. A validated backup is written
    /// first; any failure rolls the conversion transaction back.
    pub fn apply_legacy_level4_migration(
        &self,
        backup_path: &Path,
    ) -> Result<LegacyLevel4MigrationResult, String> {
        if let Some(parent) = backup_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("ساخت پوشه پشتیبان ناموفق بود: {error}"))?;
        }
        self.backup_to(backup_path)?;

        self.with_conn(|conn| {
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(db_error)?;
            let before = analyze(&tx)?;

            let converted_lines = tx
                .execute(
                    "UPDATE voucher_lines
                        SET account_id = (
                                SELECT b.id
                                  FROM accounts a
                                  JOIN accounts p ON p.code = a.parent_code
                                  JOIN accounts b ON b.code = p.parent_code
                                 WHERE a.code = voucher_lines.account_code
                            ),
                            detail1_code = (
                                SELECT p.code
                                  FROM accounts a
                                  JOIN accounts p ON p.code = a.parent_code
                                 WHERE a.code = voucher_lines.account_code
                            ),
                            account_code = (
                                SELECT b.code
                                  FROM accounts a
                                  JOIN accounts p ON p.code = a.parent_code
                                  JOIN accounts b ON b.code = p.parent_code
                                 WHERE a.code = voucher_lines.account_code
                            ),
                            legacy_level4_pending = 0
                      WHERE legacy_level4_pending = 1
                        AND EXISTS (
                            SELECT 1
                              FROM accounts a
                              JOIN accounts p ON p.code = a.parent_code
                              JOIN accounts b ON b.code = p.parent_code
                             WHERE a.code = voucher_lines.account_code
                               AND a.level = 4
                               AND p.level = 3
                               AND b.level = 2
                               AND p.parent_code = b.code
                               AND p.group_code = b.group_code
                               AND a.group_code = b.group_code
                        )",
                    [],
                )
                .map_err(db_error)?;

            let removed_accounts = tx
                .execute(
                    "DELETE FROM accounts
                      WHERE level = 4
                        AND NOT EXISTS (
                            SELECT 1 FROM voucher_lines l
                             WHERE l.account_code = accounts.code
                                OR l.detail1_code = accounts.code
                        )",
                    [],
                )
                .map_err(db_error)?;

            let pending_lines: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM voucher_lines WHERE legacy_level4_pending = 1",
                    [],
                    |row| row.get(0),
                )
                .map_err(db_error)?;

            audit::log(
                &tx,
                "migrate",
                "legacy_level4",
                None,
                Some(&serde_json::to_string(&before).map_err(|error| error.to_string())?),
                None,
                &format!(
                    "{} ردیف تبدیل شد؛ {} ردیف در انتظار بررسی ماند؛ {} حساب سطح ۴ حذف شد. پشتیبان: {}",
                    to_fa_digits(converted_lines),
                    to_fa_digits(pending_lines),
                    to_fa_digits(removed_accounts),
                    backup_path.display()
                ),
            )?;
            tx.commit().map_err(db_error)?;

            Ok(LegacyLevel4MigrationResult {
                converted_lines: converted_lines as i64,
                pending_lines,
                removed_accounts: removed_accounts as i64,
                backup_path: backup_path.to_string_lossy().to_string(),
            })
        })
    }
}

/// Helper for the command layer so it can build a safe backup name next to the
/// active database without knowing the storage layout.
pub(crate) fn legacy_migration_backup_path(database_path: &Path) -> std::path::PathBuf {
    let parent = database_path.parent().unwrap_or_else(|| Path::new("."));
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    parent
        .join("backups")
        .join(format!("level4-migration-{stamp}.sqlite3"))
}
