//! Base schema, additive migrations and the starter chart of accounts.

use rusqlite::{params, Connection, OptionalExtension};

use crate::db::classification::{
    backfill_classifications, ensure_closing_system_accounts, seed_classification_history,
    seed_starter_classifications,
};
use crate::db::common::{
    db_error, fa_norm_sql, normalize_digits, to_fa_digits, BASE_SCHEMA_VERSION,
    CURRENT_SCHEMA_VERSION,
};

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

/// Reference tables that are also created by the additive migrations. Keeping
/// the DDL in one place means a restored v4 database and a fresh database end
/// up with exactly the same fiscal-year and audit-history structures.
pub(crate) const REFERENCE_SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS fiscal_years (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        title      TEXT NOT NULL,
        start_date TEXT NOT NULL,
        end_date   TEXT NOT NULL,
        status     TEXT NOT NULL DEFAULT 'open'
                   CHECK(status IN ('open', 'closed', 'final')),
        created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime')),
        updated_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
    );

    CREATE INDEX IF NOT EXISTS idx_fiscal_years_range ON fiscal_years(start_date, end_date);

    -- Year-end closing history. A fiscal year can have at most one executed
    -- run; reversing it keeps the row for the audit trail and allows a new
    -- execution later. This is the authoritative period lock, while the
    -- fiscal-year status mirrors it for the UI.
    CREATE TABLE IF NOT EXISTS closing_runs (
        id                    INTEGER PRIMARY KEY AUTOINCREMENT,
        fiscal_year_id        INTEGER NOT NULL REFERENCES fiscal_years(id),
        status                TEXT NOT NULL DEFAULT 'executed'
                              CHECK(status IN ('executed', 'reversed')),
        temp_voucher_id       INTEGER,
        closing_voucher_id    INTEGER,
        net_income            INTEGER NOT NULL DEFAULT 0,
        income_summary_code   TEXT NOT NULL,
        retained_earnings_code TEXT NOT NULL,
        closing_balance_code  TEXT,
        executed_at           TEXT,
        reversed_at           TEXT,
        reverse_reason        TEXT
    );

    CREATE INDEX IF NOT EXISTS idx_closing_runs_year ON closing_runs(fiscal_year_id, status);
    CREATE UNIQUE INDEX IF NOT EXISTS idx_closing_runs_executed_year
        ON closing_runs(fiscal_year_id) WHERE status = 'executed';

    CREATE TABLE IF NOT EXISTS audit_logs (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        action       TEXT NOT NULL,
        entity_type  TEXT NOT NULL,
        entity_id    INTEGER,
        before_value TEXT,
        after_value  TEXT,
        details      TEXT NOT NULL DEFAULT '',
        app_version  TEXT NOT NULL,
        created_at   TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
    );

    CREATE INDEX IF NOT EXISTS idx_audit_logs_created ON audit_logs(created_at);
    CREATE INDEX IF NOT EXISTS idx_audit_logs_entity ON audit_logs(entity_type, entity_id);

    -- Dated snapshots of each account's effective statement classification.
    -- Reports resolve the newest snapshot whose valid_from is not after their
    -- report date, so a classification change made today cannot silently
    -- rewrite a statement that was already issued for an earlier period.
    -- valid_to is exclusive and is kept for interval inspection; reports use
    -- the newest valid_from at or before the report date.
    CREATE TABLE IF NOT EXISTS account_classification_history (
        id                    INTEGER PRIMARY KEY AUTOINCREMENT,
        account_code          TEXT NOT NULL REFERENCES accounts(code)
                              ON UPDATE CASCADE ON DELETE CASCADE,
        fs_line               TEXT NOT NULL,
        classification_source TEXT NOT NULL DEFAULT 'auto',
        group_type            TEXT NOT NULL DEFAULT '',
        default_section       TEXT NOT NULL DEFAULT '',
        contra_of             TEXT NOT NULL DEFAULT '',
        contra_kind           TEXT NOT NULL DEFAULT '',
        opposite_fs_line      TEXT NOT NULL DEFAULT '',
        split_by_sign         INTEGER NOT NULL DEFAULT 0
                              CHECK(split_by_sign IN (0, 1)),
        valid_from            TEXT NOT NULL,
        valid_to              TEXT,
        change_reason         TEXT NOT NULL DEFAULT '',
        changed_by            TEXT NOT NULL DEFAULT '',
        changed_at            TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
    );

    CREATE UNIQUE INDEX IF NOT EXISTS idx_account_classification_history_from
        ON account_classification_history(account_code, valid_from);
    CREATE INDEX IF NOT EXISTS idx_account_classification_history_range
        ON account_classification_history(account_code, valid_from, valid_to);

    -- One row per successfully completed bulk import. A retried request with
    -- the same file content returns the original result instead of inserting
    -- the same vouchers twice.
    CREATE TABLE IF NOT EXISTS import_batches (
        payload_hash    TEXT PRIMARY KEY,
        file_name       TEXT NOT NULL DEFAULT '',
        sheet_name      TEXT NOT NULL DEFAULT '',
        voucher_count   INTEGER NOT NULL,
        imported_numbers TEXT NOT NULL DEFAULT '',
        created_at      TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
    );
"#;

/// Creates the base tables and records the version they physically match.
///
/// The base schema is intentionally pinned to `BASE_SCHEMA_VERSION`; the
/// version marker is not duplicated inside `SCHEMA`. `apply_schema_migrations`
/// then adds the later columns and upgrades the marker to
/// `CURRENT_SCHEMA_VERSION`. This guarantees a fresh and a restored database
/// pass through exactly the same migration path.
pub(crate) fn apply_base_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(SCHEMA).map_err(db_error)?;
    conn.execute_batch(REFERENCE_SCHEMA).map_err(db_error)?;
    conn.execute(
        "INSERT OR IGNORE INTO app_meta(key, value) VALUES ('schema_version', ?1)",
        params![BASE_SCHEMA_VERSION.to_string()],
    )
    .map_err(db_error)?;
    Ok(())
}

pub(crate) const DEFAULT_SETTINGS: [(&str, &str); 2] =
    [("company_name", "شرکت نمونه"), ("fiscal_year", "۱۴۰۵")];

const STARTER_GROUPS_SEEDED_KEY: &str = "starter_groups_seeded";
const STARTER_ACCOUNTS_SEEDED_KEY: &str = "starter_accounts_seeded";

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

/// Safely converts legacy voucher rows into (base account, optional detail-1).
///
/// Legacy level-4 accounts cannot be represented by the current posting
/// model. They are preserved untouched: the line keeps its original
/// `account_code`, gets `account_id = NULL` and `legacy_level4_pending = 1`.
/// The explicit migration tool can resolve them later without ever deleting
/// the original data implicitly.
fn migrate_voucher_account_pairs(conn: &Connection) -> Result<(), String> {
    let orphaned: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM voucher_lines l LEFT JOIN accounts a ON a.code = l.account_code WHERE a.id IS NULL",
            [], |r| r.get(0),
        )
        .map_err(db_error)?;
    if orphaned > 0 {
        return Err(format!(
            "ارتقای پایگاه داده متوقف شد؛ {} ردیف سند به حساب ناموجود اشاره می‌کند. ابتدا ارجاعات یتیم را بررسی کنید.",
            to_fa_digits(orphaned.to_string())
        ));
    }

    let malformed_details: i64 = conn
        .query_row(
            "SELECT COUNT(*)\n               FROM voucher_lines l\n               JOIN accounts d ON d.code = l.account_code\n               LEFT JOIN accounts p ON p.code = d.parent_code\n              WHERE d.level = 3 AND (p.id IS NULL OR p.level <> 2 OR p.group_code <> d.group_code)",
            [], |r| r.get(0),
        )
        .map_err(db_error)?;
    if malformed_details > 0 {
        return Err(format!(
            "ارتقای پایگاه داده متوقف شد؛ {} ردیف تفصیلی سطح ۳ والد معین معتبر یا گروه سازگار ندارد. ساختار حساب‌ها را اصلاح کنید؛ داده‌ها تغییر نکرده‌اند.",
            to_fa_digits(malformed_details.to_string())
        ));
    }

    let before: Vec<(i64, i64, i64, i64)> = {
        let mut stmt = conn
            .prepare("SELECT v.id, COUNT(l.id), COALESCE(SUM(l.debit),0), COALESCE(SUM(l.credit),0) FROM vouchers v LEFT JOIN voucher_lines l ON l.voucher_id=v.id GROUP BY v.id ORDER BY v.id")
            .map_err(db_error)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .map_err(db_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error)?;
        rows
    };

    let columns = table_columns(conn, "voucher_lines")?;
    if !columns.iter().any(|c| c == "account_id") {
        conn.execute(
            "ALTER TABLE voucher_lines ADD COLUMN account_id INTEGER REFERENCES accounts(id)",
            [],
        )
        .map_err(db_error)?;
    }
    if !columns.iter().any(|c| c == "detail1_code") {
        conn.execute("ALTER TABLE voucher_lines ADD COLUMN detail1_code TEXT REFERENCES accounts(code) ON UPDATE CASCADE", [])
            .map_err(db_error)?;
    }
    if !columns.iter().any(|c| c == "legacy_level4_pending") {
        conn.execute(
            "ALTER TABLE voucher_lines ADD COLUMN legacy_level4_pending INTEGER NOT NULL DEFAULT 0 CHECK(legacy_level4_pending IN (0, 1))",
            [],
        )
        .map_err(db_error)?;
    }

    conn.execute_batch(
        "DROP TABLE IF EXISTS temp._voucher_line_account_map;\n         CREATE TEMP TABLE _voucher_line_account_map AS\n         SELECT l.id AS line_id,\n                CASE WHEN a.level = 4 THEN NULL\n                     WHEN a.level = 3 THEN p.id\n                     ELSE a.id END AS base_id,\n                CASE WHEN a.level = 3 THEN p.code ELSE a.code END AS base_code,\n                CASE WHEN a.level = 3 THEN a.code ELSE NULL END AS detail_code,\n                CASE WHEN a.level = 4 THEN 1 ELSE 0 END AS pending\n           FROM voucher_lines l\n           JOIN accounts a ON a.code = l.account_code\n           LEFT JOIN accounts p ON p.code = a.parent_code;\n         UPDATE voucher_lines\n            SET account_id = (SELECT base_id FROM _voucher_line_account_map m WHERE m.line_id = voucher_lines.id),\n                account_code = (SELECT base_code FROM _voucher_line_account_map m WHERE m.line_id = voucher_lines.id),\n                detail1_code = (SELECT detail_code FROM _voucher_line_account_map m WHERE m.line_id = voucher_lines.id),\n                legacy_level4_pending = (SELECT pending FROM _voucher_line_account_map m WHERE m.line_id = voucher_lines.id);"
    ).map_err(db_error)?;

    let unmapped: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM voucher_lines WHERE account_id IS NULL AND legacy_level4_pending = 0",
            [],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    if unmapped > 0 {
        return Err(format!(
            "ارتقای پایگاه داده متوقف شد؛ {} ردیف سند قابل تبدیل به حساب پایه نیست.",
            to_fa_digits(unmapped.to_string())
        ));
    }

    // In the WebApp posting model, level 1 and 2 remain postable even when
    // they have descendants. A detail-1 row is stored separately and does not
    // cause the parent to become a non-postable heading.
    conn.execute(
        "UPDATE accounts SET is_postable = 1 WHERE level IN (1, 2)",
        [],
    )
    .map_err(db_error)?;

    let after: Vec<(i64, i64, i64, i64)> = {
        let mut stmt = conn
            .prepare("SELECT v.id, COUNT(l.id), COALESCE(SUM(l.debit),0), COALESCE(SUM(l.credit),0) FROM vouchers v LEFT JOIN voucher_lines l ON l.voucher_id=v.id GROUP BY v.id ORDER BY v.id")
            .map_err(db_error)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .map_err(db_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error)?;
        rows
    };
    if before != after {
        return Err("ارتقای پایگاه داده متوقف شد؛ شمار ردیف‌ها یا جمع بدهکار/بستانکار یک یا چند سند تغییر کرده است. تراکنش بازگردانده شد.".to_string());
    }
    conn.execute_batch("DROP TABLE temp._voucher_line_account_map;")
        .map_err(db_error)?;
    Ok(())
}

/// Enforces base/detail consistency for every new or updated voucher line.
pub(crate) fn ensure_voucher_account_pair_guards(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_voucher_lines_account_id ON voucher_lines(account_id);\n         CREATE INDEX IF NOT EXISTS idx_voucher_lines_detail1_code ON voucher_lines(detail1_code);\n         DROP TRIGGER IF EXISTS voucher_lines_account_pair_insert;\n         DROP TRIGGER IF EXISTS voucher_lines_account_pair_update;\n         CREATE TRIGGER voucher_lines_account_pair_insert\n         BEFORE INSERT ON voucher_lines\n         WHEN NOT (NEW.legacy_level4_pending = 1 AND NEW.account_id IS NULL)\n          AND (NEW.account_id IS NULL\n           OR NOT EXISTS (SELECT 1 FROM accounts b WHERE b.id=NEW.account_id AND b.code=NEW.account_code AND b.level IN (1,2))\n           OR (NEW.detail1_code IS NOT NULL AND NEW.detail1_code <> '' AND NOT EXISTS (\n                SELECT 1 FROM accounts d JOIN accounts b ON b.id=NEW.account_id\n                 WHERE d.code=NEW.detail1_code AND d.level=3 AND b.level=2\n                   AND d.parent_code=b.code AND d.group_code=b.group_code)))\n         BEGIN SELECT RAISE(ABORT, 'invalid base/detail account pair'); END;\n         CREATE TRIGGER voucher_lines_account_pair_update\n         BEFORE UPDATE OF account_id, account_code, detail1_code ON voucher_lines\n         WHEN NOT (NEW.legacy_level4_pending = 1 AND NEW.account_id IS NULL)\n          AND (NEW.account_id IS NULL\n           OR NOT EXISTS (SELECT 1 FROM accounts b WHERE b.id=NEW.account_id AND b.code=NEW.account_code AND b.level IN (1,2))\n           OR (NEW.detail1_code IS NOT NULL AND NEW.detail1_code <> '' AND NOT EXISTS (\n                SELECT 1 FROM accounts d JOIN accounts b ON b.id=NEW.account_id\n                 WHERE d.code=NEW.detail1_code AND d.level=3 AND b.level=2\n                   AND d.parent_code=b.code AND d.group_code=b.group_code)))\n         BEGIN SELECT RAISE(ABORT, 'invalid base/detail account pair'); END;"
    ).map_err(db_error)
}

/// Indexes used by the read paths that scan the most data: voucher lists,
/// journals, ledgers and the trial balance. They are created on every open
/// (idempotently) so an existing database gets them without a schema-version
/// bump, and a restored database is brought up to date by the migration that
/// runs after restore.
pub(crate) fn ensure_query_indexes(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        // Reports group by the account a line actually posts to: its level-3
        // detail when one is selected, otherwise the base account. Without
        // this expression index SQLite has to scan every voucher line for
        // every account in the trial balance and the financial statements.
        "CREATE INDEX IF NOT EXISTS idx_voucher_lines_effective_account
             ON voucher_lines(COALESCE(NULLIF(detail1_code, ''), account_code));
         -- Voucher lists and the journal order by date and number; the
         -- composite index serves both the range filter and that ordering.
         CREATE INDEX IF NOT EXISTS idx_vouchers_date_number
             ON vouchers(date, number);
         -- Report scopes filter is_temporary before the date range, so keep
         -- that column first and keep the sort columns in the same order.
         CREATE INDEX IF NOT EXISTS idx_vouchers_scope_date_number
             ON vouchers(is_temporary, date, number);",
    )
    .map_err(db_error)
}

/// Permanent vouchers are immutable across every database write path.
///
/// The only exception is an atomic closing reversal: it fills the
/// `_closing_reversal_allowed` allowlist inside its own transaction, deletes
/// those rows, and clears the allowlist before commit. The table is cleared on
/// every open so a stale or restored row can never unlock a voucher.
///
/// The allowlist must be a normal table (not `TEMP`): a trigger created in the
/// main schema resolves unqualified names to `main`, so it cannot see a temp
/// table.
pub(crate) fn ensure_permanent_voucher_guards(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _closing_reversal_allowed (
             voucher_id INTEGER PRIMARY KEY
         );
         DELETE FROM _closing_reversal_allowed;
         DROP TRIGGER IF EXISTS vouchers_permanent_update;
         DROP TRIGGER IF EXISTS vouchers_permanent_delete;
         DROP TRIGGER IF EXISTS voucher_lines_permanent_insert;
         DROP TRIGGER IF EXISTS voucher_lines_permanent_update;
         DROP TRIGGER IF EXISTS voucher_lines_permanent_delete;
         CREATE TRIGGER vouchers_permanent_update
         BEFORE UPDATE ON vouchers
         WHEN OLD.is_temporary = 0
          AND OLD.id NOT IN (SELECT voucher_id FROM _closing_reversal_allowed)
         BEGIN SELECT RAISE(ABORT, 'voucher is permanent'); END;
         CREATE TRIGGER vouchers_permanent_delete
         BEFORE DELETE ON vouchers
         WHEN OLD.is_temporary = 0
          AND OLD.id NOT IN (SELECT voucher_id FROM _closing_reversal_allowed)
         BEGIN SELECT RAISE(ABORT, 'voucher is permanent'); END;
         CREATE TRIGGER voucher_lines_permanent_insert
         BEFORE INSERT ON voucher_lines
         WHEN EXISTS (SELECT 1 FROM vouchers v WHERE v.id = NEW.voucher_id AND v.is_temporary = 0)
         BEGIN SELECT RAISE(ABORT, 'voucher is permanent'); END;
         CREATE TRIGGER voucher_lines_permanent_update
         BEFORE UPDATE ON voucher_lines
         WHEN EXISTS (SELECT 1 FROM vouchers v WHERE v.id = OLD.voucher_id AND v.is_temporary = 0)
           OR EXISTS (SELECT 1 FROM vouchers v WHERE v.id = NEW.voucher_id AND v.is_temporary = 0)
         BEGIN SELECT RAISE(ABORT, 'voucher is permanent'); END;
         CREATE TRIGGER voucher_lines_permanent_delete
         BEFORE DELETE ON voucher_lines
         WHEN EXISTS (SELECT 1 FROM vouchers v WHERE v.id = OLD.voucher_id AND v.is_temporary = 0)
          AND OLD.voucher_id NOT IN (SELECT voucher_id FROM _closing_reversal_allowed)
         BEGIN SELECT RAISE(ABORT, 'voucher is permanent'); END;",
    )
    .map_err(db_error)
}

/// Gives existing vouchers a fiscal-year link without changing their numbers
/// or meaning. Missing standard Jalali years are created only during the
/// upgrade to the fiscal-year schema, so deleting an empty year is not undone
/// on the next launch. Each voucher is linked to any configured year that
/// already covers its date.
fn backfill_fiscal_years(conn: &Connection, create_missing_years: bool) -> Result<(), String> {
    let mut years: Vec<i32> = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT DISTINCT substr(date, 1, 4) FROM vouchers WHERE length(date) >= 8")
            .map_err(db_error)?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(db_error)?;
        for row in rows {
            if let Ok(year) = row.map_err(db_error)?.parse::<i32>() {
                years.push(year);
            }
        }
    }
    if table_exists(conn, "settings")? {
        let setting: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'fiscal_year'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if let Some(text) = setting {
            let digits: String = normalize_digits(&text)
                .chars()
                .filter(|ch| ch.is_ascii_digit())
                .take(4)
                .collect();
            if digits.len() == 4 {
                if let Ok(year) = digits.parse::<i32>() {
                    years.push(year);
                }
            }
        }
    }
    years.sort_unstable();
    years.dedup();
    if !create_missing_years {
        years.clear();
    }

    for year in years {
        if !(1..=3177).contains(&year) {
            continue;
        }
        let start = format!("{year:04}/01/01");
        let end = format!(
            "{year:04}/12/{:02}",
            if jalali_calendar::is_leap_year(year) {
                30
            } else {
                29
            }
        );
        // Never create a partially overlapping year: the automatic range is a
        // convenience for legacy data, not a reason to make period lookup
        // ambiguous. A custom range that already touches it wins.
        let overlapping: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM fiscal_years
                  WHERE start_date <= ?2 AND end_date >= ?1",
                params![start, end],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if overlapping == 0 {
            conn.execute(
                "INSERT INTO fiscal_years (title, start_date, end_date, status)
                 VALUES (?1, ?2, ?3, 'open')",
                params![format!("سال مالی {}", to_fa_digits(year)), start, end],
            )
            .map_err(db_error)?;
        }
    }

    if table_exists(conn, "vouchers")? {
        conn.execute(
            "UPDATE vouchers
                SET fiscal_year_id = (
                    SELECT f.id FROM fiscal_years f
                     WHERE vouchers.date BETWEEN f.start_date AND f.end_date
                     ORDER BY f.id LIMIT 1
                )
              WHERE fiscal_year_id IS NULL",
            [],
        )
        .map_err(db_error)?;
    }
    Ok(())
}

/// Brings stored Persian text to the same canonical form used by new writes.
/// Temporary vouchers can be updated here; permanent vouchers reject changes
/// by trigger, so their descriptions are matched through `fa_norm_sql` at
/// search time instead of being rewritten.
fn normalize_existing_text(conn: &Connection) -> Result<(), String> {
    let group_name = fa_norm_sql("name");
    conn.execute(
        &format!("UPDATE account_groups SET name = {group_name} WHERE name <> {group_name}"),
        [],
    )
    .map_err(db_error)?;

    let account_name = fa_norm_sql("name");
    let parent_name = fa_norm_sql("parent_name");
    conn.execute(
        &format!(
            "UPDATE accounts
                SET name = {account_name}, parent_name = {parent_name}
              WHERE name <> {account_name} OR parent_name <> {parent_name}"
        ),
        [],
    )
    .map_err(db_error)?;

    let company_name = fa_norm_sql("value");
    conn.execute(
        &format!(
            "UPDATE settings SET value = {company_name}
              WHERE key = 'company_name' AND value <> {company_name}"
        ),
        [],
    )
    .map_err(db_error)?;

    let voucher_description = fa_norm_sql("description");
    conn.execute(
        &format!(
            "UPDATE vouchers SET description = {voucher_description}
              WHERE is_temporary = 1 AND description <> {voucher_description}"
        ),
        [],
    )
    .map_err(db_error)?;

    let line_description = fa_norm_sql("description");
    conn.execute(
        &format!(
            "UPDATE voucher_lines SET description = {line_description}
              WHERE description <> {line_description}
                AND EXISTS (
                    SELECT 1 FROM vouchers v
                     WHERE v.id = voucher_lines.voucher_id AND v.is_temporary = 1
                )"
        ),
        [],
    )
    .map_err(db_error)?;

    Ok(())
}

/// A database that has already been opened by an older application version
/// must not receive the starter chart again merely because the operator deleted
/// every account. Fresh databases (still at `BASE_SCHEMA_VERSION`) are exempt,
/// so first-launch seeding still runs.
fn mark_starter_chart_seeded(conn: &Connection) -> Result<(), String> {
    for key in [STARTER_GROUPS_SEEDED_KEY, STARTER_ACCOUNTS_SEEDED_KEY] {
        conn.execute(
            "INSERT OR IGNORE INTO app_meta(key, value) VALUES (?1, '1')",
            params![key],
        )
        .map_err(db_error)?;
    }
    Ok(())
}

/// Applies additive schema changes and upgrades the recorded version.
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
    tx.execute_batch(REFERENCE_SCHEMA).map_err(db_error)?;
    if table_exists(&tx, "accounts")? {
        let account_columns = table_columns(&tx, "accounts")?;
        if !account_columns.iter().any(|column| column == "is_active") {
            tx.execute(
                "ALTER TABLE accounts ADD COLUMN is_active INTEGER NOT NULL DEFAULT 1 CHECK(is_active IN (0, 1))",
                [],
            )
            .map_err(db_error)?;
        }

        // Per-account statement classification (schema v11). The columns are
        // added here rather than in the base schema so a fresh and a restored
        // database pass through the same migration and backfill path.
        let classification_columns = [
            ("fs_line", "TEXT"),
            ("contra_of", "TEXT"),
            ("contra_kind", "TEXT"),
            ("opposite_fs_line", "TEXT"),
            (
                "split_by_sign",
                "INTEGER NOT NULL DEFAULT 0 CHECK(split_by_sign IN (0, 1))",
            ),
            ("system_role", "TEXT"),
            ("classification_source", "TEXT NOT NULL DEFAULT 'auto'"),
        ];
        for (name, definition) in classification_columns {
            if !account_columns.iter().any(|column| column == name) {
                tx.execute(
                    &format!("ALTER TABLE accounts ADD COLUMN {name} {definition}"),
                    [],
                )
                .map_err(db_error)?;
            }
        }
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_accounts_fs_line ON accounts(fs_line);
             CREATE UNIQUE INDEX IF NOT EXISTS idx_accounts_system_role
                 ON accounts(system_role)
              WHERE system_role IS NOT NULL AND system_role <> '';",
        )
        .map_err(db_error)?;
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
    if table_exists(&tx, "voucher_lines")? {
        let line_columns = table_columns(&tx, "voucher_lines")?;
        if !line_columns
            .iter()
            .any(|column| column == "legacy_level4_pending")
        {
            tx.execute(
                "ALTER TABLE voucher_lines ADD COLUMN legacy_level4_pending INTEGER NOT NULL DEFAULT 0 CHECK(legacy_level4_pending IN (0, 1))",
                [],
            )
            .map_err(db_error)?;
        }
    }
    if version < 5 {
        migrate_voucher_account_pairs(&tx)?;
    }

    // Existing documents remain temporary after upgrade. The migration never
    // silently locks historical accounting records.
    if table_exists(&tx, "vouchers")? {
        let voucher_columns = table_columns(&tx, "vouchers")?;
        if !voucher_columns
            .iter()
            .any(|column| column == "is_temporary")
        {
            tx.execute(
                "ALTER TABLE vouchers ADD COLUMN is_temporary INTEGER NOT NULL DEFAULT 1 CHECK(is_temporary IN (0, 1))",
                [],
            )
            .map_err(db_error)?;
        }
        if !voucher_columns
            .iter()
            .any(|column| column == "fiscal_year_id")
        {
            tx.execute(
                "ALTER TABLE vouchers ADD COLUMN fiscal_year_id INTEGER REFERENCES fiscal_years(id)",
                [],
            )
            .map_err(db_error)?;
        }
    }

    // Give every existing voucher a fiscal-year link by its date. The tool
    // creates an open year only where no configured year covers the date, so
    // historical documents never end up in a locked or missing period.
    backfill_fiscal_years(&tx, version < CURRENT_SCHEMA_VERSION)?;

    // Persist request fingerprints so a retry after a lost response returns
    // the already-created or already-updated voucher instead of writing twice.
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS voucher_save_idempotency (
            idempotency_key TEXT PRIMARY KEY,
            payload_signature TEXT NOT NULL,
            voucher_id INTEGER NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
        );",
    )
    .map_err(db_error)?;

    // Re-create guards and read-path indexes on every open, including after
    // a database restore.
    ensure_voucher_account_pair_guards(&tx)?;
    ensure_permanent_voucher_guards(&tx)?;
    ensure_query_indexes(&tx)?;

    if version < 11 {
        backfill_classifications(&tx)?;
    }

    // Capture the classification every existing account has at upgrade time.
    // This baseline is what historical reports will keep using from now on;
    // later changes add dated snapshots without touching it.
    seed_classification_history(&tx)?;

    // Existing databases are upgraded with the same canonical Persian
    // spelling used by new writes. Permanent voucher rows reject updates by
    // trigger, so their descriptions stay byte-for-byte intact and search
    // folds them through `fa_norm_sql` instead.
    if version < CURRENT_SCHEMA_VERSION {
        normalize_existing_text(&tx)?;
    }

    // A database that predates the seed markers already has its starter chart
    // (or the operator deliberately removed it). Mark both charts as seeded
    // before the version bump so a later launch never resurrects a deleted
    // chart. A fresh database is still at `BASE_SCHEMA_VERSION` and seeds
    // normally through `seed_default_*`.
    if version > BASE_SCHEMA_VERSION && version < CURRENT_SCHEMA_VERSION {
        mark_starter_chart_seeded(&tx)?;
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

fn classification_error(message: String) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(message)))
}

fn meta_flag(conn: &Connection, key: &str) -> rusqlite::Result<bool> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM app_meta WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()?;
    Ok(value.as_deref() == Some("1"))
}

fn set_meta_flag(conn: &Connection, key: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO app_meta (key, value) VALUES (?1, '1')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key],
    )?;
    Ok(())
}

pub(crate) fn seed_default_groups(conn: &mut Connection) -> rusqlite::Result<()> {
    if meta_flag(conn, STARTER_GROUPS_SEEDED_KEY)? {
        return Ok(());
    }
    let tx = conn.transaction()?;
    let count: i64 = tx.query_row("SELECT COUNT(*) FROM account_groups", [], |row| row.get(0))?;
    if count == 0 {
        let mut stmt = tx.prepare(
            "INSERT OR IGNORE INTO account_groups (code, name, type) VALUES (?1, ?2, ?3)",
        )?;
        for (code, name, group_type) in DEFAULT_GROUPS {
            stmt.execute(params![code, name, group_type])?;
        }
        drop(stmt);
    }
    // The flag records the decision even when the table was not empty: from
    // now on an empty chart means the operator deleted it, not a first launch.
    set_meta_flag(&tx, STARTER_GROUPS_SEEDED_KEY)?;
    tx.commit()?;
    Ok(())
}

/// True once the v11 classification columns exist. Some legacy tests seed the
/// starter chart on the base schema before running migrations; the seed must
/// not fail there, and the migration later backfills the classification.
fn has_classification_columns(conn: &Connection) -> rusqlite::Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('accounts')
          WHERE name IN ('fs_line', 'system_role', 'classification_source')",
        [],
        |row| row.get(0),
    )?;
    Ok(count == 3)
}

pub(crate) fn seed_default_accounts(conn: &mut Connection) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    let starter_seeded = meta_flag(&tx, STARTER_ACCOUNTS_SEEDED_KEY)?;
    let count: i64 = tx.query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))?;
    let classified = has_classification_columns(&tx)?;
    if !starter_seeded && count == 0 {
        for (code, name, level, group_code, nature, parent_code, _postable) in DEFAULT_ACCOUNTS {
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
                    i64::from(level <= 2)
                ],
            )?;
        }
        // The migration runs before the chart exists on a fresh database, so
        // the starter classification is applied here, after the accounts.
        if classified {
            seed_starter_classifications(&tx).map_err(classification_error)?;
        }
    }
    // Record the seeding decision even when accounts already existed before
    // the flag was introduced, so deleting them later is not undone.
    set_meta_flag(&tx, STARTER_ACCOUNTS_SEEDED_KEY)?;
    // Required by the closing workflow; provisioning is idempotent and never
    // invents a role for an existing account.
    if classified {
        ensure_closing_system_accounts(&tx).map_err(classification_error)?;
        // A fresh database gets its chart after the migration ran, so the
        // baseline snapshot is seeded here. On later opens this only fills in
        // accounts that were created outside the normal write path.
        seed_classification_history(&tx).map_err(classification_error)?;
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
