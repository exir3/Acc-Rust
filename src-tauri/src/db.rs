//! SQLite storage for the accounting application.
//!
//! All definitions (account groups, chart of accounts), vouchers and settings
//! live in one local database. SQLite is bundled with the executable, so the
//! application is fully usable offline after installation.

use rusqlite::{
    backup::Backup, params, Connection, OpenFlags, OptionalExtension, TransactionBehavior,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Manager};

use crate::vouchers::MAX_VOUCHER_NUMBER;

pub const DB_FILE_NAME: &str = "hesabdari.sqlite3";
const CURRENT_SCHEMA_VERSION: i64 = 3;
const LEGACY_DB_FILE_NAME: &str = "audit-analyzer.sqlite3";
const LEGACY_IDENTIFIER: &str = "ir.audit.account-analyzer";

/// Account-group types accepted by the form and the database constraint.
pub const ACCOUNT_GROUP_TYPES: [&str; 6] = ["دارایی", "بدهی", "سرمایه", "درآمد", "هزینه", "سایر"];

pub const ACCOUNT_NATURES: [&str; 3] = ["debtor", "creditor", "dual"];

const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS app_meta (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS account_groups (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        code       TEXT NOT NULL UNIQUE,
        name       TEXT NOT NULL,
        type       TEXT NOT NULL CHECK(type IN ('دارایی', 'بدهی', 'سرمایه', 'درآمد', 'هزینه', 'سایر')),
        created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime')),
        updated_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
    );

    CREATE TABLE IF NOT EXISTS accounts (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        code        TEXT NOT NULL UNIQUE,
        name        TEXT NOT NULL,
        level       INTEGER NOT NULL CHECK(level BETWEEN 1 AND 4),
        group_code  TEXT NOT NULL REFERENCES account_groups(code) ON UPDATE CASCADE,
        nature      TEXT NOT NULL DEFAULT 'debtor' CHECK(nature IN ('debtor', 'creditor', 'dual')),
        parent_code TEXT REFERENCES accounts(code),
        parent_name TEXT NOT NULL DEFAULT '',
        is_postable INTEGER NOT NULL DEFAULT 1 CHECK(is_postable IN (0, 1)),
        postable_before_children INTEGER CHECK(postable_before_children IN (0, 1)),
        created_at  TEXT NOT NULL DEFAULT (datetime('now', 'localtime')),
        updated_at  TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
    );

    CREATE INDEX IF NOT EXISTS idx_accounts_group_code ON accounts(group_code);
    CREATE INDEX IF NOT EXISTS idx_accounts_parent_code ON accounts(parent_code);
    CREATE INDEX IF NOT EXISTS idx_accounts_level ON accounts(level);

    CREATE TABLE IF NOT EXISTS vouchers (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        number      INTEGER NOT NULL UNIQUE,
        date        TEXT NOT NULL,
        type        TEXT NOT NULL DEFAULT 'سند حسابداری عادی',
        description TEXT NOT NULL DEFAULT '',
        created_at  TEXT NOT NULL DEFAULT (datetime('now', 'localtime')),
        updated_at  TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
    );

    CREATE INDEX IF NOT EXISTS idx_vouchers_date ON vouchers(date);
    CREATE INDEX IF NOT EXISTS idx_vouchers_number ON vouchers(number);

    CREATE TABLE IF NOT EXISTS voucher_lines (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        voucher_id   INTEGER NOT NULL REFERENCES vouchers(id) ON DELETE CASCADE,
        row_no       INTEGER NOT NULL,
        account_code TEXT NOT NULL REFERENCES accounts(code) ON UPDATE CASCADE,
        description  TEXT NOT NULL DEFAULT '',
        debit        INTEGER NOT NULL DEFAULT 0 CHECK(debit >= 0),
        credit       INTEGER NOT NULL DEFAULT 0 CHECK(credit >= 0)
    );

    CREATE INDEX IF NOT EXISTS idx_voucher_lines_voucher ON voucher_lines(voucher_id);
    CREATE INDEX IF NOT EXISTS idx_voucher_lines_account ON voucher_lines(account_code);

    CREATE TRIGGER IF NOT EXISTS vouchers_positive_number_insert
    BEFORE INSERT ON vouchers
    WHEN NEW.number <= 0
    BEGIN
        SELECT RAISE(ABORT, 'voucher number must be positive');
    END;

    CREATE TRIGGER IF NOT EXISTS vouchers_positive_number_update
    BEFORE UPDATE OF number ON vouchers
    WHEN NEW.number <= 0
    BEGIN
        SELECT RAISE(ABORT, 'voucher number must be positive');
    END;

    CREATE TRIGGER IF NOT EXISTS voucher_lines_valid_insert
    BEFORE INSERT ON voucher_lines
    WHEN NEW.row_no <= 0
      OR NOT ((NEW.debit > 0 AND NEW.credit = 0) OR (NEW.credit > 0 AND NEW.debit = 0))
      OR EXISTS (
          SELECT 1 FROM voucher_lines
           WHERE voucher_id = NEW.voucher_id AND row_no = NEW.row_no
      )
    BEGIN
        SELECT RAISE(ABORT, 'invalid voucher line');
    END;

    CREATE TRIGGER IF NOT EXISTS voucher_lines_valid_update
    BEFORE UPDATE ON voucher_lines
    WHEN NEW.row_no <= 0
      OR NOT ((NEW.debit > 0 AND NEW.credit = 0) OR (NEW.credit > 0 AND NEW.debit = 0))
      OR EXISTS (
          SELECT 1 FROM voucher_lines
           WHERE voucher_id = NEW.voucher_id AND row_no = NEW.row_no AND id <> OLD.id
      )
    BEGIN
        SELECT RAISE(ABORT, 'invalid voucher line');
    END;

    INSERT OR IGNORE INTO app_meta(key, value) VALUES ('schema_version', '3');
"#;

const DEFAULT_SETTINGS: [(&str, &str); 2] =
    [("company_name", "شرکت نمونه"), ("fiscal_year", "۱۴۰۵")];

/// The starter chart used on first launch, so the application can be used
/// immediately instead of opening on an empty account list.
const DEFAULT_GROUPS: [(&str, &str, &str); 11] = [
    ("1", "دارایی‌های جاری", "دارایی"),
    ("2", "دارایی‌های غیر جاری", "دارایی"),
    ("3", "سایر دارایی‌ها", "دارایی"),
    ("4", "بدهی‌های جاری", "بدهی"),
    ("5", "بدهی‌های غیر جاری", "بدهی"),
    ("6", "حقوق مالکانه", "سرمایه"),
    ("7", "فروش و درآمدها", "درآمد"),
    ("8", "هزینه‌های فعالیت", "هزینه"),
    ("9", "بهای تمام شده فروش", "سایر"),
    ("10", "سایر درآمدها و هزینه‌های عملیاتی", "سایر"),
    ("11", "سایر درآمدها و هزینه‌های غیر عملیاتی", "سایر"),
];

/// (code, name, level, group code, nature, parent code, postable)
const DEFAULT_ACCOUNTS: [(&str, &str, i64, &str, &str, &str, bool); 34] = [
    ("1101", "صندوق", 1, "1", "debtor", "", false),
    ("1102", "بانک‌ها", 1, "1", "debtor", "", false),
    ("1103", "تنخواه گردان‌ها", 1, "1", "debtor", "", false),
    (
        "1201",
        "حساب‌های دریافتنی تجاری",
        1,
        "1",
        "debtor",
        "",
        false,
    ),
    ("1301", "موجودی مواد و کالا", 1, "1", "debtor", "", false),
    (
        "2101",
        "حساب‌های پرداختنی تجاری",
        1,
        "4",
        "creditor",
        "",
        false,
    ),
    ("2201", "مالیات پرداختنی", 1, "4", "creditor", "", false),
    ("3101", "سرمایه", 1, "6", "creditor", "", false),
    ("4101", "درآمد فروش", 1, "7", "creditor", "", false),
    (
        "5101",
        "بهای تمام‌شده کالای فروش‌رفته",
        1,
        "9",
        "debtor",
        "",
        false,
    ),
    (
        "6101",
        "هزینه‌های عمومی و اداری",
        1,
        "8",
        "debtor",
        "",
        false,
    ),
    ("110101", "صندوق مرکزی", 2, "1", "debtor", "1101", true),
    ("110201", "بانک ملت", 2, "1", "debtor", "1102", false),
    ("110202", "بانک ملی ایران", 2, "1", "debtor", "1102", true),
    ("120101", "بدهکاران تجاری", 2, "1", "debtor", "1201", false),
    ("130101", "مواد اولیه", 2, "1", "debtor", "1301", true),
    (
        "210101",
        "بستانکاران تجاری",
        2,
        "4",
        "creditor",
        "2101",
        false,
    ),
    (
        "220101",
        "مالیات بر ارزش افزوده",
        2,
        "4",
        "creditor",
        "2201",
        true,
    ),
    ("310101", "سرمایه اولیه", 2, "6", "creditor", "3101", true),
    ("410101", "فروش محصولات", 2, "7", "creditor", "4101", true),
    ("610101", "حقوق و دستمزد", 2, "8", "debtor", "6101", true),
    ("610102", "اجاره", 2, "8", "debtor", "6101", true),
    (
        "110201001",
        "حساب جاری بانک ملت",
        3,
        "1",
        "debtor",
        "110201",
        true,
    ),
    (
        "110201002",
        "حساب سپرده کوتاه‌مدت بانک ملت",
        3,
        "1",
        "debtor",
        "110201",
        true,
    ),
    (
        "120101001",
        "شرکت پخش آریا",
        3,
        "1",
        "debtor",
        "120101",
        true,
    ),
    (
        "120101002",
        "شرکت بازرگانی پارس",
        3,
        "1",
        "debtor",
        "120101",
        true,
    ),
    (
        "210101001",
        "شرکت تأمین قطعات آریا",
        3,
        "4",
        "creditor",
        "210101",
        true,
    ),
    (
        "510101",
        "مواد و دستمزد مستقیم بهای تمام‌شده",
        2,
        "9",
        "debtor",
        "5101",
        true,
    ),
    (
        "7101",
        "سایر درآمدها و هزینه‌های عملیاتی",
        1,
        "10",
        "dual",
        "",
        false,
    ),
    (
        "710101",
        "درآمد خدمات جانبی",
        2,
        "10",
        "creditor",
        "7101",
        true,
    ),
    (
        "710102",
        "هزینه عملیاتی متفرقه",
        2,
        "10",
        "debtor",
        "7101",
        true,
    ),
    (
        "8101",
        "سایر درآمدها و هزینه‌های غیرعملیاتی",
        1,
        "11",
        "dual",
        "",
        false,
    ),
    (
        "810101",
        "درآمد غیرعملیاتی",
        2,
        "11",
        "creditor",
        "8101",
        true,
    ),
    (
        "810102",
        "هزینه غیرعملیاتی",
        2,
        "11",
        "debtor",
        "8101",
        true,
    ),
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountGroup {
    pub id: i64,
    pub code: String,
    pub name: String,
    #[serde(rename = "type")]
    pub group_type: String,
    pub account_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountGroupList {
    pub groups: Vec<AccountGroup>,
    pub database_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: i64,
    pub code: String,
    pub name: String,
    pub level: i64,
    pub group_code: String,
    pub group_name: String,
    pub group_type: String,
    pub nature: String,
    pub parent_code: String,
    pub parent_name: String,
    pub is_postable: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountList {
    pub accounts: Vec<Account>,
    pub database_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub company_name: String,
    pub fiscal_year: String,
    pub today: String,
}

/// Holds the single write connection. SQLite access is cheap and the forms
/// have very little traffic, so one mutex-guarded connection keeps ordering
/// guarantees simple.
#[derive(Default)]
pub struct Database {
    conn: Mutex<Option<Connection>>,
    path: Mutex<Option<PathBuf>>,
    /// Set when a restore failed and its safety copy could not be rolled back.
    /// Every later read/write is refused so an uncertain database is never used.
    failed: Mutex<Option<String>>,
}

/// A validated-at-save account definition used by batch creation.
pub struct NewAccount {
    pub code: String,
    pub name: String,
    pub level: i64,
    pub group_code: String,
    pub nature: String,
    pub parent_code: String,
    pub is_postable: bool,
}

impl NewAccount {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        code: &str,
        name: &str,
        level: i64,
        group_code: &str,
        nature: &str,
        parent_code: &str,
        is_postable: bool,
    ) -> Self {
        Self {
            code: code.to_string(),
            name: name.to_string(),
            level,
            group_code: group_code.to_string(),
            nature: nature.to_string(),
            parent_code: parent_code.to_string(),
            is_postable,
        }
    }
}

fn insert_account(conn: &Connection, account: &NewAccount) -> Result<Account, String> {
    let group_exists = conn
        .query_row(
            "SELECT 1 FROM account_groups WHERE code = ?1",
            params![account.group_code],
            |_| Ok(()),
        )
        .optional()
        .map_err(db_error)?
        .is_some();
    if !group_exists {
        return Err("گروه حساب انتخاب‌شده وجود ندارد.".to_string());
    }

    let duplicate = conn
        .query_row(
            "SELECT 1 FROM accounts WHERE code = ?1",
            params![account.code],
            |_| Ok(()),
        )
        .optional()
        .map_err(db_error)?;
    if duplicate.is_some() {
        return Err("این کد حساب قبلاً ثبت شده است.".to_string());
    }

    let (parent_code, parent_name) = resolve_parent(
        conn,
        &account.code,
        account.level,
        &account.parent_code,
        &account.group_code,
    )?;

    // An account that already carries vouchers cannot also become a heading
    // for sub-accounts: its balance would be counted twice in the ledger.
    if let Some(parent) = &parent_code {
        if account_has_lines(conn, parent)? {
            return Err(
                "حساب بالاتر در اسناد استفاده شده است؛ برای آن زیرحساب تعریف نمی‌شود."
                    .to_string(),
            );
        }
    }

    let postable = account.level > 1 && account.is_postable;
    conn.execute(
        "INSERT INTO accounts
            (code, name, level, group_code, nature, parent_code, parent_name, is_postable)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            account.code,
            account.name,
            account.level,
            account.group_code,
            account.nature,
            parent_code,
            parent_name,
            i64::from(postable)
        ],
    )
    .map_err(db_error)?;
    let new_id = conn.last_insert_rowid();

    // The parent is now a heading and can no longer receive vouchers.
    if let Some(parent) = &parent_code {
        conn.execute(
            "UPDATE accounts
                SET postable_before_children = COALESCE(postable_before_children, is_postable),
                    is_postable = 0,
                    updated_at = datetime('now', 'localtime')
              WHERE code = ?1",
            params![parent],
        )
        .map_err(db_error)?;
    }

    fetch_account(conn, new_id)
}

impl Database {
    pub fn attach(&self, connection: Connection, path: PathBuf) {
        // A panic in an earlier database operation must not make the
        // application panic again while attaching/reopening its connection.
        *self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(connection);
        *self
            .path
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(path);
    }

    pub fn path_string(&self) -> String {
        self.path
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    pub fn backup_to(&self, destination: &Path) -> Result<(), String> {
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
                // Do not publish a snapshot that is structurally valid SQLite
                // but already contains invalid accounting data or references.
                validate_backup_connection(&target)
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
        validate_backup_connection(&source)?;
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
            .and_then(|()| validate_backup_connection(target))
            .and_then(|()| reset_schema_triggers(target));
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
    pub(crate) fn with_conn<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, String>,
    ) -> Result<T, String> {
        if let Some(reason) = self
            .failed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            return Err(reason.clone());
        }
        // The connection remains owned and usable after mutex poisoning; a
        // panicking caller will have dropped any in-flight SQLite transaction.
        let mut guard = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let conn = guard
            .as_mut()
            .ok_or_else(|| "پایگاه داده هنوز آماده نشده است.".to_string())?;
        f(conn)
    }

    /* ── Account groups ───────────────────────────────────────── */

    pub fn list_account_groups(&self) -> Result<AccountGroupList, String> {
        let groups = self.with_conn(|conn| list_groups(conn))?;
        Ok(AccountGroupList {
            groups,
            database_path: self.path_string(),
        })
    }

    pub fn create_account_group(
        &self,
        code: &str,
        name: &str,
        group_type: &str,
    ) -> Result<AccountGroup, String> {
        let code = validate_group_code(code)?;
        let name = validate_name(name, "نام گروه")?;
        let group_type = validate_type(group_type)?;

        self.with_conn(|conn| {
            let duplicate = conn
                .query_row(
                    "SELECT id FROM account_groups WHERE code = ?1",
                    params![code],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(db_error)?;
            if duplicate.is_some() {
                return Err("این کد گروه قبلاً ثبت شده است.".to_string());
            }

            conn.execute(
                "INSERT INTO account_groups (code, name, type) VALUES (?1, ?2, ?3)",
                params![code, name, group_type],
            )
            .map_err(db_error)?;
            fetch_group(conn, conn.last_insert_rowid())
        })
    }

    pub fn update_account_group(
        &self,
        id: i64,
        code: &str,
        name: &str,
        group_type: &str,
    ) -> Result<AccountGroup, String> {
        if id <= 0 {
            return Err("شناسه گروه حساب نامعتبر است.".to_string());
        }
        let code = validate_group_code(code)?;
        let name = validate_name(name, "نام گروه")?;
        let group_type = validate_type(group_type)?;

        self.with_conn(|conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            let existing = tx
                .query_row(
                    "SELECT code, type FROM account_groups WHERE id = ?1",
                    params![id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(db_error)?;
            let (existing_code, existing_type) =
                existing.ok_or_else(|| "گروه حساب یافت نشد.".to_string())?;
            if existing_code != code {
                return Err("تغییر کد گروه حساب مجاز نیست.".to_string());
            }
            if existing_type != group_type {
                let used_lines: i64 = tx
                    .query_row(
                        "SELECT COUNT(*)
                           FROM voucher_lines l
                           JOIN accounts a ON a.code = l.account_code
                          WHERE a.group_code = ?1",
                        params![code],
                        |row| row.get(0),
                    )
                    .map_err(db_error)?;
                if used_lines > 0 {
                    return Err(
                        "نوع این گروه در اسناد ثبت‌شده استفاده شده است؛ تغییر آن می‌تواند گزارش‌های مالی دوره‌های قبل را تغییر دهد.".to_string(),
                    );
                }
            }

            tx.execute(
                "UPDATE account_groups
                    SET name = ?1, type = ?2, updated_at = datetime('now', 'localtime')
                  WHERE id = ?3",
                params![name, group_type, id],
            )
            .map_err(db_error)?;
            let group = fetch_group(&tx, id)?;
            tx.commit().map_err(db_error)?;
            Ok(group)
        })
    }

    pub fn delete_account_group(&self, id: i64) -> Result<(), String> {
        if id <= 0 {
            return Err("شناسه گروه حساب نامعتبر است.".to_string());
        }

        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction().map_err(db_error)?;

            let code = tx
                .query_row(
                    "SELECT code FROM account_groups WHERE id = ?1",
                    params![id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(db_error)?;
            let code = code.ok_or_else(|| "گروه حساب با این شناسه یافت نشد.".to_string())?;

            let total: i64 = tx
                .query_row("SELECT COUNT(*) FROM account_groups", [], |row| row.get(0))
                .map_err(db_error)?;
            if total <= 1 {
                return Err(
                    "حداقل یک گروه حساب باید وجود داشته باشد؛ حذف آخرین گروه مجاز نیست."
                        .to_string(),
                );
            }

            let accounts: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM accounts WHERE group_code = ?1",
                    params![code],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            if accounts > 0 {
                return Err(format!(
                    "این گروه دارای {} حساب است. ابتدا حساب‌ها را حذف یا جابجا کنید.",
                    to_fa_digits(accounts)
                ));
            }

            tx.execute("DELETE FROM account_groups WHERE id = ?1", params![id])
                .map_err(db_error)?;
            tx.commit().map_err(db_error)?;
            Ok(())
        })
    }

    /* ── Chart of accounts ────────────────────────────────────── */

    pub fn list_accounts(&self) -> Result<AccountList, String> {
        let accounts = self.with_conn(list_accounts)?;
        Ok(AccountList {
            accounts,
            database_path: self.path_string(),
        })
    }

    pub fn create_account(
        &self,
        code: &str,
        name: &str,
        level: i64,
        group_code: &str,
        nature: &str,
        parent_code: &str,
        is_postable: bool,
    ) -> Result<Account, String> {
        let mut created = self.create_accounts_atomic(vec![NewAccount::new(
            code,
            name,
            level,
            group_code,
            nature,
            parent_code,
            is_postable,
        )])?;
        created
            .pop()
            .ok_or_else(|| "ساخت حساب انجام نشد.".to_string())
    }

    /// Creates a group of accounts in one transaction. If any account in the
    /// batch fails validation or insertion, none of them is saved. This keeps
    /// an imported hierarchy level from being left behind half-created.
    pub fn create_accounts_atomic(&self, accounts: Vec<NewAccount>) -> Result<Vec<Account>, String> {
        if accounts.is_empty() {
            return Ok(Vec::new());
        }
        let mut normalized = Vec::with_capacity(accounts.len());
        for account in accounts {
            normalized.push(NewAccount {
                code: validate_account_code(&account.code)?,
                name: validate_name(&account.name, "نام حساب")?,
                level: validate_level(account.level)?,
                group_code: normalize_code(&account.group_code),
                nature: validate_nature(&account.nature)?,
                parent_code: normalize_code(&account.parent_code),
                is_postable: account.is_postable,
            });
        }

        self.with_conn(move |conn| {
            let tx = conn.transaction().map_err(db_error)?;
            let mut created = Vec::with_capacity(normalized.len());
            for account in &normalized {
                created.push(insert_account(&tx, account)?);
            }
            tx.commit().map_err(db_error)?;
            Ok(created)
        })
    }

    pub fn update_account(
        &self,
        id: i64,
        name: &str,
        group_code: &str,
        nature: &str,
        parent_code: &str,
        is_postable: bool,
    ) -> Result<Account, String> {
        if id <= 0 {
            return Err("شناسه حساب نامعتبر است.".to_string());
        }
        let name = validate_name(name, "نام حساب")?;
        let nature = validate_nature(nature)?;
        let group_code = normalize_code(group_code);
        let parent_code = normalize_code(parent_code);

        self.with_conn(|conn| {
            let tx = conn.transaction().map_err(db_error)?;
            let existing = tx
                .query_row(
                    "SELECT code, level, group_code, COALESCE(parent_code, '') FROM accounts WHERE id = ?1",
                    params![id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )
                .optional()
                .map_err(db_error)?
                .ok_or_else(|| "حساب یافت نشد.".to_string())?;

            let (code, level, old_group, old_parent_code) = existing;

            let group_exists = tx
                .query_row(
                    "SELECT 1 FROM account_groups WHERE code = ?1",
                    params![group_code],
                    |_| Ok(()),
                )
                .optional()
                .map_err(db_error)?
                .is_some();
            if !group_exists {
                return Err("گروه حساب انتخاب‌شده وجود ندارد.".to_string());
            }

            let children: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM accounts WHERE parent_code = ?1",
                    params![code],
                    |row| row.get(0),
                )
                .map_err(db_error)?;

            if children > 0 && old_group != group_code {
                return Err(
                    "این حساب دارای زیرحساب است؛ ابتدا گروه زیرحساب‌ها را هماهنگ کنید."
                        .to_string(),
                );
            }
            let has_history = account_subtree_has_lines(&tx, &code)?;
            if has_history && (old_group != group_code || old_parent_code != parent_code) {
                return Err(
                    "این حساب در اسناد ثبت‌شده استفاده شده است؛ تغییر گروه یا والد آن می‌تواند طبقه‌بندی و مانده گزارش‌های تاریخی را تغییر دهد. ابتدا حساب اصلاحی یا ساختار جایگزین تعریف کنید."
                        .to_string(),
                );
            }

            let (parent_code, parent_name) =
                resolve_parent(&tx, &code, level, &parent_code, &group_code)?;
            let new_parent_code = parent_code.as_deref().unwrap_or("");
            if old_parent_code != new_parent_code
                && !new_parent_code.is_empty()
                && account_has_lines(&tx, new_parent_code)?
            {
                return Err(
                    "حساب بالاتر در اسناد استفاده شده است؛ این حساب به‌عنوان زیرحساب انتخاب نمی‌شود."
                        .to_string(),
                );
            }
            let postable = level > 1 && is_postable && children == 0;
            if !postable && level > 1 && children == 0 && account_has_lines(&tx, &code)? {
                return Err(
                    "این حساب در اسناد استفاده شده است و باید قابل ثبت بماند.".to_string(),
                );
            }

            tx.execute(
                "UPDATE accounts
                    SET name = ?1, group_code = ?2, nature = ?3,
                        parent_code = ?4, parent_name = ?5, is_postable = ?6,
                        postable_before_children = CASE
                            WHEN ?8 = 1 THEN COALESCE(postable_before_children, ?9)
                            ELSE NULL
                        END,
                        updated_at = datetime('now', 'localtime')
                  WHERE id = ?7",
                params![
                    name,
                    group_code,
                    nature,
                    parent_code,
                    parent_name,
                    i64::from(postable),
                    id,
                    i64::from(children > 0),
                    i64::from(is_postable)
                ],
            )
            .map_err(db_error)?;

            // parent_name is denormalized for fast hierarchy rendering. Keep
            // direct children in sync with the parent's new display name.
            tx.execute(
                "UPDATE accounts
                    SET parent_name = ?1, updated_at = datetime('now', 'localtime')
                  WHERE parent_code = ?2",
                params![name, code],
            )
            .map_err(db_error)?;

            if old_parent_code != new_parent_code {
                if !old_parent_code.is_empty() {
                    let remaining_children: i64 = tx
                        .query_row(
                            "SELECT COUNT(*) FROM accounts WHERE parent_code = ?1",
                            params![old_parent_code],
                            |row| row.get(0),
                        )
                        .map_err(db_error)?;
                    if remaining_children == 0 {
                        tx.execute(
                            "UPDATE accounts
                                SET is_postable = COALESCE(postable_before_children, is_postable),
                                    postable_before_children = NULL,
                                    updated_at = datetime('now', 'localtime')
                              WHERE code = ?1",
                            params![old_parent_code],
                        )
                        .map_err(db_error)?;
                    }
                }

                if !new_parent_code.is_empty() {
                    tx.execute(
                        "UPDATE accounts
                            SET postable_before_children = COALESCE(postable_before_children, is_postable),
                                is_postable = 0,
                                updated_at = datetime('now', 'localtime')
                          WHERE code = ?1",
                        params![new_parent_code],
                    )
                    .map_err(db_error)?;
                }
            }

            let updated = fetch_account(&tx, id)?;
            tx.commit().map_err(db_error)?;
            Ok(updated)
        })
    }

    pub fn delete_account(&self, id: i64) -> Result<(), String> {
        if id <= 0 {
            return Err("شناسه حساب نامعتبر است.".to_string());
        }

        self.with_conn(|conn| {
            let tx = conn.transaction().map_err(db_error)?;
            let (code, parent_code) = tx
                .query_row(
                    "SELECT code, COALESCE(parent_code, '') FROM accounts WHERE id = ?1",
                    params![id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(db_error)?
                .ok_or_else(|| "حساب با این شناسه یافت نشد.".to_string())?;

            let children: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM accounts WHERE parent_code = ?1",
                    params![code],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            if children > 0 {
                return Err("این حساب دارای زیرحساب است؛ ابتدا زیرحساب‌ها را حذف کنید.".to_string());
            }

            if account_has_lines(&tx, &code)? {
                return Err("این حساب در اسناد حسابداری استفاده شده و قابل حذف نیست.".to_string());
            }

            tx.execute("DELETE FROM accounts WHERE id = ?1", params![id])
                .map_err(db_error)?;

            if !parent_code.is_empty() {
                let remaining_children: i64 = tx
                    .query_row(
                        "SELECT COUNT(*) FROM accounts WHERE parent_code = ?1",
                        params![parent_code],
                        |row| row.get(0),
                    )
                    .map_err(db_error)?;
                if remaining_children == 0 {
                    tx.execute(
                        "UPDATE accounts
                            SET is_postable = COALESCE(postable_before_children, is_postable),
                                postable_before_children = NULL,
                                updated_at = datetime('now', 'localtime')
                          WHERE code = ?1",
                        params![parent_code],
                    )
                    .map_err(db_error)?;
                }
            }

            tx.commit().map_err(db_error)?;
            Ok(())
        })
    }

    /* ── Settings ─────────────────────────────────────────────── */

    pub fn get_settings(&self) -> Result<Settings, String> {
        self.with_conn(|conn| {
            let company_name =
                get_setting(conn, "company_name")?.unwrap_or_else(|| "شرکت نمونه".to_string());
            let fiscal_year =
                get_setting(conn, "fiscal_year")?.unwrap_or_else(|| "۱۴۰۵".to_string());
            Ok(Settings {
                company_name,
                fiscal_year,
                today: today_jalali(),
            })
        })
    }

    pub fn save_settings(&self, company_name: &str, fiscal_year: &str) -> Result<Settings, String> {
        let company_name = validate_name(company_name, "نام شرکت")?;
        let fiscal_year = fiscal_year.trim();
        if fiscal_year.chars().count() > 40 {
            return Err("عنوان سال مالی نمی‌تواند بیشتر از ۴۰ کاراکتر باشد.".to_string());
        }

        self.with_conn(|conn| {
            let tx = conn.transaction().map_err(db_error)?;
            set_setting(&tx, "company_name", &company_name)?;
            set_setting(&tx, "fiscal_year", fiscal_year)?;
            tx.commit().map_err(db_error)?;
            Ok(Settings {
                company_name,
                fiscal_year: fiscal_year.to_string(),
                today: today_jalali(),
            })
        })
    }
}

/// Opens (and creates when needed) the application database.
pub fn initialize(app: &AppHandle) -> Result<(Connection, PathBuf), String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("مسیر داده برنامه در دسترس نیست: {e}"))?;
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| format!("ساخت پوشه پایگاه داده ناموفق بود: {e}"))?;

    let path = data_dir.join(DB_FILE_NAME);

    // Bring along data from the previous application versions so an existing
    // chart of accounts does not disappear after the rename.
    migrate_legacy_database(&data_dir, &path)?;

    let connection =
        Connection::open(&path).map_err(|e| format!("باز کردن پایگاه داده ناموفق بود: {e}"))?;

    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )
        .map_err(db_error)?;
    connection.execute_batch(SCHEMA).map_err(db_error)?;
    apply_schema_migrations(&connection)?;
    seed_default_groups(&connection).map_err(db_error)?;
    seed_default_accounts(&connection).map_err(db_error)?;
    seed_default_settings(&connection).map_err(db_error)?;

    Ok((connection, path))
}

/// Copies the previous application's database into `destination` once. The
/// old file may still have most of its data in a `-wal` sidecar, so copying
/// only the main file would silently lose it. `VACUUM INTO` writes a single
/// consistent file that includes everything the source connection can see.
fn migrate_legacy_database(data_dir: &Path, destination: &Path) -> Result<(), String> {
    if destination.exists() {
        return Ok(());
    }

    let mut candidates = vec![data_dir.join(LEGACY_DB_FILE_NAME)];
    if let Some(parent) = data_dir.parent() {
        candidates.push(parent.join(LEGACY_IDENTIFIER).join(LEGACY_DB_FILE_NAME));
    }

    let mut existing: Vec<PathBuf> = candidates
        .into_iter()
        .filter(|path| path.exists())
        .collect();
    if existing.is_empty() {
        return Ok(());
    }
    // Prefer the most recently written database; a stale copy in the old
    // identifier folder must not win over newer data in the app data folder.
    existing.sort_by(|left, right| {
        let modified = |path: &Path| std::fs::metadata(path).and_then(|meta| meta.modified()).ok();
        modified(right).cmp(&modified(left))
    });

    let temporary = destination.with_extension("sqlite3.migrating");
    let mut failures = Vec::new();
    for source in existing {
        let _ = std::fs::remove_file(&temporary);
        match copy_sqlite_database(&source, &temporary) {
            Ok(()) => {
                std::fs::rename(&temporary, destination).map_err(|error| {
                    let _ = std::fs::remove_file(&temporary);
                    format!(
                        "پایگاه داده قدیمی کپی شد اما جایگزینی امن فایل مقصد ناموفق بود: {error}"
                    )
                })?;
                return Ok(());
            }
            Err(error) => {
                let _ = std::fs::remove_file(&temporary);
                failures.push(format!("{}: {}", source.display(), error));
            }
        }
    }

    Err(format!(
        "انتقال پایگاه داده قدیمی ناموفق بود؛ برای جلوگیری از ساخت پایگاه داده خالی و پنهان‌شدن اطلاعات قبلی، اجرای برنامه متوقف شد. جزئیات: {}",
        failures.join(" | ")
    ))
}

/// Applies additive, versioned schema changes. Migration 3 remembers whether
/// a parent account was manually postable before its first child was added.
fn apply_schema_migrations(conn: &Connection) -> Result<(), String> {
    let version_text: String = conn
        .query_row(
            "SELECT value FROM app_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    let version = version_text
        .parse::<i64>()
        .map_err(|_| "نسخه ساختار پایگاه داده معتبر نیست.".to_string())?;
    if version > CURRENT_SCHEMA_VERSION {
        return Err(format!(
            "نسخه پایگاه داده ({}) از نسخه قابل پشتیبانی برنامه جدیدتر است.",
            version
        ));
    }

    // DDL and the version marker must move together. If any step fails, the
    // transaction is rolled back instead of leaving a half-applied migration.
    let tx = conn.unchecked_transaction().map_err(db_error)?;
    let columns = {
        let mut statement = tx
            .prepare("PRAGMA table_info(accounts)")
            .map_err(db_error)?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(db_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error)?;
        columns
    };
    if !columns
        .iter()
        .any(|column| column == "postable_before_children")
    {
        tx.execute(
            "ALTER TABLE accounts ADD COLUMN postable_before_children INTEGER CHECK(postable_before_children IN (0, 1))",
            [],
        )
        .map_err(db_error)?;
    }
    if version < CURRENT_SCHEMA_VERSION {
        tx.execute(
            "UPDATE app_meta SET value = ?1 WHERE key = 'schema_version'",
            params![CURRENT_SCHEMA_VERSION.to_string()],
        )
        .map_err(db_error)?;
    }
    tx.commit().map_err(db_error)
}

fn copy_sqlite_database(source: &Path, destination: &Path) -> Result<(), String> {
    let connection = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "VACUUM INTO ?1",
            params![destination.to_string_lossy().into_owned()],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

const SCHEMA_TRIGGERS: [&str; 4] = [
    "vouchers_positive_number_insert",
    "vouchers_positive_number_update",
    "voucher_lines_valid_insert",
    "voucher_lines_valid_update",
];

/// `SCHEMA` uses `CREATE TRIGGER IF NOT EXISTS`, so a restored database could
/// keep a tampered trigger that happens to use an allowed name. Drop the
/// known triggers first and recreate them from the application's own schema.
fn reset_schema_triggers(conn: &Connection) -> Result<(), String> {
    for trigger in SCHEMA_TRIGGERS {
        conn.execute(&format!("DROP TRIGGER IF EXISTS \"{trigger}\""), [])
            .map_err(db_error)?;
    }
    conn.execute_batch(SCHEMA).map_err(db_error)
}

fn validate_backup_connection(conn: &Connection) -> Result<(), String> {
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
                                        'voucher_lines_valid_update'
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
    if version.parse::<i64>().ok() != Some(CURRENT_SCHEMA_VERSION) {
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
            &["id", "code", "name", "type", "created_at", "updated_at"],
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
                "postable_before_children",
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

    if let Some(code) = backup_first_text(
        conn,
        "SELECT code FROM accounts
          WHERE code = '' OR length(code) > 20 OR code GLOB '*[^0-9]*' OR trim(code, '0') = ''
          LIMIT 1",
    )? {
        return Err(format!("کد حساب «{code}» در فایل پشتیبان معتبر نیست."));
    }
    if let Some(code) = backup_first_text(
        conn,
        "SELECT code FROM accounts
          WHERE level NOT BETWEEN 1 AND 4
             OR is_postable NOT IN (0, 1)
             OR nature NOT IN ('debtor', 'creditor', 'dual')
          LIMIT 1",
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
             OR a.code NOT LIKE p.code || '%'
             OR length(a.code) <= length(p.code)
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
          WHERE account_code = '' OR length(account_code) > 20
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
    if let Some(line) = backup_first_text(
        conn,
        "SELECT CAST(l.id AS TEXT)
           FROM voucher_lines l
           LEFT JOIN accounts a ON a.code = l.account_code
          WHERE a.code IS NULL
             OR a.is_postable = 0
             OR EXISTS (SELECT 1 FROM accounts child WHERE child.parent_code = a.code)
          LIMIT 1",
    )? {
        return Err(format!(
            "حساب ردیف {line} در فایل پشتیبان وجود ندارد یا قابل ثبت نیست."
        ));
    }
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

fn seed_default_groups(conn: &Connection) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    let count: i64 = tx.query_row("SELECT COUNT(*) FROM account_groups", [], |row| row.get(0))?;
    if count > 0 {
        return Ok(());
    }

    let mut stmt =
        tx.prepare("INSERT OR IGNORE INTO account_groups (code, name, type) VALUES (?1, ?2, ?3)")?;
    for (code, name, group_type) in DEFAULT_GROUPS {
        stmt.execute(params![code, name, group_type])?;
    }
    drop(stmt);
    tx.commit()?;
    Ok(())
}

fn seed_default_accounts(conn: &Connection) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    let count: i64 = tx.query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))?;
    if count > 0 {
        return Ok(());
    }

    for (code, name, level, group_code, nature, parent_code, postable) in DEFAULT_ACCOUNTS {
        let group_exists: bool = tx
            .query_row(
                "SELECT 1 FROM account_groups WHERE code = ?1",
                params![group_code],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if !group_exists {
            continue;
        }

        let parent_name: String = if parent_code.is_empty() {
            String::new()
        } else {
            tx.query_row(
                "SELECT name FROM accounts WHERE code = ?1",
                params![parent_code],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or_default()
        };
        if !parent_code.is_empty() && parent_name.is_empty() {
            continue;
        }

        tx.execute(
            "INSERT OR IGNORE INTO accounts
                (code, name, level, group_code, nature, parent_code, parent_name, is_postable)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                code,
                name,
                level,
                group_code,
                nature,
                if parent_code.is_empty() {
                    None
                } else {
                    Some(parent_code)
                },
                parent_name,
                i64::from(postable)
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn seed_default_settings(conn: &Connection) -> rusqlite::Result<()> {
    for (key, value) in DEFAULT_SETTINGS {
        conn.execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
    }
    Ok(())
}

fn list_groups(conn: &Connection) -> Result<Vec<AccountGroup>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT g.id, g.code, g.name, g.type, g.created_at, g.updated_at,
                    (SELECT COUNT(*) FROM accounts a WHERE a.group_code = g.code)
               FROM account_groups g
              ORDER BY CAST(g.code AS INTEGER), g.code",
        )
        .map_err(db_error)?;
    let rows = stmt.query_map([], row_to_group).map_err(db_error)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)
}

fn list_accounts(conn: &mut Connection) -> Result<Vec<Account>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.code, a.name, a.level, a.group_code, g.name, g.type,
                    a.nature, COALESCE(a.parent_code, ''), COALESCE(p.name, ''),
                    CASE WHEN EXISTS(
                        SELECT 1 FROM accounts child WHERE child.parent_code = a.code
                    ) THEN 0 ELSE a.is_postable END,
                    a.created_at, a.updated_at
               FROM accounts a
               JOIN account_groups g ON g.code = a.group_code
               LEFT JOIN accounts p ON p.code = a.parent_code
              ORDER BY a.code",
        )
        .map_err(db_error)?;
    let rows = stmt.query_map([], row_to_account).map_err(db_error)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)
}

fn fetch_group(conn: &Connection, id: i64) -> Result<AccountGroup, String> {
    conn.query_row(
        "SELECT g.id, g.code, g.name, g.type, g.created_at, g.updated_at,
                (SELECT COUNT(*) FROM accounts a WHERE a.group_code = g.code)
           FROM account_groups g
          WHERE g.id = ?1",
        params![id],
        row_to_group,
    )
    .optional()
    .map_err(db_error)?
    .ok_or_else(|| "گروه حساب یافت نشد.".to_string())
}

fn fetch_account(conn: &Connection, id: i64) -> Result<Account, String> {
    conn.query_row(
        "SELECT a.id, a.code, a.name, a.level, a.group_code, g.name, g.type,
                a.nature, COALESCE(a.parent_code, ''), COALESCE(p.name, ''),
                CASE WHEN EXISTS(
                    SELECT 1 FROM accounts child WHERE child.parent_code = a.code
                ) THEN 0 ELSE a.is_postable END,
                a.created_at, a.updated_at
           FROM accounts a
           JOIN account_groups g ON g.code = a.group_code
           LEFT JOIN accounts p ON p.code = a.parent_code
          WHERE a.id = ?1",
        params![id],
        row_to_account,
    )
    .optional()
    .map_err(db_error)?
    .ok_or_else(|| "حساب یافت نشد.".to_string())
}

fn row_to_group(row: &rusqlite::Row<'_>) -> rusqlite::Result<AccountGroup> {
    Ok(AccountGroup {
        id: row.get(0)?,
        code: row.get(1)?,
        name: row.get(2)?,
        group_type: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        account_count: row.get(6)?,
    })
}

fn row_to_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    let postable: i64 = row.get(10)?;
    Ok(Account {
        id: row.get(0)?,
        code: row.get(1)?,
        name: row.get(2)?,
        level: row.get(3)?,
        group_code: row.get(4)?,
        group_name: row.get(5)?,
        group_type: row.get(6)?,
        nature: row.get(7)?,
        parent_code: row.get(8)?,
        parent_name: row.get(9)?,
        is_postable: postable != 0,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn account_has_lines(conn: &Connection, code: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM voucher_lines WHERE account_code = ?1)",
        params![code],
        |row| row.get(0),
    )
    .map_err(db_error)
}

fn account_subtree_has_lines(conn: &Connection, code: &str) -> Result<bool, String> {
    conn.query_row(
        "WITH RECURSIVE account_tree(code) AS (
            SELECT code FROM accounts WHERE code = ?1
            UNION
            SELECT child.code
              FROM accounts child
              JOIN account_tree parent ON child.parent_code = parent.code
        )
        SELECT EXISTS (
            SELECT 1 FROM voucher_lines WHERE account_code IN (SELECT code FROM account_tree)
        )",
        params![code],
        |row| row.get(0),
    )
    .map_err(db_error)
}

fn resolve_parent(
    conn: &Connection,
    code: &str,
    level: i64,
    parent_code: &str,
    group_code: &str,
) -> Result<(Option<String>, String), String> {
    if level == 1 {
        return Ok((None, String::new()));
    }
    if parent_code.is_empty() {
        return Err("برای این سطح، انتخاب حساب بالاتر الزامی است.".to_string());
    }

    let parent = conn
        .query_row(
            "SELECT name, level, group_code FROM accounts WHERE code = ?1",
            params![parent_code],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(db_error)?
        .ok_or_else(|| "حساب بالاتر انتخاب‌شده وجود ندارد.".to_string())?;

    if parent.1 != level - 1 {
        return Err("سطح حساب بالاتر باید یک سطح کمتر از حساب جاری باشد.".to_string());
    }
    if parent.2 != group_code {
        return Err("حساب بالاتر باید به همان گروه حساب تعلق داشته باشد.".to_string());
    }
    if !code.starts_with(parent_code) || code.len() <= parent_code.len() {
        return Err("کد حساب باید با کد حساب بالاتر آغاز شود.".to_string());
    }

    Ok((Some(parent_code.to_string()), parent.0))
}

pub(crate) fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(db_error)
}

fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(db_error)?;
    Ok(())
}

pub(crate) fn today_jalali() -> String {
    let d = jalali_calendar::JalaliDate::today();
    format!("{:04}/{:02}/{:02}", d.year(), d.month(), d.day())
}

pub(crate) fn db_error(error: rusqlite::Error) -> String {
    format!("خطا در پایگاه داده: {error}")
}

/// Converts Persian/Arabic digits to ASCII so codes work with either keyboard.
pub fn normalize_digits(input: &str) -> String {
    input
        .chars()
        .map(|ch| match ch {
            '\u{06F0}'..='\u{06F9}' => char::from(b'0' + (ch as u32 - 0x06F0) as u8),
            '\u{0660}'..='\u{0669}' => char::from(b'0' + (ch as u32 - 0x0660) as u8),
            _ => ch,
        })
        .collect()
}

pub(crate) fn normalize_code(input: &str) -> String {
    normalize_digits(input).trim().replace(' ', "")
}

fn validate_group_code(input: &str) -> Result<String, String> {
    let code = normalize_code(input);
    if code.is_empty() {
        return Err("کد گروه الزامی است.".to_string());
    }
    if code.chars().count() > 50 {
        return Err("کد گروه نمی‌تواند بیشتر از ۵۰ کاراکتر باشد.".to_string());
    }
    if !code.chars().all(|c| c.is_ascii_digit()) || !code.chars().any(|c| c != '0') {
        return Err("کد گروه باید فقط شامل ارقام انگلیسی و بزرگ‌تر از صفر باشد.".to_string());
    }
    Ok(code)
}

fn validate_account_code(input: &str) -> Result<String, String> {
    let code = normalize_code(input);
    if code.is_empty() {
        return Err("کد حساب الزامی است.".to_string());
    }
    if code.chars().count() > 20 {
        return Err("کد حساب نمی‌تواند بیشتر از ۲۰ رقم باشد.".to_string());
    }
    if !code.chars().all(|c| c.is_ascii_digit()) {
        return Err("کد حساب باید فقط شامل ارقام باشد.".to_string());
    }
    if !code.chars().any(|c| c != '0') {
        return Err("کد حساب باید بزرگ‌تر از صفر باشد.".to_string());
    }
    Ok(code)
}

fn validate_name(input: &str, label: &str) -> Result<String, String> {
    let name = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err(format!("{label} الزامی است."));
    }
    if name.chars().count() > 255 {
        return Err(format!("{label} نمی‌تواند بیشتر از ۲۵۵ کاراکتر باشد."));
    }
    Ok(name)
}

fn validate_level(input: i64) -> Result<i64, String> {
    if (1..=3).contains(&input) {
        Ok(input)
    } else {
        Err("سطح حساب جدید باید یکی از سطوح کل، معین یا تفصیلی ۱ باشد.".to_string())
    }
}

fn validate_nature(input: &str) -> Result<String, String> {
    let value = input.trim();
    if ACCOUNT_NATURES.contains(&value) {
        Ok(value.to_string())
    } else {
        Err("ماهیت حساب انتخاب‌شده معتبر نیست.".to_string())
    }
}

fn validate_type(input: &str) -> Result<String, String> {
    let value = input.trim();
    if ACCOUNT_GROUP_TYPES.contains(&value) {
        Ok(value.to_string())
    } else {
        Err("نوع حساب انتخاب‌شده معتبر نیست.".to_string())
    }
}

pub(crate) fn to_fa_digits(value: impl ToString) -> String {
    value
        .to_string()
        .chars()
        .map(|ch| match ch {
            '0'..='9' => char::from_u32(0x06F0 + (ch as u32 - '0' as u32)).unwrap_or(ch),
            _ => ch,
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn memory_database() -> Database {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    conn.execute_batch(SCHEMA).unwrap();
    seed_default_groups(&conn).unwrap();
    seed_default_accounts(&conn).unwrap();
    seed_default_settings(&conn).unwrap();
    let db = Database::default();
    db.attach(conn, PathBuf::from(":memory:"));
    db
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_database() -> Database {
        memory_database()
    }

    fn file_database(path: &Path) -> Database {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        apply_schema_migrations(&conn).unwrap();
        seed_default_groups(&conn).unwrap();
        seed_default_accounts(&conn).unwrap();
        seed_default_settings(&conn).unwrap();
        let db = Database::default();
        db.attach(conn, path.to_path_buf());
        db
    }

    #[test]
    fn database_recovers_from_poisoned_connection_and_path_mutexes() {
        let db = test_database();

        let path_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = db.path.lock().unwrap();
            panic!("simulate a panic while holding the path mutex");
        }));
        assert!(path_panic.is_err());
        assert_eq!(db.path_string(), ":memory:");

        let connection_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = db.conn.lock().unwrap();
            panic!("simulate a panic while holding the connection mutex");
        }));
        assert!(connection_panic.is_err());
        assert_eq!(db.list_account_groups().unwrap().groups.len(), 11);

        // Reattaching after a poisoned lock must also remain non-panicking.
        db.attach(
            Connection::open_in_memory().unwrap(),
            PathBuf::from(":memory:reopened"),
        );
        assert_eq!(db.path_string(), ":memory:reopened");
    }

    #[test]
    fn backup_refuses_to_publish_a_snapshot_with_invalid_accounting_data() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hesabdari-invalid-backup-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let active_path = dir.join(DB_FILE_NAME);
        let backup_path = dir.join("backup.sqlite3");
        let db = file_database(&active_path);
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO vouchers (number, date, type) VALUES (1, '1405/13/01', ?1)",
                params![crate::vouchers::VOUCHER_TYPES[0]],
            )
            .map_err(db_error)?;
            Ok(())
        })
        .unwrap();

        let error = db.backup_to(&backup_path).unwrap_err();
        assert!(error.contains("تاریخ"));
        assert!(!backup_path.exists());
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sqlite_rejects_invalid_voucher_numbers_and_lines() {
        let db = test_database();
        db.with_conn(|conn| {
            assert!(conn
                .execute(
                    "INSERT INTO vouchers (number, date, type) VALUES (0, '1405/01/01', ?1)",
                    params![crate::vouchers::VOUCHER_TYPES[0]],
                )
                .is_err());
            conn.execute(
                "INSERT INTO vouchers (number, date, type) VALUES (1, '1405/01/01', ?1)",
                params![crate::vouchers::VOUCHER_TYPES[0]],
            )
            .map_err(db_error)?;
            let voucher_id = conn.last_insert_rowid();

            assert!(conn
                .execute(
                    "INSERT INTO voucher_lines (voucher_id, row_no, account_code, debit, credit) VALUES (?1, 1, '110101', 0, 0)",
                    params![voucher_id],
                )
                .is_err());
            conn.execute(
                "INSERT INTO voucher_lines (voucher_id, row_no, account_code, debit, credit) VALUES (?1, 1, '110101', 100, 0)",
                params![voucher_id],
            )
            .map_err(db_error)?;
            assert!(conn
                .execute(
                    "INSERT INTO voucher_lines (voucher_id, row_no, account_code, debit, credit) VALUES (?1, 1, '410101', 0, 100)",
                    params![voucher_id],
                )
                .is_err());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn seeds_default_groups_and_accounts_once() {
        let db = test_database();
        let list = db.list_account_groups().unwrap();
        assert_eq!(list.groups.len(), 11);
        assert_eq!(list.groups[0].code, "1");

        let accounts = db.list_accounts().unwrap();
        assert_eq!(accounts.accounts.len(), 34);
        assert_eq!(accounts.accounts[0].code, "1101");
        assert!(!accounts.accounts[0].is_postable);
        assert!(accounts
            .accounts
            .iter()
            .any(|account| account.code == "510101" && account.is_postable));
        assert!(accounts
            .accounts
            .iter()
            .any(|account| account.code == "710101" && account.is_postable));
        assert!(accounts
            .accounts
            .iter()
            .any(|account| account.code == "810102" && account.is_postable));
    }

    #[test]
    fn legacy_parent_is_reported_non_postable_without_changing_stored_flag() {
        let db = test_database();
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE accounts SET is_postable = 1 WHERE code = '110201'",
                [],
            )
            .map_err(db_error)?;
            Ok(())
        })
        .unwrap();

        let parent = db
            .list_accounts()
            .unwrap()
            .accounts
            .into_iter()
            .find(|account| account.code == "110201")
            .unwrap();
        let stored_flag = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT is_postable FROM accounts WHERE code = '110201'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(db_error)
            })
            .unwrap();

        assert!(!parent.is_postable);
        assert_eq!(stored_flag, 1);
    }

    #[test]
    fn database_backup_restores_data_and_keeps_a_safety_copy() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hesabdari-backup-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let active_path = dir.join(DB_FILE_NAME);
        let backup_path = dir.join("backup.sqlite3");
        let db = file_database(&active_path);
        let group = db
            .create_account_group("90", "گروه پشتیبان", "دارایی")
            .unwrap();

        db.backup_to(&backup_path).unwrap();
        db.delete_account_group(group.id).unwrap();
        assert!(!db
            .list_account_groups()
            .unwrap()
            .groups
            .iter()
            .any(|item| item.code == "90"));

        let safety_path = db.restore_from(&backup_path).unwrap();
        assert!(safety_path.exists());
        assert!(db
            .list_account_groups()
            .unwrap()
            .groups
            .iter()
            .any(|item| item.code == "90"));
        drop(db);

        let invalid_path = dir.join("invalid.sqlite3");
        std::fs::write(&invalid_path, "not a database").unwrap();
        let db = file_database(&active_path);
        assert!(db.restore_from(&invalid_path).is_err());
        let old_schema_path = dir.join("old-schema.sqlite3");
        let old_schema = Connection::open(&old_schema_path).unwrap();
        old_schema.execute_batch(SCHEMA).unwrap();
        old_schema
            .execute(
                "UPDATE app_meta SET value = '2' WHERE key = 'schema_version'",
                [],
            )
            .unwrap();
        drop(old_schema);
        assert!(db
            .restore_from(&old_schema_path)
            .unwrap_err()
            .contains("سازگار نیست"));
        assert!(db
            .list_account_groups()
            .unwrap()
            .groups
            .iter()
            .any(|item| item.code == "90"));
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backup_validation_rejects_inconsistent_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        apply_schema_migrations(&conn).unwrap();
        seed_default_groups(&conn).unwrap();
        seed_default_accounts(&conn).unwrap();
        seed_default_settings(&conn).unwrap();
        validate_backup_connection(&conn).unwrap();

        conn.execute(
            "INSERT INTO vouchers (number, date, type)
             VALUES (1, '1405/13/01', 'سند حسابداری عادی')",
            [],
        )
        .unwrap();
        assert!(validate_backup_connection(&conn)
            .unwrap_err()
            .contains("تاریخ"));
        conn.execute("DELETE FROM vouchers", []).unwrap();

        conn.execute(
            "INSERT INTO vouchers (number, date, type)
             VALUES (1, '1405/01/01', 'سند حسابداری عادی')",
            [],
        )
        .unwrap();
        let voucher_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO voucher_lines (voucher_id, row_no, account_code, debit, credit)
             VALUES (?1, 1, '110101', 1000, 0)",
            params![voucher_id],
        )
        .unwrap();
        assert!(validate_backup_connection(&conn)
            .unwrap_err()
            .contains("تراز"));
        conn.execute("DELETE FROM voucher_lines", []).unwrap();

        conn.execute(
            "INSERT INTO voucher_lines (voucher_id, row_no, account_code, debit, credit)
             VALUES (?1, 1, '999999', 1000, 0)",
            params![voucher_id],
        )
        .unwrap();
        assert!(validate_backup_connection(&conn)
            .unwrap_err()
            .contains("کلید خارجی"));
    }

    #[test]
    fn backup_validation_rejects_unexpected_triggers() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        apply_schema_migrations(&conn).unwrap();
        seed_default_groups(&conn).unwrap();
        seed_default_accounts(&conn).unwrap();
        seed_default_settings(&conn).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER unexpected_voucher_trigger AFTER INSERT ON vouchers
             BEGIN UPDATE settings SET value = 'changed' WHERE key = 'company_name'; END;",
        )
        .unwrap();

        assert!(validate_backup_connection(&conn)
            .unwrap_err()
            .contains("trigger ناشناخته"));
    }

    #[test]
    fn legacy_migration_prefers_the_most_recent_database() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("hesabdari-legacy-{stamp}"));
        let data_dir = root.join("app-data");
        let old_dir = root.join(LEGACY_IDENTIFIER);
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::create_dir_all(&old_dir).unwrap();
        let destination = data_dir.join(DB_FILE_NAME);

        let write = |path: &Path, value: &str| {
            let conn = Connection::open(path).unwrap();
            conn.execute_batch("CREATE TABLE sample (value TEXT NOT NULL);")
                .unwrap();
            conn.execute("INSERT INTO sample (value) VALUES (?1)", params![value])
                .unwrap();
        };
        write(&old_dir.join(LEGACY_DB_FILE_NAME), "قدیمی");
        std::thread::sleep(Duration::from_millis(30));
        write(&data_dir.join(LEGACY_DB_FILE_NAME), "جدید");

        migrate_legacy_database(&data_dir, &destination).unwrap();
        let copied = Connection::open(&destination).unwrap();
        let value: String = copied
            .query_row("SELECT value FROM sample", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "جدید");

        drop(copied);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn creates_updates_and_deletes_group() {
        let db = test_database();
        let created = db
            .create_account_group("۱۲", "  گروه   آزمایشی ", "دارایی")
            .unwrap();
        assert_eq!(created.code, "12");
        assert_eq!(created.name, "گروه آزمایشی");

        let updated = db
            .update_account_group(created.id, "12", "گروه اصلاح‌شده", "هزینه")
            .unwrap();
        assert_eq!(updated.name, "گروه اصلاح‌شده");
        assert_eq!(updated.group_type, "هزینه");

        db.delete_account_group(created.id).unwrap();
        assert_eq!(db.list_account_groups().unwrap().groups.len(), 11);
    }

    #[test]
    fn creates_and_updates_account() {
        let db = test_database();
        let created = db
            .create_account(
                "110101003",
                "حساب آزمایشی",
                3,
                "1",
                "debtor",
                "110101",
                true,
            )
            .unwrap();
        assert_eq!(created.group_name, "دارایی‌های جاری");
        assert_eq!(created.parent_name, "صندوق مرکزی");
        assert!(created.is_postable);

        let updated = db
            .update_account(created.id, "حساب آزمایشی ۲", "1", "debtor", "110101", false)
            .unwrap();
        assert_eq!(updated.name, "حساب آزمایشی ۲");
        assert!(!updated.is_postable);

        db.delete_account(created.id).unwrap();
        let restored_parent = db
            .list_accounts()
            .unwrap()
            .accounts
            .into_iter()
            .find(|account| account.code == "110101")
            .unwrap();
        assert!(restored_parent.is_postable);
        assert_eq!(db.list_accounts().unwrap().accounts.len(), 34);
    }

    #[test]
    fn account_batch_is_atomic_when_a_later_level_fails() {
        let db = test_database();
        let before = db.list_accounts().unwrap().accounts.len();

        let error = db
            .create_accounts_atomic(vec![
                NewAccount::new("110199", "معین موقت", 2, "1", "debtor", "1101", true),
                NewAccount::new("110199", "معین تکراری", 2, "1", "debtor", "1101", true),
            ])
            .unwrap_err();

        assert!(error.contains("قبلاً"));
        assert_eq!(db.list_accounts().unwrap().accounts.len(), before);
    }

    #[test]
    fn changing_account_parent_updates_both_parent_postability_states() {
        let db = test_database();
        db.create_account("1102020", "والد قدیمی", 2, "1", "debtor", "1102", true)
            .unwrap();
        let child = db
            .create_account("11020201", "زیرحساب", 3, "1", "debtor", "1102020", true)
            .unwrap();

        let before = db.list_accounts().unwrap().accounts;
        assert!(
            !before
                .iter()
                .find(|account| account.code == "1102020")
                .unwrap()
                .is_postable
        );
        assert!(
            before
                .iter()
                .find(|account| account.code == "110202")
                .unwrap()
                .is_postable
        );

        db.update_account(
            child.id,
            &child.name,
            &child.group_code,
            &child.nature,
            "110202",
            true,
        )
        .unwrap();

        let after = db.list_accounts().unwrap().accounts;
        assert!(
            after
                .iter()
                .find(|account| account.code == "1102020")
                .unwrap()
                .is_postable
        );
        assert!(
            !after
                .iter()
                .find(|account| account.code == "110202")
                .unwrap()
                .is_postable
        );
    }

    #[test]
    fn renaming_parent_refreshes_denormalized_child_parent_names() {
        let db = test_database();
        let accounts = db.list_accounts().unwrap().accounts;
        let parent = accounts
            .iter()
            .find(|account| account.code == "120101")
            .unwrap()
            .clone();
        let child = accounts
            .iter()
            .find(|account| account.code == "120101001")
            .unwrap();
        assert_eq!(child.parent_name, "بدهکاران تجاری");

        db.update_account(
            parent.id,
            "بدهکاران اصلاح‌شده",
            &parent.group_code,
            &parent.nature,
            &parent.parent_code,
            parent.is_postable,
        )
        .unwrap();

        let refreshed = db
            .list_accounts()
            .unwrap()
            .accounts
            .into_iter()
            .find(|account| account.code == "120101001")
            .unwrap();
        assert_eq!(refreshed.parent_name, "بدهکاران اصلاح‌شده");
    }

    #[test]
    fn rejects_invalid_account_code_and_parent() {
        let db = test_database();
        assert!(db
            .create_account("abc", "بد", 2, "1", "debtor", "120101", true)
            .is_err());
        assert!(db
            .create_account("120103", "بدون والد", 2, "1", "debtor", "", true)
            .is_err());
        assert!(db
            .create_account("120104", "والد اشتباه", 3, "1", "debtor", "1101", true)
            .is_err());
        assert!(db
            .create_account(
                "130101",
                "کد با والد ناسازگار",
                2,
                "1",
                "debtor",
                "1201",
                true
            )
            .is_err());
    }

    #[test]
    fn migrates_schema_v2_to_v3_without_losing_account_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE app_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE accounts (id INTEGER PRIMARY KEY, code TEXT NOT NULL);
             INSERT INTO accounts (code) VALUES ('110101');
             INSERT INTO app_meta (key, value) VALUES ('schema_version', '2');",
        )
        .unwrap();

        apply_schema_migrations(&conn).unwrap();
        let version: String = conn
            .query_row(
                "SELECT value FROM app_meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let column_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('accounts') WHERE name = 'postable_before_children'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let account_code: String = conn
            .query_row("SELECT code FROM accounts WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, "3");
        assert_eq!(column_count, 1);
        assert_eq!(account_code, "110101");
    }

    #[test]
    fn prevents_reclassifying_groups_and_posted_accounts() {
        let db = test_database();
        let group = db
            .list_account_groups()
            .unwrap()
            .groups
            .into_iter()
            .find(|group| group.code == "1")
            .unwrap();
        let account = db
            .list_accounts()
            .unwrap()
            .accounts
            .into_iter()
            .find(|account| account.code == "110101")
            .unwrap();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO vouchers (number, date, type) VALUES (1, '1405/01/01', 'سند حسابداری عادی')",
                [],
            )
            .map_err(db_error)?;
            let voucher_id = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO voucher_lines (voucher_id, row_no, account_code, debit, credit)
                 VALUES (?1, 1, ?2, 1000, 0)",
                params![voucher_id, account.code],
            )
            .map_err(db_error)?;
            Ok(())
        })
        .unwrap();

        assert!(db
            .update_account_group(group.id, &group.code, &group.name, "بدهی")
            .is_err());
        assert!(db
            .update_account(
                account.id,
                &account.name,
                "4",
                &account.nature,
                &account.parent_code,
                true
            )
            .is_err());
    }

    #[test]
    fn migrates_legacy_database_including_wal_data() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hesabdari-migrate-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join(LEGACY_DB_FILE_NAME);
        let destination = dir.join(DB_FILE_NAME);

        {
            let conn = Connection::open(&source).unwrap();
            conn.execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA wal_autocheckpoint = 0;
                 CREATE TABLE sample (value TEXT NOT NULL);
                 INSERT INTO sample (value) VALUES ('داده قدیمی');",
            )
            .unwrap();

            // The source connection stays open with its data in the WAL, which
            // is exactly the situation a plain file copy would get wrong.
            migrate_legacy_database(&dir, &destination).unwrap();
        }

        let copied = Connection::open(&destination).unwrap();
        let value: String = copied
            .query_row("SELECT value FROM sample", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "داده قدیمی");

        drop(copied);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_to_delete_group_or_account_in_use() {
        let db = test_database();
        let group = db.list_account_groups().unwrap().groups[0].clone();
        assert!(db.delete_account_group(group.id).is_err());

        let account = db
            .list_accounts()
            .unwrap()
            .accounts
            .into_iter()
            .find(|a| a.code == "110101")
            .unwrap();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO vouchers (number, date, type) VALUES (1, '1405/01/01', 'عادی')",
                [],
            )
            .map_err(db_error)?;
            let id = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO voucher_lines (voucher_id, row_no, account_code, debit, credit)
                 VALUES (?1, 1, ?2, 1000, 0)",
                params![id, account.code],
            )
            .map_err(db_error)?;
            Ok(())
        })
        .unwrap();
        assert!(db.delete_account(account.id).is_err());
    }
    #[test]
    fn account_with_vouchers_cannot_gain_children_or_lose_postable() {
        let db = test_database();
        let account = |db: &Database, code: &str| {
            db.list_accounts()
                .unwrap()
                .accounts
                .into_iter()
                .find(|a| a.code == code)
                .unwrap()
        };
        let used = account(&db, "110101");
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO vouchers (number, date, type) VALUES (1, '1405/01/01', 'عادی')",
                [],
            )
            .map_err(db_error)?;
            let id = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO voucher_lines (voucher_id, row_no, account_code, debit, credit)
                 VALUES (?1, 1, '110101', 1000, 0)",
                params![id],
            )
            .map_err(db_error)?;
            Ok(())
        })
        .unwrap();

        // Used accounts keep accepting vouchers and get no sub-accounts.
        assert!(db
            .create_account("110101001", "زیرحساب", 3, "1", "debtor", "110101", true)
            .is_err());
        assert!(db
            .update_account(used.id, &used.name, "1", "debtor", "1101", false)
            .is_err());

        // A fresh parent becomes a heading once it gets a child.
        let parent = account(&db, "110202");
        assert!(parent.is_postable);
        db.create_account("110202001", "زیرحساب", 3, "1", "debtor", "110202", true)
            .unwrap();
        assert!(!account(&db, "110202").is_postable);
    }

    #[test]
    fn account_with_descendant_history_cannot_change_parent() {
        let db = test_database();
        db.create_account("1102010", "والد جایگزین", 2, "1", "debtor", "1102", true)
            .unwrap();
        let old_parent = db
            .list_accounts()
            .unwrap()
            .accounts
            .into_iter()
            .find(|account| account.code == "110201001")
            .unwrap();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO accounts
                    (code, name, level, group_code, nature, parent_code, parent_name, is_postable)
                 VALUES ('110201001001', 'تفصیلی قدیمی', 4, '1', 'debtor', '110201001', ?1, 1)",
                params![old_parent.name],
            )
            .map_err(db_error)?;
            conn.execute(
                "INSERT INTO vouchers (number, date, type) VALUES (1, '1405/01/01', 'عادی')",
                [],
            )
            .map_err(db_error)?;
            let voucher_id = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO voucher_lines (voucher_id, row_no, account_code, debit, credit)
                 VALUES (?1, 1, '110201001001', 1000, 0)",
                params![voucher_id],
            )
            .map_err(db_error)?;
            Ok(())
        })
        .unwrap();

        let error = db
            .update_account(
                old_parent.id,
                &old_parent.name,
                &old_parent.group_code,
                &old_parent.nature,
                "1102010",
                false,
            )
            .unwrap_err();
        assert!(error.contains("اسناد ثبت‌شده"));
    }
}
