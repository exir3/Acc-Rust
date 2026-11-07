//! Print-preview commands.

use tauri::{AppHandle, State, WebviewWindow};

use crate::commands::state::{lock, report_meta, require_preview_window, require_window, AppState};
use crate::commands::windows::show_preview_window;
use crate::report;
use crate::vouchers::{IntegrityReport, ReportScope};

/// Official is the safe default for printed statements; temporary vouchers are
/// included only when the caller explicitly asks for a working report.
fn report_scope(scope: Option<String>) -> Result<ReportScope, String> {
    ReportScope::parse(scope.as_deref().unwrap_or("official"))
}

/// An official statement with integrity errors is not a printable final
/// document. Working/temporary previews stay available so the user can review
/// the same figures with the full health panel before fixing them.
fn ensure_printable(scope: ReportScope, integrity: &IntegrityReport) -> Result<(), String> {
    if scope == ReportScope::Official && !integrity.ok {
        return Err(
            "گزارش رسمی به دلیل وجود خطای سلامت حسابداری قابل چاپ نیست؛ ابتدا خطاهای Integrity را برطرف کنید یا گزارش را در حالت کاری تهیه کنید."
                .to_string(),
        );
    }
    Ok(())
}

/* ── Print preview ──────────────────────────────────────────── */

#[tauri::command(async)]
pub(crate) fn open_journal_preview(
    from: String,
    to: String,
    search: String,
    scope: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher"])?;
    let report = state
        .database
        .get_journal_scoped(&from, &to, &search, report_scope(scope)?)?;
    let meta = report_meta(&state, &from, &to)?;
    show_preview_window(&app, report::journal_print_doc(&report, &meta))
}

#[tauri::command(async)]
pub(crate) fn open_ledger_preview(
    account_code: String,
    from: String,
    to: String,
    scope: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher"])?;
    let report =
        state
            .database
            .get_ledger_scoped(&account_code, &from, &to, report_scope(scope)?)?;
    let meta = report_meta(&state, &from, &to)?;
    show_preview_window(&app, report::ledger_print_doc(&report, &meta))
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn open_trial_balance_preview(
    from: String,
    to: String,
    scope: Option<String>,
    account_from: Option<String>,
    account_to: Option<String>,
    show_zero: Option<bool>,
    multi_level: Option<bool>,
    balance_type: Option<String>,
    show_account_code: Option<bool>,
    show_account_name: Option<bool>,
    show_print_date_time: Option<bool>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher", "trial-balance"])?;
    let report = state.database.get_trial_balance_filtered_scoped(
        &from,
        &to,
        account_from.as_deref().unwrap_or(""),
        account_to.as_deref().unwrap_or(""),
        show_zero.unwrap_or(false),
        multi_level.unwrap_or(true),
        report_scope(scope)?,
    )?;
    let mut meta = report_meta(&state, &from, &to)?;
    meta.show_print_date_time = show_print_date_time.unwrap_or(true);
    show_preview_window(
        &app,
        report::trial_balance_print_doc(
            &report,
            &meta,
            balance_type.as_deref().unwrap_or("شش ستونی"),
            multi_level.unwrap_or(true),
            show_account_code.unwrap_or(true),
            show_account_name.unwrap_or(true),
        )?,
    )
}

#[tauri::command(async)]
pub(crate) fn open_income_statement_preview(
    from: String,
    to: String,
    scope: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher"])?;
    let scope = report_scope(scope)?;
    let report = state
        .database
        .get_income_statement_scoped(&from, &to, scope)?;
    ensure_printable(scope, &report.integrity)?;
    let meta = report_meta(&state, &from, &to)?;
    show_preview_window(&app, report::income_statement_print_doc(&report, &meta))
}

#[tauri::command(async)]
pub(crate) fn open_balance_sheet_preview(
    as_of: String,
    scope: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher"])?;
    let scope = report_scope(scope)?;
    let report = state.database.get_balance_sheet_scoped(&as_of, scope)?;
    ensure_printable(scope, &report.integrity)?;
    let meta = report_meta(&state, &as_of, &as_of)?;
    let document = report::balance_sheet_print_doc(&report, &meta)?;
    show_preview_window(&app, document)
}

#[tauri::command(async)]
pub(crate) fn open_equity_changes_preview(
    as_of: String,
    scope: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher"])?;
    let scope = report_scope(scope)?;
    let report = state.database.get_equity_changes_scoped(&as_of, scope)?;
    ensure_printable(scope, &report.integrity)?;
    let meta = report_meta(&state, &as_of, &as_of)?;
    show_preview_window(&app, report::equity_changes_print_doc(&report, &meta))
}

#[tauri::command(async)]
pub(crate) fn open_cogs_preview(
    from: String,
    to: String,
    scope: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher"])?;
    let scope = report_scope(scope)?;
    let report = state.database.get_cogs_scoped(&from, &to, scope)?;
    ensure_printable(scope, &report.integrity)?;
    let meta = report_meta(&state, &report.from_date, &report.to_date)?;
    show_preview_window(&app, report::cogs_print_doc(&report, &meta))
}

#[tauri::command(async)]
pub(crate) fn open_profit_appropriation_preview(
    as_of: String,
    scope: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher"])?;
    let scope = report_scope(scope)?;
    let report = state
        .database
        .get_profit_appropriation_scoped(&as_of, scope)?;
    ensure_printable(scope, &report.integrity)?;
    let meta = report_meta(&state, &as_of, &as_of)?;
    show_preview_window(&app, report::appropriation_print_doc(&report, &meta))
}

#[tauri::command(async)]
pub(crate) fn open_cash_flow_preview(
    as_of: String,
    scope: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher"])?;
    let scope = report_scope(scope)?;
    let report = state.database.get_cash_flow_scoped(&as_of, scope)?;
    ensure_printable(scope, &report.integrity)?;
    let meta = report_meta(&state, &as_of, &as_of)?;
    show_preview_window(&app, report::cash_flow_print_doc(&report, &meta))
}

#[tauri::command(async)]
pub(crate) fn open_voucher_preview(
    number: i64,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher", "voucher-list"])?;
    let voucher = state
        .database
        .get_voucher(number)?
        .ok_or_else(|| "برای چاپ، ابتدا سند را ذخیره کنید.".to_string())?;
    let meta = report_meta(&state, &voucher.date, &voucher.date)?;
    show_preview_window(&app, report::voucher_print_doc(&voucher, &meta))
}

/// The preview window asks for the document it should render.
#[tauri::command]
pub(crate) fn get_print_doc(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<report::PrintDoc, String> {
    require_preview_window(&window)?;
    lock(&state.print_docs)
        .remove(window.label())
        .ok_or_else(|| "گزارشی برای پیش‌نمایش آماده نیست.".to_string())
}

/// Saves the standalone (browser-printable) HTML, used by the preview toolbar.
#[tauri::command]
pub(crate) fn save_report_html(
    default_name: String,
    html: String,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    require_preview_window(&window)?;
    let Some(picked) = rfd::FileDialog::new()
        .set_title("ذخیره گزارش قابل چاپ")
        .set_file_name(default_name)
        .add_filter("HTML", &["html", "htm"])
        .save_file()
    else {
        return Ok(None);
    };
    let mut path = picked.to_string_lossy().to_string();
    if !path.to_lowercase().ends_with(".html") && !path.to_lowercase().ends_with(".htm") {
        path.push_str(".html");
    }
    std::fs::write(&path, html).map_err(|e| format!("ذخیره فایل ناموفق بود: {e}"))?;
    if let Ok(canonical) = std::fs::canonicalize(&path) {
        lock(&state.report_files).insert(canonical);
    }
    Ok(Some(path))
}
