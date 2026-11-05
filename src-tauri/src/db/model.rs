//! Database wire types and the connection holder shared by every db module.

use rusqlite::Connection;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::vouchers::{
    DEFAULT_JOURNAL_PAGE_SIZE, DEFAULT_VOUCHER_PAGE_SIZE, MAX_AMOUNT, MAX_DESCRIPTION, MAX_LINES,
    MAX_LINE_DESCRIPTION, MAX_PAGE_SIZE, MAX_VOUCHER_NUMBER,
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
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountList {
    pub accounts: Vec<Account>,
    pub database_path: String,
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
pub struct NewAccount {
    pub code: String,
    pub name: String,
    pub level: i64,
    pub group_code: String,
    pub nature: String,
    pub parent_code: String,
    pub is_postable: bool,
    pub is_active: bool,
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
        }
    }

    /// Marks the account inactive at creation time. A new account is active
    /// unless the caller explicitly asks for an inactive definition.
    pub fn with_active(mut self, is_active: bool) -> Self {
        self.is_active = is_active;
        self
    }
}
