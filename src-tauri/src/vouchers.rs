//! Voucher persistence and the accounting reports that are built from it.
//!
//! Amounts are stored as whole Rials (`i64`). Every voucher must balance
//! before it is written, so the reports can rely on a sound ledger.

use crate::db::{self, normalize_digits, Database};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

pub const VOUCHER_TYPES: [&str; 4] = [
    "سند حسابداری عادی",
    "سند حسابداری افتتاحیه",
    "سند حسابداری اختتامیه",
    "سند بستن حساب‌های موقت",
];

const MAX_DESCRIPTION: usize = 1000;
const MAX_LINE_DESCRIPTION: usize = 500;
const MAX_LINES: usize = 500;

/// Largest amount accepted on one line (1e15 Rials). It stays below 2^53 so
/// the UI, which holds numbers as JavaScript doubles, never loses precision,
/// and it leaves room for sums of `MAX_LINES` lines inside an `i64`.
pub const MAX_AMOUNT: i64 = 999_999_999_999_999;

/// Voucher types that close the books. They are real vouchers (the trial
/// balance and ledgers show them) but must not hide the period's income and
/// expenses from the income statement.
const CLOSING_TEMPORARY: &str = VOUCHER_TYPES[3];
const CLOSING_FINAL: &str = VOUCHER_TYPES[2];

/* ── Input / output types ────────────────────────────────────── */

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherLineInput {
    #[serde(default)]
    pub account_code: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub debit: i64,
    #[serde(default)]
    pub credit: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherInput {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub number: Option<i64>,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub voucher_type: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub lines: Vec<VoucherLineInput>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherLine {
    pub row_no: i64,
    pub account_code: String,
    pub account_name: String,
    pub description: String,
    pub debit: i64,
    pub credit: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Voucher {
    pub id: i64,
    pub number: i64,
    pub date: String,
    #[serde(rename = "type")]
    pub voucher_type: String,
    pub description: String,
    pub lines: Vec<VoucherLine>,
    pub total_debit: i64,
    pub total_credit: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherSummary {
    pub id: i64,
    pub number: i64,
    pub date: String,
    #[serde(rename = "type")]
    pub voucher_type: String,
    pub description: String,
    pub total_debit: i64,
    pub total_credit: i64,
    pub line_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherList {
    pub vouchers: Vec<VoucherSummary>,
    /// Number of vouchers in the whole book, ignoring any filter.
    pub total_count: i64,
    pub total_debit: i64,
    pub total_credit: i64,
    pub database_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalRow {
    pub voucher_id: i64,
    pub number: i64,
    pub date: String,
    #[serde(rename = "type")]
    pub voucher_type: String,
    pub voucher_description: String,
    pub row_no: i64,
    pub account_code: String,
    pub account_name: String,
    pub line_description: String,
    pub debit: i64,
    pub credit: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalReport {
    pub from: String,
    pub to: String,
    pub search: String,
    pub rows: Vec<JournalRow>,
    pub total_debit: i64,
    pub total_credit: i64,
    pub voucher_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerRow {
    pub voucher_id: i64,
    pub account_code: String,
    pub account_name: String,
    pub number: i64,
    pub date: String,
    #[serde(rename = "type")]
    pub voucher_type: String,
    pub voucher_description: String,
    pub line_description: String,
    pub debit: i64,
    pub credit: i64,
    pub running_debit: i64,
    pub running_credit: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerReport {
    pub account_code: String,
    pub account_name: String,
    pub group_name: String,
    /// True when the selected account has sub-accounts, whose activity is
    /// included in the report.
    pub includes_children: bool,
    pub from: String,
    pub to: String,
    pub opening_debit: i64,
    pub opening_credit: i64,
    pub total_debit: i64,
    pub total_credit: i64,
    pub closing_debit: i64,
    pub closing_credit: i64,
    pub rows: Vec<LedgerRow>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrialBalanceRow {
    pub account_code: String,
    pub account_name: String,
    pub group_name: String,
    pub level: i64,
    pub opening_debit: i64,
    pub opening_credit: i64,
    pub period_debit: i64,
    pub period_credit: i64,
    pub closing_debit: i64,
    pub closing_credit: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrialBalanceReport {
    pub from: String,
    pub to: String,
    pub rows: Vec<TrialBalanceRow>,
    pub opening_debit: i64,
    pub opening_credit: i64,
    pub period_debit: i64,
    pub period_credit: i64,
    pub closing_debit: i64,
    pub closing_credit: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AmountRow {
    pub account_code: String,
    pub account_name: String,
    pub amount: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncomeStatementReport {
    pub from: String,
    pub to: String,
    pub revenues: Vec<AmountRow>,
    pub revenue_total: i64,
    pub cogs: Vec<AmountRow>,
    pub cogs_total: i64,
    pub expenses: Vec<AmountRow>,
    pub expense_total: i64,
    pub gross_profit: i64,
    pub operating_profit: i64,
    pub net_profit: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceSheetReport {
    pub as_of: String,
    pub assets: Vec<AmountRow>,
    pub assets_total: i64,
    pub liabilities: Vec<AmountRow>,
    pub liabilities_total: i64,
    pub equity: Vec<AmountRow>,
    pub equity_total: i64,
    pub profit: i64,
    pub total_liabilities_equity: i64,
    pub balanced: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    pub company_name: String,
    pub fiscal_year: String,
    pub today: String,
    pub database_path: String,
    pub account_count: i64,
    pub group_count: i64,
    pub voucher_count: i64,
    pub line_count: i64,
    pub total_debit: i64,
    pub total_credit: i64,
    pub assets_total: i64,
    pub liabilities_total: i64,
    pub equity_total: i64,
    pub income_total: i64,
    pub expense_total: i64,
    pub net_profit: i64,
    pub recent: Vec<VoucherSummary>,
}

/* ── Validation helpers ─────────────────────────────────────── */

fn normalize_date(input: &str, label: &str) -> Result<String, String> {
    let text = normalize_digits(input).trim().replace('-', "/");
    if text.is_empty() {
        return Err(format!("{label} الزامی است."));
    }
    let parts: Vec<&str> = text.split('/').collect();
    if parts.len() != 3 {
        return Err(format!("{label} باید به شکل ۱۴۰۵/۰۱/۰۱ وارد شود."));
    }
    let year = parts[0]
        .parse::<i32>()
        .map_err(|_| format!("{label} معتبر نیست."))?;
    let month = parts[1]
        .parse::<u32>()
        .map_err(|_| format!("{label} معتبر نیست."))?;
    let day = parts[2]
        .parse::<u32>()
        .map_err(|_| format!("{label} معتبر نیست."))?;
    jalali_calendar::JalaliDate::new(year, month, day)
        .map_err(|_| format!("{label} یک تاریخ شمسی معتبر نیست."))?;
    Ok(format!("{year:04}/{month:02}/{day:02}"))
}

/// Empty text means "unbounded"; anything else must be a real Jalali date.
pub fn normalize_filter_date(input: &str, label: &str) -> Result<String, String> {
    if input.trim().is_empty() {
        return Ok(String::new());
    }
    normalize_date(input, label)
}

/// Text typed in a search box, prepared for `LIKE ... ESCAPE '\\'`.
struct SearchTerms {
    /// Persian/Arabic digits and Arabic yeh/kaf folded to their usual forms.
    folded: String,
    /// What the user actually typed.
    raw: String,
}

fn escape_like(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn search_terms(input: &str) -> SearchTerms {
    let raw = input.trim();
    let folded: String = normalize_digits(raw)
        .chars()
        .map(|ch| match ch {
            '\u{064A}' | '\u{0649}' => '\u{06CC}',
            '\u{0643}' => '\u{06A9}',
            other => other,
        })
        .collect();
    SearchTerms {
        folded: escape_like(&folded),
        raw: escape_like(raw),
    }
}

/// SQL predicate matching a voucher against the search placeholder `p`
/// (`?3` or `?4`). Wildcards in the text are escaped by `search_terms`.
fn search_clause(p: &str) -> String {
    format!(
        "CAST(v.number AS TEXT) LIKE '%' || {p} || '%' ESCAPE '\\'
         OR v.description LIKE '%' || {p} || '%' ESCAPE '\\'
         OR v.type LIKE '%' || {p} || '%' ESCAPE '\\'
         OR EXISTS (
             SELECT 1
               FROM voucher_lines l2
               JOIN accounts a2 ON a2.code = l2.account_code
              WHERE l2.voucher_id = v.id
                AND (a2.name LIKE '%' || {p} || '%' ESCAPE '\\'
                     OR l2.account_code LIKE '%' || {p} || '%' ESCAPE '\\'
                     OR l2.description LIKE '%' || {p} || '%' ESCAPE '\\')
         )"
    )
}

fn search_predicate() -> String {
    format!("(?3 = '' OR {} OR {})", search_clause("?3"), search_clause("?4"))
}

fn check_range(from: &str, to: &str) -> Result<(), String> {
    if !from.is_empty() && !to.is_empty() && from > to {
        return Err("تاریخ شروع نمی‌تواند بعد از تاریخ پایان باشد.".to_string());
    }
    Ok(())
}

fn validate_description(input: &str, label: &str, max: usize) -> Result<String, String> {
    let value = input.trim().to_string();
    if value.chars().count() > max {
        return Err(format!(
            "{label} نمی‌تواند بیشتر از {} کاراکتر باشد.",
            db::to_fa_digits(max)
        ));
    }
    Ok(value)
}

fn validate_voucher_type(input: &str) -> Result<String, String> {
    let value = input.trim();
    if value.is_empty() {
        return Ok(VOUCHER_TYPES[0].to_string());
    }
    if VOUCHER_TYPES.contains(&value) {
        Ok(value.to_string())
    } else {
        Err("نوع سند انتخاب‌شده معتبر نیست.".to_string())
    }
}

/* ── Voucher commands ───────────────────────────────────────── */

impl Database {
    pub fn next_voucher_number(&self) -> Result<i64, String> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT COALESCE(MAX(number), 0) + 1 FROM vouchers",
                [],
                |row| row.get(0),
            )
            .map_err(db::db_error)
        })
    }

    pub fn list_vouchers(
        &self,
        from: &str,
        to: &str,
        search: &str,
    ) -> Result<VoucherList, String> {
        let from = normalize_filter_date(from, "تاریخ شروع")?;
        let to = normalize_filter_date(to, "تاریخ پایان")?;
        check_range(&from, &to)?;
        let terms = search_terms(search);

        let (vouchers, total_count) = self.with_conn(|conn| {
            let vouchers = list_vouchers(conn, &from, &to, &terms)?;
            let total = count(conn, "SELECT COUNT(*) FROM vouchers")?;
            Ok((vouchers, total))
        })?;
        let total_debit = vouchers.iter().map(|v| v.total_debit).sum();
        let total_credit = vouchers.iter().map(|v| v.total_credit).sum();
        Ok(VoucherList {
            vouchers,
            total_count,
            total_debit,
            total_credit,
            database_path: self.path_string(),
        })
    }

    pub fn get_voucher(&self, number: i64) -> Result<Option<Voucher>, String> {
        if number <= 0 {
            return Ok(None);
        }
        self.with_conn(|conn| fetch_voucher_by_number(conn, number))
    }

    pub fn save_voucher(&self, input: VoucherInput) -> Result<Voucher, String> {
        let date = normalize_date(&input.date, "تاریخ سند")?;
        let voucher_type = validate_voucher_type(&input.voucher_type)?;
        let description = validate_description(&input.description, "شرح سند", MAX_DESCRIPTION)?;

        if input.lines.len() > MAX_LINES {
            return Err(format!(
                "تعداد ردیف‌های سند نمی‌تواند بیشتر از {} باشد.",
                db::to_fa_digits(MAX_LINES)
            ));
        }

        // Drop rows the user left entirely blank, then validate what remains.
        let mut lines: Vec<VoucherLineInput> = Vec::new();
        for line in input.lines {
            let account_code = db::normalize_code(&line.account_code);
            let line_description =
                validate_description(&line.description, "شرح ردیف", MAX_LINE_DESCRIPTION)?;
            let blank = account_code.is_empty()
                && line_description.is_empty()
                && line.debit == 0
                && line.credit == 0;
            if blank {
                continue;
            }
            if account_code.is_empty() {
                return Err("برای هر ردیف، کد حساب الزامی است.".to_string());
            }
            if line.debit < 0 || line.credit < 0 {
                return Err("مبلغ بدهکار یا بستانکار نمی‌تواند منفی باشد.".to_string());
            }
            if line.debit > MAX_AMOUNT || line.credit > MAX_AMOUNT {
                return Err(format!(
                    "مبلغ هر ردیف نمی‌تواند بیشتر از {} ریال باشد.",
                    db::to_fa_digits(format_amount(MAX_AMOUNT))
                ));
            }
            if line.debit == 0 && line.credit == 0 {
                return Err("برای هر ردیف، مبلغ بدهکار یا بستانکار الزامی است.".to_string());
            }
            if line.debit > 0 && line.credit > 0 {
                return Err("در یک ردیف فقط یکی از مبالغ بدهکار یا بستانکار ثبت می‌شود.".to_string());
            }
            lines.push(VoucherLineInput {
                account_code,
                description: line_description,
                debit: line.debit,
                credit: line.credit,
            });
        }

        if lines.len() < 2 {
            return Err("سند حسابداری باید حداقل دو ردیف داشته باشد.".to_string());
        }

        // Each line is capped and the line count is capped, so these sums fit
        // in an i64; `checked_add` keeps that guarantee explicit.
        let sum = |pick: fn(&VoucherLineInput) -> i64| -> Result<i64, String> {
            lines.iter().try_fold(0i64, |acc, l| {
                acc.checked_add(pick(l))
                    .ok_or_else(|| "جمع مبالغ سند بیش از حد مجاز است.".to_string())
            })
        };
        let total_debit = sum(|l| l.debit)?;
        let total_credit = sum(|l| l.credit)?;
        if total_debit == 0 {
            return Err("مبلغ سند باید بزرگ‌تر از صفر باشد.".to_string());
        }
        if total_debit != total_credit {
            return Err(format!(
                "سند تراز نیست: جمع بدهکار {} و جمع بستانکار {} است.",
                db::to_fa_digits(format_amount(total_debit)),
                db::to_fa_digits(format_amount(total_credit))
            ));
        }

        let id = input.id.unwrap_or(0);
        let requested_number = input.number.unwrap_or(0);
        if requested_number < 0 {
            return Err("شماره سند نمی‌تواند منفی باشد.".to_string());
        }

        self.with_conn(move |conn| {
            let tx = conn.unchecked_transaction().map_err(db::db_error)?;

            // Every line must point at a real, postable account.
            for line in &lines {
                let account = tx
                    .query_row(
                        "SELECT name, is_postable FROM accounts WHERE code = ?1",
                        params![line.account_code],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .optional()
                    .map_err(db::db_error)?
                    .ok_or_else(|| {
                        format!("حساب با کد {} یافت نشد.", db::to_fa_digits(&line.account_code))
                    })?;
                if account.1 == 0 {
                    return Err(format!(
                        "حساب «{}» قابل ثبت نیست؛ حساب کل یا حساب غیرقابل ثبت را انتخاب نکنید.",
                        account.0
                    ));
                }
            }

            let existing_id = if id > 0 {
                tx.query_row("SELECT id FROM vouchers WHERE id = ?1", params![id], |row| {
                    row.get::<_, i64>(0)
                })
                .optional()
                .map_err(db::db_error)?
            } else {
                None
            };
            if id > 0 && existing_id.is_none() {
                return Err("سند حسابداری یافت نشد.".to_string());
            }

            let number = if requested_number > 0 {
                requested_number
            } else {
                tx.query_row(
                    "SELECT COALESCE(MAX(number), 0) + 1 FROM vouchers",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(db::db_error)?
            };

            let duplicate = tx
                .query_row(
                    "SELECT id FROM vouchers WHERE number = ?1 AND id <> ?2",
                    params![number, if id > 0 { id } else { 0 }],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(db::db_error)?;
            if duplicate.is_some() {
                return Err("شماره سند تکراری است؛ شماره دیگری انتخاب کنید.".to_string());
            }

            let voucher_id = if id > 0 {
                tx.execute(
                    "UPDATE vouchers
                        SET number = ?1, date = ?2, type = ?3, description = ?4,
                            updated_at = datetime('now', 'localtime')
                      WHERE id = ?5",
                    params![number, date, voucher_type, description, id],
                )
                .map_err(db::db_error)?;
                tx.execute(
                    "DELETE FROM voucher_lines WHERE voucher_id = ?1",
                    params![id],
                )
                .map_err(db::db_error)?;
                id
            } else {
                tx.execute(
                    "INSERT INTO vouchers (number, date, type, description)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![number, date, voucher_type, description],
                )
                .map_err(db::db_error)?;
                tx.last_insert_rowid()
            };

            for (index, line) in lines.iter().enumerate() {
                tx.execute(
                    "INSERT INTO voucher_lines
                        (voucher_id, row_no, account_code, description, debit, credit)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        voucher_id,
                        index as i64 + 1,
                        line.account_code,
                        line.description,
                        line.debit,
                        line.credit
                    ],
                )
                .map_err(db::db_error)?;
            }

            tx.commit().map_err(db::db_error)?;
            fetch_voucher_by_id(conn, voucher_id)
        })
    }

    pub fn delete_voucher(&self, id: i64) -> Result<(), String> {
        if id <= 0 {
            return Err("شناسه سند نامعتبر است.".to_string());
        }
        self.with_conn(|conn| {
            let affected = conn
                .execute("DELETE FROM vouchers WHERE id = ?1", params![id])
                .map_err(db::db_error)?;
            if affected == 0 {
                return Err("سند حسابداری یافت نشد.".to_string());
            }
            Ok(())
        })
    }

    /* ── Reports ─────────────────────────────────────────────── */

    pub fn get_journal(
        &self,
        from: &str,
        to: &str,
        search: &str,
    ) -> Result<JournalReport, String> {
        let from = normalize_filter_date(from, "تاریخ شروع")?;
        let to = normalize_filter_date(to, "تاریخ پایان")?;
        check_range(&from, &to)?;
        let terms = search_terms(search);
        let search = search.trim().to_string();

        self.with_conn(|conn| {
            let sql = format!(
                "SELECT v.id, v.number, v.date, v.type, v.description,
                        l.row_no, l.account_code, a.name, l.description,
                        l.debit, l.credit
                   FROM voucher_lines l
                   JOIN vouchers v ON v.id = l.voucher_id
                   JOIN accounts a ON a.code = l.account_code
                  WHERE (?1 = '' OR v.date >= ?1)
                    AND (?2 = '' OR v.date <= ?2)
                    AND {}
                  ORDER BY v.date, v.number, l.row_no",
                search_predicate()
            );
            let mut stmt = conn.prepare(&sql).map_err(db::db_error)?;

            let rows = stmt
                .query_map(params![from, to, terms.folded, terms.raw], |row| {
                    Ok(JournalRow {
                        voucher_id: row.get(0)?,
                        number: row.get(1)?,
                        date: row.get(2)?,
                        voucher_type: row.get(3)?,
                        voucher_description: row.get(4)?,
                        row_no: row.get(5)?,
                        account_code: row.get(6)?,
                        account_name: row.get(7)?,
                        line_description: row.get(8)?,
                        debit: row.get(9)?,
                        credit: row.get(10)?,
                    })
                })
                .map_err(db::db_error)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db::db_error)?;

            let total_debit = rows.iter().map(|r| r.debit).sum();
            let total_credit = rows.iter().map(|r| r.credit).sum();
            let voucher_count = rows
                .iter()
                .map(|r| r.voucher_id)
                .collect::<std::collections::HashSet<_>>()
                .len() as i64;

            Ok(JournalReport {
                from,
                to,
                search,
                rows,
                total_debit,
                total_credit,
                voucher_count,
            })
        })
    }

    pub fn get_ledger(&self, account_code: &str, from: &str, to: &str) -> Result<LedgerReport, String> {
        let code = db::normalize_code(account_code);
        if code.is_empty() {
            return Err("برای گزارش دفتر کل، حساب را انتخاب کنید.".to_string());
        }
        let from = normalize_filter_date(from, "تاریخ شروع")?;
        let to = normalize_filter_date(to, "تاریخ پایان")?;
        check_range(&from, &to)?;

        self.with_conn(|conn| {
            let (account_name, group_name) = conn
                .query_row(
                    "SELECT a.name, g.name
                       FROM accounts a
                       JOIN account_groups g ON g.code = a.group_code
                      WHERE a.code = ?1",
                    params![code],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(db::db_error)?
                .ok_or_else(|| "حساب انتخاب‌شده یافت نشد.".to_string())?;

            // Vouchers are only posted to leaf accounts, so the ledger of a
            // parent account is the combined activity of all its descendants.
            const TREE: &str = "WITH RECURSIVE tree(code) AS (
                    SELECT code FROM accounts WHERE code = ?1
                    UNION ALL
                    SELECT a.code FROM accounts a JOIN tree t ON a.parent_code = t.code
                )";

            let includes_children: bool = conn
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM accounts WHERE parent_code = ?1)",
                    params![code],
                    |row| row.get(0),
                )
                .map_err(db::db_error)?;

            let (opening_debit, opening_credit): (i64, i64) = conn
                .query_row(
                    &format!(
                        "{TREE}
                         SELECT COALESCE(SUM(l.debit), 0), COALESCE(SUM(l.credit), 0)
                           FROM voucher_lines l
                           JOIN vouchers v ON v.id = l.voucher_id
                          WHERE l.account_code IN (SELECT code FROM tree)
                            AND (?2 <> '' AND v.date < ?2)"
                    ),
                    params![code, from],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(db::db_error)?;

            let mut stmt = conn
                .prepare(&format!(
                    "{TREE}
                     SELECT v.id, v.number, v.date, v.type, v.description,
                            l.description, l.debit, l.credit, l.account_code, a.name
                       FROM voucher_lines l
                       JOIN vouchers v ON v.id = l.voucher_id
                       JOIN accounts a ON a.code = l.account_code
                      WHERE l.account_code IN (SELECT code FROM tree)
                        AND (?2 = '' OR v.date >= ?2)
                        AND (?3 = '' OR v.date <= ?3)
                      ORDER BY v.date, v.number, l.row_no"
                ))
                .map_err(db::db_error)?;

            let raw_rows = stmt
                .query_map(params![code, from, to], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                    ))
                })
                .map_err(db::db_error)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db::db_error)?;

            let opening_net = opening_debit - opening_credit;
            let mut running = opening_net;
            let mut total_debit = 0i64;
            let mut total_credit = 0i64;
            let mut rows = Vec::with_capacity(raw_rows.len());
            for (
                voucher_id,
                number,
                date,
                voucher_type,
                voucher_description,
                line_description,
                debit,
                credit,
                row_account_code,
                row_account_name,
            ) in raw_rows
            {
                running += debit - credit;
                total_debit += debit;
                total_credit += credit;
                rows.push(LedgerRow {
                    voucher_id,
                    account_code: row_account_code,
                    account_name: row_account_name,
                    number,
                    date,
                    voucher_type,
                    voucher_description,
                    line_description,
                    debit,
                    credit,
                    running_debit: running.max(0),
                    running_credit: (-running).max(0),
                });
            }

            let closing = opening_net + total_debit - total_credit;
            Ok(LedgerReport {
                account_code: code,
                account_name,
                group_name,
                includes_children,
                from,
                to,
                opening_debit: opening_net.max(0),
                opening_credit: (-opening_net).max(0),
                total_debit,
                total_credit,
                closing_debit: closing.max(0),
                closing_credit: (-closing).max(0),
                rows,
            })
        })
    }

    pub fn get_trial_balance(&self, from: &str, to: &str) -> Result<TrialBalanceReport, String> {
        let from = normalize_filter_date(from, "تاریخ شروع")?;
        let to = normalize_filter_date(to, "تاریخ پایان")?;
        check_range(&from, &to)?;

        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT a.code, a.name, g.name, a.level,
                            COALESCE(SUM(CASE
                                WHEN v.date IS NOT NULL AND ?1 <> '' AND v.date < ?1 AND (?2 = '' OR v.date <= ?2)
                                THEN l.debit ELSE 0 END), 0) AS opening_debit,
                            COALESCE(SUM(CASE
                                WHEN v.date IS NOT NULL AND ?1 <> '' AND v.date < ?1 AND (?2 = '' OR v.date <= ?2)
                                THEN l.credit ELSE 0 END), 0) AS opening_credit,
                            COALESCE(SUM(CASE
                                WHEN v.date IS NOT NULL AND (?1 = '' OR v.date >= ?1) AND (?2 = '' OR v.date <= ?2)
                                THEN l.debit ELSE 0 END), 0) AS period_debit,
                            COALESCE(SUM(CASE
                                WHEN v.date IS NOT NULL AND (?1 = '' OR v.date >= ?1) AND (?2 = '' OR v.date <= ?2)
                                THEN l.credit ELSE 0 END), 0) AS period_credit
                       FROM accounts a
                       JOIN account_groups g ON g.code = a.group_code
                       LEFT JOIN voucher_lines l ON l.account_code = a.code
                       LEFT JOIN vouchers v ON v.id = l.voucher_id
                      GROUP BY a.code
                      ORDER BY a.code",
                )
                .map_err(db::db_error)?;

            let raw = stmt
                .query_map(params![from, to], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                    ))
                })
                .map_err(db::db_error)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db::db_error)?;

            let mut rows = Vec::new();
            for (code, name, group_name, level, od, oc, pd, pc) in raw {
                if od == 0 && oc == 0 && pd == 0 && pc == 0 {
                    continue;
                }
                let opening_net = od - oc;
                let closing_net = opening_net + pd - pc;
                rows.push(TrialBalanceRow {
                    account_code: code,
                    account_name: name,
                    group_name,
                    level,
                    opening_debit: opening_net.max(0),
                    opening_credit: (-opening_net).max(0),
                    period_debit: pd,
                    period_credit: pc,
                    closing_debit: closing_net.max(0),
                    closing_credit: (-closing_net).max(0),
                });
            }

            let opening_debit = rows.iter().map(|r| r.opening_debit).sum();
            let opening_credit = rows.iter().map(|r| r.opening_credit).sum();
            let period_debit = rows.iter().map(|r| r.period_debit).sum();
            let period_credit = rows.iter().map(|r| r.period_credit).sum();
            let closing_debit = rows.iter().map(|r| r.closing_debit).sum();
            let closing_credit = rows.iter().map(|r| r.closing_credit).sum();

            Ok(TrialBalanceReport {
                from,
                to,
                rows,
                opening_debit,
                opening_credit,
                period_debit,
                period_credit,
                closing_debit,
                closing_credit,
            })
        })
    }

    pub fn get_income_statement(
        &self,
        from: &str,
        to: &str,
    ) -> Result<IncomeStatementReport, String> {
        let from = normalize_filter_date(from, "تاریخ شروع")?;
        let to = normalize_filter_date(to, "تاریخ پایان")?;
        check_range(&from, &to)?;

        self.with_conn(|conn| {
            // Closing vouchers move income and expenses to equity; leaving
            // them in would show every closed period as having no activity.
            let activities = account_activities(
                conn,
                &from,
                &to,
                false,
                &[CLOSING_TEMPORARY, CLOSING_FINAL],
            )?;
            build_income_statement(from, to, activities)
        })
    }

    pub fn get_balance_sheet(&self, as_of: &str) -> Result<BalanceSheetReport, String> {
        let as_of = normalize_filter_date(as_of, "تاریخ ترازنامه")?;
        self.with_conn(|conn| {
            // The final closing voucher zeroes every balance-sheet account,
            // so the statement is prepared from the books before it.
            let activities = account_activities(conn, "", &as_of, true, &[CLOSING_FINAL])?;
            build_balance_sheet(as_of, activities)
        })
    }

    pub fn get_dashboard(&self) -> Result<Dashboard, String> {
        let settings = self.get_settings()?;
        self.with_conn(|conn| {
            let account_count = count(conn, "SELECT COUNT(*) FROM accounts")?;
            let group_count = count(conn, "SELECT COUNT(*) FROM account_groups")?;
            let voucher_count = count(conn, "SELECT COUNT(*) FROM vouchers")?;
            let line_count = count(conn, "SELECT COUNT(*) FROM voucher_lines")?;
            let (total_debit, total_credit): (i64, i64) = conn
                .query_row(
                    "SELECT COALESCE(SUM(debit), 0), COALESCE(SUM(credit), 0) FROM voucher_lines",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(db::db_error)?;

            let mut assets_total = 0i64;
            let mut liabilities_total = 0i64;
            let mut equity_total = 0i64;
            let mut income_total = 0i64;
            let mut expense_total = 0i64;

            // Balance-sheet figures come from the books before the final
            // closing voucher; income and expenses also leave out the
            // voucher that closes the temporary accounts.
            for activity in &account_activities(conn, "", "", false, &[CLOSING_FINAL])? {
                let net = activity.debit - activity.credit;
                match classify(&activity.group_type, &activity.account_code) {
                    AccountClass::Asset => assets_total += net,
                    AccountClass::Liability => liabilities_total += -net,
                    AccountClass::Equity => equity_total += -net,
                    AccountClass::Income | AccountClass::Expense => {}
                }
            }
            for activity in &account_activities(
                conn,
                "",
                "",
                false,
                &[CLOSING_TEMPORARY, CLOSING_FINAL],
            )? {
                let net = activity.debit - activity.credit;
                match classify(&activity.group_type, &activity.account_code) {
                    AccountClass::Income => income_total += -net,
                    AccountClass::Expense => expense_total += net,
                    _ => {}
                }
            }

            let recent = list_vouchers(conn, "", "", &search_terms(""))?
                .into_iter()
                .take(8)
                .collect();

            Ok(Dashboard {
                company_name: settings.company_name,
                fiscal_year: settings.fiscal_year,
                today: settings.today,
                database_path: self.path_string(),
                account_count,
                group_count,
                voucher_count,
                line_count,
                total_debit,
                total_credit,
                assets_total,
                liabilities_total,
                equity_total,
                income_total,
                expense_total,
                net_profit: income_total - expense_total,
                recent,
            })
        })
    }
}

/* ── Internal queries ───────────────────────────────────────── */

fn count(conn: &Connection, sql: &str) -> Result<i64, String> {
    conn.query_row(sql, [], |row| row.get(0))
        .map_err(db::db_error)
}

fn list_vouchers(
    conn: &Connection,
    from: &str,
    to: &str,
    terms: &SearchTerms,
) -> Result<Vec<VoucherSummary>, String> {
    let sql = format!(
        "SELECT v.id, v.number, v.date, v.type, v.description,
                COALESCE(SUM(l.debit), 0), COALESCE(SUM(l.credit), 0), COUNT(l.id)
           FROM vouchers v
           LEFT JOIN voucher_lines l ON l.voucher_id = v.id
          WHERE (?1 = '' OR v.date >= ?1)
            AND (?2 = '' OR v.date <= ?2)
            AND {}
          GROUP BY v.id
          ORDER BY v.date DESC, v.number DESC",
        search_predicate()
    );
    let mut stmt = conn.prepare(&sql).map_err(db::db_error)?;

    let rows = stmt
        .query_map(params![from, to, terms.folded, terms.raw], |row| {
            Ok(VoucherSummary {
                id: row.get(0)?,
                number: row.get(1)?,
                date: row.get(2)?,
                voucher_type: row.get(3)?,
                description: row.get(4)?,
                total_debit: row.get(5)?,
                total_credit: row.get(6)?,
                line_count: row.get(7)?,
            })
        })
        .map_err(db::db_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db::db_error)?;
    Ok(rows)
}

fn fetch_voucher_by_number(conn: &Connection, number: i64) -> Result<Option<Voucher>, String> {
    conn.query_row(
        "SELECT id FROM vouchers WHERE number = ?1",
        params![number],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map_err(db::db_error)?
    .map(|id| fetch_voucher_by_id(conn, id))
    .transpose()
}

fn fetch_voucher_by_id(conn: &Connection, id: i64) -> Result<Voucher, String> {
    let (number, date, voucher_type, description) = conn
        .query_row(
            "SELECT number, date, type, description FROM vouchers WHERE id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(db::db_error)?
        .ok_or_else(|| "سند حسابداری یافت نشد.".to_string())?;

    let mut stmt = conn
        .prepare(
            "SELECT l.row_no, l.account_code, a.name, l.description, l.debit, l.credit
               FROM voucher_lines l
               JOIN accounts a ON a.code = l.account_code
              WHERE l.voucher_id = ?1
              ORDER BY l.row_no",
        )
        .map_err(db::db_error)?;
    let lines = stmt
        .query_map(params![id], |row| {
            Ok(VoucherLine {
                row_no: row.get(0)?,
                account_code: row.get(1)?,
                account_name: row.get(2)?,
                description: row.get(3)?,
                debit: row.get(4)?,
                credit: row.get(5)?,
            })
        })
        .map_err(db::db_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db::db_error)?;

    let total_debit = lines.iter().map(|l| l.debit).sum();
    let total_credit = lines.iter().map(|l| l.credit).sum();
    Ok(Voucher {
        id,
        number,
        date,
        voucher_type,
        description,
        lines,
        total_debit,
        total_credit,
    })
}

#[derive(Debug, Clone)]
struct AccountActivity {
    account_code: String,
    account_name: String,
    group_code: String,
    group_type: String,
    debit: i64,
    credit: i64,
}

/// Per-account totals for a date range. Vouchers whose type is in
/// `skip_types` are left out of the totals.
fn account_activities(
    conn: &Connection,
    from: &str,
    to: &str,
    cumulative: bool,
    skip_types: &[&str],
) -> Result<Vec<AccountActivity>, String> {
    let skip_a = skip_types.first().copied().unwrap_or("");
    let skip_b = skip_types.get(1).copied().unwrap_or(skip_a);
    // The filters live in the join condition so accounts with no matching
    // lines still come back (with zero totals) instead of disappearing.
    let mut stmt = conn
        .prepare(
            "SELECT a.code, a.name, g.code, g.type,
                    COALESCE(SUM(l.debit), 0), COALESCE(SUM(l.credit), 0)
               FROM accounts a
               JOIN account_groups g ON g.code = a.group_code
               LEFT JOIN (
                    voucher_lines l
                    JOIN vouchers v ON v.id = l.voucher_id
                     AND (?1 = '' OR v.date >= ?1)
                     AND (?2 = '' OR v.date <= ?2)
                     AND v.type <> ?3
                     AND v.type <> ?4
               ) ON l.account_code = a.code
              GROUP BY a.code
              ORDER BY a.code",
        )
        .map_err(db::db_error)?;

    let (from_value, to_value) = if cumulative {
        ("".to_string(), to.to_string())
    } else {
        (from.to_string(), to.to_string())
    };

    let rows = stmt
        .query_map(params![from_value, to_value, skip_a, skip_b], |row| {
            Ok(AccountActivity {
                account_code: row.get(0)?,
                account_name: row.get(1)?,
                group_code: row.get(2)?,
                group_type: row.get(3)?,
                debit: row.get(4)?,
                credit: row.get(5)?,
            })
        })
        .map_err(db::db_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db::db_error)?;
    Ok(rows)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccountClass {
    Asset,
    Liability,
    Equity,
    Income,
    Expense,
}

/// Group type is the primary classification. «سایر» falls back to the
/// conventional first digit of the account code (1/2/3 balance sheet,
/// 4 income, 5-9 expenses).
fn classify(group_type: &str, account_code: &str) -> AccountClass {
    match group_type {
        "دارایی" => AccountClass::Asset,
        "بدهی" => AccountClass::Liability,
        "سرمایه" => AccountClass::Equity,
        "درآمد" => AccountClass::Income,
        "هزینه" => AccountClass::Expense,
        _ => match account_code.chars().next() {
            Some('1') => AccountClass::Asset,
            Some('2') => AccountClass::Liability,
            Some('3') => AccountClass::Equity,
            Some('4') => AccountClass::Income,
            _ => AccountClass::Expense,
        },
    }
}

fn build_income_statement(
    from: String,
    to: String,
    activities: Vec<AccountActivity>,
) -> Result<IncomeStatementReport, String> {
    let mut revenues = Vec::new();
    let mut cogs = Vec::new();
    let mut expenses = Vec::new();

    for activity in activities {
        let net = activity.debit - activity.credit;
        if net == 0 {
            continue;
        }
        match classify(&activity.group_type, &activity.account_code) {
            AccountClass::Income => {
                let amount = -net;
                if amount != 0 {
                    revenues.push(AmountRow {
                        account_code: activity.account_code,
                        account_name: activity.account_name,
                        amount,
                    });
                }
            }
            AccountClass::Expense => {
                let amount = net;
                if amount == 0 {
                    continue;
                }
                let row = AmountRow {
                    account_code: activity.account_code.clone(),
                    account_name: activity.account_name,
                    amount,
                };
                let is_cogs = activity.group_code == "9"
                    || activity.account_code.starts_with('5');
                if is_cogs {
                    cogs.push(row);
                } else {
                    expenses.push(row);
                }
            }
            _ => {}
        }
    }

    let revenue_total: i64 = revenues.iter().map(|r| r.amount).sum();
    let cogs_total: i64 = cogs.iter().map(|r| r.amount).sum();
    let expense_total: i64 = expenses.iter().map(|r| r.amount).sum();
    let gross_profit = revenue_total - cogs_total;
    let operating_profit = gross_profit - expense_total;
    Ok(IncomeStatementReport {
        from,
        to,
        revenues,
        revenue_total,
        cogs,
        cogs_total,
        expenses,
        expense_total,
        gross_profit,
        operating_profit,
        net_profit: operating_profit,
    })
}

fn build_balance_sheet(
    as_of: String,
    activities: Vec<AccountActivity>,
) -> Result<BalanceSheetReport, String> {
    let mut assets = Vec::new();
    let mut liabilities = Vec::new();
    let mut equity = Vec::new();
    let mut revenue = 0;
    let mut expenses = 0;

    for activity in activities {
        let net = activity.debit - activity.credit;
        if net == 0 {
            continue;
        }
        match classify(&activity.group_type, &activity.account_code) {
            AccountClass::Asset => assets.push(AmountRow {
                account_code: activity.account_code,
                account_name: activity.account_name,
                amount: net,
            }),
            AccountClass::Liability => liabilities.push(AmountRow {
                account_code: activity.account_code,
                account_name: activity.account_name,
                amount: -net,
            }),
            AccountClass::Equity => equity.push(AmountRow {
                account_code: activity.account_code,
                account_name: activity.account_name,
                amount: -net,
            }),
            AccountClass::Income => revenue += -net,
            AccountClass::Expense => expenses += net,
        }
    }

    let assets_total: i64 = assets.iter().map(|r| r.amount).sum();
    let liabilities_total: i64 = liabilities.iter().map(|r| r.amount).sum();
    let equity_total: i64 = equity.iter().map(|r| r.amount).sum();
    let profit = revenue - expenses;
    let total_liabilities_equity = liabilities_total + equity_total + profit;
    Ok(BalanceSheetReport {
        as_of,
        assets,
        assets_total,
        liabilities,
        liabilities_total,
        equity,
        equity_total,
        profit,
        total_liabilities_equity,
        balanced: assets_total == total_liabilities_equity,
    })
}

pub fn format_amount(value: i64) -> String {
    let negative = value < 0;
    let digits = value.abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if negative {
        out.insert(0, '-');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(account_code: &str, debit: i64, credit: i64) -> VoucherLineInput {
        VoucherLineInput {
            account_code: account_code.to_string(),
            description: String::new(),
            debit,
            credit,
        }
    }

    fn voucher(date: &str, lines: Vec<VoucherLineInput>) -> VoucherInput {
        VoucherInput {
            id: None,
            number: None,
            date: date.to_string(),
            voucher_type: String::new(),
            description: "سند آزمایشی".to_string(),
            lines,
        }
    }

    fn cash_sale(date: &str, amount: i64) -> VoucherInput {
        voucher(
            date,
            vec![
                line("110101", amount, 0),
                line("410101", 0, amount),
            ],
        )
    }

    #[test]
    fn saves_balanced_voucher_and_builds_reports() {
        let db = db::memory_database();
        let saved = db.save_voucher(cash_sale("1405/01/10", 1_000_000)).unwrap();

        assert_eq!(saved.number, 1);
        assert_eq!(saved.voucher_type, "سند حسابداری عادی");
        assert_eq!(saved.lines.len(), 2);
        assert_eq!(saved.lines[0].account_name, "صندوق مرکزی");
        assert_eq!(saved.total_debit, 1_000_000);
        assert_eq!(saved.total_credit, 1_000_000);

        let list = db.list_vouchers("", "", "").unwrap();
        assert_eq!(list.vouchers.len(), 1);
        assert_eq!(list.total_debit, 1_000_000);

        let journal = db.get_journal("", "", "").unwrap();
        assert_eq!(journal.rows.len(), 2);
        assert_eq!(journal.voucher_count, 1);
        assert_eq!(journal.total_debit, journal.total_credit);

        let trial = db.get_trial_balance("1405/01/01", "1405/01/31").unwrap();
        let cash = trial
            .rows
            .iter()
            .find(|r| r.account_code == "110101")
            .unwrap();
        assert_eq!(cash.period_debit, 1_000_000);
        assert_eq!(cash.closing_debit, 1_000_000);
        let revenue = trial
            .rows
            .iter()
            .find(|r| r.account_code == "410101")
            .unwrap();
        assert_eq!(revenue.period_credit, 1_000_000);
        assert_eq!(revenue.closing_credit, 1_000_000);
        assert_eq!(trial.closing_debit, trial.closing_credit);

        // With no start date there is no opening balance; everything is period
        // activity. This guards against counting the same lines twice.
        let all_trial = db.get_trial_balance("", "").unwrap();
        let cash_all = all_trial
            .rows
            .iter()
            .find(|r| r.account_code == "110101")
            .unwrap();
        assert_eq!(cash_all.opening_debit, 0);
        assert_eq!(cash_all.period_debit, 1_000_000);
        assert_eq!(cash_all.closing_debit, 1_000_000);

        let ledger = db.get_ledger("110101", "", "").unwrap();
        assert_eq!(ledger.account_name, "صندوق مرکزی");
        assert_eq!(ledger.rows.len(), 1);
        assert_eq!(ledger.total_debit, 1_000_000);
        assert_eq!(ledger.closing_debit, 1_000_000);
        assert_eq!(ledger.rows[0].running_debit, 1_000_000);

        let income = db.get_income_statement("1405/01/01", "1405/01/31").unwrap();
        assert_eq!(income.revenue_total, 1_000_000);
        assert_eq!(income.cogs_total, 0);
        assert_eq!(income.net_profit, 1_000_000);

        let balance = db.get_balance_sheet("1405/01/31").unwrap();
        assert_eq!(balance.assets_total, 1_000_000);
        assert_eq!(balance.profit, 1_000_000);
        assert_eq!(balance.total_liabilities_equity, 1_000_000);
        assert!(balance.balanced);

        let dashboard = db.get_dashboard().unwrap();
        assert_eq!(dashboard.voucher_count, 1);
        assert_eq!(dashboard.line_count, 2);
        assert_eq!(dashboard.total_debit, dashboard.total_credit);
        assert_eq!(dashboard.assets_total, 1_000_000);
        assert_eq!(dashboard.net_profit, 1_000_000);
        assert_eq!(dashboard.recent.len(), 1);
    }

    #[test]
    fn rejects_unbalanced_and_invalid_vouchers() {
        let db = db::memory_database();

        let unbalanced = db
            .save_voucher(voucher(
                "1405/01/10",
                vec![line("110101", 100_000, 0), line("410101", 0, 90_000)],
            ))
            .unwrap_err();
        assert!(unbalanced.contains("تراز نیست"));

        let non_postable = db
            .save_voucher(voucher(
                "1405/01/10",
                vec![line("1101", 100_000, 0), line("410101", 0, 100_000)],
            ))
            .unwrap_err();
        assert!(non_postable.contains("قابل ثبت نیست"));

        let unknown = db
            .save_voucher(voucher(
                "1405/01/10",
                vec![line("999999", 100_000, 0), line("410101", 0, 100_000)],
            ))
            .unwrap_err();
        assert!(unknown.contains("یافت نشد"));

        let single = db
            .save_voucher(voucher("1405/01/10", vec![line("110101", 100_000, 0)]))
            .unwrap_err();
        assert!(single.contains("حداقل دو ردیف"));

        let both_sides = db
            .save_voucher(voucher(
                "1405/01/10",
                vec![
                    line("110101", 100_000, 50_000),
                    line("410101", 0, 50_000),
                ],
            ))
            .unwrap_err();
        assert!(both_sides.contains("فقط یکی"));

        let zero = db
            .save_voucher(voucher(
                "1405/01/10",
                vec![line("110101", 0, 0), line("410101", 0, 0)],
            ))
            .unwrap_err();
        assert!(zero.contains("مبلغ بدهکار یا بستانکار"));

        let negative = db
            .save_voucher(voucher(
                "1405/01/10",
                vec![line("110101", -1, 0), line("410101", 0, -1)],
            ))
            .unwrap_err();
        assert!(negative.contains("منفی"));

        let bad_date = db
            .save_voucher(cash_sale("1405/13/01", 100_000))
            .unwrap_err();
        assert!(bad_date.contains("تاریخ"));

        let mut invalid_type = cash_sale("1405/01/10", 100_000);
        invalid_type.voucher_type = "سند نامعتبر".to_string();
        assert!(db.save_voucher(invalid_type).is_err());

        assert!(db.list_vouchers("", "", "").unwrap().vouchers.is_empty());
    }

    #[test]
    fn updates_existing_voucher_without_duplicating_it() {
        let db = db::memory_database();
        let saved = db.save_voucher(cash_sale("1405/01/10", 500_000)).unwrap();

        let updated = db
            .save_voucher(VoucherInput {
                id: Some(saved.id),
                number: Some(saved.number),
                date: "1405/01/11".to_string(),
                voucher_type: "سند حسابداری افتتاحیه".to_string(),
                description: "اصلاح سند".to_string(),
                lines: vec![
                    line("110101", 800_000, 0),
                    line("410101", 0, 800_000),
                ],
            })
            .unwrap();

        assert_eq!(updated.id, saved.id);
        assert_eq!(updated.number, saved.number);
        assert_eq!(updated.date, "1405/01/11");
        assert_eq!(updated.voucher_type, "سند حسابداری افتتاحیه");
        assert_eq!(updated.total_debit, 800_000);

        let list = db.list_vouchers("", "", "").unwrap();
        assert_eq!(list.vouchers.len(), 1);
        assert_eq!(list.vouchers[0].description, "اصلاح سند");
        assert_eq!(list.vouchers[0].line_count, 2);

        let second = db.save_voucher(cash_sale("1405/01/12", 200_000)).unwrap();
        assert_eq!(second.number, 2);

        let duplicate = db
            .save_voucher(VoucherInput {
                id: Some(second.id),
                number: Some(1),
                date: "1405/01/12".to_string(),
                voucher_type: String::new(),
                description: String::new(),
                lines: vec![
                    line("110101", 200_000, 0),
                    line("410101", 0, 200_000),
                ],
            })
            .unwrap_err();
        assert!(duplicate.contains("تکراری"));

        let missing = db
            .save_voucher(VoucherInput {
                id: Some(9999),
                number: None,
                date: "1405/01/12".to_string(),
                voucher_type: String::new(),
                description: String::new(),
                lines: vec![
                    line("110101", 200_000, 0),
                    line("410101", 0, 200_000),
                ],
            })
            .unwrap_err();
        assert!(missing.contains("یافت نشد"));
    }

    #[test]
    fn deletes_voucher_and_its_lines() {
        let db = db::memory_database();
        let saved = db.save_voucher(cash_sale("1405/01/10", 300_000)).unwrap();
        assert_eq!(db.get_voucher(saved.number).unwrap().unwrap().lines.len(), 2);

        db.delete_voucher(saved.id).unwrap();
        assert!(db.get_voucher(saved.number).unwrap().is_none());
        assert!(db.list_vouchers("", "", "").unwrap().vouchers.is_empty());
        assert!(db.get_journal("", "", "").unwrap().rows.is_empty());
        assert_eq!(db.get_dashboard().unwrap().line_count, 0);
        assert!(db.delete_voucher(saved.id).is_err());
    }

    #[test]
    fn trial_balance_splits_opening_and_period_activity() {
        let db = db::memory_database();
        db.save_voucher(cash_sale("1405/01/05", 500_000)).unwrap();
        db.save_voucher(cash_sale("1405/02/05", 300_000)).unwrap();

        let trial = db.get_trial_balance("1405/02/01", "1405/02/31").unwrap();
        let cash = trial
            .rows
            .iter()
            .find(|r| r.account_code == "110101")
            .unwrap();
        assert_eq!(cash.opening_debit, 500_000);
        assert_eq!(cash.period_debit, 300_000);
        assert_eq!(cash.closing_debit, 800_000);

        let revenue = trial
            .rows
            .iter()
            .find(|r| r.account_code == "410101")
            .unwrap();
        assert_eq!(revenue.opening_credit, 500_000);
        assert_eq!(revenue.period_credit, 300_000);
        assert_eq!(revenue.closing_credit, 800_000);
        assert_eq!(trial.opening_debit, trial.opening_credit);
        assert_eq!(trial.closing_debit, trial.closing_credit);

        let ledger = db.get_ledger("110101", "1405/02/01", "1405/02/31").unwrap();
        assert_eq!(ledger.opening_debit, 500_000);
        assert_eq!(ledger.total_debit, 300_000);
        assert_eq!(ledger.closing_debit, 800_000);

        let income = db.get_income_statement("1405/02/01", "1405/02/31").unwrap();
        assert_eq!(income.revenue_total, 300_000);

        let balance = db.get_balance_sheet("1405/02/31").unwrap();
        assert_eq!(balance.assets_total, 800_000);
        assert!(balance.balanced);
    }

    #[test]
    fn filters_vouchers_by_date_and_text() {
        let db = db::memory_database();
        let mut first = cash_sale("1405/01/10", 100_000);
        first.description = "فروش نقدی".to_string();
        db.save_voucher(first).unwrap();
        let mut second = cash_sale("1405/02/10", 250_000);
        second.description = "دریافت از مشتری".to_string();
        db.save_voucher(second).unwrap();

        assert_eq!(
            db.list_vouchers("1405/02/01", "1405/02/31", "")
                .unwrap()
                .vouchers
                .len(),
            1
        );
        assert_eq!(
            db.list_vouchers("", "", "نقدی").unwrap().vouchers.len(),
            1
        );
        assert_eq!(
            db.list_vouchers("", "", "دریافت").unwrap().vouchers.len(),
            1
        );
        assert_eq!(db.list_vouchers("", "", "ناموجود").unwrap().vouchers.len(), 0);
        // Account names are searchable too.
        assert_eq!(
            db.list_vouchers("", "", "صندوق").unwrap().vouchers.len(),
            2
        );

        let journal = db.get_journal("", "", "دریافت").unwrap();
        assert_eq!(journal.rows.len(), 2);
        assert_eq!(journal.voucher_count, 1);
    }

    #[test]
    fn classifies_accounts_by_group_or_code() {
        assert_eq!(classify("دارایی", "9999"), AccountClass::Asset);
        assert_eq!(classify("بدهی", "9999"), AccountClass::Liability);
        assert_eq!(classify("سرمایه", "9999"), AccountClass::Equity);
        assert_eq!(classify("درآمد", "9999"), AccountClass::Income);
        assert_eq!(classify("هزینه", "9999"), AccountClass::Expense);
        assert_eq!(classify("سایر", "1101"), AccountClass::Asset);
        assert_eq!(classify("سایر", "2101"), AccountClass::Liability);
        assert_eq!(classify("سایر", "3101"), AccountClass::Equity);
        assert_eq!(classify("سایر", "4101"), AccountClass::Income);
        assert_eq!(classify("سایر", "5101"), AccountClass::Expense);
        assert_eq!(classify("سایر", "9101"), AccountClass::Expense);
    }

    #[test]
    fn formats_amounts_with_grouping() {
        assert_eq!(format_amount(0), "0");
        assert_eq!(format_amount(999), "999");
        assert_eq!(format_amount(1_000), "1,000");
        assert_eq!(format_amount(1_234_567), "1,234,567");
        assert_eq!(format_amount(-1_500), "-1,500");
    }
    #[test]
    fn closing_vouchers_do_not_hide_income_statement_figures() {
        let db = db::memory_database();
        db.save_voucher(cash_sale("1405/01/10", 1_000_000)).unwrap();

        // Closing the temporary accounts moves the revenue into equity.
        let mut closing = voucher(
            "1405/12/29",
            vec![line("410101", 1_000_000, 0), line("310101", 0, 1_000_000)],
        );
        closing.voucher_type = "سند بستن حساب‌های موقت".to_string();
        db.save_voucher(closing).unwrap();

        let income = db.get_income_statement("1405/01/01", "1405/12/29").unwrap();
        assert_eq!(income.revenue_total, 1_000_000);
        assert_eq!(income.net_profit, 1_000_000);

        let balance = db.get_balance_sheet("1405/12/29").unwrap();
        assert_eq!(balance.assets_total, 1_000_000);
        assert_eq!(balance.equity_total, 1_000_000);
        assert_eq!(balance.profit, 0);
        assert!(balance.balanced);

        let dashboard = db.get_dashboard().unwrap();
        assert_eq!(dashboard.income_total, 1_000_000);
        assert_eq!(dashboard.net_profit, 1_000_000);

        // The trial balance still shows every voucher, closing ones included.
        let trial = db.get_trial_balance("", "").unwrap();
        let revenue = trial.rows.iter().find(|r| r.account_code == "410101").unwrap();
        assert_eq!(revenue.closing_debit + revenue.closing_credit, 0);
        assert_eq!(trial.closing_debit, trial.closing_credit);

        // After the final closing voucher the balance sheet still reads the
        // books as they stood before it.
        let mut final_closing = voucher(
            "1405/12/29",
            vec![line("310101", 1_000_000, 0), line("110101", 0, 1_000_000)],
        );
        final_closing.voucher_type = "سند حسابداری اختتامیه".to_string();
        db.save_voucher(final_closing).unwrap();
        let balance = db.get_balance_sheet("1405/12/29").unwrap();
        assert_eq!(balance.assets_total, 1_000_000);
        assert!(balance.balanced);
    }

    #[test]
    fn ledger_of_parent_account_includes_sub_accounts() {
        let db = db::memory_database();
        db.save_voucher(voucher(
            "1405/01/10",
            vec![line("110201001", 100_000, 0), line("410101", 0, 100_000)],
        ))
        .unwrap();
        db.save_voucher(voucher(
            "1405/01/11",
            vec![line("110201002", 50_000, 0), line("410101", 0, 50_000)],
        ))
        .unwrap();

        let parent = db.get_ledger("110201", "", "").unwrap();
        assert!(parent.includes_children);
        assert_eq!(parent.rows.len(), 2);
        assert_eq!(parent.total_debit, 150_000);
        assert_eq!(parent.closing_debit, 150_000);
        assert_eq!(parent.rows[0].account_code, "110201001");
        assert_eq!(parent.rows[1].running_debit, 150_000);

        // A grandparent reaches the leaf accounts through the middle level.
        let top = db.get_ledger("1102", "", "").unwrap();
        assert_eq!(top.total_debit, 150_000);

        let leaf = db.get_ledger("110201001", "", "").unwrap();
        assert!(!leaf.includes_children);
        assert_eq!(leaf.total_debit, 100_000);

        // Opening balance of the parent covers its sub-accounts as well.
        let later = db.get_ledger("110201", "1405/01/11", "").unwrap();
        assert_eq!(later.opening_debit, 100_000);
        assert_eq!(later.total_debit, 50_000);
    }

    #[test]
    fn search_treats_wildcards_literally_and_folds_digits() {
        let db = db::memory_database();
        let mut first = cash_sale("1405/01/10", 100_000);
        first.description = "تخفیف 50% مشتری".to_string();
        db.save_voucher(first).unwrap();
        let mut second = cash_sale("1405/01/11", 100_000);
        second.description = "تخفیف 5000 ریالی".to_string();
        db.save_voucher(second).unwrap();

        assert_eq!(db.list_vouchers("", "", "50%").unwrap().vouchers.len(), 1);
        assert_eq!(db.list_vouchers("", "", "۵۰%").unwrap().vouchers.len(), 1);
        assert_eq!(db.list_vouchers("", "", "5_00").unwrap().vouchers.len(), 0);
        assert_eq!(db.list_vouchers("", "", "%").unwrap().vouchers.len(), 1);
        // Voucher numbers can be searched with Persian digits.
        assert_eq!(db.list_vouchers("", "", "۲").unwrap().vouchers.len(), 1);
        // Arabic yeh/kaf match the Persian letters stored in names.
        assert_eq!(db.list_vouchers("", "", "مشتري").unwrap().vouchers.len(), 1);
        assert_eq!(db.get_journal("", "", "50%").unwrap().voucher_count, 1);
    }

    #[test]
    fn list_reports_total_count_and_rejects_reversed_range() {
        let db = db::memory_database();
        db.save_voucher(cash_sale("1405/01/10", 1_000)).unwrap();
        db.save_voucher(cash_sale("1405/02/10", 2_000)).unwrap();

        let list = db.list_vouchers("1405/02/01", "", "").unwrap();
        assert_eq!(list.vouchers.len(), 1);
        assert_eq!(list.total_count, 2);

        assert!(db.list_vouchers("1405/03/01", "1405/02/01", "").is_err());
        assert!(db.get_journal("1405/03/01", "1405/02/01", "").is_err());
        assert!(db.get_ledger("110101", "1405/03/01", "1405/02/01").is_err());
        assert!(db.get_trial_balance("1405/03/01", "1405/02/01").is_err());
        assert!(db.get_income_statement("1405/03/01", "1405/02/01").is_err());
    }

    #[test]
    fn rejects_amounts_above_the_safe_limit() {
        let db = db::memory_database();
        let err = db
            .save_voucher(cash_sale("1405/01/10", MAX_AMOUNT + 1))
            .unwrap_err();
        assert!(err.contains("بیشتر از"));
        assert!(db.save_voucher(cash_sale("1405/01/10", MAX_AMOUNT)).is_ok());
    }
}
