//! Read-only report commands.

use tauri::{State, WebviewWindow};

use crate::commands::state::{require_window, AppState};
use crate::vouchers;

/* ── Reports ────────────────────────────────────────────────── */

#[tauri::command(async)]
pub(crate) fn get_journal(
    from: String,
    to: String,
    search: String,
    offset: Option<i64>,
    limit: Option<i64>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<vouchers::JournalReport, String> {
    require_window(&window, &["main", "voucher-list", "trial-balance"])?;
    state.database.get_journal_page(
        &from,
        &to,
        &search,
        offset,
        limit.or(Some(vouchers::DEFAULT_JOURNAL_PAGE_SIZE)),
    )
}

#[tauri::command(async)]
pub(crate) fn get_ledger(
    account_code: String,
    from: String,
    to: String,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<vouchers::LedgerReport, String> {
    require_window(&window, &["main", "voucher", "voucher-list", "trial-balance"])?;
    state.database.get_ledger(&account_code, &from, &to)
}

#[tauri::command(async)]
pub(crate) fn get_trial_balance(
    from: String,
    to: String,
    account_from: Option<String>,
    account_to: Option<String>,
    show_zero: Option<bool>,
    multi_level: Option<bool>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<vouchers::TrialBalanceReport, String> {
    require_window(&window, &["main", "voucher-list", "trial-balance"])?;
    state.database.get_trial_balance_filtered(
        &from,
        &to,
        account_from.as_deref().unwrap_or(""),
        account_to.as_deref().unwrap_or(""),
        show_zero.unwrap_or(false),
        multi_level.unwrap_or(true),
    )
}

#[tauri::command(async)]
pub(crate) fn get_income_statement(
    from: String,
    to: String,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<vouchers::IncomeStatementReport, String> {
    require_window(&window, &["main", "voucher-list", "trial-balance"])?;
    state.database.get_income_statement(&from, &to)
}

#[tauri::command(async)]
pub(crate) fn get_balance_sheet(
    as_of: String,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<vouchers::BalanceSheetReport, String> {
    require_window(&window, &["main", "voucher-list", "trial-balance"])?;
    state.database.get_balance_sheet(&as_of)
}
