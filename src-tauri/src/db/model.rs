//! Database wire types and the connection holder shared by every db module.

use rusqlite::Connection;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::vouchers::{
    serialize_exact_amount, DEFAULT_JOURNAL_PAGE_SIZE, DEFAULT_VOUCHER_PAGE_SIZE, MAX_AMOUNT,
    MAX_DESCRIPTION, MAX_LINES, MAX_LINE_DESCRIPTION, MAX_PAGE_SIZE, MAX_VOUCHER_NUMBER,
};

/// Account-group types accepted by the form and the database constraint.
pub const ACCOUNT_GROUP_TYPES: [&str; 6] = ["دارایی", "بدهی", "سرمایه", "درآمد", "هزینه", "سایر"];

pub const ACCOUNT_NATURES: [&str; 3] = ["debtor", "creditor", "dual"];

/// Financial-statement sections a group can be assigned to by default.
/// These keys match the WebApp ``Section`` values.
pub const ACCOUNT_GROUP_SECTIONS: [&str; 14] = [
    "non_current_asset",
    "current_asset",
    "equity",
    "non_current_liability",
    "current_liability",
    "revenue",
    "cogs",
    "selling_expense",
    "admin_expense",
    "other_operating",
    "finance_cost",
    "other_non_operating",
    "income_tax",
    "memorandum",
];

/// Account rows are numbered independently from account groups: 1=کل، 2=معین، 3=تفصیلی ۱.
/// Level 4 is retained in the physical schema only to read older databases; it is not creatable.
pub const MAX_ACCOUNT_LEVEL: i64 = 3;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountGroup {
    pub id: i64,
    pub code: String,
    pub name: String,
    #[serde(rename = "type")]
    pub group_type: String,
    pub default_section: String,
    pub account_count: i64,
    /// Voucher lines posted to accounts of this group. Used to require an
    /// explicit confirmation before a change that would reclassify history.
    pub posted_line_count: i64,
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
    pub is_active: bool,
    /// Explicit financial-statement line stored on the account (empty when the
    /// account inherits or follows its group).
    pub fs_line: String,
    /// Line actually used by the reports after inheritance/group resolution.
    pub effective_fs_line: String,
    pub effective_fs_line_title: String,
    pub effective_section: String,
    pub effective_statement: String,
    /// `manual`, `seed`, `inherit`/`inherited`, or `auto`.
    pub classification_source: String,
    pub system_role: String,
    pub contra_of: String,
    pub contra_kind: String,
    pub opposite_fs_line: String,
    pub split_by_sign: bool,
    /// True when the account's nature is opposite to its line's normal side.
    pub is_contra: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountList {
    pub accounts: Vec<Account>,
    pub database_path: String,
}

/// One entry of the financial-statement line catalog, shaped for the account
/// forms. The catalog is static data, so the UI loads it once per window.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsLineInfo {
    pub code: String,
    pub title: String,
    pub statement: String,
    pub statement_title: String,
    pub section: String,
    pub section_title: String,
    pub normal_side: String,
    pub permanence: String,
    pub permanence_title: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsCatalogOption {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsCatalog {
    pub lines: Vec<FsLineInfo>,
    pub contra_kinds: Vec<FsCatalogOption>,
    pub system_roles: Vec<FsCatalogOption>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkAccountDeleteFailure {
    pub code: String,
    pub name: String,
    pub error: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkAccountDeleteResult {
    pub deleted_count: usize,
    pub blocked: Vec<BulkAccountDeleteFailure>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    /// Maximum number of lines accepted in one voucher.
    pub max_voucher_lines: usize,
    /// Largest amount accepted on one line.
    pub max_amount: i64,
    /// Highest voucher number the backend accepts.
    pub max_voucher_number: i64,
    /// Longest voucher description.
    pub max_description: usize,
    /// Longest description on one voucher line.
    pub max_line_description: usize,
    pub voucher_page_size: i64,
    pub journal_page_size: i64,
    pub max_page_size: i64,
}

impl Limits {
    pub fn current() -> Self {
        Self {
            max_voucher_lines: MAX_LINES,
            max_amount: MAX_AMOUNT,
            max_voucher_number: MAX_VOUCHER_NUMBER,
            max_description: MAX_DESCRIPTION,
            max_line_description: MAX_LINE_DESCRIPTION,
            voucher_page_size: DEFAULT_VOUCHER_PAGE_SIZE,
            journal_page_size: DEFAULT_JOURNAL_PAGE_SIZE,
            max_page_size: MAX_PAGE_SIZE,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub company_name: String,
    pub fiscal_year: String,
    pub today: String,
    /// Backend limits sent to the UI so both layers use the same values.
    pub limits: Limits,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FiscalYear {
    pub id: i64,
    pub title: String,
    pub start_date: String,
    pub end_date: String,
    pub status: String,
    pub voucher_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FiscalYearList {
    pub years: Vec<FiscalYear>,
    pub database_path: String,
}

/// One executed or reversed year-end closing run. A year has at most one
/// `executed` run; reversing it records the audit fields and reopens the year.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosingRunRecord {
    pub id: i64,
    pub fiscal_year_id: i64,
    pub status: String,
    pub temp_voucher_id: Option<i64>,
    pub closing_voucher_id: Option<i64>,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub net_income: i64,
    pub income_summary_code: String,
    pub retained_earnings_code: String,
    pub closing_balance_code: Option<String>,
    pub executed_at: Option<String>,
    pub reversed_at: Option<String>,
    pub reverse_reason: Option<String>,
}

/// An account that carries one of the closing workflow's system roles.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosingRoleAccount {
    pub code: String,
    pub name: String,
    pub fs_line: Option<String>,
    pub statement: Option<String>,
    pub is_postable: bool,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosingRoles {
    pub income_summary: Option<ClosingRoleAccount>,
    pub retained_earnings: Option<ClosingRoleAccount>,
    pub closing_balance: Option<ClosingRoleAccount>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosingBlocker {
    pub code: String,
    pub severity: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<i64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub accounts: Vec<String>,
}

/// One proposed closing-voucher line. `amount` is signed: positive = debit.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosingLine {
    pub account_code: String,
    pub detail1_code: String,
    pub account_name: String,
    pub fs_line: String,
    /// `temporary`, `income_summary`, `retained_earnings`, `permanent` or
    /// `closing_balance`.
    pub role: String,
    /// `close`, `transfer` or `closing`.
    pub block: String,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub amount: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub credit: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosingLineTotals {
    #[serde(serialize_with = "serialize_exact_amount")]
    pub debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub credit: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosingTotals {
    pub temp: ClosingLineTotals,
    pub closing: ClosingLineTotals,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosingCounts {
    pub temporary_accounts: i64,
    pub permanent_accounts: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosingPreview {
    pub fiscal_year_id: i64,
    pub fiscal_year_title: String,
    /// `open` or `closed`.
    pub status: String,
    pub run: Option<ClosingRunRecord>,
    pub latest_run: Option<ClosingRunRecord>,
    pub roles: ClosingRoles,
    pub blockers: Vec<ClosingBlocker>,
    pub can_execute: bool,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub net_income: i64,
    pub profit: bool,
    pub temp_lines: Vec<ClosingLine>,
    pub closing_lines: Vec<ClosingLine>,
    pub totals: ClosingTotals,
    pub counts: ClosingCounts,
    pub as_of_date: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosingVoucherRef {
    pub id: i64,
    pub number: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosingExecutionResult {
    pub run_id: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub net_income: i64,
    pub temp_voucher: Option<ClosingVoucherRef>,
    pub closing_voucher: Option<ClosingVoucherRef>,
    pub temp_line_count: i64,
    pub closing_line_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosingReversalResult {
    pub run_id: i64,
    pub deleted_vouchers: i64,
}

/// One proposed opening-voucher line for the target fiscal year.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CarryForwardLine {
    pub account_code: String,
    pub account_name: String,
    pub detail1_code: String,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub debit: i64,
    #[serde(serialize_with = "serialize_exact_amount")]
    pub credit: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CarryForwardPlan {
    pub source_fiscal_year_id: i64,
    pub source_fiscal_year_title: String,
    pub target_fiscal_year_id: i64,
    pub target_fiscal_year_title: String,
    pub source: String,
    pub lines: Vec<CarryForwardLine>,
    pub totals: ClosingLineTotals,
    pub account_count: i64,
    pub has_closing_voucher: bool,
    pub existing_opening_voucher: Option<ClosingVoucherRef>,
    pub requires_overwrite: bool,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CarryForwardExecutionResult {
    pub voucher_id: i64,
    pub voucher_number: i64,
    pub line_count: i64,
    pub account_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditLogEntry {
    pub id: i64,
    pub action: String,
    pub entity_type: String,
    pub entity_id: Option<i64>,
    pub before_value: Option<String>,
    pub after_value: Option<String>,
    pub details: String,
    pub app_version: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditLogList {
    pub entries: Vec<AuditLogEntry>,
    pub total: i64,
    pub database_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyLevel4Account {
    pub code: String,
    pub name: String,
    pub parent_code: String,
    pub line_count: i64,
    /// True when the level-3 parent and its level-2 base form a sound pair.
    pub convertible: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyLevel4Analysis {
    pub account_count: i64,
    pub line_count: i64,
    pub voucher_count: i64,
    pub convertible_line_count: i64,
    pub ambiguous_line_count: i64,
    pub accounts: Vec<LegacyLevel4Account>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyLevel4MigrationResult {
    pub converted_lines: i64,
    pub pending_lines: i64,
    pub removed_accounts: i64,
    pub backup_path: String,
}

/// Before/after view of mapping one ambiguous level-4 account to a real
/// level-3 (detail 1) account and its level-2 base account.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyLevel4ResolutionPreview {
    pub account_code: String,
    pub account_name: String,
    pub line_count: i64,
    pub voucher_count: i64,
    pub total_debit: i64,
    pub total_credit: i64,
    pub balance: i64,
    pub target_account_code: String,
    pub target_account_name: String,
    pub target_detail1_code: String,
    pub target_detail1_name: String,
}

/// Holds the single write connection. SQLite access is cheap and the forms
/// have very little traffic, so one mutex-guarded connection keeps ordering
/// guarantees simple.
#[derive(Default)]
pub struct Database {
    pub(crate) conn: Mutex<Option<Connection>>,
    pub(crate) path: Mutex<Option<PathBuf>>,
    /// Set when a restore failed and its safety copy could not be rolled back.
    /// Every later read/write is refused so an uncertain database is never used.
    pub(crate) failed: Mutex<Option<String>>,
}

/// A validated-at-save account definition used by batch creation.
#[derive(Debug, Clone)]
pub struct NewAccount {
    pub code: String,
    pub name: String,
    pub level: i64,
    pub group_code: String,
    pub nature: String,
    pub parent_code: String,
    pub is_postable: bool,
    pub is_active: bool,
    pub fs_line: String,
    pub contra_of: String,
    pub contra_kind: String,
    pub opposite_fs_line: String,
    pub split_by_sign: bool,
    pub system_role: String,
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
            is_active: true,
            fs_line: String::new(),
            contra_of: String::new(),
            contra_kind: String::new(),
            opposite_fs_line: String::new(),
            split_by_sign: false,
            system_role: String::new(),
        }
    }

    /// Marks the account inactive at creation time. A new account is active
    /// unless the caller explicitly asks for an inactive definition.
    pub fn with_active(mut self, is_active: bool) -> Self {
        self.is_active = is_active;
        self
    }

    /// Sets an explicit financial-statement line (validated by the caller).
    pub fn with_fs_line(mut self, fs_line: &str) -> Self {
        self.fs_line = fs_line.to_string();
        self
    }

    pub fn with_classification(
        mut self,
        contra_of: &str,
        contra_kind: &str,
        opposite_fs_line: &str,
        split_by_sign: bool,
        system_role: &str,
    ) -> Self {
        self.contra_of = contra_of.to_string();
        self.contra_kind = contra_kind.to_string();
        self.opposite_fs_line = opposite_fs_line.to_string();
        self.split_by_sign = split_by_sign;
        self.system_role = system_role.to_string();
        self
    }
}
