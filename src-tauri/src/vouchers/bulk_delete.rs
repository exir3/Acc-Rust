//! Previewing and deleting vouchers by date or number range.

use crate::db::{self, Database};
use crate::vouchers::model::{
    BulkVoucherDeleteDateRange, BulkVoucherDeleteInput, BulkVoucherDeleteNumberRange,
    BulkVoucherDeletePreview, BulkVoucherDeleteResult, BulkVoucherDeleteSample,
    MAX_BULK_DELETE_TARGETS, MAX_VOUCHER_NUMBER,
};
use crate::vouchers::validation::{check_range, normalize_date};
use rusqlite::{params, Connection, TransactionBehavior};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;

#[derive(Debug, Clone, Hash)]
enum BulkVoucherDeleteScope {
    All,
    DateRange { from: String, to: String },
    NumberRange { from: i64, to: i64 },
}

impl BulkVoucherDeleteScope {
    fn as_str(&self) -> &'static str {
        match self {
            Self::All => "all",
            Self::DateRange { .. } => "dateRange",
            Self::NumberRange { .. } => "numberRange",
        }
    }
}

#[derive(Debug, Clone, Hash)]
struct BulkVoucherDeleteTarget {
    id: i64,
    number: i64,
    date: String,
    voucher_type: String,
    description: String,
    updated_at: String,
    is_temporary: bool,
}

fn parse_bulk_delete_scope(
    input: &BulkVoucherDeleteInput,
) -> Result<BulkVoucherDeleteScope, String> {
    match input.scope.trim() {
        "all" => Ok(BulkVoucherDeleteScope::All),
        "dateRange" => {
            let from = normalize_date(&input.from_date, "تاریخ شروع")?;
            let to = normalize_date(&input.to_date, "تاریخ پایان")?;
            check_range(&from, &to)?;
            Ok(BulkVoucherDeleteScope::DateRange { from, to })
        }
        "numberRange" => {
            let from = input
                .from_number
                .ok_or_else(|| "شماره سند شروع الزامی است.".to_string())?;
            let to = input
                .to_number
                .ok_or_else(|| "شماره سند پایان الزامی است.".to_string())?;
            if from <= 0 || to <= 0 {
                return Err("شماره سند باید بزرگ‌تر از صفر باشد.".to_string());
            }
            if from > to {
                return Err("شماره سند شروع نمی‌تواند بزرگ‌تر از شماره پایان باشد.".to_string());
            }
            if to > MAX_VOUCHER_NUMBER {
                return Err(format!(
                    "شماره سند پایان نمی‌تواند بیشتر از {} باشد.",
                    db::to_fa_digits(MAX_VOUCHER_NUMBER.to_string())
                ));
            }
            Ok(BulkVoucherDeleteScope::NumberRange { from, to })
        }
        _ => Err("محدوده حذف انتخاب‌شده معتبر نیست.".to_string()),
    }
}

fn read_bulk_delete_targets(
    conn: &Connection,
    scope: &BulkVoucherDeleteScope,
) -> Result<Vec<BulkVoucherDeleteTarget>, String> {
    const COLUMNS: &str = "SELECT id, number, date, type, description, updated_at, is_temporary FROM vouchers";
    // Read one row beyond the cap so an oversized range is rejected without
    // loading the whole ledger into memory.
    let limit = MAX_BULK_DELETE_TARGETS + 1;
    let read_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<BulkVoucherDeleteTarget> {
        Ok(BulkVoucherDeleteTarget {
            id: row.get(0)?,
            number: row.get(1)?,
            date: row.get(2)?,
            voucher_type: row.get(3)?,
            description: row.get(4)?,
            updated_at: row.get(5)?,
            is_temporary: row.get::<_, i64>(6)? != 0,
        })
    };

    match scope {
        BulkVoucherDeleteScope::All => conn
            .prepare(&format!("{COLUMNS} ORDER BY number, id LIMIT {limit}"))
            .map_err(db::db_error)?
            .query_map([], read_row)
            .map_err(db::db_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db::db_error),
        BulkVoucherDeleteScope::DateRange { from, to } => conn
            .prepare(&format!(
                "{COLUMNS} WHERE date >= ?1 AND date <= ?2 ORDER BY number, id LIMIT {limit}"
            ))
            .map_err(db::db_error)?
            .query_map(params![from, to], read_row)
            .map_err(db::db_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db::db_error),
        BulkVoucherDeleteScope::NumberRange { from, to } => conn
            .prepare(&format!(
                "{COLUMNS} WHERE number >= ?1 AND number <= ?2 ORDER BY number, id LIMIT {limit}"
            ))
            .map_err(db::db_error)?
            .query_map(params![from, to], read_row)
            .map_err(db::db_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db::db_error),
    }
}

fn bulk_delete_fingerprint(
    scope: &BulkVoucherDeleteScope,
    targets: &[BulkVoucherDeleteTarget],
) -> String {
    let mut hasher = DefaultHasher::new();
    scope.hash(&mut hasher);
    targets.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn bulk_delete_sample(target: &BulkVoucherDeleteTarget) -> BulkVoucherDeleteSample {
    BulkVoucherDeleteSample {
        voucher_number: target.number,
        display_date: target.date.clone(),
        description: target.description.clone(),
    }
}

fn bulk_delete_targets_error(count: usize) -> String {
    format!(
        "تعداد اسناد این محدوده ({} سند) از سقف مجاز ({} سند) بیشتر است؛ بازه را کوچک‌تر کنید.",
        db::to_fa_digits(count),
        db::to_fa_digits(MAX_BULK_DELETE_TARGETS)
    )
}

fn check_bulk_delete_limit(targets: &[BulkVoucherDeleteTarget]) -> Result<(), String> {
    if targets.len() > MAX_BULK_DELETE_TARGETS {
        return Err(bulk_delete_targets_error(targets.len()));
    }
    Ok(())
}

fn local_timestamp(conn: &Connection) -> Result<String, String> {
    conn.query_row(
        "SELECT strftime('%Y-%m-%d %H:%M:%f', 'now', 'localtime')",
        [],
        |row| row.get(0),
    )
    .map_err(db::db_error)
}

impl Database {
    fn bulk_delete_backup_directory(&self) -> Option<PathBuf> {
        let database_path = PathBuf::from(self.path_string());
        if database_path.as_os_str().is_empty() || database_path.as_os_str() == ":memory:" {
            return None;
        }
        let parent = database_path.parent()?;
        if parent.as_os_str().is_empty() {
            return None;
        }
        Some(parent.join("backups").join("pre-delete"))
    }

    pub fn preview_bulk_voucher_delete(
        &self,
        input: &BulkVoucherDeleteInput,
    ) -> Result<BulkVoucherDeletePreview, String> {
        let input = input.clone();
        let backup_directory = self
            .bulk_delete_backup_directory()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_default();
        self.with_conn(move |conn| {
            let scope = parse_bulk_delete_scope(&input)?;
            let targets = read_bulk_delete_targets(conn, &scope)?;
            check_bulk_delete_limit(&targets)?;
            let plan_token = bulk_delete_fingerprint(&scope, &targets);
            let timestamp = local_timestamp(conn)?;
            let deletable_count = targets.iter().filter(|target| target.is_temporary).count();
            let locked_count = targets.len() - deletable_count;
            let deletable_sample = targets.iter().filter(|target| target.is_temporary)
                .take(20).map(bulk_delete_sample).collect();
            let locked_sample = targets.iter().filter(|target| !target.is_temporary)
                .take(20).map(bulk_delete_sample).collect();
            Ok(BulkVoucherDeletePreview {
                scope: scope.as_str().to_string(),
                plan_token,
                total_matched: targets.len(),
                deletable_count,
                locked_count,
                locked_sample,
                deletable_sample,
                date_range: match &scope {
                    BulkVoucherDeleteScope::DateRange { from, to } => {
                        Some(BulkVoucherDeleteDateRange {
                            from: from.clone(),
                            to: to.clone(),
                        })
                    }
                    _ => None,
                },
                number_range: match &scope {
                    BulkVoucherDeleteScope::NumberRange { from, to } => {
                        Some(BulkVoucherDeleteNumberRange {
                            from: *from,
                            to: *to,
                        })
                    }
                    _ => None,
                },
                timestamp,
                backup_directory,
            })
        })
    }

    pub fn apply_bulk_voucher_delete(
        &self,
        input: &BulkVoucherDeleteInput,
        plan_token: &str,
        with_backup: bool,
    ) -> Result<BulkVoucherDeleteResult, String> {
        let token = plan_token.trim();
        if token.is_empty() {
            return Err("پیش‌نمایش منقضی یا نامعتبر است؛ دوباره پیش‌نمایش بگیرید.".to_string());
        }
        let token = token.to_string();
        let input = input.clone();

        // Reject a stale preview before writing a backup file.
        {
            let input = input.clone();
            let token = token.clone();
            self.with_conn(move |conn| {
                let scope = parse_bulk_delete_scope(&input)?;
                let targets = read_bulk_delete_targets(conn, &scope)?;
                check_bulk_delete_limit(&targets)?;
                if bulk_delete_fingerprint(&scope, &targets) != token {
                    return Err(
                        "اسناد پس از پیش‌نمایش تغییر کرده‌اند؛ پیش‌نمایش جدید بگیرید.".to_string()
                    );
                }
                Ok(())
            })?;
        }

        let backup_path = if with_backup {
            let directory = self.bulk_delete_backup_directory().ok_or_else(|| {
                "مسیر پایگاه داده برای تهیه پشتیبان مشخص نیست؛ حذف انجام نشد.".to_string()
            })?;
            std::fs::create_dir_all(&directory)
                .map_err(|error| format!("ساخت پوشه پشتیبان ناموفق بود: {error}"))?;
            let stamp: String = self.with_conn(|conn| {
                conn.query_row(
                    "SELECT strftime('%Y%m%d_%H%M%S', 'now', 'localtime')",
                    [],
                    |row| row.get(0),
                )
                .map_err(db::db_error)
            })?;
            let suffix = &token[..token.len().min(8)];
            Some(directory.join(format!("pre_delete_{stamp}_{suffix}.sqlite3")))
        } else {
            None
        };

        // The backup must succeed before anything is deleted.
        if let Some(path) = &backup_path {
            self.backup_to_for_bulk_delete(path)?;
        }
        let backup_file = backup_path.map(|path| path.to_string_lossy().to_string());

        self.with_conn(move |conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db::db_error)?;
            let scope = parse_bulk_delete_scope(&input)?;
            let targets = read_bulk_delete_targets(&tx, &scope)?;
            check_bulk_delete_limit(&targets)?;
            if bulk_delete_fingerprint(&scope, &targets) != token {
                return Err("اسناد پس از پیش‌نمایش تغییر کرده‌اند؛ پیش‌نمایش جدید بگیرید.".to_string());
            }
            let locked_count = targets.iter().filter(|target| !target.is_temporary).count();
            let deletable: Vec<_> = targets.iter().filter(|target| target.is_temporary).collect();
            for target in &deletable {
                tx.execute(
                    "DELETE FROM vouchers WHERE id = ?1 AND is_temporary = 1",
                    params![target.id],
                )
                .map_err(db::db_error)?;
            }
            let deleted_count = deletable.len();
            let timestamp = local_timestamp(&tx)?;
            tx.commit().map_err(db::db_error)?;
            let message = if deleted_count == 0 {
                "هیچ سندی در این محدوده یافت نشد.".to_string()
            } else {
                format!("{} سند با موفقیت حذف شد.", db::to_fa_digits(deleted_count))
            };
            Ok(BulkVoucherDeleteResult {
                scope: scope.as_str().to_string(),
                message,
                deleted_count,
                locked_count,
                timestamp,
                with_backup,
                pre_delete_backup_file: backup_file,
            })
        })
    }
}
