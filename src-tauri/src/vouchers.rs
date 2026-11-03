//! Voucher persistence and the accounting reports that are built from it.
//!
//! Amounts are stored as whole Rials (`i64`). Every voucher must balance
//! before it is written, so the reports can rely on a sound ledger.

use crate::db::{self, normalize_digits, Database};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize, Serializer};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;

fn serialize_exact_amount<S>(value: &i64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if value.unsigned_abs() <= 9_007_199_254_740_991 {
        serializer.serialize_i64(*value)
    } else {
        serializer.serialize_str(&value.to_string())
    }
}

pub const VOUCHER_TYPES: [&str; 4] = [
    "سند حسابداری عادی",
    "سند حسابداری افتتاحیه",
    "سند حسابداری اختتامیه",
    "سند بستن حساب‌های موقت",
];

pub(crate) const MAX_DESCRIPTION: usize = 1000;
pub(crate) const MAX_LINE_DESCRIPTION: usize = 500;
pub(crate) const MAX_LINES: usize = 1000;
pub const MAX_VOUCHER_NUMBER: i64 = 9_007_199_254_740_991;

/// Rows returned by the paged lists. A caller that omits `limit` still gets
/// every row, so reports and tests keep their previous behavior.
pub const DEFAULT_VOUCHER_PAGE_SIZE: i64 = 200;
pub const DEFAULT_JOURNAL_PAGE_SIZE: i64 = 500;
pub const MAX_PAGE_SIZE: i64 = 2000;

/// Largest amount accepted on one line (1e15 Rials). It stays below 2^53 so
/// the UI, which holds numbers as JavaScript doubles, never loses precision,
/// and it leaves room for sums of `MAX_LINES` lines inside an `i64`.
pub const MAX_AMOUNT: i64 = 999_999_999_999_999;

/// SQL expression for the new `updated_at` of a voucher. It is strictly
/// greater than the current value even when two writes happen in the same
/// millisecond, so an optimistic-lock token can never repeat.
const TOUCHED_AT: &str = "CASE
        WHEN strftime('%Y-%m-%d %H:%M:%f', 'now', 'localtime') <= updated_at
        THEN strftime('%Y-%m-%d %H:%M:%f', updated_at, '+0.001 seconds')
        ELSE strftime('%Y-%m-%d %H:%M:%f', 'now', 'localtime')
    END";

/// Voucher types that close the books. They are real vouchers (the trial
/// balance and ledgers show them) but must not hide the period's income and
/// expenses from the income statement.
const CLOSING_TEMPORARY: &str = VOUCHER_TYPES[3];
const CLOSING_FINAL: &str = VOUCHER_TYPES[2];

/* ── Input / output types ────────────────────────────────────── */

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherLineInput {
    /// Stable row id of a saved line, when the client is editing that line.
    /// A new row has no id and must pass the normal active/postable checks.
    #[serde(default)]
    pub id: Option<i64>,
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
    #[serde(serialize_with = "serialize_exact_amount")]
    pub total_debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
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

/* ── Bulk voucher deletion ──────────────────────────────────── */

/// Largest number of vouchers one bulk-delete operation may target. It keeps
/// the preview rows and the final transaction bounded, like the WebApp form.
pub const MAX_BULK_DELETE_TARGETS: usize = 5000;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkVoucherDeleteInput {
    /// `all`, `dateRange` or `numberRange`.
    pub scope: String,
    #[serde(default)]
    pub from_date: String,
    #[serde(default)]
    pub to_date: String,
    #[serde(default)]
    pub from_number: Option<i64>,
    #[serde(default)]
    pub to_number: Option<i64>,
}

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
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkVoucherDeleteSample {
    pub voucher_number: i64,
    pub display_date: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkVoucherDeleteDateRange {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkVoucherDeleteNumberRange {
    pub from: i64,
    pub to: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkVoucherDeletePreview {
    pub scope: String,
    pub plan_token: String,
    pub total_matched: usize,
    pub deletable_count: usize,
    /// Reserved for parity with the WebApp form. This application has no
    /// permanent-voucher flag, so every matched voucher is deletable.
    pub locked_count: usize,
    pub locked_sample: Vec<BulkVoucherDeleteSample>,
    pub deletable_sample: Vec<BulkVoucherDeleteSample>,
    pub date_range: Option<BulkVoucherDeleteDateRange>,
    pub number_range: Option<BulkVoucherDeleteNumberRange>,
    pub timestamp: String,
    /// Where an automatic pre-delete backup is written, when known.
    pub backup_directory: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkVoucherDeleteResult {
    pub scope: String,
    pub message: String,
    pub deleted_count: usize,
    pub locked_count: usize,
    pub timestamp: String,
    pub with_backup: bool,
    pub pre_delete_backup_file: Option<String>,
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
    const COLUMNS: &str = "SELECT id, number, date, type, description, updated_at FROM vouchers";
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
    #[serde(serialize_with = "serialize_exact_amount")]
    pub debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub credit: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Voucher {
    pub id: i64,
    pub number: i64,
    pub date: String,
    pub updated_at: String,
    #[serde(rename = "type")]
    pub voucher_type: String,
    pub description: String,
    pub lines: Vec<VoucherLine>,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub total_debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub total_credit: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherSummary {
    pub id: i64,
    pub number: i64,
    pub date: String,
    pub updated_at: String,
    #[serde(rename = "type")]
    pub voucher_type: String,
    pub description: String,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub total_debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub total_credit: i64,
    pub line_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherList {
    pub vouchers: Vec<VoucherSummary>,
    /// Number of vouchers in the whole book, ignoring any filter.
    pub total_count: i64,
    /// Number of vouchers matching the active date and search filters.
    pub matched_count: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub total_debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
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
    #[serde(serialize_with = "serialize_exact_amount")]
    pub debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub credit: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalReport {
    pub from: String,
    pub to: String,
    pub search: String,
    pub rows: Vec<JournalRow>,
    /// Number of journal rows matching the filter, before paging.
    pub row_count: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub total_debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
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
    #[serde(serialize_with = "serialize_exact_amount")]
    pub debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub credit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub running_debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
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
    #[serde(serialize_with = "serialize_exact_amount")]
    pub opening_debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub opening_credit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub total_debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub total_credit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub closing_debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
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
    #[serde(serialize_with = "serialize_exact_amount")]
    pub opening_debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub opening_credit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub period_debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub period_credit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub closing_debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub closing_credit: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrialBalanceReport {
    pub from: String,
    pub to: String,
    pub rows: Vec<TrialBalanceRow>,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub opening_debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub opening_credit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub period_debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub period_credit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub closing_debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub closing_credit: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AmountRow {
    pub account_code: String,
    pub account_name: String,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub amount: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncomeStatementReport {
    pub from: String,
    pub to: String,
    pub revenues: Vec<AmountRow>,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub revenue_total: i64,
    pub cogs: Vec<AmountRow>,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub cogs_total: i64,
    pub other_operating_revenues: Vec<AmountRow>,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub other_operating_revenue_total: i64,
    pub other_operating_expenses: Vec<AmountRow>,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub other_operating_expense_total: i64,
    pub expenses: Vec<AmountRow>,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub expense_total: i64,
    pub non_operating_revenues: Vec<AmountRow>,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub non_operating_revenue_total: i64,
    pub non_operating_expenses: Vec<AmountRow>,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub non_operating_expense_total: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub gross_profit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub operating_profit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub net_profit: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceSheetReport {
    pub as_of: String,
    pub assets: Vec<AmountRow>,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub assets_total: i64,
    pub liabilities: Vec<AmountRow>,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub liabilities_total: i64,
    pub equity: Vec<AmountRow>,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub equity_total: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub profit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub total_liabilities_equity: i64,
    pub balanced: bool,
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

/// One account's own ledger amounts before parent roll-up.
struct TrialAccountRaw {
    code: String,
    name: String,
    group_name: String,
    level: i64,
    parent_code: String,
    opening_debit: i64,
    opening_credit: i64,
    period_debit: i64,
    period_credit: i64,
}

/// Adds all child amounts to their parent, recursively, and memoizes results.
fn roll_up_trial_amounts(
    code: &str,
    accounts: &[TrialAccountRaw],
    index_by_code: &std::collections::HashMap<String, usize>,
    children: &std::collections::HashMap<String, Vec<String>>,
    memo: &mut std::collections::HashMap<String, (i64, i64, i64, i64)>,
) -> Result<(i64, i64, i64, i64), String> {
    if let Some(amounts) = memo.get(code) {
        return Ok(*amounts);
    }
    let Some(&index) = index_by_code.get(code) else {
        return Ok((0, 0, 0, 0));
    };
    let account = &accounts[index];
    let mut total = (
        account.opening_debit,
        account.opening_credit,
        account.period_debit,
        account.period_credit,
    );
    if let Some(child_codes) = children.get(code) {
        for child in child_codes {
            let amounts = roll_up_trial_amounts(child, accounts, index_by_code, children, memo)?;
            total.0 = checked_add(total.0, amounts.0, "جمع تراز آزمایشی")?;
            total.1 = checked_add(total.1, amounts.1, "جمع تراز آزمایشی")?;
            total.2 = checked_add(total.2, amounts.2, "جمع تراز آزمایشی")?;
            total.3 = checked_add(total.3, amounts.3, "جمع تراز آزمایشی")?;
        }
    }
    memo.insert(code.to_string(), total);
    Ok(total)
}

/// Account codes are compared level by level (two digits per level), so
/// "2" sorts after "10" and a range like "11".."11" covers its whole subtree.
fn trial_code_segments(code: &str) -> Vec<u32> {
    let digits = normalize_code_filter(code);
    digits
        .as_bytes()
        .chunks(2)
        .map(|chunk| {
            std::str::from_utf8(chunk)
                .ok()
                .and_then(|text| text.parse::<u32>().ok())
                .unwrap_or(0)
        })
        .collect()
}

fn compare_trial_segments(left: &[u32], right: &[u32]) -> std::cmp::Ordering {
    let length = left.len().min(right.len());
    for index in 0..length {
        match left[index].cmp(&right[index]) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

fn normalize_code_filter(input: &str) -> String {
    db::normalize_code(input)
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect()
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
            let maximum: i64 = conn
                .query_row("SELECT COALESCE(MAX(number), 0) FROM vouchers", [], |row| {
                    row.get(0)
                })
                .map_err(db::db_error)?;
            let next = maximum
                .checked_add(1)
                .ok_or_else(|| "شماره سند بعدی از محدوده مجاز خارج است.".to_string())?;
            if next > MAX_VOUCHER_NUMBER {
                return Err(format!(
                    "شماره سند بعدی از حداکثر {} بیشتر است؛ شماره سند جدیدی ثبت نمی‌شود.",
                    db::to_fa_digits(MAX_VOUCHER_NUMBER.to_string())
                ));
            }
            Ok(next)
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

    pub fn voucher_count(&self) -> Result<i64, String> {
        self.with_conn(|conn| count(conn, "SELECT COUNT(*) FROM vouchers"))
    }

    pub fn list_vouchers(&self, from: &str, to: &str, search: &str) -> Result<VoucherList, String> {
        self.list_vouchers_page(from, to, search, None, None)
    }

    /// Returns one page of the voucher list. Both totals and `matched_count`
    /// are still calculated over every voucher matching the filter, not only
    /// the returned page.
    pub fn list_vouchers_page(
        &self,
        from: &str,
        to: &str,
        search: &str,
        offset: Option<i64>,
        limit: Option<i64>,
    ) -> Result<VoucherList, String> {
        let from = normalize_filter_date(from, "تاریخ شروع")?;
        let to = normalize_filter_date(to, "تاریخ پایان")?;
        check_range(&from, &to)?;
        let terms = search_terms(search);
        let page = page_window(offset, limit)?;

        let (vouchers, total_count, matched_count, total_debit, total_credit) =
            self.with_conn(|conn| {
                let vouchers = list_vouchers(conn, &from, &to, &terms, page)?;
                let total = count(conn, "SELECT COUNT(*) FROM vouchers")?;
                let (matched, debit, credit) = voucher_list_totals(conn, &from, &to, &terms)?;
                Ok((vouchers, total, matched, debit, credit))
            })?;
        Ok(VoucherList {
            vouchers,
            total_count,
            matched_count,
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
                    &format!(
                        "UPDATE vouchers
                            SET description = ?1, updated_at = {TOUCHED_AT}
                          WHERE id = ?2"
                    ),
                    params![group.description, group.keep_id],
                )
                .map_err(db::db_error)?;
            }

            for (index, (voucher_id, _)) in plan.number_changes.iter().enumerate() {
                let offset = i64::try_from(index)
                    .map_err(|_| "تعداد اسناد برای بازشماره‌گذاری بیش از حد مجاز است.".to_string())?;
                let temporary_number = MAX_VOUCHER_NUMBER
                    .checked_add(1)
                    .and_then(|base| base.checked_add(offset))
                    .ok_or_else(|| "شماره موقت بازشماره‌گذاری از محدوده خارج است.".to_string())?;
                tx.execute(
                    "UPDATE vouchers SET number = ?1 WHERE id = ?2",
                    params![temporary_number, voucher_id],
                )
                .map_err(db::db_error)?;
            }
            for (voucher_id, change) in &plan.number_changes {
                tx.execute(
                    &format!(
                        "UPDATE vouchers
                            SET number = ?1, updated_at = {TOUCHED_AT}
                          WHERE id = ?2"
                    ),
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
            let deletable_sample = targets.iter().take(20).map(bulk_delete_sample).collect();
            Ok(BulkVoucherDeletePreview {
                scope: scope.as_str().to_string(),
                plan_token,
                total_matched: targets.len(),
                deletable_count: targets.len(),
                locked_count: 0,
                locked_sample: Vec::new(),
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
            for target in &targets {
                tx.execute("DELETE FROM vouchers WHERE id = ?1", params![target.id])
                    .map_err(db::db_error)?;
            }
            let deleted_count = targets.len();
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
                locked_count: 0,
                timestamp,
                with_backup,
                pre_delete_backup_file: backup_file,
            })
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
        if input.target_voucher_number <= 0 || input.target_voucher_number > MAX_VOUCHER_NUMBER {
            return Err(format!(
                "شماره سند مقصد باید بین ۱ و {} باشد.",
                db::to_fa_digits(MAX_VOUCHER_NUMBER.to_string())
            ));
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
                            id: None,
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
                if assert_postable_leaf(&tx, &line.account_code).is_err() {
                    return Err(format!(
                        "حساب ردیف {} دیگر قابل ثبت نیست؛ انتقال متوقف شد.",
                        db::to_fa_digits(*row_no)
                    ));
                }
                assert_active_account(&tx, &line.account_code)?;
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

            let existing_target = tx
                .query_row(
                    "SELECT id, type FROM vouchers WHERE number = ?1",
                    params![target_number],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(db::db_error)?;
            if existing_target.as_ref().map(|(id, _)| *id) == Some(source_id) {
                return Err("سند مقصد نمی‌تواند همان سند مبدأ باشد.".to_string());
            }
            let target_type = existing_target
                .as_ref()
                .map(|(_, kind)| kind.clone())
                .unwrap_or_else(|| source_type.clone());

            // The selected lines join the target voucher, so they must agree
            // with the target's type even when they came from another type.
            let selected_lines: Vec<VoucherLineInput> = selected
                .iter()
                .map(|(_, _, line)| line.clone())
                .collect();
            assert_voucher_type_matches_lines(&tx, &target_type, &selected_lines)?;

            let (target_id, target_count, target_debit, target_credit, created_target) =
                if let Some((id, _)) = existing_target {
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
                    &format!("UPDATE vouchers SET updated_at = {TOUCHED_AT} WHERE id = ?1"),
                    params![source_id],
                ).map_err(db::db_error)?;
            }
            tx.execute(
                &format!("UPDATE vouchers SET updated_at = {TOUCHED_AT} WHERE id = ?1"),
                params![target_id],
            ).map_err(db::db_error)?;

            tx.commit().map_err(db::db_error)?;
            let target = fetch_voucher_by_id(conn, target_id)?;
            let source = if is_move { Some(fetch_voucher_by_id(conn, source_id)?) } else { None };
            Ok(VoucherTransferResult { mode, created_target, target, source })
        })
    }

    pub fn save_voucher(&self, input: VoucherInput) -> Result<Voucher, String> {
        self.save_voucher_inner(input, None)
    }

    pub fn save_voucher_if_unchanged(
        &self,
        input: VoucherInput,
        expected_updated_at: Option<&str>,
    ) -> Result<Voucher, String> {
        if input.id.is_some() && expected_updated_at.is_none() {
            return Err("نسخه سند مشخص نیست؛ سند را دوباره بارگذاری کنید.".to_string());
        }
        self.save_voucher_inner(input, expected_updated_at.map(str::to_string))
    }

    fn save_voucher_inner(
        &self,
        input: VoucherInput,
        expected_updated_at: Option<String>,
    ) -> Result<Voucher, String> {
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
                id: line.id,
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

        if input.id.is_some_and(|id| id <= 0) {
            return Err("شناسه سند نامعتبر است.".to_string());
        }
        let id = input.id.unwrap_or(0);
        let requested_number = input.number.unwrap_or(0);
        if requested_number < 0 {
            return Err("شماره سند نمی‌تواند منفی باشد.".to_string());
        }

        self.with_conn(move |conn| {
            let tx = conn.transaction().map_err(db::db_error)?;

            // Saving and exporting use the same account-eligibility rules.
            // A saved inactive/legacy account is allowed only on its original
            // line id; a new or changed row must pass the current rules.
            validate_voucher_line_accounts(&tx, (id > 0).then_some(id), &lines)?;
            assert_voucher_type_matches_lines(&tx, &voucher_type, &lines)?;

            let existing = if id > 0 {
                tx.query_row(
                    "SELECT number, updated_at FROM vouchers WHERE id = ?1",
                    params![id],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(db::db_error)?
            } else {
                None
            };
            if id > 0 && existing.is_none() {
                return Err("سند حسابداری یافت نشد.".to_string());
            }
            if let (Some(expected), Some((_, current))) =
                (expected_updated_at.as_deref(), existing.as_ref())
            {
                if expected != current {
                    return Err(
                        "سند پس از باز شدن تغییر کرده است؛ دوباره بارگذاری کنید.".to_string()
                    );
                }
            }

            let existing_number = existing.as_ref().map(|(number, _)| *number);
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
                let updated = if let Some(expected) = expected_updated_at.as_deref() {
                    tx.execute(
                        &format!(
                            "UPDATE vouchers
                                SET number = ?1, date = ?2, type = ?3, description = ?4,
                                    updated_at = {TOUCHED_AT}
                              WHERE id = ?5 AND updated_at = ?6"
                        ),
                        params![number, date, voucher_type, description, id, expected],
                    )
                } else {
                    tx.execute(
                        &format!(
                            "UPDATE vouchers
                                SET number = ?1, date = ?2, type = ?3, description = ?4,
                                    updated_at = {TOUCHED_AT}
                              WHERE id = ?5"
                        ),
                        params![number, date, voucher_type, description, id],
                    )
                }
                .map_err(db::db_error)?;
                if updated == 0 && expected_updated_at.is_some() {
                    return Err(
                        "سند پس از باز شدن تغییر کرده است؛ دوباره بارگذاری کنید.".to_string()
                    );
                }
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
        let sheet_name = import.sheet_name.clone();
        let account_rows = import.accounts.clone();
        let account_codes: HashSet<String> = account_rows
            .iter()
            .map(|account| db::normalize_code(&account.code))
            .collect();
        let imported_parent_children: HashMap<String, String> = account_rows
            .iter()
            .filter_map(|account| {
                let parent = db::normalize_code(&account.parent_code);
                (!parent.is_empty()).then(|| {
                    (parent, db::normalize_code(&account.code))
                })
            })
            .collect();
        let referenced_codes: HashSet<String> = import
            .vouchers
            .iter()
            .flat_map(|voucher| voucher.lines.iter())
            .map(|line| db::normalize_code(&line.account_code))
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
            // Validate all groups and existing account structures during preview,
            // not only after the user presses the final import button.
            let mut missing_groups: std::collections::BTreeMap<String, Vec<String>> =
                std::collections::BTreeMap::new();
            for account in &account_rows {
                let group_code = db::normalize_code(&account.group_code);
                let exists = conn
                    .query_row(
                        "SELECT 1 FROM account_groups WHERE code = ?1",
                        params![group_code],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(db::db_error)?
                    .is_some();
                if !exists {
                    missing_groups
                        .entry(group_code)
                        .or_default()
                        .push(account.code.clone());
                }
            }
            if !missing_groups.is_empty() {
                let detail = missing_groups
                    .iter()
                    .map(|(group, codes)| {
                        let shown = codes
                            .iter()
                            .take(5)
                            .map(db::to_fa_digits)
                            .collect::<Vec<_>>()
                            .join("، ");
                        let more = if codes.len() > 5 {
                            format!(" و {} حساب دیگر", db::to_fa_digits(codes.len() - 5))
                        } else {
                            String::new()
                        };
                        format!("گروه «{}» برای حساب‌های {shown}{more}", db::to_fa_digits(group))
                    })
                    .collect::<Vec<_>>()
                    .join(" | ");
                return Err(format!(
                    "پیش از ورود اسناد، گروه حساب‌های نامعتبر را اصلاح کنید: {detail}"
                ));
            }

            let mut new_account_count = 0;
            for account in &account_rows {
                let code = db::normalize_code(&account.code);
                let group_code = db::normalize_code(&account.group_code);
                let parent_code = db::normalize_code(&account.parent_code);
                let existing = conn
                    .query_row(
                        "SELECT level, group_code, COALESCE(parent_code, ''), nature
                           FROM accounts WHERE code = ?1",
                        params![code],
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
                    .map_err(db::db_error)?;

                if let Some((level, existing_group, existing_parent, existing_nature)) = existing {
                    if level != account.level
                        || existing_group != group_code
                        || existing_parent != parent_code
                    {
                        return Err(format!(
                            "حساب {} از قبل با گروه، سطح یا حساب بالاتر متفاوت ثبت شده است.",
                            db::to_fa_digits(&code)
                        ));
                    }
                    if account.nature_is_explicit && existing_nature != account.nature {
                        return Err(format!(
                            "ماهیت حساب {} با مقدار واردشده در فایل اکسل متفاوت است.",
                            db::to_fa_digits(&code)
                        ));
                    }
                } else {
                    if !matches!(account.nature.as_str(), "debtor" | "creditor" | "dual") {
                        return Err(format!(
                            "ماهیت حساب جدید {} مشخص نیست؛ برای گروه‌های سایر درآمدها و هزینه‌ها، ستون ماهیت مربوط به حساب را با مقدار بدهکار، بستانکار یا دوگانه تکمیل کنید.",
                            db::to_fa_digits(&code)
                        ));
                    }
                    new_account_count += 1;
                    if account.level > 1 && !parent_code.is_empty() {
                        let parent = conn
                            .query_row(
                                "SELECT level, group_code, is_active FROM accounts WHERE code = ?1",
                                params![parent_code],
                                |row| {
                                    Ok((
                                        row.get::<_, i64>(0)?,
                                        row.get::<_, String>(1)?,
                                        row.get::<_, i64>(2)?,
                                    ))
                                },
                            )
                            .optional()
                            .map_err(db::db_error)?;
                        if let Some((parent_level, parent_group, parent_is_active)) = parent {
                            if parent_level != account.level - 1 || parent_group != group_code {
                                return Err(format!(
                                    "گروه یا سطح حساب بالاتر {} با حساب {} سازگار نیست.",
                                    db::to_fa_digits(&parent_code),
                                    db::to_fa_digits(&code)
                                ));
                            }
                            if parent_is_active == 0 {
                                return Err(format!(
                                    "حساب بالاتر {} غیرفعال است؛ ابتدا آن را فعال کنید.",
                                    db::to_fa_digits(&parent_code)
                                ));
                            }
                            let parent_has_lines = conn
                                .query_row(
                                    "SELECT EXISTS(SELECT 1 FROM voucher_lines WHERE account_code = ?1)",
                                    params![parent_code],
                                    |row| row.get::<_, bool>(0),
                                )
                                .map_err(db::db_error)?;
                            if parent_has_lines {
                                return Err(format!(
                                    "حساب بالاتر {} در اسناد استفاده شده است؛ برای آن نمی‌توان زیرحساب جدید ساخت.",
                                    db::to_fa_digits(&parent_code)
                                ));
                            }
                        } else if !account_codes.contains(&parent_code) {
                            return Err(format!(
                                "حساب بالاتر {} برای حساب {} در فایل یا پایگاه داده پیدا نشد.",
                                db::to_fa_digits(&parent_code),
                                db::to_fa_digits(&code)
                            ));
                        }
                    }
                }
            }

            // A newly imported parent becomes non-postable when its child is
            // created. Catch a workbook that tries to post to both levels now,
            // rather than failing only after the user confirms the import.
            for code in &referenced_codes {
                if let Some(child_code) = imported_parent_children.get(code) {
                    return Err(format!(
                        "حساب {} در فایل اکسل زیرحساب {} دارد و پس از ورود قابل ثبت در سند نخواهد بود؛ ردیف‌های سند باید از حساب تفصیلی استفاده کنند.",
                        db::to_fa_digits(code),
                        db::to_fa_digits(child_code)
                    ));
                }
            }

            // Every voucher line must resolve either to an existing account or
            // to an account that this workbook is going to create.
            for code in &referenced_codes {
                let exists = conn
                    .query_row(
                        "SELECT 1 FROM accounts WHERE code = ?1",
                        params![code],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(db::db_error)?
                    .is_some();
                if exists {
                    assert_postable_leaf(conn, code)?;
                    assert_active_account(conn, code)?;
                } else if !account_codes.contains(code) {
                    return Err(format!(
                        "حساب ردیف سند با کد {} در فایل یا پایگاه داده پیدا نشد.",
                        db::to_fa_digits(code)
                    ));
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
                sheet_name,
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
            if number <= 0 || number > MAX_VOUCHER_NUMBER || !numbers.insert(number) {
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
                    id: line.id,
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
                    || !(1..=db::MAX_ACCOUNT_LEVEL).contains(&account.level)
                {
                    return Err("اطلاعات یکی از حساب‌های فایل اکسل معتبر نیست.".to_string());
                }
                let existing = tx
                    .query_row(
                        "SELECT level, group_code, COALESCE(parent_code, ''), nature
                           FROM accounts WHERE code = ?1",
                        params![code],
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
                    .map_err(db::db_error)?;
                if let Some((level, existing_group, existing_parent, existing_nature)) = existing {
                    if level != account.level
                        || existing_group != group_code
                        || existing_parent != parent_code
                    {
                        return Err(format!(
                            "حساب {} از قبل با گروه، سطح یا حساب بالاتر متفاوت ثبت شده است.",
                            db::to_fa_digits(&code)
                        ));
                    }
                    if account.nature_is_explicit && existing_nature != account.nature {
                        return Err(format!(
                            "ماهیت حساب {} با مقدار واردشده در فایل اکسل متفاوت است.",
                            db::to_fa_digits(&code)
                        ));
                    }
                    continue;
                }
                if !matches!(account.nature.as_str(), "debtor" | "creditor" | "dual") {
                    return Err(format!(
                        "ماهیت حساب جدید {} مشخص نیست؛ برای گروه‌های سایر درآمدها و هزینه‌ها، ستون ماهیت مربوط به حساب را تکمیل کنید.",
                        db::to_fa_digits(&code)
                    ));
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
                            "SELECT name, level, group_code, is_active FROM accounts WHERE code = ?1",
                            params![parent_code],
                            |row| {
                                Ok((
                                    row.get::<_, String>(0)?,
                                    row.get::<_, i64>(1)?,
                                    row.get::<_, String>(2)?,
                                    row.get::<_, i64>(3)?,
                                ))
                            },
                        )
                        .optional()
                        .map_err(db::db_error)?
                        .ok_or_else(|| format!("حساب بالاتر {} برای {} پیدا نشد.", db::to_fa_digits(&parent_code), db::to_fa_digits(&code)))?;
                    if parent.1 != account.level - 1 || parent.2 != group_code {
                        return Err(format!("گروه یا سطح حساب بالاتر {} با حساب {} سازگار نیست.", db::to_fa_digits(&parent_code), db::to_fa_digits(&code)));
                    }
                    if parent.3 == 0 {
                        return Err(format!(
                            "حساب بالاتر {} غیرفعال است؛ ابتدا آن را فعال کنید.",
                            db::to_fa_digits(&parent_code)
                        ));
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
                    params![code, name, account.level, group_code, account.nature, parent_code_value, parent_name, i64::from(account.level > 1)],
                )
                .map_err(db::db_error)?;
            }

            for (number, date, voucher_type, description, lines) in prepared {
                for line in &lines {
                    assert_postable_leaf(&tx, &line.account_code)?;
                    assert_active_account(&tx, &line.account_code)?;
                }
                assert_voucher_type_matches_lines(&tx, &voucher_type, &lines)?;
                let mut stored_number = number;
                if used_numbers.contains(&number) {
                    if !renumber_conflicts {
                        return Err(format!("شماره سند {} از قبل در برنامه وجود دارد.", db::to_fa_digits(number)));
                    }
                    loop {
                        next_number = next_number.checked_add(1)
                            .ok_or_else(|| "شماره سند بعدی از محدوده عددی مجاز بیشتر است.".to_string())?;
                        if next_number > MAX_VOUCHER_NUMBER {
                            return Err(format!(
                                "شماره سند بعدی از حداکثر {} بیشتر است.",
                                db::to_fa_digits(MAX_VOUCHER_NUMBER.to_string())
                            ));
                        }
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

    pub fn delete_voucher(&self, id: i64, expected_updated_at: Option<&str>) -> Result<(), String> {
        if id <= 0 {
            return Err("شناسه سند نامعتبر است.".to_string());
        }
        let expected = expected_updated_at
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "نسخه سند مشخص نیست؛ سند را دوباره بارگذاری کنید.".to_string())?
            .to_string();
        self.with_conn(move |conn| {
            let current = conn
                .query_row(
                    "SELECT updated_at FROM vouchers WHERE id = ?1",
                    params![id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(db::db_error)?;
            let Some(current) = current else {
                return Err("سند حسابداری یافت نشد.".to_string());
            };
            if current != expected {
                return Err(
                    "این سند پس از باز شدن تغییر کرده است؛ فهرست یا سند را دوباره بارگذاری کنید و حذف را تکرار کنید."
                        .to_string(),
                );
            }
            let affected = conn
                .execute(
                    "DELETE FROM vouchers WHERE id = ?1 AND updated_at = ?2",
                    params![id, expected],
                )
                .map_err(db::db_error)?;
            if affected == 0 {
                return Err(
                    "این سند پس از باز شدن تغییر کرده است؛ فهرست یا سند را دوباره بارگذاری کنید و حذف را تکرار کنید."
                        .to_string(),
                );
            }
            Ok(())
        })
    }

    /// Applies the same row-identity and account-eligibility checks as saving.
    /// This prevents an Excel export from containing postings that could not
    /// be saved, including new rows for inactive or legacy accounts.
    pub fn validate_voucher_accounts(
        &self,
        voucher_id: Option<i64>,
        lines: &[VoucherLineInput],
    ) -> Result<(), String> {
        self.with_conn(|conn| validate_voucher_line_accounts(conn, voucher_id, lines))
    }

    /* ── Reports ─────────────────────────────────────────────── */

    pub fn get_journal(&self, from: &str, to: &str, search: &str) -> Result<JournalReport, String> {
        self.get_journal_page(from, to, search, None, None)
    }

    /// Returns one page of journal rows while keeping the totals and voucher
    /// count for the whole filter.
    pub fn get_journal_page(
        &self,
        from: &str,
        to: &str,
        search: &str,
        offset: Option<i64>,
        limit: Option<i64>,
    ) -> Result<JournalReport, String> {
        let from = normalize_filter_date(from, "تاریخ شروع")?;
        let to = normalize_filter_date(to, "تاریخ پایان")?;
        check_range(&from, &to)?;
        let terms = search_terms(search);
        let search = search.trim().to_string();
        let page = page_window(offset, limit)?;

        self.with_conn(|conn| {
            let where_clause = format!(
                "WHERE (?1 = '' OR v.date >= ?1)
                    AND (?2 = '' OR v.date <= ?2)
                    AND {}",
                search_predicate()
            );
            let totals_sql = format!(
                "SELECT COUNT(*),
                        COUNT(DISTINCT v.id),
                        COALESCE(SUM(l.debit), 0),
                        COALESCE(SUM(l.credit), 0)
                   FROM voucher_lines l
                   JOIN vouchers v ON v.id = l.voucher_id
                   JOIN accounts a ON a.code = l.account_code
                 {where_clause}"
            );
            let (row_count, voucher_count, total_debit, total_credit) = conn
                .query_row(
                    &totals_sql,
                    params![from, to, terms.folded, terms.raw],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    },
                )
                .map_err(db::db_error)?;

            let mut sql = format!(
                "SELECT v.id, v.number, v.date, v.type, v.description,
                        l.row_no, l.account_code, a.name, l.description,
                        l.debit, l.credit
                   FROM voucher_lines l
                   JOIN vouchers v ON v.id = l.voucher_id
                   JOIN accounts a ON a.code = l.account_code
                 {where_clause}
                  ORDER BY v.date, v.number, l.row_no"
            );
            if page.is_some() {
                sql.push_str(" LIMIT ?5 OFFSET ?6");
            }
            let mut stmt = conn.prepare(&sql).map_err(db::db_error)?;
            let map_row = |row: &rusqlite::Row<'_>| {
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
            };
            let rows = if let Some((limit, offset)) = page {
                stmt.query_map(
                    params![from, to, terms.folded, terms.raw, limit, offset],
                    map_row,
                )
                .map_err(db::db_error)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db::db_error)?
            } else {
                stmt.query_map(
                    params![from, to, terms.folded, terms.raw],
                    map_row,
                )
                .map_err(db::db_error)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db::db_error)?
            };

            Ok(JournalReport {
                from,
                to,
                search,
                rows,
                total_debit,
                total_credit,
                row_count,
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
        // Kept for callers and tests that predate the report filters. It shows
        // every level and every account of an active branch, including rows
        // whose closing balance is zero.
        self.get_trial_balance_filtered(from, to, "", "", true, true)
    }

    /// Hierarchical trial balance with the WebApp report filters.
    ///
    /// - `account_from` / `account_to` limit the code range; either side may be
    ///   empty. When a range is given every level is searched.
    /// - `show_zero` keeps accounts whose closing balance is zero.
    /// - `multi_level` shows the whole tree; otherwise only level-1 accounts.
    pub fn get_trial_balance_filtered(
        &self,
        from: &str,
        to: &str,
        account_from: &str,
        account_to: &str,
        show_zero: bool,
        multi_level: bool,
    ) -> Result<TrialBalanceReport, String> {
        let from = normalize_filter_date(from, "تاریخ شروع")?;
        let to = normalize_filter_date(to, "تاریخ پایان")?;
        check_range(&from, &to)?;
        let code_from = normalize_code_filter(account_from);
        let code_to = normalize_code_filter(account_to);
        let has_code_range = !code_from.is_empty() || !code_to.is_empty();

        self.with_conn(|conn| {
            // Aggregate in SQLite so only one row per account crosses into Rust.
            // Parent accounts are rolled up from these raw amounts below.
            let mut stmt = conn
                .prepare(
                    "SELECT a.code, a.name, g.name, a.level, COALESCE(a.parent_code, ''),
                            COALESCE(SUM(CASE WHEN ?1 <> '' AND v.date < ?1
                                              THEN l.debit END), 0) AS opening_debit,
                            COALESCE(SUM(CASE WHEN ?1 <> '' AND v.date < ?1
                                              THEN l.credit END), 0) AS opening_credit,
                            COALESCE(SUM(CASE WHEN (?1 = '' OR v.date >= ?1)
                                               AND (?2 = '' OR v.date <= ?2)
                                              THEN l.debit END), 0) AS period_debit,
                            COALESCE(SUM(CASE WHEN (?1 = '' OR v.date >= ?1)
                                               AND (?2 = '' OR v.date <= ?2)
                                              THEN l.credit END), 0) AS period_credit
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
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                })
                .map_err(db::db_error)?;

            let mut accounts = Vec::new();
            for row in raw {
                let (code, name, group_name, level, parent_code, od, oc, pd, pc) =
                    row.map_err(db::db_error)?;
                accounts.push(TrialAccountRaw {
                    code,
                    name,
                    group_name,
                    level,
                    parent_code,
                    opening_debit: od,
                    opening_credit: oc,
                    period_debit: pd,
                    period_credit: pc,
                });
            }

            let mut index_by_code = std::collections::HashMap::new();
            for (index, account) in accounts.iter().enumerate() {
                index_by_code.insert(account.code.clone(), index);
            }
            let mut children: std::collections::HashMap<String, Vec<String>> =
                std::collections::HashMap::new();
            let mut has_direct_activity = Vec::with_capacity(accounts.len());
            for account in &accounts {
                if !account.parent_code.is_empty() && index_by_code.contains_key(&account.parent_code)
                {
                    children
                        .entry(account.parent_code.clone())
                        .or_default()
                        .push(account.code.clone());
                }
                has_direct_activity.push(
                    account.opening_debit != 0
                        || account.opening_credit != 0
                        || account.period_debit != 0
                        || account.period_credit != 0,
                );
            }

            // Roll every child's amounts into its parent so a level-1 row shows
            // the whole branch, just like the WebApp balance report.
            let mut rolled = std::collections::HashMap::new();
            for account in &accounts {
                let amounts = roll_up_trial_amounts(
                    &account.code,
                    &accounts,
                    &index_by_code,
                    &children,
                    &mut rolled,
                )?;
                rolled.insert(account.code.clone(), amounts);
            }

            // The WebApp report shows a whole branch as soon as one of its
            // accounts has activity: mark every active account, its ancestors
            // and all of its descendants. Branches without activity stay out.
            let mut relevant = vec![false; accounts.len()];
            let mut queue: Vec<usize> = has_direct_activity
                .iter()
                .enumerate()
                .filter_map(|(index, active)| active.then_some(index))
                .collect();
            while let Some(index) = queue.pop() {
                if relevant[index] {
                    continue;
                }
                relevant[index] = true;
                let parent = accounts[index].parent_code.clone();
                if let Some(&parent_index) = index_by_code.get(&parent) {
                    queue.push(parent_index);
                }
                if let Some(child_codes) = children.get(&accounts[index].code) {
                    for child in child_codes {
                        if let Some(&child_index) = index_by_code.get(child) {
                            queue.push(child_index);
                        }
                    }
                }
            }

            let mut rows = Vec::new();
            for (index, account) in accounts.iter().enumerate() {
                if !relevant[index] {
                    continue;
                }
                if !has_code_range && !multi_level && account.level != 1 {
                    continue;
                }
                if has_code_range {
                    let segments = trial_code_segments(&account.code);
                    if !code_from.is_empty()
                        && compare_trial_segments(&segments, &trial_code_segments(&code_from))
                            == std::cmp::Ordering::Less
                    {
                        continue;
                    }
                    if !code_to.is_empty()
                        && compare_trial_segments(&segments, &trial_code_segments(&code_to))
                            == std::cmp::Ordering::Greater
                    {
                        continue;
                    }
                }

                let (od, oc, pd, pc) = rolled
                    .get(&account.code)
                    .copied()
                    .unwrap_or((0, 0, 0, 0));
                let opening_net = checked_sub(od, oc, "مانده افتتاحیه تراز آزمایشی")?;
                let closing_net = checked_sub(
                    checked_add(opening_net, pd, "مانده پایان دوره تراز آزمایشی")?,
                    pc,
                    "مانده پایان دوره تراز آزمایشی",
                )?;
                let closing_debit = closing_net.max(0);
                let closing_credit =
                    checked_neg(closing_net, "مانده بستانکار پایان دوره تراز آزمایشی")?.max(0);
                if !show_zero && closing_debit == 0 && closing_credit == 0 {
                    continue;
                }
                rows.push(TrialBalanceRow {
                    account_code: account.code.clone(),
                    account_name: account.name.clone(),
                    group_name: account.group_name.clone(),
                    level: account.level,
                    opening_debit: opening_net.max(0),
                    opening_credit: checked_neg(
                        opening_net,
                        "مانده بستانکار افتتاحیه تراز آزمایشی",
                    )?
                    .max(0),
                    period_debit: pd,
                    period_credit: pc,
                    closing_debit,
                    closing_credit,
                });
            }

            // Totals come from the highest shown row of each branch; summing
            // every row would count children twice inside their parent.
            let shown: std::collections::HashSet<&str> =
                rows.iter().map(|row| row.account_code.as_str()).collect();
            let mut total_rows = Vec::new();
            for row in &rows {
                let mut parent = accounts[index_by_code[&row.account_code]]
                    .parent_code
                    .as_str();
                let mut has_shown_ancestor = false;
                let mut guard = std::collections::HashSet::new();
                while !parent.is_empty() && guard.insert(parent.to_string()) {
                    if shown.contains(parent) {
                        has_shown_ancestor = true;
                        break;
                    }
                    parent = index_by_code
                        .get(parent)
                        .map(|&index| accounts[index].parent_code.as_str())
                        .unwrap_or("");
                }
                if !has_shown_ancestor {
                    total_rows.push(row);
                }
            }

            let opening_debit = checked_sum(
                total_rows.iter().map(|r| r.opening_debit),
                "جمع بدهکار افتتاحیه تراز آزمایشی",
            )?;
            let opening_credit = checked_sum(
                total_rows.iter().map(|r| r.opening_credit),
                "جمع بستانکار افتتاحیه تراز آزمایشی",
            )?;
            let period_debit = checked_sum(
                total_rows.iter().map(|r| r.period_debit),
                "جمع بدهکار دوره تراز آزمایشی",
            )?;
            let period_credit = checked_sum(
                total_rows.iter().map(|r| r.period_credit),
                "جمع بستانکار دوره تراز آزمایشی",
            )?;
            let closing_debit = checked_sum(
                total_rows.iter().map(|r| r.closing_debit),
                "جمع بدهکار پایان دوره تراز آزمایشی",
            )?;
            let closing_credit = checked_sum(
                total_rows.iter().map(|r| r.closing_credit),
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
        let as_of = normalize_date(as_of, "تاریخ ترازنامه")?;
        self.with_conn(|conn| {
            // A final closing voucher without a later opening voucher zeroes
            // every balance-sheet account, so the statement is prepared from
            // the books before it. `account_activities` applies that rule.
            let activities = account_activities(conn, "", &as_of, true, &[])?;
            build_balance_sheet(as_of, activities)
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
    page: Option<(i64, i64)>,
) -> Result<Vec<VoucherSummary>, String> {
    let mut sql = format!(
        "SELECT v.id, v.number, v.date, v.updated_at, v.type, v.description,
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
    if page.is_some() {
        sql.push_str(" LIMIT ?5 OFFSET ?6");
    }
    let mut stmt = conn.prepare(&sql).map_err(db::db_error)?;
    let map_row = |row: &rusqlite::Row<'_>| {
        Ok(VoucherSummary {
            id: row.get(0)?,
            number: row.get(1)?,
            date: row.get(2)?,
            updated_at: row.get(3)?,
            voucher_type: row.get(4)?,
            description: row.get(5)?,
            total_debit: row.get(6)?,
            total_credit: row.get(7)?,
            line_count: row.get(8)?,
        })
    };

    let rows = if let Some((limit, offset)) = page {
        stmt.query_map(
            params![from, to, terms.folded, terms.raw, limit, offset],
            map_row,
        )
        .map_err(db::db_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db::db_error)?
    } else {
        stmt.query_map(params![from, to, terms.folded, terms.raw], map_row)
            .map_err(db::db_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db::db_error)?
    };
    Ok(rows)
}

/// Accepts the optional paging arguments sent by the UI. No limit means the
/// caller wants the whole result, which reports and export paths rely on.
fn page_window(
    offset: Option<i64>,
    limit: Option<i64>,
) -> Result<Option<(i64, i64)>, String> {
    let Some(limit) = limit else {
        if offset.unwrap_or(0) != 0 {
            return Err("برای صفحه‌بندی، اندازهٔ صفحه را هم مشخص کنید.".to_string());
        }
        return Ok(None);
    };
    let offset = offset.unwrap_or(0);
    if offset < 0 {
        return Err("موقعیت شروع صفحه نمی‌تواند منفی باشد.".to_string());
    }
    if limit <= 0 || limit > MAX_PAGE_SIZE {
        return Err(format!(
            "تعداد ردیف‌های هر صفحه باید بین ۱ و {} باشد.",
            db::to_fa_digits(MAX_PAGE_SIZE.to_string())
        ));
    }
    Ok(Some((limit, offset)))
}

fn voucher_list_totals(
    conn: &Connection,
    from: &str,
    to: &str,
    terms: &SearchTerms,
) -> Result<(i64, i64, i64), String> {
    let sql = format!(
        "SELECT COUNT(DISTINCT v.id),
                COALESCE(SUM(l.debit), 0),
                COALESCE(SUM(l.credit), 0)
           FROM vouchers v
           LEFT JOIN voucher_lines l ON l.voucher_id = v.id
          WHERE (?1 = '' OR v.date >= ?1)
            AND (?2 = '' OR v.date <= ?2)
            AND {}",
        search_predicate()
    );
    conn.query_row(
        &sql,
        params![from, to, terms.folded, terms.raw],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        },
    )
    .map_err(db::db_error)
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

fn assert_postable_leaf(conn: &Connection, account_code: &str) -> Result<String, String> {
    let account = conn
        .query_row(
            "SELECT name, level,
                    CASE
                        WHEN level > 3 THEN 0
                        WHEN level = 3 AND EXISTS(
                            SELECT 1 FROM accounts child
                             WHERE child.parent_code = accounts.code AND child.level > 3
                        ) AND NOT EXISTS(
                            SELECT 1 FROM accounts child
                             WHERE child.parent_code = accounts.code AND child.level <= 3
                        ) THEN COALESCE(postable_before_children, 1)
                        ELSE is_postable
                    END,
                    EXISTS(
                        SELECT 1 FROM accounts child
                         WHERE child.parent_code = accounts.code AND child.level <= 3
                    )
               FROM accounts WHERE code = ?1",
            params![account_code],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, bool>(3)?,
                ))
            },
        )
        .optional()
        .map_err(db::db_error)?
        .ok_or_else(|| format!("حساب با کد {} یافت نشد.", db::to_fa_digits(account_code)))?;
    if account.1 > db::MAX_ACCOUNT_LEVEL || account.2 == 0 || account.3 {
        return Err(format!(
            "حساب «{}» قابل ثبت نیست؛ فقط حساب‌های معین و تفصیلی ۱ که زیرحساب فعال ندارند قابل ثبت هستند. تفصیلی ۲ فقط برای سوابق قدیمی نگهداری می‌شود.",
            account.0
        ));
    }
    Ok(account.0)
}

fn assert_active_account(conn: &Connection, account_code: &str) -> Result<(), String> {
    let account = conn
        .query_row(
            "SELECT name, is_active FROM accounts WHERE code = ?1",
            params![account_code],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(db::db_error)?
        .ok_or_else(|| format!("حساب با کد {} یافت نشد.", db::to_fa_digits(account_code)))?;
    if account.1 == 0 {
        return Err(format!(
            "حساب «{}» غیرفعال است و برای ردیف جدید سند قابل انتخاب نیست.",
            account.0
        ));
    }
    Ok(())
}

fn assert_existing_legacy_account(conn: &Connection, account_code: &str) -> Result<String, String> {
    conn.query_row(
        "SELECT name FROM accounts WHERE code = ?1 AND level > ?2
          AND NOT EXISTS (SELECT 1 FROM accounts child WHERE child.parent_code = accounts.code)",
        params![account_code, db::MAX_ACCOUNT_LEVEL],
        |row| row.get(0),
    )
    .optional()
    .map_err(db::db_error)?
    .ok_or_else(|| format!(
        "حساب قدیمی {} دیگر قابل استفاده نیست؛ برای ثبت جدید، معین یا تفصیلی ۱ را انتخاب کنید.",
        db::to_fa_digits(account_code)
    ))
}

/// The single source of truth for checking accounts referenced by voucher rows.
/// Historical exceptions are bound to a stable row id, never merely to an
/// account code; this keeps save and Excel export behavior identical.
fn validate_voucher_line_accounts(
    conn: &Connection,
    voucher_id: Option<i64>,
    lines: &[VoucherLineInput],
) -> Result<(), String> {
    let saved_lines: BTreeMap<i64, String> = if let Some(id) = voucher_id.filter(|id| *id > 0) {
        let mut statement = conn
            .prepare("SELECT id, account_code FROM voucher_lines WHERE voucher_id = ?1")
            .map_err(db::db_error)?;
        let rows = statement
            .query_map(params![id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db::db_error)?
            .collect::<rusqlite::Result<BTreeMap<_, _>>>()
            .map_err(db::db_error)?;
        rows
    } else {
        BTreeMap::new()
    };

    let mut used_line_ids = HashSet::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        let account_code = db::normalize_code(&line.account_code);
        if account_code.is_empty() {
            return Err(format!(
                "کد حساب ردیف {} الزامی است.",
                db::to_fa_digits(index + 1)
            ));
        }

        let saved_line = match line.id {
            Some(line_id) => {
                if !used_line_ids.insert(line_id) {
                    return Err(format!(
                        "شناسه ردیف ذخیره‌شده در ردیف {} تکرار شده است؛ سند را دوباره بارگذاری کنید.",
                        db::to_fa_digits(index + 1)
                    ));
                }
                let saved_code = saved_lines.get(&line_id).ok_or_else(|| {
                    format!(
                        "ردیف {} به این سند تعلق ندارد؛ سند را دوباره بارگذاری کنید.",
                        db::to_fa_digits(index + 1)
                    )
                })?;
                // Changing the account makes this a new posting. The historical
                // exception remains valid only when both id and account match.
                (saved_code == &account_code).then_some(saved_code)
            }
            None => None,
        };

        if let Err(error) = assert_postable_leaf(conn, &account_code) {
            if saved_line.is_none() {
                return Err(error);
            }
            assert_existing_legacy_account(conn, &account_code)?;
        }
        if saved_line.is_none() {
            assert_active_account(conn, &account_code)?;
        }
    }
    Ok(())
}

/// A voucher type is not a free label: the closing/opening types decide which
/// reports count the voucher. Reject lines whose accounts contradict the type.
fn assert_voucher_type_matches_lines(
    conn: &Connection,
    voucher_type: &str,
    lines: &[VoucherLineInput],
) -> Result<(), String> {
    let (forbidden, description): (&[AccountClass], &str) =
        if voucher_type == CLOSING_TEMPORARY {
            (
                &[AccountClass::Asset, AccountClass::Liability],
                "حساب‌های دارایی یا بدهی",
            )
        } else if voucher_type == CLOSING_FINAL || voucher_type == VOUCHER_TYPES[1] {
            (
                &[AccountClass::Income, AccountClass::Expense],
                "حساب‌های درآمد یا هزینه",
            )
        } else {
            return Ok(());
        };

    for line in lines {
        let class = account_class_for_type_rule(conn, &line.account_code)?;
        if forbidden.contains(&class) {
            return Err(format!(
                "نوع سند «{voucher_type}» با ردیف حساب {} سازگار نیست؛ این نوع سند نباید شامل {} باشد.",
                db::to_fa_digits(&line.account_code),
                description
            ));
        }
    }
    Ok(())
}

fn account_class_for_type_rule(
    conn: &Connection,
    account_code: &str,
) -> Result<AccountClass, String> {
    let (group_code, group_type, nature) = conn
        .query_row(
            "SELECT g.code, g.type, a.nature
               FROM accounts a
               JOIN account_groups g ON g.code = a.group_code
              WHERE a.code = ?1",
            params![account_code],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(db::db_error)?;
    if group_code == "10" || group_code == "11" {
        return Ok(if nature == "creditor" {
            AccountClass::Income
        } else {
            AccountClass::Expense
        });
    }
    Ok(classify(&group_type, account_code))
}

fn fetch_voucher_by_id(conn: &Connection, id: i64) -> Result<Voucher, String> {
    let (number, date, updated_at, voucher_type, description) = conn
        .query_row(
            "SELECT number, date, updated_at, type, description FROM vouchers WHERE id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
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
        updated_at,
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
///
/// A final closing voucher is also hidden while no opening voucher follows
/// it up to the report date. That keeps a statement prepared at the closing
/// date on the books as they stood before the close. Once the next year's
/// opening voucher exists, both vouchers are included and cancel each other,
/// so later periods are not counted twice.
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
                     AND NOT (
                           v.type = ?5
                           AND NOT EXISTS (
                               SELECT 1
                                 FROM vouchers o
                                WHERE o.type = ?6
                                  AND (?2 = '' OR o.date <= ?2)
                                  AND (o.date > v.date
                                       OR (o.date = v.date AND o.id > v.id))
                           )
                     )
               ) ON l.account_code = a.code
              ORDER BY a.code",
        )
        .map_err(db::db_error)?;

    let raw = stmt
        .query_map(
            params![
                from_value,
                to_value,
                skip_a,
                skip_b,
                CLOSING_FINAL,
                VOUCHER_TYPES[1]
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                ))
            },
        )
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

fn classify_activity(activity: &AccountActivity, net: i64) -> AccountClass {
    if matches!(activity.group_code.as_str(), "10" | "11") {
        if net < 0 {
            AccountClass::Income
        } else {
            AccountClass::Expense
        }
    } else {
        classify(&activity.group_type, &activity.account_code)
    }
}

fn build_income_statement(
    from: String,
    to: String,
    activities: Vec<AccountActivity>,
) -> Result<IncomeStatementReport, String> {
    let mut revenues = Vec::new();
    let mut cogs = Vec::new();
    let mut other_operating_revenues = Vec::new();
    let mut other_operating_expenses = Vec::new();
    let mut expenses = Vec::new();
    let mut non_operating_revenues = Vec::new();
    let mut non_operating_expenses = Vec::new();

    for activity in activities {
        let net = checked_sub(
            activity.debit,
            activity.credit,
            "مانده حساب صورت سود و زیان",
        )?;
        if net == 0 {
            continue;
        }
        if activity.group_code == "10" || activity.group_code == "11" {
            let amount = if net < 0 {
                checked_neg(net, "مبلغ درآمد گروه‌های سایر")?
            } else {
                net
            };
            let row = AmountRow {
                account_code: activity.account_code,
                account_name: activity.account_name,
                amount,
            };
            match (activity.group_code.as_str(), net < 0) {
                ("10", true) => other_operating_revenues.push(row),
                ("10", false) => other_operating_expenses.push(row),
                ("11", true) => non_operating_revenues.push(row),
                ("11", false) => non_operating_expenses.push(row),
                _ => unreachable!(),
            }
            continue;
        }
        match classify_activity(&activity, net) {
            AccountClass::Income => {
                let amount = checked_neg(net, "مبلغ درآمد")?;
                if amount != 0 {
                    let row = AmountRow {
                        account_code: activity.account_code,
                        account_name: activity.account_name,
                        amount,
                    };
                    revenues.push(row);
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
                if activity.group_code == "9" {
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
    let other_operating_revenue_total = checked_sum(
        other_operating_revenues.iter().map(|r| r.amount),
        "جمع سایر درآمدهای عملیاتی",
    )?;
    let other_operating_expense_total = checked_sum(
        other_operating_expenses.iter().map(|r| r.amount),
        "جمع سایر هزینه‌های عملیاتی",
    )?;
    let expense_total = checked_sum(
        expenses.iter().map(|r| r.amount),
        "جمع هزینه صورت سود و زیان",
    )?;
    let non_operating_revenue_total = checked_sum(
        non_operating_revenues.iter().map(|r| r.amount),
        "جمع درآمدهای غیرعملیاتی",
    )?;
    let non_operating_expense_total = checked_sum(
        non_operating_expenses.iter().map(|r| r.amount),
        "جمع هزینه‌های غیرعملیاتی",
    )?;
    let gross_profit = checked_sub(revenue_total, cogs_total, "سود ناخالص")?;
    let operating_profit = checked_sub(
        checked_add(
            gross_profit,
            other_operating_revenue_total,
            "سود عملیاتی پس از سایر درآمدها",
        )?,
        checked_add(
            expense_total,
            other_operating_expense_total,
            "هزینه‌های عملیاتی",
        )?,
        "سود عملیاتی",
    )?;
    let net_profit = checked_sub(
        checked_add(
            operating_profit,
            non_operating_revenue_total,
            "سود پیش از هزینه‌های غیرعملیاتی",
        )?,
        non_operating_expense_total,
        "سود خالص",
    )?;
    Ok(IncomeStatementReport {
        from,
        to,
        revenues,
        revenue_total,
        cogs,
        cogs_total,
        other_operating_revenues,
        other_operating_revenue_total,
        other_operating_expenses,
        other_operating_expense_total,
        expenses,
        expense_total,
        non_operating_revenues,
        non_operating_revenue_total,
        non_operating_expenses,
        non_operating_expense_total,
        gross_profit,
        operating_profit,
        net_profit,
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
        match classify_activity(&activity, net) {
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
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
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
            id: None,
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
    fn serializes_large_amounts_as_exact_decimal_strings() {
        #[derive(Serialize)]
        struct AmountValue {
            #[serde(serialize_with = "serialize_exact_amount")]
            amount: i64,
        }

        assert_eq!(
            serde_json::to_string(&AmountValue {
                amount: 9_007_199_254_740_991,
            })
            .unwrap(),
            "{\"amount\":9007199254740991}"
        );
        assert_eq!(
            serde_json::to_string(&AmountValue {
                amount: 9_007_199_254_740_992,
            })
            .unwrap(),
            "{\"amount\":\"9007199254740992\"}"
        );
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
    fn voucher_list_separates_book_count_from_filtered_count() {
        let db = db::memory_database();
        db.save_voucher(cash_sale("1405/01/10", 1_000)).unwrap();
        db.save_voucher(cash_sale("1405/02/10", 2_000)).unwrap();

        let by_date = db.list_vouchers("۱۴۰۵/۰۲/۰۱", "۱۴۰۵/۰۲/۳۱", "").unwrap();
        assert_eq!(by_date.vouchers.len(), 1);
        assert_eq!(by_date.total_count, 2);
        assert_eq!(by_date.matched_count, 1);

        let by_search = db.list_vouchers("", "", "۲").unwrap();
        assert_eq!(by_search.vouchers.len(), 1);
        assert_eq!(by_search.vouchers[0].number, 2);
        assert_eq!(by_search.total_count, 2);
        assert_eq!(by_search.matched_count, 1);
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
    fn refuses_posting_to_parent_even_if_legacy_flag_allows_it() {
        let db = db::memory_database();
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE accounts SET is_postable = 1 WHERE code = '110201'",
                [],
            )
            .map_err(db::db_error)?;
            Ok(())
        })
        .unwrap();

        let result = db.save_voucher(voucher(
            "1405/01/10",
            vec![line("110201", 1_000, 0), line("410101", 0, 1_000)],
        ));

        assert!(result.unwrap_err().contains("قابل ثبت نیست"));
    }

    #[test]
    fn refuses_transferring_legacy_rows_posted_to_parent_accounts() {
        let db = db::memory_database();
        let saved = db.save_voucher(cash_sale("1405/01/10", 1_000)).unwrap();
        let first_line_id = saved.lines[0].id;
        let line_ids = saved.lines.iter().map(|line| line.id).collect();
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE accounts SET is_postable = 1 WHERE code = '110201'",
                [],
            )
            .map_err(db::db_error)?;
            conn.execute(
                "UPDATE voucher_lines SET account_code = '110201' WHERE id = ?1",
                params![first_line_id],
            )
            .map_err(db::db_error)?;
            Ok(())
        })
        .unwrap();

        let error = db
            .transfer_voucher_lines(VoucherTransferInput {
                source_voucher_id: saved.id,
                source_voucher_number: saved.number,
                target_voucher_number: 2,
                line_ids,
                mode: "copy".to_string(),
            })
            .unwrap_err();

        assert!(error.contains("قابل ثبت نیست"));
    }

    #[test]
    fn transfer_cannot_put_income_lines_into_a_final_closing_voucher() {
        let db = db::memory_database();
        let source = db.save_voucher(cash_sale("1405/01/10", 1_000)).unwrap();
        let mut target = voucher(
            "1405/01/10",
            vec![line("310101", 1_000, 0), line("110101", 0, 1_000)],
        );
        target.number = Some(2);
        target.voucher_type = CLOSING_FINAL.to_string();
        db.save_voucher(target).unwrap();

        let error = db
            .transfer_voucher_lines(VoucherTransferInput {
                source_voucher_id: source.id,
                source_voucher_number: source.number,
                target_voucher_number: 2,
                line_ids: source.lines.iter().map(|line| line.id).collect(),
                mode: "copy".to_string(),
            })
            .unwrap_err();

        assert!(error.contains("سازگار نیست"));
        assert_eq!(db.get_voucher(2).unwrap().unwrap().lines.len(), 2);
    }

    #[test]
    fn transfer_rejects_target_numbers_above_the_safe_limit() {
        let db = db::memory_database();
        let error = db
            .transfer_voucher_lines(VoucherTransferInput {
                source_voucher_id: 1,
                source_voucher_number: 1,
                target_voucher_number: MAX_VOUCHER_NUMBER + 1,
                line_ids: Vec::new(),
                mode: "copy".to_string(),
            })
            .unwrap_err();

        assert!(error.contains("شماره سند مقصد"));
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
    }

    #[test]
    fn non_operating_accounts_change_net_profit_not_operating_profit() {
        let db = db::memory_database();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO accounts (code, name, level, group_code, nature, is_postable)
                   VALUES ('1305', 'درآمد عملیاتی آزمایشی', 1, '10', 'dual', 1),
                       ('1405', 'هزینه عملیاتی آزمایشی', 1, '10', 'dual', 1),
                       ('1105', 'درآمد غیرعملیاتی آزمایشی', 1, '11', 'dual', 1),
                       ('1205', 'هزینه غیرعملیاتی آزمایشی', 1, '11', 'dual', 1)",
                [],
            )
            .map_err(db::db_error)?;
            Ok(())
        })
        .unwrap();

        db.save_voucher(voucher(
            "1405/01/10",
            vec![line("110101", 1_000, 0), line("410101", 0, 1_000)],
        ))
        .unwrap();
        db.save_voucher(voucher(
            "1405/01/11",
            vec![line("610101", 200, 0), line("110101", 0, 200)],
        ))
        .unwrap();
        db.save_voucher(voucher(
            "1405/01/12",
            vec![line("110101", 40, 0), line("1305", 0, 40)],
        ))
        .unwrap();
        db.save_voucher(voucher(
            "1405/01/13",
            vec![line("1405", 10, 0), line("110101", 0, 10)],
        ))
        .unwrap();
        db.save_voucher(voucher(
            "1405/01/14",
            vec![line("110101", 100, 0), line("1105", 0, 100)],
        ))
        .unwrap();
        db.save_voucher(voucher(
            "1405/01/15",
            vec![line("1205", 50, 0), line("110101", 0, 50)],
        ))
        .unwrap();

        let report = db.get_income_statement("1405/01/01", "1405/01/31").unwrap();
        assert_eq!(report.revenue_total, 1_000);
        assert_eq!(report.gross_profit, 1_000);
        assert_eq!(report.other_operating_revenue_total, 40);
        assert_eq!(report.other_operating_expense_total, 10);
        assert_eq!(report.operating_profit, 830);
        assert_eq!(report.non_operating_revenue_total, 100);
        assert_eq!(report.non_operating_expense_total, 50);
        assert_eq!(report.net_profit, 880);

        let balance = db.get_balance_sheet("1405/01/31").unwrap();
        assert_eq!(balance.assets_total, 880);
        assert_eq!(balance.profit, report.net_profit);
        assert!(balance.balanced);
    }

    #[test]
    fn legacy_detail2_is_not_selectable_but_its_detail1_parent_remains_postable() {
        let db = db::memory_database();
        db.create_account("991", "کل آزمایشی", 1, "1", "debtor", "", false)
            .unwrap();
        db.create_account("99101", "معین آزمایشی", 2, "1", "debtor", "991", true)
            .unwrap();
        let detail1 = db
            .create_account("9910101", "تفصیلی ۱ آزمایشی", 3, "1", "debtor", "99101", true)
            .unwrap();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO accounts (code, name, level, group_code, nature, parent_code, parent_name, is_postable) VALUES ('991010101', 'تفصیلی ۲ قدیمی', 4, '1', 'debtor', '9910101', 'تفصیلی ۱ آزمایشی', 1)",
                [],
            )
            .map_err(db::db_error)?;
            conn.execute(
                "UPDATE accounts SET is_postable = 0, postable_before_children = 1 WHERE code = '9910101'",
                [],
            )
            .map_err(db::db_error)?;
            Ok(())
        }).unwrap();

        let accounts = db.list_accounts().unwrap().accounts;
        assert!(accounts.iter().find(|a| a.code == detail1.code).unwrap().is_postable);
        assert!(!accounts.iter().find(|a| a.code == "991010101").unwrap().is_postable);

        let saved = db.save_voucher(voucher(
            "1405/01/01",
            vec![line("9910101", 100, 0), line("110101", 0, 100)],
        )).unwrap();
        assert_eq!(saved.lines[0].account_code, "9910101");
        let rejected = db.save_voucher(voucher(
            "1405/01/02",
            vec![line("991010101", 100, 0), line("110101", 0, 100)],
        ));
        assert!(rejected.is_err());
    }

    #[test]
    fn an_existing_voucher_can_be_saved_without_replacing_its_legacy_detail2_code() {
        let db = db::memory_database();
        db.create_account("992", "کل آزمایشی", 1, "1", "debtor", "", false)
            .unwrap();
        db.create_account("99201", "معین آزمایشی", 2, "1", "debtor", "992", true)
            .unwrap();
        db.create_account("9920101", "تفصیلی ۱ آزمایشی", 3, "1", "debtor", "99201", true)
            .unwrap();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO accounts (code, name, level, group_code, nature, parent_code, parent_name, is_postable) VALUES ('992010101', 'تفصیلی ۲ قدیمی', 4, '1', 'debtor', '9920101', 'تفصیلی ۱ آزمایشی', 1)",
                [],
            ).map_err(db::db_error)?;
            conn.execute(
                "INSERT INTO vouchers (number, date, type, description) VALUES (777, '1405/01/01', ?1, 'سند قدیمی')",
                params![VOUCHER_TYPES[0]],
            ).map_err(db::db_error)?;
            let voucher_id = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO voucher_lines (voucher_id, row_no, account_code, description, debit, credit) VALUES (?1, 1, '992010101', '', 100, 0)",
                params![voucher_id],
            ).map_err(db::db_error)?;
            conn.execute(
                "INSERT INTO voucher_lines (voucher_id, row_no, account_code, description, debit, credit) VALUES (?1, 2, '110101', '', 0, 100)",
                params![voucher_id],
            ).map_err(db::db_error)?;
            Ok(())
        }).unwrap();
        let old = db.get_voucher(777).unwrap().unwrap();
        let input = VoucherInput {
            id: Some(old.id),
            number: Some(old.number),
            date: old.date.clone(),
            voucher_type: old.voucher_type.clone(),
            description: old.description.clone(),
            lines: old.lines.iter().map(|line| VoucherLineInput {
                id: Some(line.id),
                account_code: line.account_code.clone(),
                description: line.description.clone(),
                debit: line.debit,
                credit: line.credit,
            }).collect(),
        };
        let saved = db.save_voucher_if_unchanged(input, Some(&old.updated_at)).unwrap();
        assert_eq!(saved.lines[0].account_code, "992010101");
    }

    #[test]
    fn saves_voucher_to_a_new_detail1_account() {
        let db = db::memory_database();
        db.create_account("991", "کل آزمایشی", 1, "1", "debtor", "", false)
            .unwrap();
        db.create_account("99101", "معین آزمایشی", 2, "1", "debtor", "991", true)
            .unwrap();
        let detail1 = db
            .create_account("9910101", "تفصیلی ۱ آزمایشی", 3, "1", "debtor", "99101", true)
            .unwrap();
        assert_eq!(detail1.level, 3);
        assert!(detail1.is_postable);

        let saved = db
            .save_voucher(voucher(
                "1405/01/01",
                vec![line("9910101", 100, 0), line("110101", 0, 100)],
            ))
            .unwrap();
        assert_eq!(saved.lines[0].account_code, "9910101");
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
                    nature_is_explicit: false,
                },
                crate::excel::ImportedAccount {
                    code: subsidiary_code.to_string(),
                    name: "حساب معین آزمایشی".to_string(),
                    parent_code: general_code.to_string(),
                    group_code: "1".to_string(),
                    level: 2,
                    nature: "debtor".to_string(),
                    nature_is_explicit: false,
                },
            ]
        };
        let make_import = |number, general_code: &str, subsidiary_code: &str, lines| {
            crate::excel::BulkVoucherImport {
                file_name: "آزمایشی.xlsx".to_string(),
                sheet_name: "اسناد".to_string(),
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
    fn bulk_import_supports_detail1_as_a_postable_leaf() {
        let db = db::memory_database();
        let import = crate::excel::BulkVoucherImport {
            file_name: "detail1.xlsx".to_string(),
            sheet_name: "اسناد".to_string(),
            accounts: vec![
                crate::excel::ImportedAccount {
                    code: "991".to_string(),
                    name: "گروه آزمایشی کل".to_string(),
                    parent_code: String::new(),
                    group_code: "1".to_string(),
                    level: 1,
                    nature: "debtor".to_string(),
                    nature_is_explicit: false,
                },
                crate::excel::ImportedAccount {
                    code: "99101".to_string(),
                    name: "حساب معین آزمایشی".to_string(),
                    parent_code: "991".to_string(),
                    group_code: "1".to_string(),
                    level: 2,
                    nature: "debtor".to_string(),
                    nature_is_explicit: false,
                },
                crate::excel::ImportedAccount {
                    code: "9910101".to_string(),
                    name: "حساب تفصیلی ۱ آزمایشی".to_string(),
                    parent_code: "99101".to_string(),
                    group_code: "1".to_string(),
                    level: 3,
                    nature: "debtor".to_string(),
                    nature_is_explicit: false,
                },
            ],
            vouchers: vec![VoucherInput {
                id: None,
                number: Some(1),
                date: "1405/01/01".to_string(),
                voucher_type: VOUCHER_TYPES[0].to_string(),
                description: String::new(),
                lines: vec![line("9910101", 250, 0), line("110101", 0, 250)],
            }],
        };

        assert_eq!(db.import_vouchers(import, false).unwrap(), 1);
        let accounts = db.list_accounts().unwrap().accounts;
        let detail1 = accounts.iter().find(|account| account.code == "9910101").unwrap();
        let subsidiary = accounts.iter().find(|account| account.code == "99101").unwrap();
        assert_eq!(detail1.level, 3);
        assert!(detail1.is_postable);
        assert!(!subsidiary.is_postable);
        let saved = db.get_voucher(1).unwrap().unwrap();
        assert_eq!(saved.lines[0].account_code, "9910101");
    }

    #[test]
    fn bulk_import_rejects_inactive_existing_parent_accounts() {
        let db = db::memory_database();
        db.with_conn(|conn| {
            conn.execute("UPDATE accounts SET is_active = 0 WHERE code = '1101'", [])
                .map_err(db::db_error)?;
            Ok(())
        })
        .unwrap();

        let import = crate::excel::BulkVoucherImport {
            file_name: "inactive-parent.xlsx".to_string(),
            sheet_name: "اسناد".to_string(),
            accounts: vec![
                crate::excel::ImportedAccount {
                    code: "1101".to_string(),
                    name: "صندوق".to_string(),
                    parent_code: String::new(),
                    group_code: "1".to_string(),
                    level: 1,
                    nature: "debtor".to_string(),
                    nature_is_explicit: false,
                },
                crate::excel::ImportedAccount {
                    code: "110199".to_string(),
                    name: "صندوق جدید".to_string(),
                    parent_code: "1101".to_string(),
                    group_code: "1".to_string(),
                    level: 2,
                    nature: "debtor".to_string(),
                    nature_is_explicit: false,
                },
            ],
            vouchers: vec![VoucherInput {
                id: None,
                number: Some(1),
                date: "1405/01/01".to_string(),
                voucher_type: VOUCHER_TYPES[0].to_string(),
                description: String::new(),
                lines: vec![line("110199", 100, 0), line("410101", 0, 100)],
            }],
        };

        let preview_error = db.preview_bulk_import(&import).unwrap_err();
        assert!(preview_error.contains("غیرفعال"));
        let error = db.import_vouchers(import, false).unwrap_err();
        assert!(error.contains("غیرفعال"));
        assert!(db.get_voucher(1).unwrap().is_none());
        assert!(!db
            .list_accounts()
            .unwrap()
            .accounts
            .iter()
            .any(|account| account.code == "110199"));
    }

    #[test]
    fn bulk_import_preview_rejects_posted_account_with_imported_child() {
        let db = db::memory_database();
        let import = crate::excel::BulkVoucherImport {
            file_name: "mixed-level.xlsx".to_string(),
            sheet_name: "اسناد".to_string(),
            accounts: vec![
                crate::excel::ImportedAccount {
                    code: "991".to_string(),
                    name: "گروه آزمایشی کل".to_string(),
                    parent_code: String::new(),
                    group_code: "1".to_string(),
                    level: 1,
                    nature: "debtor".to_string(),
                    nature_is_explicit: false,
                },
                crate::excel::ImportedAccount {
                    code: "99101".to_string(),
                    name: "حساب معین آزمایشی".to_string(),
                    parent_code: "991".to_string(),
                    group_code: "1".to_string(),
                    level: 2,
                    nature: "debtor".to_string(),
                    nature_is_explicit: false,
                },
                crate::excel::ImportedAccount {
                    code: "9910101".to_string(),
                    name: "تفصیلی آزمایشی".to_string(),
                    parent_code: "99101".to_string(),
                    group_code: "1".to_string(),
                    level: 3,
                    nature: "debtor".to_string(),
                    nature_is_explicit: false,
                },
            ],
            vouchers: vec![VoucherInput {
                id: None,
                number: Some(1),
                date: "1405/01/01".to_string(),
                voucher_type: VOUCHER_TYPES[0].to_string(),
                description: String::new(),
                lines: vec![line("99101", 100, 0), line("410101", 0, 100)],
            }],
        };

        let error = db.preview_bulk_import(&import).unwrap_err();
        assert!(error.contains("زیرحساب"));
        assert!(error.contains("99101"));
    }

    #[test]
    fn bulk_import_preview_requires_nature_for_new_other_group_children() {
        let db = db::memory_database();
        let import = crate::excel::BulkVoucherImport {
            file_name: "other-group.xlsx".to_string(),
            sheet_name: "اسناد".to_string(),
            accounts: vec![
                crate::excel::ImportedAccount {
                    code: "7201".to_string(),
                    name: "سایر درآمدها و هزینه‌های عملیاتی جدید".to_string(),
                    parent_code: String::new(),
                    group_code: "10".to_string(),
                    level: 1,
                    nature: "dual".to_string(),
                    nature_is_explicit: false,
                },
                crate::excel::ImportedAccount {
                    code: "720101".to_string(),
                    name: "حساب فرعی جدید".to_string(),
                    parent_code: "7201".to_string(),
                    group_code: "10".to_string(),
                    level: 2,
                    nature: "unknown".to_string(),
                    nature_is_explicit: false,
                },
            ],
            vouchers: vec![VoucherInput {
                id: None,
                number: Some(1),
                date: "1405/01/01".to_string(),
                voucher_type: VOUCHER_TYPES[0].to_string(),
                description: String::new(),
                lines: vec![line("720101", 100, 0), line("410101", 0, 100)],
            }],
        };

        let error = db.preview_bulk_import(&import).unwrap_err();
        assert!(error.contains("ماهیت"));
    }

    #[test]
    fn bulk_import_rejects_legacy_postable_parent_accounts() {
        let db = db::memory_database();
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE accounts SET is_postable = 1 WHERE code = '1102'",
                [],
            )
            .map_err(db::db_error)?;
            Ok(())
        })
        .unwrap();

        let import = crate::excel::BulkVoucherImport {
            file_name: "legacy.xlsx".to_string(),
            sheet_name: "اسناد".to_string(),
            accounts: Vec::new(),
            vouchers: vec![VoucherInput {
                id: None,
                number: Some(1),
                date: "1405/01/01".to_string(),
                voucher_type: VOUCHER_TYPES[0].to_string(),
                description: String::new(),
                lines: vec![line("1102", 100, 0), line("410101", 0, 100)],
            }],
        };

        let error = db.import_vouchers(import, false).unwrap_err();
        assert!(error.contains("قابل ثبت"));
        assert!(db.get_voucher(1).unwrap().is_none());
    }

    #[test]
    fn reorganize_preview_merges_then_sorts_by_date() {
        let db = db::memory_database();
        db.save_voucher(cash_sale("1405/01/02", 1_000)).unwrap();
        db.save_voucher(cash_sale("1405/01/01", 2_000)).unwrap();
        let mut source = cash_sale("1405/01/02", 500);
        source.description = "شرح سند مبدأ".to_string();
        db.save_voucher(source).unwrap();
        let mut opening = voucher(
            "1405/01/02",
            vec![line("110101", 750, 0), line("310101", 0, 750)],
        );
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

    fn bulk_delete_input(scope: &str) -> BulkVoucherDeleteInput {
        BulkVoucherDeleteInput {
            scope: scope.to_string(),
            from_date: String::new(),
            to_date: String::new(),
            from_number: None,
            to_number: None,
        }
    }

    #[test]
    fn bulk_delete_previews_and_deletes_a_date_range() {
        let db = db::memory_database();
        db.save_voucher(cash_sale("1405/01/05", 1_000)).unwrap();
        db.save_voucher(cash_sale("1405/02/05", 2_000)).unwrap();
        db.save_voucher(cash_sale("1405/03/05", 3_000)).unwrap();

        let input = BulkVoucherDeleteInput {
            from_date: "۱۴۰۵/۰۲/۰۱".to_string(),
            to_date: "1405/02/31".to_string(),
            ..bulk_delete_input("dateRange")
        };
        let preview = db.preview_bulk_voucher_delete(&input).unwrap();
        assert_eq!(preview.scope, "dateRange");
        assert_eq!(preview.total_matched, 1);
        assert_eq!(preview.deletable_count, 1);
        assert_eq!(preview.locked_count, 0);
        assert_eq!(preview.deletable_sample[0].voucher_number, 2);
        assert_eq!(preview.date_range.as_ref().unwrap().from, "1405/02/01");

        let result = db
            .apply_bulk_voucher_delete(&input, &preview.plan_token, false)
            .unwrap();
        assert_eq!(result.deleted_count, 1);
        assert_eq!(result.locked_count, 0);
        assert!(result.message.contains("۱ سند"));
        assert_eq!(db.list_vouchers("", "", "").unwrap().total_count, 2);
        assert!(db.get_voucher(2).unwrap().is_none());
        let remaining_lines: i64 = db
            .with_conn(|conn| {
                conn.query_row("SELECT COUNT(*) FROM voucher_lines", [], |row| row.get(0))
                    .map_err(db::db_error)
            })
            .unwrap();
        assert_eq!(remaining_lines, 4);
    }

    #[test]
    fn bulk_delete_handles_number_range_and_all_scopes() {
        let db = db::memory_database();
        db.save_voucher(cash_sale("1405/01/05", 1_000)).unwrap();
        db.save_voucher(cash_sale("1405/01/06", 2_000)).unwrap();
        db.save_voucher(cash_sale("1405/01/07", 3_000)).unwrap();

        let number_input = BulkVoucherDeleteInput {
            from_number: Some(2),
            to_number: Some(3),
            ..bulk_delete_input("numberRange")
        };
        let number_preview = db.preview_bulk_voucher_delete(&number_input).unwrap();
        assert_eq!(number_preview.total_matched, 2);
        assert_eq!(number_preview.number_range.as_ref().unwrap().from, 2);
        db.apply_bulk_voucher_delete(&number_input, &number_preview.plan_token, false)
            .unwrap();
        assert_eq!(db.list_vouchers("", "", "").unwrap().total_count, 1);

        let all_input = bulk_delete_input("all");
        let all_preview = db.preview_bulk_voucher_delete(&all_input).unwrap();
        assert_eq!(all_preview.total_matched, 1);
        let result = db
            .apply_bulk_voucher_delete(&all_input, &all_preview.plan_token, false)
            .unwrap();
        assert_eq!(result.deleted_count, 1);
        assert_eq!(db.list_vouchers("", "", "").unwrap().total_count, 0);
    }

    #[test]
    fn bulk_delete_rejects_a_stale_preview() {
        let db = db::memory_database();
        db.save_voucher(cash_sale("1405/01/05", 1_000)).unwrap();
        let preview = db
            .preview_bulk_voucher_delete(&bulk_delete_input("all"))
            .unwrap();
        db.save_voucher(cash_sale("1405/01/06", 2_000)).unwrap();

        assert!(db
            .apply_bulk_voucher_delete(&bulk_delete_input("all"), &preview.plan_token, false)
            .unwrap_err()
            .contains("تغییر کرده‌اند"));
        assert_eq!(db.list_vouchers("", "", "").unwrap().total_count, 2);
    }

    #[test]
    fn bulk_delete_validates_ranges_and_scope() {
        let db = db::memory_database();

        let reversed_dates = BulkVoucherDeleteInput {
            from_date: "1405/02/01".to_string(),
            to_date: "1405/01/01".to_string(),
            ..bulk_delete_input("dateRange")
        };
        assert!(db
            .preview_bulk_voucher_delete(&reversed_dates)
            .unwrap_err()
            .contains("بعد از تاریخ پایان"));

        let reversed_numbers = BulkVoucherDeleteInput {
            from_number: Some(10),
            to_number: Some(2),
            ..bulk_delete_input("numberRange")
        };
        assert!(db
            .preview_bulk_voucher_delete(&reversed_numbers)
            .unwrap_err()
            .contains("بزرگ‌تر"));

        assert!(db
            .preview_bulk_voucher_delete(&bulk_delete_input("unknown"))
            .unwrap_err()
            .contains("معتبر نیست"));
    }

    #[test]
    fn bulk_delete_backup_failure_keeps_vouchers() {
        let db = db::memory_database();
        db.save_voucher(cash_sale("1405/01/05", 1_000)).unwrap();
        let input = bulk_delete_input("all");
        let preview = db.preview_bulk_voucher_delete(&input).unwrap();
        assert_eq!(preview.backup_directory, "");

        let error = db
            .apply_bulk_voucher_delete(&input, &preview.plan_token, true)
            .unwrap_err();
        assert!(error.contains("پشتیبان"));
        assert_eq!(db.list_vouchers("", "", "").unwrap().total_count, 1);
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
                lines: vec![line("110101", 800_000, 0), line("310101", 0, 800_000)],
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
    fn stale_voucher_version_cannot_overwrite_a_newer_edit() {
        let db = db::memory_database();
        let saved = db.save_voucher(cash_sale("1405/01/10", 500_000)).unwrap();
        let mut first_edit = cash_sale("1405/01/11", 600_000);
        first_edit.id = Some(saved.id);
        let updated = db
            .save_voucher_if_unchanged(first_edit, Some(&saved.updated_at))
            .unwrap();

        let mut stale_edit = cash_sale("1405/01/12", 700_000);
        stale_edit.id = Some(saved.id);
        let error = db
            .save_voucher_if_unchanged(stale_edit, Some(&saved.updated_at))
            .unwrap_err();

        assert!(error.contains("پس از باز شدن تغییر کرده"));
        assert_eq!(
            db.get_voucher(saved.number).unwrap().unwrap().date,
            updated.date
        );
    }

    #[test]
    fn deletes_voucher_and_its_lines() {
        let db = db::memory_database();
        let saved = db.save_voucher(cash_sale("1405/01/10", 300_000)).unwrap();
        assert_eq!(
            db.get_voucher(saved.number).unwrap().unwrap().lines.len(),
            2
        );

        db.delete_voucher(saved.id, Some(&saved.updated_at)).unwrap();
        assert!(db.get_voucher(saved.number).unwrap().is_none());
        assert!(db.list_vouchers("", "", "").unwrap().vouchers.is_empty());
        assert!(db.get_journal("", "", "").unwrap().rows.is_empty());
        assert!(db.delete_voucher(saved.id, Some(&saved.updated_at)).is_err());
    }

    #[test]
    fn delete_rejects_a_stale_voucher_version() {
        let db = db::memory_database();
        let saved = db.save_voucher(cash_sale("1405/01/10", 300_000)).unwrap();

        let error = db
            .delete_voucher(saved.id, Some("2000-01-01 00:00:00.000"))
            .unwrap_err();
        assert!(error.contains("تغییر کرده"));
        assert!(db.get_voucher(saved.number).unwrap().is_some());

        db.delete_voucher(saved.id, Some(&saved.updated_at)).unwrap();
        assert!(db.get_voucher(saved.number).unwrap().is_none());
    }

    #[test]
    fn rejects_a_voucher_type_that_contradicts_its_accounts() {
        let db = db::memory_database();

        let mut opening = cash_sale("1405/01/10", 100_000);
        opening.voucher_type = VOUCHER_TYPES[1].to_string();
        let error = db.save_voucher(opening).unwrap_err();
        assert!(error.contains("سازگار نیست"));

        let mut closing = voucher(
            "1405/01/10",
            vec![line("110101", 100_000, 0), line("310101", 0, 100_000)],
        );
        closing.voucher_type = CLOSING_TEMPORARY.to_string();
        let error = db.save_voucher(closing).unwrap_err();
        assert!(error.contains("سازگار نیست"));
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
    fn trial_balance_rolls_up_level_one_and_filters_code_range() {
        let db = db::memory_database();
        db.save_voucher(cash_sale("1405/01/05", 500_000)).unwrap();
        db.save_voucher(cash_sale("1405/01/06", 300_000)).unwrap();

        // Default view: only level-1 rows, each showing its whole branch.
        let level_one = db
            .get_trial_balance_filtered("1405/01/01", "1405/01/31", "", "", false, false)
            .unwrap();
        assert!(level_one.rows.iter().all(|row| row.level == 1));
        let cash_parent = level_one
            .rows
            .iter()
            .find(|row| row.account_code == "1101")
            .unwrap();
        assert_eq!(cash_parent.period_debit, 800_000);
        assert_eq!(cash_parent.closing_debit, 800_000);
        let revenue_parent = level_one
            .rows
            .iter()
            .find(|row| row.account_code == "4101")
            .unwrap();
        assert_eq!(revenue_parent.period_credit, 800_000);
        assert_eq!(level_one.period_debit, level_one.period_credit);

        // A code range searches every level, but a shown parent keeps its
        // children out of the totals so nothing is counted twice.
        let range = db
            .get_trial_balance_filtered("1405/01/01", "1405/01/31", "1101", "1101", false, false)
            .unwrap();
        assert!(range
            .rows
            .iter()
            .all(|row| row.account_code.starts_with("1101")));
        assert_eq!(range.rows.len(), 2);
        assert_eq!(range.period_debit, 800_000);
        assert_eq!(range.period_credit, 0);
    }

    #[test]
    fn voucher_save_rejects_inactive_accounts_but_keeps_saved_lines_editable() {
        let db = db::memory_database();
        let inactive = db
            .create_account_active(
                "110202001",
                "بانک غیرفعال",
                3,
                "1",
                "debtor",
                "110202",
                true,
                false,
            )
            .unwrap();
        let error = db
            .save_voucher(voucher(
                "1405/01/05",
                vec![line(&inactive.code, 1_000, 0), line("410101", 0, 1_000)],
            ))
            .unwrap_err();
        assert!(error.contains("غیرفعال"));

        // A line saved before its account was deactivated stays editable.
        let saved = db.save_voucher(cash_sale("1405/01/05", 500_000)).unwrap();
        let cash = db
            .list_accounts()
            .unwrap()
            .accounts
            .into_iter()
            .find(|account| account.code == "110101")
            .unwrap();
        db.set_account_active(cash.id, false).unwrap();
        let mut edited = cash_sale("1405/01/05", 500_000);
        edited.id = Some(saved.id);
        // The stable id of each saved row is what preserves the exception; a
        // line without an id is treated as a new posting.
        for line in &mut edited.lines {
            line.id = saved
                .lines
                .iter()
                .find(|saved_line| saved_line.account_code == line.account_code)
                .map(|saved_line| saved_line.id);
        }
        db.save_voucher(edited).unwrap();

        // Adding another row with the same inactive account must still fail.
        // Re-read the voucher because saving replaced its line ids.
        let saved = db.get_voucher(saved.number).unwrap().unwrap();
        let mut added = cash_sale("1405/01/05", 500_000);
        added.id = Some(saved.id);
        for line in &mut added.lines {
            line.id = saved
                .lines
                .iter()
                .find(|saved_line| saved_line.account_code == line.account_code)
                .map(|saved_line| saved_line.id);
        }
        added.lines.push(line(&cash.code, 1_000, 0));
        added.lines.push(line("410101", 0, 1_000));
        let error = db.save_voucher(added).unwrap_err();
        assert!(error.contains("غیرفعال"), "{error}");
    }

    #[test]
    fn saved_line_ids_cannot_be_reused_and_export_checks_new_account_postings() {
        let db = db::memory_database();
        let saved = db.save_voucher(cash_sale("1405/01/05", 500_000)).unwrap();
        let cash = db
            .list_accounts()
            .unwrap()
            .accounts
            .into_iter()
            .find(|account| account.code == "110101")
            .unwrap();
        db.set_account_active(cash.id, false).unwrap();

        // Reusing one saved row id twice used to copy its historical inactive
        // account exception to an additional posting.
        let mut duplicated = cash_sale("1405/01/05", 600_000);
        duplicated.id = Some(saved.id);
        for row in &mut duplicated.lines {
            row.id = saved
                .lines
                .iter()
                .find(|saved_row| saved_row.account_code == row.account_code)
                .map(|saved_row| saved_row.id);
        }
        let mut extra_cash = duplicated
            .lines
            .iter()
            .find(|row| row.account_code == cash.code)
            .unwrap()
            .clone();
        extra_cash.debit = 100_000;
        duplicated
            .lines
            .iter_mut()
            .find(|row| row.account_code == "410101")
            .unwrap()
            .credit = 600_000;
        duplicated.lines.push(extra_cash);

        let error = db.save_voucher(duplicated.clone()).unwrap_err();
        assert!(error.contains("تکرار"), "{error}");
        let export_error = db
            .validate_voucher_accounts(Some(saved.id), &duplicated.lines)
            .unwrap_err();
        assert!(export_error.contains("تکرار"), "{export_error}");

        // Even when original rows retain their ids, an extra row cannot post
        // to the inactive account merely because that code exists in the voucher.
        let mut new_posting = cash_sale("1405/01/05", 500_000);
        new_posting.id = Some(saved.id);
        for row in &mut new_posting.lines {
            row.id = saved
                .lines
                .iter()
                .find(|saved_row| saved_row.account_code == row.account_code)
                .map(|saved_row| saved_row.id);
        }
        let mut new_cash = line(&cash.code, 100_000, 0);
        new_cash.id = None;
        new_posting
            .lines
            .iter_mut()
            .find(|row| row.account_code == "410101")
            .unwrap()
            .credit = 600_000;
        new_posting.lines.push(new_cash);
        let error = db
            .validate_voucher_accounts(Some(saved.id), &new_posting.lines)
            .unwrap_err();
        assert!(error.contains("غیرفعال"), "{error}");

        // A non-positive voucher id must not silently create a new document.
        let mut invalid_id = cash_sale("1405/01/05", 1_000);
        invalid_id.id = Some(-1);
        let error = db.save_voucher(invalid_id).unwrap_err();
        assert!(error.contains("شناسه سند نامعتبر"), "{error}");
        assert_eq!(db.voucher_count().unwrap(), 1);
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
    fn income_statement_classifies_cogs_by_group_not_code_prefix() {
        let db = db::memory_database();
        db.create_account("5102", "بهای تمام‌شده", 1, "9", "debtor", "", false)
            .unwrap();
        db.create_account("510201", "مواد مصرفی", 2, "9", "debtor", "5102", true)
            .unwrap();
        db.create_account("5111", "هزینه‌های عملیاتی", 1, "8", "debtor", "", false)
            .unwrap();
        db.create_account("511101", "اجاره", 2, "8", "debtor", "5111", true)
            .unwrap();

        db.save_voucher(voucher(
            "1405/01/10",
            vec![
                line("510201", 100, 0),
                line("511101", 40, 0),
                line("110101", 0, 140),
            ],
        ))
        .unwrap();

        let income = db.get_income_statement("1405/01/01", "1405/12/29").unwrap();
        assert_eq!(income.cogs_total, 100);
        assert_eq!(income.cogs.len(), 1);
        assert_eq!(income.cogs[0].account_code, "510201");
        assert_eq!(income.expense_total, 40);
        assert_eq!(income.expenses.len(), 1);
        assert_eq!(income.expenses[0].account_code, "511101");
        assert_eq!(income.gross_profit, -100);
        assert_eq!(income.net_profit, -140);
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
    fn opening_voucher_cancels_the_previous_final_closing() {
        let db = db::memory_database();
        db.save_voucher(cash_sale("1405/01/10", 1_000_000)).unwrap();

        let mut closing = voucher(
            "1405/12/29",
            vec![line("410101", 1_000_000, 0), line("310101", 0, 1_000_000)],
        );
        closing.voucher_type = CLOSING_TEMPORARY.to_string();
        db.save_voucher(closing).unwrap();

        let mut final_closing = voucher(
            "1405/12/29",
            vec![line("310101", 1_000_000, 0), line("110101", 0, 1_000_000)],
        );
        final_closing.voucher_type = CLOSING_FINAL.to_string();
        db.save_voucher(final_closing).unwrap();

        // Before the next year is opened the statement still shows the books
        // as they stood before the final closing voucher.
        let before_opening = db.get_balance_sheet("1405/12/29").unwrap();
        assert_eq!(before_opening.assets_total, 1_000_000);
        assert_eq!(before_opening.equity_total, 1_000_000);
        assert!(before_opening.balanced);

        let mut opening = voucher(
            "1406/01/01",
            vec![line("110101", 1_000_000, 0), line("310101", 0, 1_000_000)],
        );
        opening.voucher_type = "سند حسابداری افتتاحیه".to_string();
        db.save_voucher(opening).unwrap();

        // With the opening voucher present, the closing and opening pair is
        // included and cancels out instead of doubling the balances.
        let after_opening = db.get_balance_sheet("1406/06/01").unwrap();
        assert_eq!(after_opening.assets_total, 1_000_000);
        assert_eq!(after_opening.equity_total, 1_000_000);
        assert_eq!(after_opening.profit, 0);
        assert!(after_opening.balanced);
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
        assert_eq!(list.matched_count, 1);

        assert!(db.list_vouchers("1405/03/01", "1405/02/01", "").is_err());
        assert!(db.get_journal("1405/03/01", "1405/02/01", "").is_err());
        assert!(db.get_ledger("110101", "1405/03/01", "1405/02/01").is_err());
        assert!(db.get_trial_balance("1405/03/01", "1405/02/01").is_err());
        assert!(db.get_income_statement("1405/03/01", "1405/02/01").is_err());
    }

    #[test]
    fn paged_lists_keep_totals_for_the_whole_filter() {
        let db = db::memory_database();
        db.save_voucher(cash_sale("1405/01/10", 1_000)).unwrap();
        db.save_voucher(cash_sale("1405/01/11", 2_000)).unwrap();
        db.save_voucher(cash_sale("1405/01/12", 3_000)).unwrap();

        let page = db
            .list_vouchers_page("", "", "", Some(1), Some(1))
            .unwrap();
        assert_eq!(page.vouchers.len(), 1);
        assert_eq!(page.total_count, 3);
        assert_eq!(page.matched_count, 3);
        assert_eq!(page.total_debit, 6_000);
        assert_eq!(page.total_credit, 6_000);

        let journal = db
            .get_journal_page("", "", "", Some(0), Some(2))
            .unwrap();
        assert_eq!(journal.rows.len(), 2);
        assert_eq!(journal.row_count, 6);
        assert_eq!(journal.voucher_count, 3);
        assert_eq!(journal.total_debit, 6_000);
        assert_eq!(journal.total_credit, 6_000);

        assert!(db
            .list_vouchers_page("", "", "", Some(-1), Some(10))
            .is_err());
        assert!(db
            .list_vouchers_page("", "", "", Some(0), Some(0))
            .is_err());
        assert!(db
            .list_vouchers_page("", "", "", Some(10), None)
            .is_err());
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
