//! Read-only report commands.

use tauri::{State, WebviewWindow};

use crate::commands::state::{require_window, AppState};
use crate::vouchers::{self, ReportScope};

/// Official reports show permanent vouchers. A caller can explicitly request
/// `working` (permanent + temporary) or `temporary` (temporary only), but the
/// default never mixes draft vouchers into an official statement.
fn report_scope(scope: Option<String>) -> Result<ReportScope, String> {
    ReportScope::parse(scope.as_deref().unwrap_or("official"))
}

/* ── Reports ────────────────────────────────────────────────── */

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn get_journal(
    from: String,
    to: String,
    search: String,
    scope: Option<String>,
    offset: Option<i64>,
    limit: Option<i64>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<vouchers::JournalReport, String> {
    require_window(&window, &["main", "voucher-list", "trial-balance"])?;
    state.database.get_journal_page_scoped(
        &from,
        &to,
        &search,
        offset,
        limit.or(Some(vouchers::DEFAULT_JOURNAL_PAGE_SIZE)),
        report_scope(scope)?,
    )
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn get_ledger(
    account_code: String,
    from: String,
    to: String,
    scope: Option<String>,
    offset: Option<i64>,
    limit: Option<i64>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<vouchers::LedgerReport, String> {
    require_window(
        &window,
        &["main", "voucher", "voucher-list", "trial-balance"],
    )?;
    state.database.get_ledger_page_scoped(
        &account_code,
        &from,
        &to,
        offset,
        limit,
        report_scope(scope)?,
    )
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn get_trial_balance(
    from: String,
    to: String,
    scope: Option<String>,
    account_from: Option<String>,
    account_to: Option<String>,
    show_zero: Option<bool>,
    multi_level: Option<bool>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<vouchers::TrialBalanceReport, String> {
    require_window(&window, &["main", "voucher-list", "trial-balance"])?;
    state.database.get_trial_balance_filtered_scoped(
        &from,
        &to,
        account_from.as_deref().unwrap_or(""),
        account_to.as_deref().unwrap_or(""),
        show_zero.unwrap_or(false),
        multi_level.unwrap_or(true),
        report_scope(scope)?,
    )
}

#[tauri::command(async)]
pub(crate) fn get_income_statement(
    from: String,
    to: String,
    scope: Option<String>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<vouchers::IncomeStatementReport, String> {
    require_window(&window, &["main", "voucher-list", "trial-balance"])?;
    state
        .database
        .get_income_statement_scoped(&from, &to, report_scope(scope)?)
}

#[tauri::command(async)]
pub(crate) fn get_balance_sheet(
    as_of: String,
    scope: Option<String>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<vouchers::BalanceSheetReport, String> {
    require_window(&window, &["main", "voucher-list", "trial-balance"])?;
    state
        .database
        .get_balance_sheet_scoped(&as_of, report_scope(scope)?)
}
