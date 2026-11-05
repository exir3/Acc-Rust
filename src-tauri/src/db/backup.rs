//! Backup, restore and backup-file validation.

use rusqlite::{
    backup::Backup, params, Connection, OpenFlags, OptionalExtension,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::db::common::{
    db_error, to_fa_digits, BULK_DELETE_SNAPSHOT_KEY, BULK_DELETE_SNAPSHOT_VALUE,
    CURRENT_SCHEMA_VERSION,
};
use crate::db::model::Database;
use crate::db::schema::{apply_base_schema, apply_schema_migrations, ensure_voucher_account_pair_guards, table_exists};
use crate::vouchers::MAX_VOUCHER_NUMBER;

impl Database {
    pub fn backup_to(&self, destination: &Path) -> Result<(), String> {
        self.backup_to_internal(destination, false)
    }

    /// Creates a reversible pre-delete snapshot. Unlike a user-selected backup,
    /// this must preserve legacy voucher defects so deleting another range is
    /// not blocked by an unrelated historical document. SQLite integrity,
    /// schema, account hierarchy, line-level constraints and foreign keys are
    /// still validated. A marker lets restore apply this same narrow exception.
    pub(crate) fn backup_to_for_bulk_delete(&self, destination: &Path) -> Result<(), String> {
        self.backup_to_internal(destination, true)
    }

    pub(crate) fn backup_to_internal(
        &self,
        destination: &Path,
        preserve_legacy_voucher_structure: bool,
    ) -> Result<(), String> {
        let current_path = PathBuf::from(self.path_string());
        if same_path(&current_path, destination) {
            return Err("مسیر پشتیبان نمی‌تواند همان مسیر پایگاه دادهٔ فعال باشد.".to_string());
        }

        self.with_conn(|source| {
            let temporary = unique_sibling_path(destination, "backup-tmp");
            let _ = std::fs::remove_file(&temporary);
            let mut target = Connection::open(&temporary).map_err(db_error)?;
            let validation = (|| {
                {
                    let backup = Backup::new(source, &mut target).map_err(db_error)?;
                    backup
                        .run_to_completion(128, Duration::from_millis(10), None)
                        .map_err(db_error)?;
                }

                if preserve_legacy_voucher_structure {
                    validate_backup_connection_with_options(&target, true)?;
                    target
                        .execute(
                            "INSERT INTO app_meta(key, value) VALUES (?1, ?2)
                             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                            params![BULK_DELETE_SNAPSHOT_KEY, BULK_DELETE_SNAPSHOT_VALUE],
                        )
                        .map_err(db_error)?;
                    Ok(())
                } else {
                    // Do not publish a regular backup containing invalid
                    // accounting data or references.
                    validate_backup_connection(&target)
                }
            })();
            drop(target);
            if let Err(error) = validation {
                let _ = std::fs::remove_file(&temporary);
                return Err(error);
            }
            replace_file(&temporary, destination)
        })
    }

    pub fn restore_from(&self, source_path: &Path) -> Result<PathBuf, String> {
        let current_path = PathBuf::from(self.path_string());
        if same_path(&current_path, source_path) {
            return Err("فایل انتخاب‌شده پایگاه دادهٔ فعال است، نه یک پشتیبان.".to_string());
        }

        let source = Connection::open_with_flags(source_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| format!("باز کردن فایل پشتیبان ناموفق بود: {error}"))?;
        let is_bulk_delete_snapshot = is_bulk_delete_snapshot_file(&source)?;
        if is_bulk_delete_snapshot {
            validate_backup_connection_with_options(&source, true)?;
        } else {
            validate_backup_connection(&source)?;
        }
        let safety_path = unique_sibling_path(&current_path, "before-restore");

        self.with_conn(|target| {
            let mut safety = Connection::open(&safety_path).map_err(db_error)?;
            {
                let backup = Backup::new(target, &mut safety).map_err(db_error)?;
                backup
                    .run_to_completion(128, Duration::from_millis(10), None)
                    .map_err(db_error)?;
            }
            drop(safety);

            let restore_result = {
                let backup = Backup::new(&source, target).map_err(db_error)?;
                backup
                    .run_to_completion(128, Duration::from_millis(10), None)
                    .map_err(db_error)
            }
            .and_then(|()| {
                // Upgrade a supported v4 backup inside the same restore safety
                // envelope. A v5 migration failure (for example, legacy level-4
                // accounts) rolls the whole restore back to the active database.
                apply_schema_migrations(target)?;
                validate_backup_connection_with_options(target, is_bulk_delete_snapshot)
            })
            .and_then(|()| reset_schema_triggers(target))
            .and_then(|()| {
                if is_bulk_delete_snapshot {
                    target
                        .execute(
                            "DELETE FROM app_meta WHERE key = ?1",
                            params![BULK_DELETE_SNAPSHOT_KEY],
                        )
                        .map_err(db_error)?;
                }
                Ok(())
            });
            if let Err(error) = restore_result {
                let rollback_result = (|| -> Result<(), String> {
                    let safety = Connection::open(&safety_path).map_err(db_error)?;
                    {
                        let rollback = Backup::new(&safety, target).map_err(db_error)?;
                        rollback
                            .run_to_completion(128, Duration::from_millis(10), None)
                            .map_err(db_error)?;
                    }
                    Ok(())
                })();
                if let Err(rollback_error) = rollback_result {
                    let reason = format!(
                        "بازیابی پشتیبان و بازگردانی خودکار نسخهٔ ایمنی هر دو ناموفق بودند. برای جلوگیری از خواندن یا نوشتن روی پایگاه دادهٔ نامطمئن، برنامه تا راه‌اندازی مجدد متوقف شد. نسخهٔ ایمنی در «{}» نگهداری می‌شود؛ آن را دستی بازیابی کنید. خطای بازیابی: {error} | خطای بازگردانی: {rollback_error}",
                        safety_path.display()
                    );
                    *self
                        .failed
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(reason.clone());
                    return Err(reason);
                }
                return Err(format!(
                    "بازیابی انجام نشد و پایگاه قبلی بازگردانده شد: {error}"
                ));
            }
            Ok(safety_path.clone())
        })
    }
}

const SCHEMA_TRIGGERS: [&str; 6] = [
    "vouchers_positive_number_insert",
    "vouchers_positive_number_update",
    "voucher_lines_valid_insert",
    "voucher_lines_valid_update",
    "voucher_lines_account_pair_insert",
    "voucher_lines_account_pair_update",
];

/// `SCHEMA` uses `CREATE TRIGGER IF NOT EXISTS`, so a restored database could
/// keep a tampered trigger that happens to use an allowed name. Drop the
/// known triggers first and recreate them from the application's own schema.
fn reset_schema_triggers(conn: &Connection) -> Result<(), String> {
    for trigger in SCHEMA_TRIGGERS {
        conn.execute(&format!("DROP TRIGGER IF EXISTS \"{trigger}\""), [])
            .map_err(db_error)?;
    }
    apply_base_schema(conn)?;
    ensure_voucher_account_pair_guards(conn)
}

pub(crate) fn validate_backup_connection(conn: &Connection) -> Result<(), String> {
    validate_backup_connection_with_options(conn, false)
}

fn is_bulk_delete_snapshot_file(conn: &Connection) -> Result<bool, String> {
    if !table_exists(conn, "app_meta")? {
        return Ok(false);
    }
    let value = conn
        .query_row(
            "SELECT value FROM app_meta WHERE key = ?1",
            params![BULK_DELETE_SNAPSHOT_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(db_error)?;
    Ok(value.as_deref() == Some(BULK_DELETE_SNAPSHOT_VALUE))
}

fn validate_backup_connection_with_options(
    conn: &Connection,
    preserve_legacy_voucher_structure: bool,
) -> Result<(), String> {
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(db_error)?;
    if integrity != "ok" {
        return Err(format!("سلامت فایل SQLite تأیید نشد: {integrity}"));
    }
    let unexpected_trigger: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master
                            WHERE type = 'trigger'
                                AND name NOT IN (
                                        'vouchers_positive_number_insert',
                                        'vouchers_positive_number_update',
                                        'voucher_lines_valid_insert',
                                        'voucher_lines_valid_update',
                                        'voucher_lines_account_pair_insert',
                                        'voucher_lines_account_pair_update'
                                )
                            LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_error)?;
    if let Some(name) = unexpected_trigger {
        return Err(format!(
            "فایل پشتیبان دارای trigger ناشناخته «{name}» است و قابل بازیابی نیست."
        ));
    }
    let version: String = conn
        .query_row(
            "SELECT value FROM app_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    let version_number = version
        .parse::<i64>()
        .map_err(|_| "نسخه ساختار فایل پشتیبان معتبر نیست.".to_string())?;
    // Version 4 backups are accepted as migration inputs. They are upgraded
    // only after copying into the active DB, where restore's safety snapshot
    // can roll everything back if v5's level-4/reference preflight fails.
    if !(4..=CURRENT_SCHEMA_VERSION).contains(&version_number) {
        return Err("نسخه ساختار فایل پشتیبان با این نسخه برنامه سازگار نیست.".to_string());
    }
    for table in [
        "app_meta",
        "settings",
        "account_groups",
        "accounts",
        "vouchers",
        "voucher_lines",
    ] {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                params![table],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if !exists {
            return Err(format!(
                "فایل انتخاب‌شده پشتیبان معتبر نیست؛ جدول {table} وجود ندارد."
            ));
        }
    }

    const REQUIRED_COLUMNS: [(&str, &[&str]); 6] = [
        ("app_meta", &["key", "value"]),
        ("settings", &["key", "value"]),
        (
            "account_groups",
            &[
                "id",
                "code",
                "name",
                "type",
                "default_section",
                "created_at",
                "updated_at",
            ],
        ),
        (
            "accounts",
            &[
                "id",
                "code",
                "name",
                "level",
                "group_code",
                "nature",
                "parent_code",
                "parent_name",
                "is_postable",
                "is_active",
                "created_at",
                "updated_at",
            ],
        ),
        (
            "vouchers",
            &[
                "id",
                "number",
                "date",
                "type",
                "description",
                "created_at",
                "updated_at",
            ],
        ),
        (
            "voucher_lines",
            &[
                "id",
                "voucher_id",
                "row_no",
                "account_code",
                "description",
                "debit",
                "credit",
            ],
        ),
    ];
    for (table, required) in REQUIRED_COLUMNS {
        let columns = backup_table_columns(conn, table)?;
        for column in required {
            if !columns.iter().any(|name| name == column) {
                return Err(format!(
                    "فایل انتخاب‌شده پشتیبان معتبر نیست؛ ستون {table}.{column} وجود ندارد."
                ));
            }
        }
        if table == "voucher_lines" && version_number >= 5 {
            for column in ["account_id", "detail1_code"] {
                if !columns.iter().any(|name| name == column) {
                    return Err(format!(
                        "فایل پشتیبان نسخه ۵ معتبر نیست؛ ستون voucher_lines.{column} وجود ندارد."
                    ));
                }
            }
        }
    }

    {
        let mut statement = conn.prepare("PRAGMA foreign_key_check").map_err(db_error)?;
        let mut rows = statement.query([]).map_err(db_error)?;
        if let Some(row) = rows.next().map_err(db_error)? {
            let table: String = row.get(0).map_err(db_error)?;
            let row_id: i64 = row.get(1).map_err(db_error)?;
            return Err(format!(
                "فایل پشتیبان رابطه کلید خارجی نامعتبر دارد (جدول {table}، ردیف {row_id})."
            ));
        }
    }

    if let Some(code) = backup_first_text(
        conn,
        "SELECT code FROM account_groups
          WHERE code = '' OR length(code) > 50 OR code GLOB '*[^0-9]*' OR trim(code, '0') = ''
          LIMIT 1",
    )? {
        return Err(format!("کد گروه «{code}» در فایل پشتیبان معتبر نیست."));
    }
    if let Some(code) = backup_first_text(
        conn,
        "SELECT code FROM account_groups
          WHERE type NOT IN ('دارایی', 'بدهی', 'سرمایه', 'درآمد', 'هزینه', 'سایر')
          LIMIT 1",
    )? {
        return Err(format!(
            "نوع گروه «{code}» در فایل پشتیبان معتبر نیست."
        ));
    }
    if let Some(name) = backup_first_text(
        conn,
        "SELECT name FROM account_groups
          WHERE trim(name) = '' OR length(name) > 255
          LIMIT 1",
    )? {
        return Err(format!(
            "نام گروه «{name}» در فایل پشتیبان خالی یا بیش از حد بلند است."
        ));
    }

    if let Some(code) = backup_first_text(
        conn,
        "SELECT code FROM accounts
          WHERE code = '' OR length(code) > 50 OR code GLOB '*[^0-9]*' OR trim(code, '0') = ''
          LIMIT 1",
    )? {
        return Err(format!("کد حساب «{code}» در فایل پشتیبان معتبر نیست."));
    }
    if let Some(name) = backup_first_text(
        conn,
        "SELECT name FROM accounts
          WHERE trim(name) = '' OR length(name) > 255
          LIMIT 1",
    )? {
        return Err(format!(
            "نام حساب «{name}» در فایل پشتیبان خالی یا بیش از حد بلند است."
        ));
    }
    if let Some(code) = backup_first_text(
        conn,
        &format!("SELECT code FROM accounts
          WHERE level NOT BETWEEN 1 AND {}
             OR is_postable NOT IN (0, 1)
             OR nature NOT IN ('debtor', 'creditor', 'dual')
          LIMIT 1", if version_number < 5 { 4 } else { 3 }),
    )? {
        return Err(format!(
            "سطح، ماهیت یا وضعیت ثبت حساب «{code}» در فایل پشتیبان معتبر نیست."
        ));
    }
    if let Some(code) = backup_first_text(
        conn,
        "SELECT code FROM accounts
          WHERE (level = 1 AND parent_code IS NOT NULL AND parent_code <> '')
             OR (level > 1 AND (parent_code IS NULL OR parent_code = ''))
          LIMIT 1",
    )? {
        return Err(format!(
            "حساب «{code}» در فایل پشتیبان حساب بالاتر معتبری ندارد."
        ));
    }
    if let Some(code) = backup_first_text(
        conn,
        "SELECT a.code
           FROM accounts a
           JOIN accounts p ON p.code = a.parent_code
          WHERE a.level <> p.level + 1
             OR a.group_code <> p.group_code
          LIMIT 1",
    )? {
        return Err(format!(
            "ساختار حساب «{code}» با حساب بالاتر آن در فایل پشتیبان سازگار نیست."
        ));
    }

    if let Some(number) = backup_first_text(
        conn,
        &format!(
            "SELECT CAST(number AS TEXT) FROM vouchers
              WHERE number <= 0 OR number > {MAX_VOUCHER_NUMBER}
              LIMIT 1"
        ),
    )? {
        return Err(format!(
            "شماره سند «{number}» در فایل پشتیبان خارج از محدوده مجاز است."
        ));
    }
    if let Some(number) = backup_first_text(
        conn,
        "SELECT CAST(number AS TEXT) FROM vouchers
          GROUP BY number HAVING COUNT(*) > 1
          LIMIT 1",
    )? {
        return Err(format!("شماره سند «{number}» در فایل پشتیبان تکراری است."));
    }

    {
        let mut statement = conn
            .prepare("SELECT number, date, type, description FROM vouchers ORDER BY id")
            .map_err(db_error)?;
        let mut rows = statement.query([]).map_err(db_error)?;
        while let Some(row) = rows.next().map_err(db_error)? {
            let number: i64 = row.get(0).map_err(db_error)?;
            let date: String = row.get(1).map_err(db_error)?;
            let voucher_type: String = row.get(2).map_err(db_error)?;
            let description: String = row.get(3).map_err(db_error)?;
            let shown_number = to_fa_digits(number.to_string());
            if crate::vouchers::validate_voucher_date(&date).is_err() {
                return Err(format!(
                    "تاریخ سند شماره {shown_number} در فایل پشتیبان معتبر نیست."
                ));
            }
            if !crate::vouchers::VOUCHER_TYPES.contains(&voucher_type.as_str()) {
                return Err(format!(
                    "نوع سند شماره {shown_number} در فایل پشتیبان معتبر نیست."
                ));
            }
            if description.chars().count() > crate::vouchers::MAX_DESCRIPTION {
                return Err(format!(
                    "شرح سند شماره {shown_number} در فایل پشتیبان بیش از حد مجاز است."
                ));
            }
        }
    }

    if let Some(line) = backup_first_text(
        conn,
        "SELECT CAST(id AS TEXT) FROM voucher_lines
          WHERE account_code = '' OR length(account_code) > 50
             OR account_code GLOB '*[^0-9]*'
          LIMIT 1",
    )? {
        return Err(format!(
            "کد حساب ردیف {line} در فایل پشتیبان معتبر نیست."
        ));
    }
    if let Some(line) = backup_first_text(
        conn,
        &format!(
            "SELECT CAST(id AS TEXT) FROM voucher_lines
              WHERE row_no <= 0
                 OR debit < 0 OR credit < 0
                 OR debit > {} OR credit > {}
                 OR (debit > 0 AND credit > 0)
                 OR (debit = 0 AND credit = 0)
                 OR length(description) > {}
              LIMIT 1",
            crate::vouchers::MAX_AMOUNT,
            crate::vouchers::MAX_AMOUNT,
            crate::vouchers::MAX_LINE_DESCRIPTION
        ),
    )? {
        return Err(format!(
            "مبلغ، شماره ردیف یا شرح ردیف {line} در فایل پشتیبان معتبر نیست."
        ));
    }
    let account_reference_check = if version_number >= 5 {
        "SELECT CAST(l.id AS TEXT)
           FROM voucher_lines l
           LEFT JOIN accounts a ON a.id = l.account_id AND a.code = l.account_code
           LEFT JOIN accounts d ON d.code = l.detail1_code
          WHERE a.id IS NULL OR a.level NOT IN (1,2) OR a.is_postable = 0
             OR (l.detail1_code IS NOT NULL AND l.detail1_code <> '' AND
                 (d.id IS NULL OR d.level <> 3 OR a.level <> 2 OR d.parent_code <> a.code OR d.group_code <> a.group_code))
          LIMIT 1"
    } else {
        "SELECT CAST(l.id AS TEXT)
           FROM voucher_lines l
           LEFT JOIN accounts a ON a.code = l.account_code
          WHERE a.id IS NULL OR a.level NOT BETWEEN 1 AND 4
          LIMIT 1"
    };
    if let Some(line) = backup_first_text(conn, account_reference_check)? {
        return Err(format!(
            "حساب ردیف {line} در فایل پشتیبان وجود ندارد یا ساختار ارجاع آن معتبر نیست."
        ));
    }
    // A marked pre-delete snapshot is a reversible archive of the exact live
    // database state. It may contain an old voucher with an invalid aggregate
    // row count/balance; all SQLite, schema, foreign-key and line-level checks
    // above still run. Regular backups remain fully strict.
    if !preserve_legacy_voucher_structure {
        if let Some(number) = backup_first_text(
            conn,
            &format!(
                "SELECT CAST(v.number AS TEXT)
                   FROM vouchers v
                   LEFT JOIN voucher_lines l ON l.voucher_id = v.id
                  GROUP BY v.id
                 HAVING COUNT(l.id) < 2
                     OR COUNT(l.id) > {}
                     OR COUNT(DISTINCT l.row_no) <> COUNT(l.id)
                     OR COALESCE(SUM(l.debit), 0) = 0
                     OR COALESCE(SUM(l.debit), 0) <> COALESCE(SUM(l.credit), 0)
                  LIMIT 1",
                crate::vouchers::MAX_LINES
            ),
        )? {
            return Err(format!(
                "ساختار ردیف‌ها یا تراز سند شماره {number} در فایل پشتیبان معتبر نیست."
            ));
        }
    }

    Ok(())
}

fn backup_table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, String> {
    let mut statement = conn
        .prepare(&format!("PRAGMA table_info(\"{table}\")"))
        .map_err(db_error)?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(db_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db_error)?;
    Ok(columns)
}

fn backup_first_text(conn: &Connection, sql: &str) -> Result<Option<String>, String> {
    conn.query_row(sql, [], |row| row.get::<_, Option<String>>(0))
        .optional()
        .map_err(db_error)
        .map(Option::flatten)
}

fn unique_sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("database");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    path.with_file_name(format!(
        "{file_name}.{suffix}-{}-{stamp}",
        std::process::id()
    ))
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn replace_file(temporary: &Path, destination: &Path) -> Result<(), String> {
    let previous = unique_sibling_path(destination, "previous");
    if destination.exists() {
        std::fs::rename(destination, &previous)
            .map_err(|error| format!("آماده‌سازی فایل قبلی ناموفق بود: {error}"))?;
    }
    if let Err(error) = std::fs::rename(temporary, destination) {
        if previous.exists() {
            let _ = std::fs::rename(&previous, destination);
        }
        let _ = std::fs::remove_file(temporary);
        return Err(format!("جایگزینی فایل پشتیبان ناموفق بود: {error}"));
    }
    let _ = std::fs::remove_file(previous);
    Ok(())
}

/// Keeps the newest regular backups so the backups folder cannot grow without
/// bound. Older files are removed only when they match the backup naming
/// pattern used by this application.
pub(crate) fn prune_backup_files(directory: &Path) -> Result<usize, String> {
    prune_matching_files(directory, BACKUP_RETENTION, |name| {
        name.starts_with("hesabdari-backup-") && name.ends_with(".sqlite3")
    })
}

/// Removes older `.before-restore-*` safety copies next to the active
/// database. The newest copies remain available for manual recovery.
pub(crate) fn prune_restore_safety_files(database_path: &Path) -> Result<usize, String> {
    let directory = database_path
        .parent()
        .ok_or_else(|| "مسیر پوشه پایگاه داده پیدا نشد.".to_string())?;
    prune_matching_files(directory, RESTORE_SAFETY_RETENTION, |name| {
        name.contains(".before-restore-")
    })
}

fn prune_matching_files<F>(directory: &Path, keep: usize, matches: F) -> Result<usize, String>
where
    F: Fn(&str) -> bool,
{
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(format!("خواندن پوشه پشتیبان ناموفق بود: {error}")),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("خواندن فایل پشتیبان ناموفق بود: {error}"))?;
        if !entry
            .file_type()
            .map_err(|error| format!("تعیین نوع فایل پشتیبان ناموفق بود: {error}"))?
            .is_file()
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !matches(&name) {
            continue;
        }
        // Both backup and safety names end in a nanosecond timestamp, so the
        // suffix orders them without depending on filesystem time precision.
        let stem = name.strip_suffix(".sqlite3").unwrap_or(&name);
        let stamp = stem
            .rsplit('-')
            .next()
            .and_then(|value| value.parse::<u128>().ok())
            .unwrap_or(0);
        files.push((stamp, name));
    }
    if files.len() <= keep {
        return Ok(0);
    }
    files.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    let mut removed = 0;
    for (_, name) in files.into_iter().skip(keep) {
        let path = directory.join(name);
        std::fs::remove_file(&path).map_err(|error| {
            format!("حذف پشتیبان قدیمی «{}» ناموفق بود: {error}", path.display())
        })?;
        removed += 1;
    }
    Ok(removed)
}

const BACKUP_RETENTION: usize = 30;
const RESTORE_SAFETY_RETENTION: usize = 5;
