//! Base schema, additive migrations and the starter chart of accounts.

use rusqlite::{params, Connection, OptionalExtension};

use crate::db::common::{db_error, CURRENT_SCHEMA_VERSION};

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
        default_section TEXT NOT NULL DEFAULT '',
        created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime')),
        updated_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
    );

    CREATE TABLE IF NOT EXISTS accounts (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        code        TEXT NOT NULL UNIQUE,
        name        TEXT NOT NULL,
        -- Level 4 is retained for legacy database compatibility; application writes stop at MAX_ACCOUNT_LEVEL.
        level       INTEGER NOT NULL CHECK(level BETWEEN 1 AND 4),
        group_code  TEXT NOT NULL REFERENCES account_groups(code) ON UPDATE CASCADE,
        nature      TEXT NOT NULL DEFAULT 'debtor' CHECK(nature IN ('debtor', 'creditor', 'dual')),
        parent_code TEXT REFERENCES accounts(code),
        parent_name TEXT NOT NULL DEFAULT '',
        is_postable INTEGER NOT NULL DEFAULT 1 CHECK(is_postable IN (0, 1)),
        is_active   INTEGER NOT NULL DEFAULT 1 CHECK(is_active IN (0, 1)),
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
"#;

/// Creates the base tables and records the current schema version.
///
/// The version marker is written from `CURRENT_SCHEMA_VERSION` instead of being
/// duplicated inside `SCHEMA`, so bumping the constant can never leave a fresh
/// database tagged with an older version.
pub(crate) fn apply_base_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(SCHEMA).map_err(db_error)?;
    conn.execute(
        "INSERT OR IGNORE INTO app_meta(key, value) VALUES ('schema_version', ?1)",
        params![CURRENT_SCHEMA_VERSION.to_string()],
    )
    .map_err(db_error)?;
    Ok(())
}

pub(crate) const DEFAULT_SETTINGS: [(&str, &str); 2] =
    [("company_name", "شرکت نمونه"), ("fiscal_year", "۱۴۰۵")];

/// The starter chart used on first launch, so the application can be used
/// immediately instead of opening on an empty account list.
pub(crate) const DEFAULT_GROUPS: [(&str, &str, &str); 11] = [
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
pub(crate) const DEFAULT_ACCOUNTS: [(&str, &str, i64, &str, &str, &str, bool); 34] = [
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

pub(crate) fn table_exists(conn: &Connection, table: &str) -> Result<bool, String> {
    let found: i64 = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            params![table],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    Ok(found != 0)
}

/// Collects the column names of a table so migrations can be idempotent.
pub(crate) fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, String> {
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

/// Applies additive, versioned schema changes. Migration 3 remembers whether
/// a parent account was manually postable before its first child was added.
pub(crate) fn apply_schema_migrations(conn: &mut Connection) -> Result<(), String> {
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
    let tx = conn.transaction().map_err(db_error)?;
    if table_exists(&tx, "accounts")? {
        let account_columns = table_columns(&tx, "accounts")?;
        if !account_columns
            .iter()
            .any(|column| column == "postable_before_children")
        {
            tx.execute(
                "ALTER TABLE accounts ADD COLUMN postable_before_children INTEGER CHECK(postable_before_children IN (0, 1))",
                [],
            )
            .map_err(db_error)?;
        }
        if !account_columns.iter().any(|column| column == "is_active") {
            tx.execute(
                "ALTER TABLE accounts ADD COLUMN is_active INTEGER NOT NULL DEFAULT 1 CHECK(is_active IN (0, 1))",
                [],
            )
            .map_err(db_error)?;
        }
    }
    if table_exists(&tx, "account_groups")? {
        let group_columns = table_columns(&tx, "account_groups")?;
        if !group_columns
            .iter()
            .any(|column| column == "default_section")
        {
            tx.execute(
                "ALTER TABLE account_groups ADD COLUMN default_section TEXT NOT NULL DEFAULT ''",
                [],
            )
            .map_err(db_error)?;
        }
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

pub(crate) fn seed_default_groups(conn: &mut Connection) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
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

pub(crate) fn seed_default_accounts(conn: &mut Connection) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
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

pub(crate) fn seed_default_settings(conn: &Connection) -> rusqlite::Result<()> {
    for (key, value) in DEFAULT_SETTINGS {
        conn.execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
    }
    Ok(())
}
