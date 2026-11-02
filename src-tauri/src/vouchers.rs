//! Voucher persistence and the accounting reports that are built from it.
//!
//! Amounts are stored as whole Rials (`i64`). Every voucher must balance
//! before it is written, so the reports can rely on a sound ledger.

use crate::db::{self, normalize_digits, Database};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

pub const VOUCHER_TYPES: [&str; 4] = [
    "سند حسابداری عادی",
    "سند حسابداری افتتاحیه",
    "سند حسابداری اختتامیه",
    "سند بستن حساب‌های موقت",
];

const MAX_DESCRIPTION: usize = 1000;
const MAX_LINE_DESCRIPTION: usize = 500;
const MAX_LINES: usize = 1000;
const MAX_VOUCHER_NUMBER: i64 = 9_007_199_254_740_991;

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
pub struct VoucherTransferInput {
    pub source_voucher_id: i64,
    pub source_voucher_number: i64,
    pub target_voucher_number: i64,
    pub line_ids: Vec<i64>,
    pub mode: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherTransferResult {
    pub mode: String,
    pub created_target: bool,
    pub target: Voucher,
    pub source: Option<Voucher>,
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
pub struct VoucherMergePreview {
    pub date: String,
    pub source_voucher_numbers: Vec<i64>,
    pub keep_voucher_number: i64,
    pub line_count: usize,
    pub total_debit: i64,
    pub total_credit: i64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherNumberChange {
    pub old_number: i64,
    pub new_number: i64,
    pub date: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherReorganizationPreview {
    pub operation: String,
    pub total_vouchers: usize,
    pub total_vouchers_after: usize,
    pub merge_groups: Vec<VoucherMergePreview>,
    pub unbalanced_groups: Vec<VoucherMergePreview>,
    pub removed_voucher_count: usize,
    pub number_changes: Vec<VoucherNumberChange>,
    pub plan_token: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherReorganizationResult {
    pub operation: String,
    pub merged_group_count: usize,
    pub removed_voucher_count: usize,
    pub renumbered_count: usize,
}

#[derive(Debug, Clone, Hash)]
struct ReorganizationVoucher {
    id: i64,
    number: i64,
    date: String,
    voucher_type: String,
    description: String,
}

#[derive(Debug, Clone, Hash)]
struct ReorganizationLine {
    id: i64,
    voucher_id: i64,
    row_no: i64,
    account_code: String,
    description: String,
    debit: i64,
    credit: i64,
}

#[derive(Debug, Clone, Hash)]
struct ReorganizationSnapshot {
    vouchers: Vec<ReorganizationVoucher>,
    lines: Vec<ReorganizationLine>,
}

#[derive(Debug, Clone)]
struct ReorganizationMergeGroup {
    keep_id: i64,
    source_ids: Vec<i64>,
    description: String,
}

#[derive(Debug, Clone)]
struct ReorganizationPlan {
    preview: VoucherReorganizationPreview,
    merge_groups: Vec<ReorganizationMergeGroup>,
    number_changes: Vec<(i64, VoucherNumberChange)>,
}

fn validate_reorganization_operation(operation: &str) -> Result<(), String> {
    if matches!(operation, "sort" | "merge" | "merge-then-sort") {
        Ok(())
    } else {
        Err("عملیات مرتب‌سازی یا ادغام معتبر نیست.".to_string())
    }
}

fn read_reorganization_snapshot(conn: &Connection) -> Result<ReorganizationSnapshot, String> {
    let vouchers = conn
        .prepare("SELECT id, number, date, type, description FROM vouchers ORDER BY id")
        .map_err(db::db_error)?
        .query_map([], |row| {
            Ok(ReorganizationVoucher {
                id: row.get(0)?,
                number: row.get(1)?,
                date: row.get(2)?,
                voucher_type: row.get(3)?,
                description: row.get(4)?,
            })
        })
        .map_err(db::db_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db::db_error)?;
    let lines = conn
        .prepare(
            "SELECT id, voucher_id, row_no, account_code, description, debit, credit FROM voucher_lines ORDER BY id",
        )
        .map_err(db::db_error)?
        .query_map([], |row| {
            Ok(ReorganizationLine {
                id: row.get(0)?,
                voucher_id: row.get(1)?,
                row_no: row.get(2)?,
                account_code: row.get(3)?,
                description: row.get(4)?,
                debit: row.get(5)?,
                credit: row.get(6)?,
            })
        })
        .map_err(db::db_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db::db_error)?;
    Ok(ReorganizationSnapshot { vouchers, lines })
}

fn build_reorganization_plan(
    operation: &str,
    snapshot: &ReorganizationSnapshot,
) -> Result<ReorganizationPlan, String> {
    validate_reorganization_operation(operation)?;
    let do_merge = operation != "sort";
    let do_sort = operation != "merge";
    let mut lines_by_voucher = BTreeMap::<i64, Vec<&ReorganizationLine>>::new();
    for line in &snapshot.lines {
        lines_by_voucher
            .entry(line.voucher_id)
            .or_default()
            .push(line);
    }

    let mut grouped = BTreeMap::<(String, String), Vec<&ReorganizationVoucher>>::new();
    if do_merge {
        for voucher in snapshot
            .vouchers
            .iter()
            .filter(|voucher| voucher.voucher_type == VOUCHER_TYPES[0])
        {
            let date = validate_voucher_date(&voucher.date)?;
            grouped
                .entry((date, voucher.voucher_type.clone()))
                .or_default()
                .push(voucher);
        }
    }

    let mut merge_groups = Vec::new();
    let mut unbalanced_groups = Vec::new();
    let mut merge_plan = Vec::new();
    let mut removed_ids = HashSet::new();
    for ((date, _), mut vouchers) in grouped {
        if vouchers.len() < 2 {
            continue;
        }
        vouchers.sort_by_key(|voucher| (voucher.number, voucher.id));
        let all_lines: Vec<&ReorganizationLine> = vouchers
            .iter()
            .flat_map(|voucher| {
                lines_by_voucher
                    .get(&voucher.id)
                    .into_iter()
                    .flatten()
                    .copied()
            })
            .collect();
        let total_debit = checked_sum(
            all_lines.iter().map(|line| line.debit),
            "جمع بدهکار گروه ادغام",
        )?;
        let total_credit = checked_sum(
            all_lines.iter().map(|line| line.credit),
            "جمع بستانکار گروه ادغام",
        )?;
        let balanced = total_debit > 0
            && total_debit == total_credit
            && all_lines.iter().all(|line| {
                line.debit >= 0 && line.credit >= 0 && ((line.debit > 0) ^ (line.credit > 0))
            });
        let preview = VoucherMergePreview {
            date,
            source_voucher_numbers: vouchers.iter().map(|voucher| voucher.number).collect(),
            keep_voucher_number: vouchers[0].number,
            line_count: all_lines.len(),
            total_debit,
            total_credit,
            reason: String::new(),
        };
        let mut merged_description = vouchers[0].description.clone();
        for source in vouchers.iter().skip(1) {
            let separator = if merged_description.is_empty() {
                ""
            } else {
                "\n"
            };
            merged_description.push_str(&format!(
                "{separator}ادغام از سند شماره {}: {}",
                source.number, source.description
            ));
        }
        let mut rejection_reasons = Vec::new();
        if !balanced {
            rejection_reasons.push("ردیف‌های گروه تراز نیستند.".to_string());
        }
        if all_lines.len() > MAX_LINES {
            rejection_reasons.push(format!(
                "تعداد ردیف‌ها از ظرفیت {} ردیفی فرم بیشتر است.",
                db::to_fa_digits(MAX_LINES.to_string())
            ));
        }
        if merged_description.chars().count() > MAX_DESCRIPTION {
            rejection_reasons.push(format!(
                "شرح‌های سرآیند پس از ادغام از سقف {} نویسه بیشتر می‌شوند.",
                db::to_fa_digits(MAX_DESCRIPTION.to_string())
            ));
        }
        if !rejection_reasons.is_empty() {
            let mut preview = preview;
            preview.reason = rejection_reasons.join(" ");
            unbalanced_groups.push(preview);
            continue;
        }
        let keep_id = vouchers[0].id;
        let source_ids: Vec<i64> = vouchers.iter().skip(1).map(|voucher| voucher.id).collect();
        removed_ids.extend(source_ids.iter().copied());
        merge_plan.push(ReorganizationMergeGroup {
            keep_id,
            source_ids,
            description: merged_description,
        });
        merge_groups.push(preview);
    }

    let mut number_changes = Vec::new();
    if do_sort {
        let mut remaining: Vec<(&ReorganizationVoucher, String)> = snapshot
            .vouchers
            .iter()
            .filter(|voucher| !removed_ids.contains(&voucher.id))
            .map(|voucher| Ok((voucher, validate_voucher_date(&voucher.date)?)))
            .collect::<Result<_, String>>()?;
        remaining.sort_by(|(left, left_date), (right, right_date)| {
            left_date
                .cmp(right_date)
                .then_with(|| left.number.cmp(&right.number))
                .then_with(|| left.id.cmp(&right.id))
        });
        for (index, (voucher, date)) in remaining.into_iter().enumerate() {
            let new_number = i64::try_from(index + 1)
                .map_err(|_| "تعداد اسناد از محدوده شماره‌گذاری مجاز بیشتر است.".to_string())?;
            if voucher.number != new_number {
                let change = VoucherNumberChange {
                    old_number: voucher.number,
                    new_number,
                    date,
                };
                number_changes.push((voucher.id, change));
            }
        }
    }

    let mut hasher = DefaultHasher::new();
    operation.hash(&mut hasher);
    snapshot.hash(&mut hasher);
    let plan_token = format!("{:016x}", hasher.finish());
    let removed_voucher_count = removed_ids.len();
    let preview = VoucherReorganizationPreview {
        operation: operation.to_string(),
        total_vouchers: snapshot.vouchers.len(),
        total_vouchers_after: snapshot.vouchers.len() - removed_voucher_count,
        merge_groups,
        unbalanced_groups,
        removed_voucher_count,
        number_changes: number_changes
            .iter()
            .map(|(_, change)| change.clone())
            .collect(),
        plan_token,
    };
    Ok(ReorganizationPlan {
        preview,
        merge_groups: merge_plan,
        number_changes,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherLine {
    /// Stable SQLite row identifier for row-level copy/transfer.
    pub id: i64,
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

/// Validates and normalizes a user-entered Jalali date for immediate form feedback.
/// Saving still calls `normalize_date` again, so this command is not a security boundary.
pub fn validate_voucher_date(input: &str) -> Result<String, String> {
    normalize_date(input, "تاریخ سند")
}

fn checked_add(left: i64, right: i64, label: &str) -> Result<i64, String> {
    left.checked_add(right)
        .ok_or_else(|| format!("مقدار محاسبه‌شده در بخش «{label}» از محدوده عددی مجاز بیشتر است."))
}

fn checked_sub(left: i64, right: i64, label: &str) -> Result<i64, String> {
    left.checked_sub(right)
        .ok_or_else(|| format!("مقدار محاسبه‌شده در بخش «{label}» از محدوده عددی مجاز بیشتر است."))
}

fn checked_neg(value: i64, label: &str) -> Result<i64, String> {
    value
        .checked_neg()
        .ok_or_else(|| format!("مقدار محاسبه‌شده در بخش «{label}» از محدوده عددی مجاز بیشتر است."))
}

fn checked_sum(values: impl IntoIterator<Item = i64>, label: &str) -> Result<i64, String> {
    values
        .into_iter()
        .try_fold(0i64, |total, value| checked_add(total, value, label))
}

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
    // Keep the supported calendar range aligned with WebApp's shared
    // Jalali utilities (years 1 through 3177).
    if !(1..=3177).contains(&year) {
        return Err(format!("{label} خارج از بازه پشتیبانی تقویم شمسی است."));
    }
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
    format!(
        "(?3 = '' OR {} OR {})",
        search_clause("?3"),
        search_clause("?4")
    )
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

    pub fn adjacent_voucher_number(
        &self,
        number: i64,
        direction: &str,
    ) -> Result<Option<i64>, String> {
        let query = match direction {
            "previous" => "SELECT MAX(number) FROM vouchers WHERE number < ?1",
            "next" => "SELECT MIN(number) FROM vouchers WHERE number > ?1",
            _ => return Err("جهت پیمایش سند معتبر نیست.".to_string()),
        };
        self.with_conn(|conn| {
            conn.query_row(query, params![number], |row| row.get(0))
                .map_err(db::db_error)
        })
    }

    pub fn list_vouchers(&self, from: &str, to: &str, search: &str) -> Result<VoucherList, String> {
        let from = normalize_filter_date(from, "تاریخ شروع")?;
        let to = normalize_filter_date(to, "تاریخ پایان")?;
        check_range(&from, &to)?;
        let terms = search_terms(search);

        let (vouchers, total_count) = self.with_conn(|conn| {
            let vouchers = list_vouchers(conn, &from, &to, &terms)?;
            // The list is already filtered by date and search; its count
            // must describe the visible result set, not the whole database.
            let total = vouchers.len() as i64;
            Ok((vouchers, total))
        })?;
        let total_debit = checked_sum(
            vouchers.iter().map(|v| v.total_debit),
            "جمع بدهکار فهرست اسناد",
        )?;
        let total_credit = checked_sum(
            vouchers.iter().map(|v| v.total_credit),
            "جمع بستانکار فهرست اسناد",
        )?;
        Ok(VoucherList {
            vouchers,
            total_count,
            total_debit,
            total_credit,
            database_path: self.path_string(),
        })
    }

    pub fn preview_voucher_reorganization(
        &self,
        operation: &str,
    ) -> Result<VoucherReorganizationPreview, String> {
        validate_reorganization_operation(operation)?;
        self.with_conn(|conn| {
            let snapshot = read_reorganization_snapshot(conn)?;
            Ok(build_reorganization_plan(operation, &snapshot)?.preview)
        })
    }

    pub fn apply_voucher_reorganization(
        &self,
        operation: &str,
        plan_token: &str,
    ) -> Result<VoucherReorganizationResult, String> {
        validate_reorganization_operation(operation)?;
        if plan_token.is_empty() {
            return Err("پیش‌نمایش منقضی یا نامعتبر است؛ دوباره پیش‌نمایش بگیرید.".to_string());
        }
        let operation = operation.to_string();
        let plan_token = plan_token.to_string();
        self.with_conn(move |conn| {
            let tx = conn.transaction().map_err(db::db_error)?;
            let snapshot = read_reorganization_snapshot(&tx)?;
            let plan = build_reorganization_plan(&operation, &snapshot)?;
            if plan.preview.plan_token != plan_token {
                return Err("اسناد پس از پیش‌نمایش تغییر کرده‌اند؛ پیش‌نمایش جدید بگیرید.".to_string());
            }

            let mut removed_voucher_count = 0;
            for group in &plan.merge_groups {
                let mut next_row: i64 = tx
                    .query_row(
                        "SELECT COALESCE(MAX(row_no), 0) FROM voucher_lines WHERE voucher_id = ?1",
                        params![group.keep_id],
                        |row| row.get(0),
                    )
                    .map_err(db::db_error)?;
                for source_id in &group.source_ids {
                    let mut statement = tx
                        .prepare("SELECT id FROM voucher_lines WHERE voucher_id = ?1 ORDER BY row_no, id")
                        .map_err(db::db_error)?;
                    let source_lines = statement
                        .query_map(params![source_id], |row| row.get::<_, i64>(0))
                        .map_err(db::db_error)?
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .map_err(db::db_error)?;
                    drop(statement);
                    for line_id in source_lines {
                        next_row = next_row.checked_add(1)
                            .ok_or_else(|| "شماره ردیف‌های سند از محدوده مجاز بیشتر است.".to_string())?;
                        tx.execute(
                            "UPDATE voucher_lines SET voucher_id = ?1, row_no = ?2 WHERE id = ?3",
                            params![group.keep_id, next_row, line_id],
                        )
                        .map_err(db::db_error)?;
                    }
                    tx.execute("DELETE FROM vouchers WHERE id = ?1", params![source_id])
                        .map_err(db::db_error)?;
                    removed_voucher_count += 1;
                }
                tx.execute(
                    "UPDATE vouchers
                        SET description = ?1, updated_at = datetime('now', 'localtime')
                      WHERE id = ?2",
                    params![group.description, group.keep_id],
                )
                .map_err(db::db_error)?;
            }

            for (index, (voucher_id, _)) in plan.number_changes.iter().enumerate() {
                let temporary_number = i64::MIN + index as i64;
                tx.execute(
                    "UPDATE vouchers SET number = ?1 WHERE id = ?2",
                    params![temporary_number, voucher_id],
                )
                .map_err(db::db_error)?;
            }
            for (voucher_id, change) in &plan.number_changes {
                tx.execute(
                    "UPDATE vouchers SET number = ?1, updated_at = datetime('now', 'localtime') WHERE id = ?2",
                    params![change.new_number, voucher_id],
                )
                .map_err(db::db_error)?;
            }

            let result = VoucherReorganizationResult {
                operation,
                merged_group_count: plan.merge_groups.len(),
                removed_voucher_count,
                renumbered_count: plan.number_changes.len(),
            };
            tx.commit().map_err(db::db_error)?;
            Ok(result)
        })
    }

    pub fn get_voucher(&self, number: i64) -> Result<Option<Voucher>, String> {
        if number <= 0 {
            return Ok(None);
        }
        self.with_conn(|conn| fetch_voucher_by_number(conn, number))
    }

    /// Copies or moves selected, persisted rows between two vouchers.
    /// The entire operation is atomic; the selected block and resulting
    /// vouchers must remain balanced.
    pub fn transfer_voucher_lines(
        &self,
        input: VoucherTransferInput,
    ) -> Result<VoucherTransferResult, String> {
        if input.source_voucher_id <= 0 || input.source_voucher_number <= 0 {
            return Err("سند مبدأ معتبر نیست.".to_string());
        }
        if input.target_voucher_number <= 0 {
            return Err("شماره سند مقصد باید بزرگ‌تر از صفر باشد.".to_string());
        }
        let mode = input.mode.trim().to_lowercase();
        if mode != "copy" && mode != "move" {
            return Err("نوع عملیات کپی یا انتقال معتبر نیست.".to_string());
        }
        if input.source_voucher_number == input.target_voucher_number {
            return Err("شماره سند مقصد باید با سند مبدأ متفاوت باشد.".to_string());
        }
        if input.line_ids.is_empty() || input.line_ids.len() > MAX_LINES {
            return Err("حداقل یک ردیف معتبر برای عملیات انتخاب کنید.".to_string());
        }
        let mut unique_ids = HashSet::new();
        for id in &input.line_ids {
            if *id <= 0 || !unique_ids.insert(*id) {
                return Err("شناسه ردیف‌های انتخابی نامعتبر یا تکراری است.".to_string());
            }
        }
        let source_id = input.source_voucher_id;
        let source_number = input.source_voucher_number;
        let target_number = input.target_voucher_number;
        let line_ids = input.line_ids;
        let is_move = mode == "move";

        self.with_conn(move |conn| {
            let tx = conn.transaction().map_err(db::db_error)?;
            let (source_date, source_type) = tx
                .query_row(
                    "SELECT date, type FROM vouchers WHERE id = ?1 AND number = ?2",
                    params![source_id, source_number],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(db::db_error)?
                .ok_or_else(|| "سند مبدأ پس از بازخوانی پیدا نشد؛ فرم را تازه‌سازی کنید.".to_string())?;

            let source_lines = {
                let mut stmt = tx.prepare(
                    "SELECT id, row_no, account_code, description, debit, credit
                       FROM voucher_lines WHERE voucher_id = ?1 ORDER BY row_no, id"
                ).map_err(db::db_error)?;
                let rows = stmt.query_map(params![source_id], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        VoucherLineInput {
                            account_code: row.get(2)?,
                            description: row.get(3)?,
                            debit: row.get(4)?,
                            credit: row.get(5)?,
                        },
                    ))
                }).map_err(db::db_error)?
                  .collect::<rusqlite::Result<Vec<_>>>()
                  .map_err(db::db_error)?;
                rows
            };

            let requested: HashSet<i64> = line_ids.iter().copied().collect();
            let selected: Vec<_> = source_lines.iter()
                .filter(|(id, _, _)| requested.contains(id))
                .cloned()
                .collect();
            if selected.len() != line_ids.len() {
                return Err("برخی از ردیف‌های انتخابی دیگر در سند مبدأ وجود ندارند؛ سند را دوباره باز کنید.".to_string());
            }
            if selected.len() < 2 {
                return Err("برای کپی یا انتقال، حداقل دو ردیف لازم است.".to_string());
            }

            for (_, row_no, line) in &selected {
                if line.debit < 0 || line.credit < 0
                    || line.debit > MAX_AMOUNT || line.credit > MAX_AMOUNT
                    || (line.debit == 0 && line.credit == 0)
                    || (line.debit > 0 && line.credit > 0) {
                    return Err(format!(
                        "مبالغ ردیف {} معتبر نیست؛ ابتدا سند مبدأ را اصلاح کنید.",
                        db::to_fa_digits(*row_no)
                    ));
                }
                let postable = tx.query_row(
                    "SELECT is_postable FROM accounts WHERE code = ?1",
                    params![&line.account_code],
                    |row| row.get::<_, i64>(0),
                ).optional().map_err(db::db_error)?.unwrap_or(0);
                if postable == 0 {
                    return Err(format!(
                        "حساب ردیف {} دیگر قابل ثبت نیست؛ انتقال متوقف شد.",
                        db::to_fa_digits(*row_no)
                    ));
                }
            }

            let selected_debit = checked_sum(
                selected.iter().map(|(_, _, line)| line.debit),
                "جمع بدهکار ردیف‌های انتخابی",
            )?;
            let selected_credit = checked_sum(
                selected.iter().map(|(_, _, line)| line.credit),
                "جمع بستانکار ردیف‌های انتخابی",
            )?;
            if selected_debit <= 0 || selected_debit != selected_credit {
                return Err("مجموع ردیف‌های انتخابی تراز نیست؛ ردیف‌های بدهکار و بستانکار متناظر را با هم انتخاب کنید.".to_string());
            }

            if is_move {
                let remaining: Vec<_> = source_lines.iter()
                    .filter(|(id, _, _)| !requested.contains(id))
                    .collect();
                if remaining.len() < 2 {
                    return Err("انتقال انتخاب‌شده باعث می‌شود سند مبدأ کمتر از دو ردیف داشته باشد؛ عملیات انجام نشد.".to_string());
                }
                let remaining_debit = checked_sum(
                    remaining.iter().map(|(_, _, line)| line.debit),
                    "جمع بدهکار باقی‌مانده سند مبدأ",
                )?;
                let remaining_credit = checked_sum(
                    remaining.iter().map(|(_, _, line)| line.credit),
                    "جمع بستانکار باقی‌مانده سند مبدأ",
                )?;
                if remaining_debit <= 0 || remaining_debit != remaining_credit {
                    return Err("با انتقال این ردیف‌ها سند مبدأ ناتراز می‌شود؛ ردیف‌های متناظر را نیز انتخاب کنید یا از حالت کپی استفاده کنید.".to_string());
                }
            }

            let existing_target_id = tx.query_row(
                "SELECT id FROM vouchers WHERE number = ?1",
                params![target_number],
                |row| row.get::<_, i64>(0),
            ).optional().map_err(db::db_error)?;
            if existing_target_id == Some(source_id) {
                return Err("سند مقصد نمی‌تواند همان سند مبدأ باشد.".to_string());
            }

            let (target_id, target_count, target_debit, target_credit, created_target) =
                if let Some(id) = existing_target_id {
                    let totals = tx.query_row(
                        "SELECT COUNT(*), COALESCE(SUM(debit), 0), COALESCE(SUM(credit), 0)
                           FROM voucher_lines WHERE voucher_id = ?1",
                        params![id],
                        |row| Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                        )),
                    ).map_err(db::db_error)?;
                    (id, totals.0, totals.1, totals.2, false)
                } else {
                    tx.execute(
                        "INSERT INTO vouchers (number, date, type, description)
                         VALUES (?1, ?2, ?3, '')",
                        params![target_number, source_date, source_type],
                    ).map_err(db::db_error)?;
                    (tx.last_insert_rowid(), 0, 0, 0, true)
                };

            let new_target_count = target_count.checked_add(selected.len() as i64)
                .ok_or_else(|| "تعداد ردیف‌های مقصد از محدوده مجاز بیشتر است.".to_string())?;
            if new_target_count > MAX_LINES as i64 {
                return Err(format!("سند مقصد پس از عملیات بیش از {} ردیف خواهد داشت.", db::to_fa_digits(MAX_LINES)));
            }
            if new_target_count < 2 {
                return Err("سند مقصد باید حداقل دو ردیف داشته باشد.".to_string());
            }
            let new_target_debit = checked_add(target_debit, selected_debit, "جمع بدهکار سند مقصد")?;
            let new_target_credit = checked_add(target_credit, selected_credit, "جمع بستانکار سند مقصد")?;
            if new_target_debit <= 0 || new_target_debit != new_target_credit {
                return Err("افزودن ردیف‌های انتخابی باعث ناترازی سند مقصد می‌شود؛ ردیف‌های بدهکار و بستانکار متناظر را انتخاب کنید.".to_string());
            }

            let mut next_row: i64 = tx.query_row(
                "SELECT COALESCE(MAX(row_no), 0) FROM voucher_lines WHERE voucher_id = ?1",
                params![target_id],
                |row| row.get(0),
            ).map_err(db::db_error)?;
            for (_, _, line) in &selected {
                next_row = next_row.checked_add(1)
                    .ok_or_else(|| "شماره ردیف‌های سند مقصد از محدوده مجاز بیشتر است.".to_string())?;
                tx.execute(
                    "INSERT INTO voucher_lines (voucher_id, row_no, account_code, description, debit, credit)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![target_id, next_row, &line.account_code, &line.description, line.debit, line.credit],
                ).map_err(db::db_error)?;
            }

            if is_move {
                for id in &line_ids {
                    let changed = tx.execute(
                        "DELETE FROM voucher_lines WHERE id = ?1 AND voucher_id = ?2",
                        params![id, source_id],
                    ).map_err(db::db_error)?;
                    if changed != 1 {
                        return Err("یکی از ردیف‌های سند مبدأ هم‌زمان تغییر کرده است؛ عملیات لغو شد.".to_string());
                    }
                }
                tx.execute(
                    "UPDATE vouchers SET updated_at = datetime('now', 'localtime') WHERE id = ?1",
                    params![source_id],
                ).map_err(db::db_error)?;
            }
            tx.execute(
                "UPDATE vouchers SET updated_at = datetime('now', 'localtime') WHERE id = ?1",
                params![target_id],
            ).map_err(db::db_error)?;

            tx.commit().map_err(db::db_error)?;
            let target = fetch_voucher_by_id(conn, target_id)?;
            let source = if is_move { Some(fetch_voucher_by_id(conn, source_id)?) } else { None };
            Ok(VoucherTransferResult { mode, created_target, target, source })
        })
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
                        format!(
                            "حساب با کد {} یافت نشد.",
                            db::to_fa_digits(&line.account_code)
                        )
                    })?;
                if account.1 == 0 {
                    return Err(format!(
                        "حساب «{}» قابل ثبت نیست؛ حساب کل یا حساب غیرقابل ثبت را انتخاب نکنید.",
                        account.0
                    ));
                }
            }

            let existing_number = if id > 0 {
                tx.query_row(
                    "SELECT number FROM vouchers WHERE id = ?1",
                    params![id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(db::db_error)?
            } else {
                None
            };
            if id > 0 && existing_number.is_none() {
                return Err("سند حسابداری یافت نشد.".to_string());
            }

            let number = match (requested_number, existing_number) {
                (number, _) if number > 0 => number,
                (_, Some(current_number)) => current_number,
                _ => tx
                    .query_row("SELECT COALESCE(MAX(number), 0) FROM vouchers", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .map_err(db::db_error)?
                    .checked_add(1)
                    .ok_or_else(|| "شماره سند بعدی از محدوده مجاز خارج است.".to_string())?,
            };
            if number <= 0 || number > MAX_VOUCHER_NUMBER {
                return Err(format!(
                    "شماره سند باید عدد صحیح و حداکثر {} باشد.",
                    db::to_fa_digits(MAX_VOUCHER_NUMBER.to_string())
                ));
            }

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

    pub fn preview_bulk_import(
        &self,
        import: &crate::excel::BulkVoucherImport,
    ) -> Result<crate::excel::BulkVoucherPreview, String> {
        let file_name = import.file_name.clone();
        let account_codes: Vec<String> = import
            .accounts
            .iter()
            .map(|account| account.code.clone())
            .collect();
        let numbers: Vec<i64> = import
            .vouchers
            .iter()
            .filter_map(|voucher| voucher.number)
            .collect();
        let voucher_count = import.vouchers.len();
        let line_count = import
            .vouchers
            .iter()
            .map(|voucher| voucher.lines.len())
            .sum();
        let account_count = account_codes.len();
        self.with_conn(move |conn| {
            let mut new_account_count = 0;
            for code in account_codes {
                let exists = conn
                    .query_row(
                        "SELECT 1 FROM accounts WHERE code = ?1",
                        params![db::normalize_code(&code)],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(db::db_error)?
                    .is_some();
                if !exists {
                    new_account_count += 1;
                }
            }
            let mut number_conflict_count = 0;
            for number in numbers {
                let exists = conn
                    .query_row(
                        "SELECT 1 FROM vouchers WHERE number = ?1",
                        params![number],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(db::db_error)?
                    .is_some();
                if exists {
                    number_conflict_count += 1;
                }
            }
            Ok(crate::excel::BulkVoucherPreview {
                file_name,
                voucher_count,
                line_count,
                account_count,
                new_account_count,
                number_conflict_count,
            })
        })
    }

    pub fn import_vouchers(
        &self,
        import: crate::excel::BulkVoucherImport,
        renumber_conflicts: bool,
    ) -> Result<usize, String> {
        if import.vouchers.is_empty() {
            return Err("فایلی برای ورود وجود ندارد.".to_string());
        }

        let mut prepared = Vec::with_capacity(import.vouchers.len());
        let mut numbers = HashSet::new();
        for input in import.vouchers {
            let date = normalize_date(&input.date, "تاریخ سند")?;
            let voucher_type = validate_voucher_type(&input.voucher_type)?;
            let description = validate_description(&input.description, "شرح سند", MAX_DESCRIPTION)?;
            let number = input.number.unwrap_or(0);
            if number <= 0 || !numbers.insert(number) {
                return Err(format!(
                    "شماره سند {} نامعتبر یا تکراری است.",
                    db::to_fa_digits(number)
                ));
            }
            if input.lines.len() > MAX_LINES {
                return Err(format!(
                    "تعداد ردیف‌های سند {} از حد مجاز بیشتر است.",
                    db::to_fa_digits(number)
                ));
            }

            let mut lines = Vec::with_capacity(input.lines.len());
            for line in input.lines {
                let account_code = db::normalize_code(&line.account_code);
                let line_description =
                    validate_description(&line.description, "شرح ردیف", MAX_LINE_DESCRIPTION)?;
                if account_code.is_empty() {
                    return Err(format!(
                        "کد حساب در سند {} خالی است.",
                        db::to_fa_digits(number)
                    ));
                }
                if line.debit < 0
                    || line.credit < 0
                    || line.debit > MAX_AMOUNT
                    || line.credit > MAX_AMOUNT
                {
                    return Err(format!(
                        "مبلغ سند {} خارج از محدوده مجاز است.",
                        db::to_fa_digits(number)
                    ));
                }
                if (line.debit == 0 && line.credit == 0) || (line.debit > 0 && line.credit > 0) {
                    return Err(format!(
                        "مبلغ ردیف سند {} نامعتبر است.",
                        db::to_fa_digits(number)
                    ));
                }
                lines.push(VoucherLineInput {
                    account_code,
                    description: line_description,
                    debit: line.debit,
                    credit: line.credit,
                });
            }
            if lines.len() < 2 {
                return Err(format!(
                    "سند {} باید حداقل دو ردیف داشته باشد.",
                    db::to_fa_digits(number)
                ));
            }
            let total_debit = checked_sum(lines.iter().map(|line| line.debit), "جمع بدهکار سند")?;
            let total_credit =
                checked_sum(lines.iter().map(|line| line.credit), "جمع بستانکار سند")?;
            if total_debit == 0 || total_debit != total_credit {
                return Err(format!("سند {} تراز نیست.", db::to_fa_digits(number)));
            }
            prepared.push((number, date, voucher_type, description, lines));
        }

        let mut accounts = import.accounts;
        accounts.sort_by_key(|account| account.level);
        let imported_count = prepared.len();
        self.with_conn(move |conn| {
            let tx = conn.transaction().map_err(db::db_error)?;
            let incoming_numbers: HashSet<i64> = prepared.iter().map(|item| item.0).collect();
            let mut used_numbers = HashSet::new();
            {
                let mut statement = tx.prepare("SELECT number FROM vouchers").map_err(db::db_error)?;
                let rows = statement.query_map([], |row| row.get::<_, i64>(0)).map_err(db::db_error)?;
                for row in rows {
                    used_numbers.insert(row.map_err(db::db_error)?);
                }
            }
            let mut next_number = used_numbers.iter().copied().max().unwrap_or(0);
            for account in accounts {
                let code = db::normalize_code(&account.code);
                let name = account.name.split_whitespace().collect::<Vec<_>>().join(" ");
                let parent_code = db::normalize_code(&account.parent_code);
                let group_code = db::normalize_code(&account.group_code);
                if code.is_empty()
                    || code.len() > 20
                    || !code.chars().all(|ch| ch.is_ascii_digit())
                    || name.is_empty()
                    || name.chars().count() > 255
                    || !matches!(account.level, 1 | 2)
                {
                    return Err("اطلاعات یکی از حساب‌های فایل اکسل معتبر نیست.".to_string());
                }
                let exists = tx
                    .query_row("SELECT 1 FROM accounts WHERE code = ?1", params![code], |_| Ok(()))
                    .optional()
                    .map_err(db::db_error)?
                    .is_some();
                if exists {
                    continue;
                }
                let group_exists = tx
                    .query_row("SELECT 1 FROM account_groups WHERE code = ?1", params![group_code], |_| Ok(()))
                    .optional()
                    .map_err(db::db_error)?
                    .is_some();
                if !group_exists {
                    return Err(format!("گروه حساب {} برای حساب {} پیدا نشد.", group_code, code));
                }

                let parent_name = if account.level == 1 {
                    if !parent_code.is_empty() {
                        return Err("حساب کل فایل اکسل نباید حساب بالاتر داشته باشد.".to_string());
                    }
                    String::new()
                } else {
                    if parent_code.is_empty() || !code.starts_with(&parent_code) || code.len() <= parent_code.len() {
                        return Err(format!("ساختار حساب {} در فایل اکسل معتبر نیست.", db::to_fa_digits(&code)));
                    }
                    let parent = tx
                        .query_row(
                            "SELECT name, level, group_code FROM accounts WHERE code = ?1",
                            params![parent_code],
                            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)),
                        )
                        .optional()
                        .map_err(db::db_error)?
                        .ok_or_else(|| format!("حساب بالاتر {} برای {} پیدا نشد.", db::to_fa_digits(&parent_code), db::to_fa_digits(&code)))?;
                    if parent.1 != 1 || parent.2 != group_code {
                        return Err(format!("گروه یا سطح حساب بالاتر {} با حساب {} سازگار نیست.", db::to_fa_digits(&parent_code), db::to_fa_digits(&code)));
                    }
                    let parent_has_lines = tx
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM voucher_lines WHERE account_code = ?1)",
                            params![parent_code],
                            |row| row.get::<_, bool>(0),
                        )
                        .map_err(db::db_error)?;
                    if parent_has_lines {
                        return Err(format!("حساب بالاتر {} در اسناد استفاده شده و نمی‌توان برای آن زیرحساب ساخت.", db::to_fa_digits(&parent_code)));
                    }
                    tx.execute(
                        "UPDATE accounts SET postable_before_children = COALESCE(postable_before_children, is_postable), is_postable = 0 WHERE code = ?1",
                        params![parent_code],
                    )
                    .map_err(db::db_error)?;
                    parent.0
                };
                let parent_code_value = if parent_code.is_empty() { None } else { Some(parent_code) };
                tx.execute(
                    "INSERT INTO accounts (code, name, level, group_code, nature, parent_code, parent_name, is_postable) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![code, name, account.level, group_code, account.nature, parent_code_value, parent_name, i64::from(account.level == 2)],
                )
                .map_err(db::db_error)?;
            }

            for (number, date, voucher_type, description, lines) in prepared {
                for line in &lines {
                    let account = tx
                        .query_row(
                            "SELECT name, is_postable FROM accounts WHERE code = ?1",
                            params![line.account_code],
                            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                        )
                        .optional()
                        .map_err(db::db_error)?
                        .ok_or_else(|| format!("حساب با کد {} یافت نشد.", db::to_fa_digits(&line.account_code)))?;
                    if account.1 == 0 {
                        return Err(format!("حساب «{}» قابل ثبت نیست؛ حساب کل یا حساب غیرقابل ثبت انتخاب شده است.", account.0));
                    }
                }
                let mut stored_number = number;
                if used_numbers.contains(&number) {
                    if !renumber_conflicts {
                        return Err(format!("شماره سند {} از قبل در برنامه وجود دارد.", db::to_fa_digits(number)));
                    }
                    loop {
                        next_number = next_number.checked_add(1)
                            .ok_or_else(|| "شماره سند بعدی از محدوده عددی مجاز بیشتر است.".to_string())?;
                        if !used_numbers.contains(&next_number) && !incoming_numbers.contains(&next_number) {
                            stored_number = next_number;
                            break;
                        }
                    }
                }
                used_numbers.insert(stored_number);
                tx.execute(
                    "INSERT INTO vouchers (number, date, type, description) VALUES (?1, ?2, ?3, ?4)",
                    params![stored_number, date, voucher_type, description],
                )
                .map_err(db::db_error)?;
                let voucher_id = tx.last_insert_rowid();
                for (index, line) in lines.iter().enumerate() {
                    tx.execute(
                        "INSERT INTO voucher_lines (voucher_id, row_no, account_code, description, debit, credit) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![voucher_id, index as i64 + 1, line.account_code, line.description, line.debit, line.credit],
                    )
                    .map_err(db::db_error)?;
                }
            }
            tx.commit().map_err(db::db_error)?;
            Ok(imported_count)
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

    pub fn get_journal(&self, from: &str, to: &str, search: &str) -> Result<JournalReport, String> {
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

            let total_debit = checked_sum(rows.iter().map(|r| r.debit), "جمع بدهکار دفتر روزنامه")?;
            let total_credit =
                checked_sum(rows.iter().map(|r| r.credit), "جمع بستانکار دفتر روزنامه")?;
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

    pub fn get_ledger(
        &self,
        account_code: &str,
        from: &str,
        to: &str,
    ) -> Result<LedgerReport, String> {
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

            let mut opening_stmt = conn
                .prepare(&format!(
                    "{TREE}
                     SELECT l.debit, l.credit
                       FROM voucher_lines l
                       JOIN vouchers v ON v.id = l.voucher_id
                      WHERE l.account_code IN (SELECT code FROM tree)
                        AND (?2 <> '' AND v.date < ?2)"
                ))
                .map_err(db::db_error)?;
            let opening_rows = opening_stmt
                .query_map(params![code, from], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
                })
                .map_err(db::db_error)?;
            let mut opening_debit = 0i64;
            let mut opening_credit = 0i64;
            for row in opening_rows {
                let (debit, credit) = row.map_err(db::db_error)?;
                opening_debit = checked_add(opening_debit, debit, "بدهکار افتتاحیه دفتر کل")?;
                opening_credit = checked_add(opening_credit, credit, "بستانکار افتتاحیه دفتر کل")?;
            }

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

            let opening_net = checked_sub(opening_debit, opening_credit, "مانده افتتاحیه دفتر کل")?;
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
                running = checked_add(
                    running,
                    checked_sub(debit, credit, "گردش ردیف دفتر کل")?,
                    "مانده جاری دفتر کل",
                )?;
                total_debit = checked_add(total_debit, debit, "جمع بدهکار دفتر کل")?;
                total_credit = checked_add(total_credit, credit, "جمع بستانکار دفتر کل")?;
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
                    running_credit: checked_neg(running, "مانده بستانکار جاری دفتر کل")?.max(0),
                });
            }

            let closing = checked_sub(
                checked_add(opening_net, total_debit, "مانده پایان دوره دفتر کل")?,
                total_credit,
                "مانده پایان دوره دفتر کل",
            )?;
            Ok(LedgerReport {
                account_code: code,
                account_name,
                group_name,
                includes_children,
                from,
                to,
                opening_debit: opening_net.max(0),
                opening_credit: checked_neg(opening_net, "مانده بستانکار افتتاحیه دفتر کل")?.max(0),
                total_debit,
                total_credit,
                closing_debit: closing.max(0),
                closing_credit: checked_neg(closing, "مانده بستانکار پایان دوره دفتر کل")?.max(0),
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
                    "SELECT a.code, a.name, g.name, a.level, v.date, l.debit, l.credit
                       FROM accounts a
                       JOIN account_groups g ON g.code = a.group_code
                       LEFT JOIN voucher_lines l ON l.account_code = a.code
                       LEFT JOIN vouchers v ON v.id = l.voucher_id
                      ORDER BY a.code, v.date",
                )
                .map_err(db::db_error)?;

            let raw = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                    ))
                })
                .map_err(db::db_error)?;

            let mut balances: std::collections::BTreeMap<
                String,
                (String, String, i64, i64, i64, i64, i64),
            > = std::collections::BTreeMap::new();
            for row in raw {
                let (code, name, group_name, level, date, debit, credit) =
                    row.map_err(db::db_error)?;
                let entry = balances
                    .entry(code)
                    .or_insert((name, group_name, level, 0, 0, 0, 0));
                let (Some(date), Some(debit), Some(credit)) = (date, debit, credit) else {
                    continue;
                };
                if !from.is_empty() && date < from {
                    entry.3 = checked_add(entry.3, debit, "بدهکار افتتاحیه تراز آزمایشی")?;
                    entry.4 = checked_add(entry.4, credit, "بستانکار افتتاحیه تراز آزمایشی")?;
                } else if (from.is_empty() || date >= from) && (to.is_empty() || date <= to) {
                    entry.5 = checked_add(entry.5, debit, "بدهکار دوره تراز آزمایشی")?;
                    entry.6 = checked_add(entry.6, credit, "بستانکار دوره تراز آزمایشی")?;
                }
            }

            let mut rows = Vec::new();
            for (code, (name, group_name, level, od, oc, pd, pc)) in balances {
                if od == 0 && oc == 0 && pd == 0 && pc == 0 {
                    continue;
                }
                let opening_net = checked_sub(od, oc, "مانده افتتاحیه تراز آزمایشی")?;
                let closing_net = checked_sub(
                    checked_add(opening_net, pd, "مانده پایان دوره تراز آزمایشی")?,
                    pc,
                    "مانده پایان دوره تراز آزمایشی",
                )?;
                rows.push(TrialBalanceRow {
                    account_code: code,
                    account_name: name,
                    group_name,
                    level,
                    opening_debit: opening_net.max(0),
                    opening_credit: checked_neg(
                        opening_net,
                        "مانده بستانکار افتتاحیه تراز آزمایشی",
                    )?
                    .max(0),
                    period_debit: pd,
                    period_credit: pc,
                    closing_debit: closing_net.max(0),
                    closing_credit: checked_neg(
                        closing_net,
                        "مانده بستانکار پایان دوره تراز آزمایشی",
                    )?
                    .max(0),
                });
            }

            let opening_debit = checked_sum(
                rows.iter().map(|r| r.opening_debit),
                "جمع بدهکار افتتاحیه تراز آزمایشی",
            )?;
            let opening_credit = checked_sum(
                rows.iter().map(|r| r.opening_credit),
                "جمع بستانکار افتتاحیه تراز آزمایشی",
            )?;
            let period_debit = checked_sum(
                rows.iter().map(|r| r.period_debit),
                "جمع بدهکار دوره تراز آزمایشی",
            )?;
            let period_credit = checked_sum(
                rows.iter().map(|r| r.period_credit),
                "جمع بستانکار دوره تراز آزمایشی",
            )?;
            let closing_debit = checked_sum(
                rows.iter().map(|r| r.closing_debit),
                "جمع بدهکار پایان دوره تراز آزمایشی",
            )?;
            let closing_credit = checked_sum(
                rows.iter().map(|r| r.closing_credit),
                "جمع بستانکار پایان دوره تراز آزمایشی",
            )?;

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
            let activities =
                account_activities(conn, &from, &to, false, &[CLOSING_TEMPORARY, CLOSING_FINAL])?;
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
            let mut amount_stmt = conn
                .prepare("SELECT debit, credit FROM voucher_lines")
                .map_err(db::db_error)?;
            let amount_rows = amount_stmt
                .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
                .map_err(db::db_error)?;
            let mut total_debit = 0i64;
            let mut total_credit = 0i64;
            for row in amount_rows {
                let (debit, credit) = row.map_err(db::db_error)?;
                total_debit = checked_add(total_debit, debit, "جمع بدهکار داشبورد")?;
                total_credit = checked_add(total_credit, credit, "جمع بستانکار داشبورد")?;
            }

            let mut assets_total = 0i64;
            let mut liabilities_total = 0i64;
            let mut equity_total = 0i64;
            let mut income_total = 0i64;
            let mut expense_total = 0i64;

            // Balance-sheet figures come from the books before the final
            // closing voucher; income and expenses also leave out the
            // voucher that closes the temporary accounts.
            for activity in &account_activities(conn, "", "", false, &[CLOSING_FINAL])? {
                let net = checked_sub(activity.debit, activity.credit, "مانده حساب داشبورد")?;
                match classify(&activity.group_type, &activity.account_code) {
                    AccountClass::Asset => {
                        assets_total = checked_add(assets_total, net, "جمع دارایی داشبورد")?
                    }
                    AccountClass::Liability => {
                        liabilities_total = checked_add(
                            liabilities_total,
                            checked_neg(net, "مانده بدهی داشبورد")?,
                            "جمع بدهی داشبورد",
                        )?
                    }
                    AccountClass::Equity => {
                        equity_total = checked_add(
                            equity_total,
                            checked_neg(net, "مانده سرمایه داشبورد")?,
                            "جمع سرمایه داشبورد",
                        )?
                    }
                    AccountClass::Income | AccountClass::Expense => {}
                }
            }
            for activity in
                &account_activities(conn, "", "", false, &[CLOSING_TEMPORARY, CLOSING_FINAL])?
            {
                let net = checked_sub(
                    activity.debit,
                    activity.credit,
                    "مانده درآمد و هزینه داشبورد",
                )?;
                match classify(&activity.group_type, &activity.account_code) {
                    AccountClass::Income => {
                        income_total = checked_add(
                            income_total,
                            checked_neg(net, "درآمد داشبورد")?,
                            "جمع درآمد داشبورد",
                        )?
                    }
                    AccountClass::Expense => {
                        expense_total = checked_add(expense_total, net, "جمع هزینه داشبورد")?
                    }
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
                net_profit: checked_sub(income_total, expense_total, "سود خالص داشبورد")?,
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
            "SELECT l.id, l.row_no, l.account_code, a.name, l.description, l.debit, l.credit
               FROM voucher_lines l
               JOIN accounts a ON a.code = l.account_code
              WHERE l.voucher_id = ?1
              ORDER BY l.row_no",
        )
        .map_err(db::db_error)?;
    let lines = stmt
        .query_map(params![id], |row| {
            Ok(VoucherLine {
                id: row.get(0)?,
                row_no: row.get(1)?,
                account_code: row.get(2)?,
                account_name: row.get(3)?,
                description: row.get(4)?,
                debit: row.get(5)?,
                credit: row.get(6)?,
            })
        })
        .map_err(db::db_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db::db_error)?;

    let total_debit = checked_sum(lines.iter().map(|l| l.debit), "جمع بدهکار سند")?;
    let total_credit = checked_sum(lines.iter().map(|l| l.credit), "جمع بستانکار سند")?;
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
    let (from_value, to_value) = if cumulative {
        ("".to_string(), to.to_string())
    } else {
        (from.to_string(), to.to_string())
    };

    let mut stmt = conn
        .prepare(
            "SELECT a.code, a.name, g.code, g.type, l.debit, l.credit
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
              ORDER BY a.code",
        )
        .map_err(db::db_error)?;

    let raw = stmt
        .query_map(params![from_value, to_value, skip_a, skip_b], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
            ))
        })
        .map_err(db::db_error)?;

    let mut activities: Vec<AccountActivity> = Vec::new();
    for row in raw {
        let (code, name, group_code, group_type, debit, credit) = row.map_err(db::db_error)?;
        if let Some(last) = activities
            .last_mut()
            .filter(|item| item.account_code.as_str() == code.as_str())
        {
            if let Some(debit) = debit {
                last.debit = checked_add(last.debit, debit, "گردش بدهکار حساب")?;
            }
            if let Some(credit) = credit {
                last.credit = checked_add(last.credit, credit, "گردش بستانکار حساب")?;
            }
        } else {
            activities.push(AccountActivity {
                account_code: code,
                account_name: name,
                group_code,
                group_type,
                debit: debit.unwrap_or(0),
                credit: credit.unwrap_or(0),
            });
        }
    }
    Ok(activities)
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
        let net = checked_sub(
            activity.debit,
            activity.credit,
            "مانده حساب صورت سود و زیان",
        )?;
        if net == 0 {
            continue;
        }
        match classify(&activity.group_type, &activity.account_code) {
            AccountClass::Income => {
                let amount = checked_neg(net, "مبلغ درآمد")?;
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
                let is_cogs = activity.group_code == "9" || activity.account_code.starts_with('5');
                if is_cogs {
                    cogs.push(row);
                } else {
                    expenses.push(row);
                }
            }
            _ => {}
        }
    }

    let revenue_total = checked_sum(
        revenues.iter().map(|r| r.amount),
        "جمع درآمد صورت سود و زیان",
    )?;
    let cogs_total = checked_sum(cogs.iter().map(|r| r.amount), "جمع بهای تمام‌شده")?;
    let expense_total = checked_sum(
        expenses.iter().map(|r| r.amount),
        "جمع هزینه صورت سود و زیان",
    )?;
    let gross_profit = checked_sub(revenue_total, cogs_total, "سود ناخالص")?;
    let operating_profit = checked_sub(gross_profit, expense_total, "سود عملیاتی")?;
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
        let net = checked_sub(activity.debit, activity.credit, "مانده حساب ترازنامه")?;
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
                amount: checked_neg(net, "مانده بدهی ترازنامه")?,
            }),
            AccountClass::Equity => equity.push(AmountRow {
                account_code: activity.account_code,
                account_name: activity.account_name,
                amount: checked_neg(net, "مانده سرمایه ترازنامه")?,
            }),
            AccountClass::Income => {
                revenue = checked_add(
                    revenue,
                    checked_neg(net, "درآمد ترازنامه")?,
                    "جمع درآمد ترازنامه",
                )?
            }
            AccountClass::Expense => expenses = checked_add(expenses, net, "جمع هزینه ترازنامه")?,
        }
    }

    let assets_total = checked_sum(assets.iter().map(|r| r.amount), "جمع دارایی ترازنامه")?;
    let liabilities_total = checked_sum(liabilities.iter().map(|r| r.amount), "جمع بدهی ترازنامه")?;
    let equity_total = checked_sum(equity.iter().map(|r| r.amount), "جمع سرمایه ترازنامه")?;
    let profit = checked_sub(revenue, expenses, "سود ترازنامه")?;
    let total_liabilities_equity = checked_add(
        checked_add(
            liabilities_total,
            equity_total,
            "جمع بدهی و سرمایه ترازنامه",
        )?,
        profit,
        "جمع بدهی و سرمایه با سود ترازنامه",
    )?;
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
    let digits = value.unsigned_abs().to_string();
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
            vec![line("110101", amount, 0), line("410101", 0, amount)],
        )
    }

    #[test]
    fn validates_and_normalizes_jalali_voucher_dates() {
        assert_eq!(validate_voucher_date("۱۴۰۵/۱/۱").unwrap(), "1405/01/01");
        assert!(validate_voucher_date("1405/13/01").is_err());
        assert!(validate_voucher_date("1405/02/32").is_err());
    }

    #[test]
    fn validates_jalali_leap_day_and_supported_year_range() {
        assert_eq!(validate_voucher_date("۱۴۰۳/۱۲/۳۰").unwrap(), "1403/12/30");
        assert!(validate_voucher_date("۱۴۰۴/۱۲/۳۰").is_err());
        assert_eq!(validate_voucher_date("۱۴۰۴/۱۲/۲۹").unwrap(), "1404/12/29");
        assert!(validate_voucher_date("0000/01/01").is_err());
        assert!(validate_voucher_date("3178/01/01").is_err());
    }

    #[test]
    fn voucher_list_total_count_matches_active_filters() {
        let db = db::memory_database();
        db.save_voucher(cash_sale("1405/01/10", 1_000)).unwrap();
        db.save_voucher(cash_sale("1405/02/10", 2_000)).unwrap();

        let by_date = db.list_vouchers("۱۴۰۵/۰۲/۰۱", "۱۴۰۵/۰۲/۳۱", "").unwrap();
        assert_eq!(by_date.vouchers.len(), 1);
        assert_eq!(by_date.total_count, 1);

        let by_search = db.list_vouchers("", "", "۲").unwrap();
        assert_eq!(by_search.vouchers.len(), 1);
        assert_eq!(by_search.vouchers[0].number, 2);
        assert_eq!(by_search.total_count, by_search.vouchers.len() as i64);
    }

    #[test]
    fn editing_voucher_without_number_preserves_its_number() {
        let db = db::memory_database();
        let saved = db.save_voucher(cash_sale("1405/01/10", 1_000)).unwrap();
        let mut edited = cash_sale("1405/01/11", 2_000);
        edited.id = Some(saved.id);

        let updated = db.save_voucher(edited).unwrap();

        assert_eq!(updated.number, saved.number);
        assert_eq!(updated.date, "1405/01/11");
        assert_eq!(updated.total_debit, 2_000);
    }

    #[test]
    fn adjacent_voucher_numbers_skip_gaps() {
        let db = db::memory_database();
        db.save_voucher(cash_sale("1405/01/10", 1_000)).unwrap();
        let mut third = cash_sale("1405/01/12", 3_000);
        third.number = Some(3);
        db.save_voucher(third).unwrap();

        assert_eq!(db.adjacent_voucher_number(1, "next").unwrap(), Some(3));
        assert_eq!(db.adjacent_voucher_number(3, "previous").unwrap(), Some(1));
        assert_eq!(db.adjacent_voucher_number(3, "next").unwrap(), None);
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
    fn bulk_import_is_atomic_and_supports_large_vouchers() {
        let db = db::memory_database();
        let source_accounts = |general_code: &str, subsidiary_code: &str| {
            vec![
                crate::excel::ImportedAccount {
                    code: general_code.to_string(),
                    name: "حساب کل آزمایشی".to_string(),
                    parent_code: String::new(),
                    group_code: "1".to_string(),
                    level: 1,
                    nature: "debtor".to_string(),
                },
                crate::excel::ImportedAccount {
                    code: subsidiary_code.to_string(),
                    name: "حساب معین آزمایشی".to_string(),
                    parent_code: general_code.to_string(),
                    group_code: "1".to_string(),
                    level: 2,
                    nature: "debtor".to_string(),
                },
            ]
        };
        let make_import = |number, general_code: &str, subsidiary_code: &str, lines| {
            crate::excel::BulkVoucherImport {
                file_name: "آزمایشی.xlsx".to_string(),
                accounts: source_accounts(general_code, subsidiary_code),
                vouchers: vec![VoucherInput {
                    id: None,
                    number: Some(number),
                    date: "1405/01/01".to_string(),
                    voucher_type: VOUCHER_TYPES[0].to_string(),
                    description: String::new(),
                    lines,
                }],
            }
        };

        let mut large_lines = vec![line("901101", 618, 0)];
        large_lines.extend((0..618).map(|_| line("110101", 0, 1)));
        assert_eq!(
            db.import_vouchers(make_import(1, "901", "901101", large_lines), false)
                .unwrap(),
            1
        );
        assert_eq!(db.get_voucher(1).unwrap().unwrap().lines.len(), 619);

        let conflicting = make_import(
            1,
            "902",
            "902101",
            vec![line("902101", 100, 0), line("110101", 0, 100)],
        );
        assert!(db.import_vouchers(conflicting.clone(), false).is_err());
        assert!(!db
            .list_accounts()
            .unwrap()
            .accounts
            .iter()
            .any(|account| account.code == "902"));

        assert_eq!(db.import_vouchers(conflicting, true).unwrap(), 1);
        assert!(db.get_voucher(2).unwrap().is_some());
        assert!(db
            .list_accounts()
            .unwrap()
            .accounts
            .iter()
            .any(|account| account.code == "902101"));
    }

    #[test]
    fn reorganize_preview_merges_then_sorts_by_date() {
        let db = db::memory_database();
        db.save_voucher(cash_sale("1405/01/02", 1_000)).unwrap();
        db.save_voucher(cash_sale("1405/01/01", 2_000)).unwrap();
        let mut source = cash_sale("1405/01/02", 500);
        source.description = "شرح سند مبدأ".to_string();
        db.save_voucher(source).unwrap();
        let mut opening = cash_sale("1405/01/02", 750);
        opening.voucher_type = VOUCHER_TYPES[1].to_string();
        db.save_voucher(opening).unwrap();

        let preview = db
            .preview_voucher_reorganization("merge-then-sort")
            .unwrap();
        assert_eq!(preview.total_vouchers, 4);
        assert_eq!(preview.total_vouchers_after, 3);
        assert_eq!(preview.merge_groups.len(), 1);
        assert_eq!(preview.merge_groups[0].source_voucher_numbers, vec![1, 3]);
        assert_eq!(preview.merge_groups[0].line_count, 4);
        assert_eq!(preview.number_changes.len(), 3);

        let result = db
            .apply_voucher_reorganization("merge-then-sort", &preview.plan_token)
            .unwrap();
        assert_eq!(result.merged_group_count, 1);
        assert_eq!(result.removed_voucher_count, 1);
        assert_eq!(result.renumbered_count, 3);
        assert_eq!(db.get_voucher(1).unwrap().unwrap().date, "1405/01/01");
        let merged = db.get_voucher(2).unwrap().unwrap();
        assert_eq!(merged.date, "1405/01/02");
        assert_eq!(merged.lines.len(), 4);
        assert_eq!(merged.total_debit, 1_500);
        assert!(merged
            .description
            .contains("ادغام از سند شماره 3: شرح سند مبدأ"));
        assert_eq!(
            db.get_voucher(3).unwrap().unwrap().voucher_type,
            VOUCHER_TYPES[1]
        );
        assert!(db.get_voucher(4).unwrap().is_none());
    }

    #[test]
    fn reorganize_does_not_merge_groups_over_the_form_line_limit() {
        let db = db::memory_database();
        let large_voucher = || {
            let mut lines = Vec::with_capacity(501);
            lines.push(line("110101", 251, 0));
            lines.extend((0..249).map(|_| line("110101", 1, 0)));
            lines.push(line("410101", 0, 250));
            lines.extend((0..250).map(|_| line("410101", 0, 1)));
            voucher("1405/01/02", lines)
        };
        db.save_voucher(large_voucher()).unwrap();
        db.save_voucher(large_voucher()).unwrap();

        let preview = db.preview_voucher_reorganization("merge").unwrap();
        assert!(preview.merge_groups.is_empty());
        assert_eq!(preview.unbalanced_groups.len(), 1);
        assert_eq!(preview.unbalanced_groups[0].line_count, 1_002);
        assert!(preview.unbalanced_groups[0].reason.contains("ظرفیت"));

        db.apply_voucher_reorganization("merge", &preview.plan_token)
            .unwrap();
        assert!(db.get_voucher(1).unwrap().is_some());
        assert!(db.get_voucher(2).unwrap().is_some());
    }

    #[test]
    fn reorganize_rejects_a_stale_preview() {
        let db = db::memory_database();
        db.save_voucher(cash_sale("1405/01/02", 1_000)).unwrap();
        let preview = db.preview_voucher_reorganization("sort").unwrap();
        db.save_voucher(cash_sale("1405/01/01", 500)).unwrap();

        assert!(db
            .apply_voucher_reorganization("sort", &preview.plan_token)
            .unwrap_err()
            .contains("تغییر کرده‌اند"));
        assert_eq!(db.list_vouchers("", "", "").unwrap().total_count, 2);
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
                vec![line("110101", 100_000, 50_000), line("410101", 0, 50_000)],
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
                lines: vec![line("110101", 800_000, 0), line("410101", 0, 800_000)],
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
                lines: vec![line("110101", 200_000, 0), line("410101", 0, 200_000)],
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
                lines: vec![line("110101", 200_000, 0), line("410101", 0, 200_000)],
            })
            .unwrap_err();
        assert!(missing.contains("یافت نشد"));
    }

    #[test]
    fn deletes_voucher_and_its_lines() {
        let db = db::memory_database();
        let saved = db.save_voucher(cash_sale("1405/01/10", 300_000)).unwrap();
        assert_eq!(
            db.get_voucher(saved.number).unwrap().unwrap().lines.len(),
            2
        );

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
        assert_eq!(db.list_vouchers("", "", "نقدی").unwrap().vouchers.len(), 1);
        assert_eq!(
            db.list_vouchers("", "", "دریافت").unwrap().vouchers.len(),
            1
        );
        assert_eq!(
            db.list_vouchers("", "", "ناموجود").unwrap().vouchers.len(),
            0
        );
        // Account names are searchable too.
        assert_eq!(db.list_vouchers("", "", "صندوق").unwrap().vouchers.len(), 2);

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
    fn aggregate_arithmetic_reports_overflow_instead_of_wrapping() {
        assert!(checked_add(i64::MAX, 1, "آزمون").is_err());
        assert!(checked_sub(i64::MIN, 1, "آزمون").is_err());
        assert!(checked_neg(i64::MIN, "آزمون").is_err());
        assert!(checked_sum([i64::MAX, 1], "آزمون").is_err());
        assert_eq!(checked_sum([1, 2, 3], "آزمون").unwrap(), 6);
    }

    #[test]
    fn formats_amounts_with_grouping() {
        assert_eq!(format_amount(0), "0");
        assert_eq!(format_amount(999), "999");
        assert_eq!(format_amount(1_000), "1,000");
        assert_eq!(format_amount(1_234_567), "1,234,567");
        assert_eq!(format_amount(-1_500), "-1,500");
        assert_eq!(format_amount(i64::MIN), "-9,223,372,036,854,775,808");
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
        let revenue = trial
            .rows
            .iter()
            .find(|r| r.account_code == "410101")
            .unwrap();
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
        assert_eq!(list.total_count, 1);

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

    #[test]
    fn voucher_number_limit_matches_javascript_safe_integer() {
        let db = db::memory_database();
        let mut accepted = cash_sale("1405/01/10", 1_000);
        accepted.number = Some(MAX_VOUCHER_NUMBER);
        assert!(db.save_voucher(accepted).is_ok());

        let mut rejected = cash_sale("1405/01/10", 1_000);
        rejected.number = Some(MAX_VOUCHER_NUMBER + 1);
        assert!(db.save_voucher(rejected).unwrap_err().contains("حداکثر"));
    }
}
