//! Voucher and report data structures shared by every voucher module.
//!
//! Field names and serde settings are part of the webview API and must stay
//! stable when the implementation is reorganized.

use serde::{Deserialize, Serialize, Serializer};

pub use crate::limits::MAX_AMOUNT;
pub(crate) use crate::limits::{MAX_DESCRIPTION, MAX_LINES, MAX_LINE_DESCRIPTION};

pub(crate) fn serialize_exact_amount<S>(value: &i64, serializer: S) -> Result<S::Ok, S::Error>
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

pub const MAX_VOUCHER_NUMBER: i64 = 9_007_199_254_740_991;

/// Rows returned by the paged lists. A caller that omits `limit` still gets
/// every row, so reports and tests keep their previous behavior.
pub const DEFAULT_VOUCHER_PAGE_SIZE: i64 = 200;
pub const DEFAULT_JOURNAL_PAGE_SIZE: i64 = 500;
pub const MAX_PAGE_SIZE: i64 = 2000;

/// Which vouchers an accounting report includes.
///
/// - `Official` is the default for official statements: only permanent
///   (finalized) vouchers.
/// - `Working` shows both permanent and temporary vouchers, matching the
///   pre-existing report behavior.
/// - `Temporary` shows only temporary vouchers for review work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportScope {
    Official,
    Working,
    Temporary,
}

impl ReportScope {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "official" => Ok(Self::Official),
            "working" => Ok(Self::Working),
            "temporary" => Ok(Self::Temporary),
            _ => Err("دامنه گزارش انتخاب‌شده معتبر نیست.".to_string()),
        }
    }

    /// SQL predicate for the `vouchers` alias `v`.
    pub(crate) fn sql(self) -> &'static str {
        match self {
            Self::Official => "v.is_temporary = 0",
            Self::Working => "1 = 1",
            Self::Temporary => "v.is_temporary = 1",
        }
    }
}

/// SQL expression for the new `updated_at` of a voucher. It is strictly
/// greater than the current value even when two writes happen in the same
/// millisecond, so an optimistic-lock token can never repeat.
pub(crate) const TOUCHED_AT: &str = "CASE
        WHEN strftime('%Y-%m-%d %H:%M:%f', 'now', 'localtime') <= updated_at
        THEN strftime('%Y-%m-%d %H:%M:%f', updated_at, '+0.001 seconds')
        ELSE strftime('%Y-%m-%d %H:%M:%f', 'now', 'localtime')
    END";

/// Voucher types that close the books. They are real vouchers (the trial
/// balance and ledgers show them) but must not hide the period's income and
/// expenses from the income statement.
pub(crate) const CLOSING_TEMPORARY: &str = VOUCHER_TYPES[3];
pub(crate) const CLOSING_FINAL: &str = VOUCHER_TYPES[2];

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
    /// Optional level-3 detail account. The account_code is always the base (level 1/2).
    #[serde(default)]
    pub detail1_code: Option<String>,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
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
    /// Fiscal year selected in the UI when the voucher is created or moved.
    /// `None` keeps the legacy date-based lookup used by bulk import and older
    /// callers. When present, the backend still validates the date against the
    /// year's range and rejects a closed or finalized year.
    #[serde(default)]
    pub fiscal_year_id: Option<i64>,
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
    /// Permanent vouchers are included in the preview as locked records.
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherLine {
    /// Stable SQLite row identifier for row-level copy/transfer.
    pub id: i64,
    pub row_no: i64,
    pub account_code: String,
    pub account_name: String,
    #[serde(default)]
    pub detail1_code: Option<String>,
    #[serde(default)]
    pub detail1_name: Option<String>,
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
    pub is_temporary: bool,
    #[serde(rename = "type")]
    pub voucher_type: String,
    pub description: String,
    pub lines: Vec<VoucherLine>,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub total_debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub total_credit: i64,
}

/// Counters shown in the voucher form header, mirroring the WebApp form.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherCounters {
    /// Number of vouchers in the whole book.
    pub total_records: i64,
    /// Number of vouchers whose number is at or below the open voucher.
    pub records_up_to_this_doc: i64,
    /// Number of saved lines in the open voucher.
    pub document_records: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherSummary {
    pub id: i64,
    pub number: i64,
    pub date: String,
    pub updated_at: String,
    pub is_temporary: bool,
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
    /// Number of ledger rows in the whole period, before paging.
    pub row_count: i64,
    pub rows: Vec<LedgerRow>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrialBalanceRow {
    pub account_code: String,
    pub account_name: String,
    pub group_name: String,
    pub parent_code: String,
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
    pub finance_costs: Vec<AmountRow>,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub finance_cost_total: i64,
    pub income_taxes: Vec<AmountRow>,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub income_tax_total: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub gross_profit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub operating_profit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub profit_before_tax: i64,
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
