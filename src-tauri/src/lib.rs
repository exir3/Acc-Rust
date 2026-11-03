//! Tauri backend for the accounting application.
//!
//! Accounts, vouchers and every figure shown by the UI live in one local
//! SQLite database. The commands below are the only way the webview can read
//! or change that data.

pub mod db;
pub mod excel;
pub mod report;
pub mod vouchers;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use vouchers::VoucherInput;

/// A poisoned lock only means another thread panicked while holding it; the
/// guarded value is plain data, so keep going instead of panicking too.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn notify_accounts_changed(app: &AppHandle) {
    let _ = app.emit("accounts-changed", ());
}

#[derive(Default)]
pub struct AppState {
    /// Reports waiting to be rendered, keyed by their preview-window label.
    print_docs: Mutex<HashMap<String, report::PrintDoc>>,
    /// Voucher number requested by the main window the next time the
    /// voucher form asks for one. `Some(None)` means "start a blank voucher".
    pending_voucher: Mutex<Option<Option<i64>>>,
    /// Local SQLite database.
    database: db::Database,
    /// Parsed workbook awaiting the user's bulk-import confirmation.
    bulk_voucher_import: Mutex<Option<excel::BulkVoucherImport>>,
    /// Reorganization preview awaiting explicit confirmation.
    reorganization_preview: Mutex<Option<(String, String)>>,
    /// Bulk-delete preview token awaiting explicit confirmation.
    bulk_delete_preview: Mutex<Option<String>>,
}

fn report_meta(state: &State<'_, AppState>) -> Result<report::ReportMeta, String> {
    let settings = state.database.get_settings()?;
    Ok(report::ReportMeta {
        company: settings.company_name,
        fiscal_year: settings.fiscal_year,
        today: settings.today,
        show_print_date_time: true,
    })
}

/// Defense in depth: a compromised child window must not be able to run
/// commands that belong to another window.
fn require_window(window: &WebviewWindow, allowed: &[&str]) -> Result<(), String> {
    if allowed.contains(&window.label()) {
        Ok(())
    } else {
        Err("این عملیات از پنجره جاری مجاز نیست.".to_string())
    }
}

fn require_preview_window(window: &WebviewWindow) -> Result<(), String> {
    if window.label().starts_with("print-preview-") {
        Ok(())
    } else {
        Err("این عملیات فقط از پنجره پیش‌نمایش چاپ مجاز است.".to_string())
    }
}

fn require_main_or_preview_window(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "main" || window.label().starts_with("print-preview-") {
        Ok(())
    } else {
        Err("این عملیات از پنجره جاری مجاز نیست.".to_string())
    }
}

/* ── Account groups ─────────────────────────────────────────── */

#[tauri::command(async)]
fn list_account_groups(state: State<'_, AppState>) -> Result<db::AccountGroupList, String> {
    state.database.list_account_groups()
}

#[tauri::command(async)]
fn create_account_group(
    code: String,
    name: String,
    group_type: String,
    default_section: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::AccountGroup, String> {
    require_window(&window, &["main", "account-groups"])?;
    let group = state.database.create_account_group(
        &code,
        &name,
        &group_type,
        default_section.as_deref().unwrap_or(""),
    )?;
    notify_accounts_changed(&app);
    Ok(group)
}

#[tauri::command(async)]
fn update_account_group(
    id: i64,
    code: String,
    name: String,
    group_type: String,
    default_section: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::AccountGroup, String> {
    require_window(&window, &["main", "account-groups"])?;
    let group = state.database.update_account_group(
        id,
        &code,
        &name,
        &group_type,
        default_section.as_deref().unwrap_or(""),
    )?;
    notify_accounts_changed(&app);
    Ok(group)
}

#[tauri::command(async)]
fn delete_account_group(
    id: i64,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "account-groups"])?;
    state.database.delete_account_group(id)?;
    notify_accounts_changed(&app);
    Ok(())
}

/* ── Chart of accounts ──────────────────────────────────────── */

#[tauri::command(async)]
fn list_accounts(state: State<'_, AppState>) -> Result<db::AccountList, String> {
    state.database.list_accounts()
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
fn create_account(
    code: String,
    name: String,
    level: i64,
    group_code: String,
    nature: String,
    parent_code: String,
    is_postable: bool,
    is_active: Option<bool>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::Account, String> {
    require_window(&window, &["main", "account-groups", "account-definition"])?;
    let account = state.database.create_account_active(
        &code,
        &name,
        level,
        &group_code,
        &nature,
        &parent_code,
        is_postable,
        is_active.unwrap_or(true),
    )?;
    notify_accounts_changed(&app);
    Ok(account)
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
fn update_account(
    id: i64,
    name: String,
    group_code: String,
    nature: String,
    parent_code: String,
    is_postable: bool,
    is_active: Option<bool>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::Account, String> {
    require_window(&window, &["main", "account-groups", "account-definition"])?;
    let mut account = state.database.update_account(
        id,
        &name,
        &group_code,
        &nature,
        &parent_code,
        is_postable,
    )?;
    if let Some(is_active) = is_active {
        if account.is_active != is_active {
            account = state.database.set_account_active(id, is_active)?;
        }
    }
    notify_accounts_changed(&app);
    Ok(account)
}

#[tauri::command(async)]
fn set_account_active(
    id: i64,
    is_active: bool,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::Account, String> {
    require_window(&window, &["main", "account-groups", "account-definition"])?;
    let account = state.database.set_account_active(id, is_active)?;
    notify_accounts_changed(&app);
    Ok(account)
}

#[tauri::command(async)]
fn delete_account(
    id: i64,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "account-groups", "account-definition"])?;
    state.database.delete_account(id)?;
    notify_accounts_changed(&app);
    Ok(())
}

#[tauri::command(async)]
fn delete_accounts_bulk(
    ids: Vec<i64>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::BulkAccountDeleteResult, String> {
    require_window(&window, &["main", "account-definition"])?;
    let result = state.database.delete_accounts_bulk(ids)?;
    if result.deleted_count > 0 {
        notify_accounts_changed(&app);
    }
    Ok(result)
}

#[tauri::command(async)]
fn export_account_excel(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<excel::AccountExportResult>, String> {
    require_window(&window, &["main", "account-groups", "account-definition"])?;
    excel::export_accounts(&state.database)
}

#[tauri::command(async)]
fn import_account_excel(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<excel::AccountImportResult>, String> {
    require_window(&window, &["main", "account-groups", "account-definition"])?;
    let result = excel::import_accounts(&state.database)?;
    if result
        .as_ref()
        .is_some_and(|result| result.created_count > 0)
    {
        notify_accounts_changed(&app);
    }
    Ok(result)
}

/* ── Settings ───────────────────────────────────────────────── */

#[tauri::command(async)]
fn get_settings(state: State<'_, AppState>) -> Result<db::Settings, String> {
    state.database.get_settings()
}

#[tauri::command(async)]
fn save_settings(
    company_name: String,
    fiscal_year: String,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::Settings, String> {
    require_window(&window, &["main"])?;
    state.database.save_settings(&company_name, &fiscal_year)
}

/* ── Vouchers ───────────────────────────────────────────────── */

#[tauri::command(async)]
fn validate_voucher_date(date: String) -> Result<String, String> {
    vouchers::validate_voucher_date(&date)
}

#[tauri::command(async)]
fn next_voucher_number(state: State<'_, AppState>) -> Result<i64, String> {
    state.database.next_voucher_number()
}

#[tauri::command(async)]
fn adjacent_voucher_number(
    number: i64,
    direction: String,
    state: State<'_, AppState>,
) -> Result<Option<i64>, String> {
    state.database.adjacent_voucher_number(number, &direction)
}

#[tauri::command(async)]
fn get_voucher_count(state: State<'_, AppState>) -> Result<i64, String> {
    state.database.voucher_count()
}

#[tauri::command(async)]
fn list_vouchers(
    from: String,
    to: String,
    search: String,
    offset: Option<i64>,
    limit: Option<i64>,
    state: State<'_, AppState>,
) -> Result<vouchers::VoucherList, String> {
    state.database.list_vouchers_page(
        &from,
        &to,
        &search,
        offset,
        limit.or(Some(vouchers::DEFAULT_VOUCHER_PAGE_SIZE)),
    )
}

#[tauri::command(async)]
fn get_voucher(
    number: i64,
    state: State<'_, AppState>,
) -> Result<Option<vouchers::Voucher>, String> {
    state.database.get_voucher(number)
}

#[tauri::command(async)]
fn transfer_voucher_lines(
    input: vouchers::VoucherTransferInput,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<vouchers::VoucherTransferResult, String> {
    require_window(&window, &["main", "voucher"])?;
    state.database.transfer_voucher_lines(input)
}

#[tauri::command(async)]
fn save_voucher(
    input: VoucherInput,
    expected_updated_at: Option<String>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<vouchers::Voucher, String> {
    require_window(&window, &["main", "voucher"])?;
    state
        .database
        .save_voucher_if_unchanged(input, expected_updated_at.as_deref())
}

#[tauri::command(async)]
fn delete_voucher(
    id: i64,
    expected_updated_at: Option<String>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher", "voucher-list"])?;
    state
        .database
        .delete_voucher(id, expected_updated_at.as_deref())
}

#[tauri::command(async)]
fn export_voucher_excel(
    input: VoucherInput,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    require_window(&window, &["main", "voucher"])?;
    state
        .database
        .validate_voucher_accounts(input.id, &input.lines)?;
    let accounts = state.database.list_accounts()?.accounts;
    excel::export_voucher(&input, &accounts)
}

#[tauri::command(async)]
fn import_voucher_excel(
    window: WebviewWindow,
) -> Result<Option<Vec<vouchers::VoucherLineInput>>, String> {
    require_window(&window, &["main", "voucher"])?;
    excel::import_voucher()
}

#[tauri::command(async)]
fn analyze_bulk_voucher_excel(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<excel::BulkVoucherPreview>, String> {
    require_window(&window, &["main"])?;
    *lock(&state.bulk_voucher_import) = None;
    let Some(import) = excel::import_vouchers()? else {
        return Ok(None);
    };
    let preview = state.database.preview_bulk_import(&import)?;
    *lock(&state.bulk_voucher_import) = Some(import);
    Ok(Some(preview))
}

#[tauri::command(async)]
fn commit_bulk_voucher_excel(
    renumber_conflicts: bool,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    require_window(&window, &["main"])?;
    let import = lock(&state.bulk_voucher_import)
        .clone()
        .ok_or_else(|| "ابتدا فایل اکسل را انتخاب و تحلیل کنید.".to_string())?;
    let imported_count = state.database.import_vouchers(import, renumber_conflicts)?;
    *lock(&state.bulk_voucher_import) = None;
    notify_accounts_changed(&app);
    Ok(imported_count)
}

#[tauri::command(async)]
fn preview_voucher_reorganization(
    operation: String,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<vouchers::VoucherReorganizationPreview, String> {
    require_window(&window, &["main"])?;
    *lock(&state.reorganization_preview) = None;
    let preview = state.database.preview_voucher_reorganization(&operation)?;
    *lock(&state.reorganization_preview) = Some((operation, preview.plan_token.clone()));
    Ok(preview)
}

#[tauri::command(async)]
fn apply_voucher_reorganization(
    operation: String,
    plan_token: String,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<vouchers::VoucherReorganizationResult, String> {
    require_window(&window, &["main"])?;
    let expected = lock(&state.reorganization_preview).clone();
    if expected.as_ref() != Some(&(operation.clone(), plan_token.clone())) {
        return Err("پیش‌نمایش منقضی یا نامعتبر است؛ دوباره پیش‌نمایش بگیرید.".to_string());
    }
    let result = state
        .database
        .apply_voucher_reorganization(&operation, &plan_token)?;
    *lock(&state.reorganization_preview) = None;
    Ok(result)
}

#[tauri::command(async)]
fn preview_bulk_voucher_delete(
    input: vouchers::BulkVoucherDeleteInput,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<vouchers::BulkVoucherDeletePreview, String> {
    require_window(&window, &["bulk-voucher-delete"])?;
    *lock(&state.bulk_delete_preview) = None;
    let preview = state.database.preview_bulk_voucher_delete(&input)?;
    *lock(&state.bulk_delete_preview) = Some(preview.plan_token.clone());
    Ok(preview)
}

#[tauri::command(async)]
fn apply_bulk_voucher_delete(
    input: vouchers::BulkVoucherDeleteInput,
    plan_token: String,
    with_backup: Option<bool>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<vouchers::BulkVoucherDeleteResult, String> {
    require_window(&window, &["bulk-voucher-delete"])?;
    if lock(&state.bulk_delete_preview).as_ref() != Some(&plan_token) {
        return Err("پیش‌نمایش منقضی یا نامعتبر است؛ دوباره پیش‌نمایش بگیرید.".to_string());
    }
    let result = state.database.apply_bulk_voucher_delete(
        &input,
        &plan_token,
        with_backup.unwrap_or(true),
    )?;
    *lock(&state.bulk_delete_preview) = None;
    let _ = app.emit("vouchers-changed", ());
    Ok(result)
}

/* ── Reports ────────────────────────────────────────────────── */

#[tauri::command(async)]
fn get_journal(
    from: String,
    to: String,
    search: String,
    offset: Option<i64>,
    limit: Option<i64>,
    state: State<'_, AppState>,
) -> Result<vouchers::JournalReport, String> {
    state.database.get_journal_page(
        &from,
        &to,
        &search,
        offset,
        limit.or(Some(vouchers::DEFAULT_JOURNAL_PAGE_SIZE)),
    )
}

#[tauri::command(async)]
fn get_ledger(
    account_code: String,
    from: String,
    to: String,
    state: State<'_, AppState>,
) -> Result<vouchers::LedgerReport, String> {
    state.database.get_ledger(&account_code, &from, &to)
}

#[tauri::command(async)]
fn get_trial_balance(
    from: String,
    to: String,
    account_from: Option<String>,
    account_to: Option<String>,
    show_zero: Option<bool>,
    multi_level: Option<bool>,
    state: State<'_, AppState>,
) -> Result<vouchers::TrialBalanceReport, String> {
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
fn get_income_statement(
    from: String,
    to: String,
    state: State<'_, AppState>,
) -> Result<vouchers::IncomeStatementReport, String> {
    state.database.get_income_statement(&from, &to)
}

#[tauri::command(async)]
fn get_balance_sheet(
    as_of: String,
    state: State<'_, AppState>,
) -> Result<vouchers::BalanceSheetReport, String> {
    state.database.get_balance_sheet(&as_of)
}

/* ── Print preview ──────────────────────────────────────────── */

#[tauri::command(async)]
fn open_journal_preview(
    from: String,
    to: String,
    search: String,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher"])?;
    let report = state.database.get_journal(&from, &to, &search)?;
    let meta = report_meta(&state)?;
    show_preview_window(&app, report::journal_print_doc(&report, &meta))
}

#[tauri::command(async)]
fn open_ledger_preview(
    account_code: String,
    from: String,
    to: String,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher"])?;
    let report = state.database.get_ledger(&account_code, &from, &to)?;
    let meta = report_meta(&state)?;
    show_preview_window(&app, report::ledger_print_doc(&report, &meta))
}

#[tauri::command(async)]
fn open_trial_balance_preview(
    from: String,
    to: String,
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
    let report = state.database.get_trial_balance_filtered(
        &from,
        &to,
        account_from.as_deref().unwrap_or(""),
        account_to.as_deref().unwrap_or(""),
        show_zero.unwrap_or(false),
        multi_level.unwrap_or(true),
    )?;
    let mut meta = report_meta(&state)?;
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
        ),
    )
}

#[tauri::command(async)]
fn open_income_statement_preview(
    from: String,
    to: String,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher"])?;
    let report = state.database.get_income_statement(&from, &to)?;
    let meta = report_meta(&state)?;
    show_preview_window(&app, report::income_statement_print_doc(&report, &meta))
}

#[tauri::command(async)]
fn open_balance_sheet_preview(
    as_of: String,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher"])?;
    let report = state.database.get_balance_sheet(&as_of)?;
    let meta = report_meta(&state)?;
    let document = report::balance_sheet_print_doc(&report, &meta)?;
    show_preview_window(&app, document)
}

#[tauri::command(async)]
fn open_voucher_preview(
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
    let meta = report_meta(&state)?;
    show_preview_window(&app, report::voucher_print_doc(&voucher, &meta))
}

/// The preview window asks for the document it should render.
#[tauri::command]
fn get_print_doc(
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
fn save_report_html(
    default_name: String,
    html: String,
    window: WebviewWindow,
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
    Ok(Some(path))
}

#[tauri::command]
fn backup_database(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    require_window(&window, &["main"])?;
    let Some(path) = rfd::FileDialog::new()
        .set_title("ذخیره پشتیبان پایگاه داده")
        .set_file_name("hesabdari-backup.sqlite3")
        .add_filter("SQLite database", &["sqlite3", "db"])
        .save_file()
    else {
        return Ok(None);
    };
    state.database.backup_to(&path)?;
    Ok(Some(path.to_string_lossy().to_string()))
}

#[tauri::command]
fn restore_database(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    require_window(&window, &["main"])?;
    let Some(path) = rfd::FileDialog::new()
        .set_title("انتخاب پشتیبان برای بازیابی")
        .add_filter("SQLite backup", &["sqlite3", "db"])
        .pick_file()
    else {
        return Ok(None);
    };
    let safety_path = state.database.restore_from(&path)?;
    notify_accounts_changed(&app);
    let _ = app.emit("database-restored", ());
    Ok(Some(safety_path.to_string_lossy().to_string()))
}

/// Reopens the database after a failed restore left the application blocked.
/// A failed reopen keeps the block in place so an uncertain file is never used.
#[tauri::command(async)]
fn retry_database(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main"])?;
    state.database.clear_failed();
    match db::initialize(&app) {
        Ok((connection, path)) => {
            state.database.attach(connection, path);
            notify_accounts_changed(&app);
            let _ = app.emit("database-restored", ());
            Ok(())
        }
        Err(error) => {
            state.database.mark_failed(error.clone());
            Err(error)
        }
    }
}

/// Opens a report the user saved from the preview window. The webview may
/// only ask for an existing HTML file, never for an arbitrary program.
#[tauri::command]
fn open_path(path: String, window: WebviewWindow) -> Result<(), String> {
    require_main_or_preview_window(&window)?;
    let target = std::path::Path::new(&path);
    let is_html = target
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm"))
        .unwrap_or(false);
    if !is_html || !target.is_file() {
        return Err("فقط فایل HTML ذخیره‌شده از گزارش‌ها قابل باز شدن است.".to_string());
    }
    open_external(&path)
}

/* ── Windows ────────────────────────────────────────────────── */

fn stamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

static PREVIEW_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Keeps a single preview window open: stores the new document and rebuilds
/// the window so it always shows the latest report.
fn show_preview_window(app: &AppHandle, doc: report::PrintDoc) -> Result<(), String> {
    let state = app.state::<AppState>();

    for (label, window) in app.webview_windows() {
        if label.starts_with("print-preview") {
            lock(&state.print_docs).remove(&label);
            let _ = window.close();
        }
    }

    let label = format!(
        "print-preview-{}-{}",
        stamp(),
        PREVIEW_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    lock(&state.print_docs).insert(label.clone(), doc);
    let mut builder =
        WebviewWindowBuilder::new(app, label.clone(), WebviewUrl::App("print.html".into()))
            .title("پیش‌نمایش چاپ — نرم‌افزار حسابداری")
            .inner_size(1180.0, 900.0)
            .min_inner_size(720.0, 520.0)
            .center()
            .resizable(true);

    if let Some(main) = app.get_webview_window("main") {
        builder = match builder.parent(&main) {
            Ok(builder) => builder,
            Err(error) => {
                lock(&state.print_docs).remove(&label);
                return Err(format!("تعیین پنجره اصلی به‌عنوان مالک ناموفق بود: {error}"));
            }
        };
    }

    let window = match builder.build() {
        Ok(window) => window,
        Err(error) => {
            lock(&state.print_docs).remove(&label);
            return Err(format!("باز کردن پنجره پیش‌نمایش ناموفق بود: {error}"));
        }
    };
    let _ = window.maximize();
    let _ = window.set_focus();
    Ok(())
}

/// Opens (or focuses) the voucher form. When the window is already open the
/// requested number is delivered as an event instead of rebuilding the page.
#[tauri::command(async)]
fn open_voucher_window(
    number: Option<i64>,
    app: AppHandle,
    caller: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&caller, &["main", "voucher-list"])?;
    if let Some(window) = app.get_webview_window("voucher") {
        *lock(&state.pending_voucher) = Some(number);
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.emit("voucher-request", number);
        return Ok(());
    }

    *lock(&state.pending_voucher) = Some(number);

    let mut builder =
        WebviewWindowBuilder::new(&app, "voucher", WebviewUrl::App("voucher.html".into()))
            .title("صدور سند حسابداری")
            .decorations(true)
            .resizable(true)
            .maximizable(false)
            .minimizable(false);

    if let Ok(Some(monitor)) = app.primary_monitor() {
        let logical = monitor.size().to_logical::<f64>(monitor.scale_factor());
        let width = (logical.width * 0.874).clamp(836.0, 1273.0);
        let height = (logical.height * 0.794).clamp(487.0, 776.0);
        builder = builder.inner_size(width, height);
    } else {
        builder = builder.inner_size(1178.0, 740.0);
    }

    if let Some(main) = app.get_webview_window("main") {
        builder = builder
            .parent(&main)
            .map_err(|e| format!("تعیین پنجره اصلی به‌عنوان مالک ناموفق بود: {e}"))?;
    }

    let window = builder
        .min_inner_size(836.0, 487.0)
        .center()
        .build()
        .map_err(|e| format!("باز کردن فرم سند ناموفق بود: {e}"))?;
    let _ = window.set_focus();
    Ok(())
}

/// Opens (or focuses) the voucher list in its own native OS window.
#[tauri::command(async)]
fn open_voucher_list_window(app: AppHandle, caller: WebviewWindow) -> Result<(), String> {
    require_window(&caller, &["main"])?;
    if let Some(window) = app.get_webview_window("voucher-list") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    let mut builder = WebviewWindowBuilder::new(
        &app,
        "voucher-list",
        WebviewUrl::App("index.html".into()),
    )
    .title("لیست اسناد حسابداری")
    .decorations(true)
    .resizable(true)
    .maximizable(false)
    .minimizable(false)
    .initialization_script("window.__IS_VOUCHER_LIST_POPUP__ = true;");

    if let Ok(Some(monitor)) = app.primary_monitor() {
        let logical = monitor.size().to_logical::<f64>(monitor.scale_factor());
        builder = builder.inner_size(
            (logical.width * 0.88).clamp(980.0, 1440.0),
            (logical.height * 0.82).clamp(580.0, 900.0),
        );
    } else {
        builder = builder.inner_size(1200.0, 760.0);
    }

    if let Some(main) = app.get_webview_window("main") {
        builder = builder
            .parent(&main)
            .map_err(|e| format!("تعیین پنجره اصلی به‌عنوان مالک ناموفق بود: {e}"))?;
    }

    let window = builder
        .min_inner_size(900.0, 520.0)
        .center()
        .build()
        .map_err(|e| format!("باز کردن پنجره لیست اسناد ناموفق بود: {e}"))?;
    let _ = window.set_focus();
    Ok(())
}

/// Opens (or focuses) the trial-balance report in its own native OS window.
#[tauri::command(async)]
fn open_trial_balance_window(app: AppHandle, caller: WebviewWindow) -> Result<(), String> {
    require_window(&caller, &["main"])?;
    if let Some(window) = app.get_webview_window("trial-balance") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    let mut builder = WebviewWindowBuilder::new(
        &app,
        "trial-balance",
        WebviewUrl::App("index.html".into()),
    )
    .title("تراز آزمایشی")
    .decorations(true)
    .resizable(true)
    .maximizable(false)
    .minimizable(false)
    .initialization_script("window.__IS_TRIAL_BALANCE_POPUP__ = true;");

    if let Ok(Some(monitor)) = app.primary_monitor() {
        let logical = monitor.size().to_logical::<f64>(monitor.scale_factor());
        builder = builder.inner_size(
            (logical.width * 0.86).clamp(940.0, 1420.0),
            (logical.height * 0.82).clamp(560.0, 900.0),
        );
    } else {
        builder = builder.inner_size(1160.0, 760.0);
    }

    if let Some(main) = app.get_webview_window("main") {
        builder = builder
            .parent(&main)
            .map_err(|e| format!("تعیین پنجره اصلی به‌عنوان مالک ناموفق بود: {e}"))?;
    }

    let window = builder
        .min_inner_size(860.0, 520.0)
        .center()
        .build()
        .map_err(|e| format!("باز کردن پنجره تراز آزمایشی ناموفق بود: {e}"))?;
    let _ = window.set_focus();
    Ok(())
}

/// Called once by the voucher window to learn which voucher it should open.
#[tauri::command]
fn take_voucher_request(window: WebviewWindow, state: State<'_, AppState>) -> Option<i64> {
    if require_window(&window, &["voucher"]).is_err() {
        return None;
    }
    lock(&state.pending_voucher).take().flatten()
}

/// Opens (or focuses) the account-group form in a normal OS window.
#[tauri::command(async)]
fn open_account_groups_window(
    app: AppHandle,
    caller: WebviewWindow,
) -> Result<(), String> {
    require_window(&caller, &["main"])?;
    if let Some(window) = app.get_webview_window("account-groups") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    let mut builder = WebviewWindowBuilder::new(
        &app,
        "account-groups",
        WebviewUrl::App("account-groups.html".into()),
    )
    .title("تعریف گروه حساب‌ها")
    .decorations(true)
    .minimizable(false)
    .maximizable(false)
    .resizable(true);

    if let Ok(Some(monitor)) = app.primary_monitor() {
        let logical = monitor.size().to_logical::<f64>(monitor.scale_factor());
        let width = (logical.width * 0.523).clamp(722.0, 903.0) * 0.9;
        let height = (logical.height * 0.741).clamp(570.0, 779.0) * 0.95;
        builder = builder.inner_size(width, height);
    } else {
        builder = builder.inner_size(747.0, 650.0);
    }

    if let Some(main) = app.get_webview_window("main") {
        builder = builder
            .parent(&main)
            .map_err(|e| format!("تعیین پنجره اصلی به‌عنوان مالک ناموفق بود: {e}"))?;
    }

    let window = builder
        .min_inner_size(650.0, 505.0)
        .center()
        .build()
        .map_err(|e| format!("باز کردن فرم گروه حساب ناموفق بود: {e}"))?;
    let _ = window.set_focus();
    Ok(())
}

#[tauri::command(async)]
fn open_account_definition_window(app: AppHandle, caller: WebviewWindow) -> Result<(), String> {
    require_window(&caller, &["main"])?;
    if let Some(window) = app.get_webview_window("account-definition") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    let mut builder = WebviewWindowBuilder::new(
        &app,
        "account-definition",
        WebviewUrl::App("account-definition.html".into()),
    )
    .title("تعریف حساب‌های کل، معین و تفصیلی")
    .decorations(true)
    .minimizable(false)
    .maximizable(false)
    .resizable(true);

    if let Ok(Some(monitor)) = app.primary_monitor() {
        let logical = monitor.size().to_logical::<f64>(monitor.scale_factor());
        builder = builder.inner_size(
            (logical.width * 0.82).clamp(900.0, 1320.0),
            (logical.height * 0.82).clamp(620.0, 900.0),
        );
    } else {
        builder = builder.inner_size(1120.0, 760.0);
    }

    if let Some(main) = app.get_webview_window("main") {
        builder = builder
            .parent(&main)
            .map_err(|e| format!("تعیین پنجره اصلی به‌عنوان مالک ناموفق بود: {e}"))?;
    }

    let window = builder
        .min_inner_size(800.0, 560.0)
        .center()
        .build()
        .map_err(|e| format!("باز کردن فرم تعریف حساب ناموفق بود: {e}"))?;
    let _ = window.set_focus();
    Ok(())
}

/// Opens (or focuses) the bulk voucher deletion form in its own native OS
/// window, with the same frame and classic accounting-form styling as the
/// account-definition form.
#[tauri::command(async)]
fn open_bulk_voucher_delete_window(app: AppHandle, caller: WebviewWindow) -> Result<(), String> {
    require_window(&caller, &["main"])?;
    if let Some(window) = app.get_webview_window("bulk-voucher-delete") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    let mut builder = WebviewWindowBuilder::new(
        &app,
        "bulk-voucher-delete",
        WebviewUrl::App("bulk-voucher-delete.html".into()),
    )
    .title("حذف دسته‌جمعی اسناد")
    .decorations(true)
    .minimizable(false)
    .maximizable(false)
    .resizable(true);

    if let Ok(Some(monitor)) = app.primary_monitor() {
        let logical = monitor.size().to_logical::<f64>(monitor.scale_factor());
        builder = builder.inner_size(
            (logical.width * 0.74).clamp(880.0, 1200.0),
            (logical.height * 0.80).clamp(600.0, 880.0),
        );
    } else {
        builder = builder.inner_size(1040.0, 740.0);
    }

    if let Some(main) = app.get_webview_window("main") {
        builder = builder
            .parent(&main)
            .map_err(|e| format!("تعیین پنجره اصلی به‌عنوان مالک ناموفق بود: {e}"))?;
    }

    let window = builder
        .min_inner_size(760.0, 540.0)
        .center()
        .build()
        .map_err(|e| format!("باز کردن فرم حذف دسته‌جمعی ناموفق بود: {e}"))?;
    let _ = window.set_focus();
    Ok(())
}

fn open_external(path: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer.exe")
            .arg(path)
            .spawn()
            .map_err(|e| format!("باز کردن فایل ناموفق بود: {e}"))?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        std::process::Command::new(opener)
            .arg(path)
            .spawn()
            .map_err(|e| format!("باز کردن فایل ناموفق بود: {e}"))?;
        Ok(())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .setup(|app| match db::initialize(app.handle()) {
            Ok((connection, path)) => {
                app.state::<AppState>().database.attach(connection, path);
                Ok(())
            }
            Err(message) => {
                let _ = rfd::MessageDialog::new()
                    .set_level(rfd::MessageLevel::Error)
                    .set_title("خطا در راه‌اندازی")
                    .set_description(&message)
                    .show();
                Err(std::io::Error::other(message).into())
            }
        })
        .on_window_event(|window, event| {
            // Child forms must not outlive the main window.
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { .. } = event {
                    for label in [
                        "voucher",
                        "voucher-list",
                        "account-groups",
                        "account-definition",
                        "bulk-voucher-delete",
                        "trial-balance",
                    ] {
                        if let Some(child) = window.app_handle().get_webview_window(label) {
                            let _ = child.close();
                        }
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_account_groups,
            create_account_group,
            update_account_group,
            delete_account_group,
            list_accounts,
            create_account,
            update_account,
            set_account_active,
            delete_account,
            delete_accounts_bulk,
            export_account_excel,
            import_account_excel,
            get_settings,
            save_settings,
            validate_voucher_date,
            next_voucher_number,
            adjacent_voucher_number,
            list_vouchers,
            get_voucher_count,
            get_voucher,
            transfer_voucher_lines,
            save_voucher,
            delete_voucher,
            export_voucher_excel,
            import_voucher_excel,
            analyze_bulk_voucher_excel,
            commit_bulk_voucher_excel,
            preview_voucher_reorganization,
            apply_voucher_reorganization,
            preview_bulk_voucher_delete,
            apply_bulk_voucher_delete,
            get_journal,
            get_ledger,
            get_trial_balance,
            get_income_statement,
            get_balance_sheet,
            open_journal_preview,
            open_ledger_preview,
            open_trial_balance_preview,
            open_income_statement_preview,
            open_balance_sheet_preview,
            open_voucher_preview,
            get_print_doc,
            save_report_html,
            backup_database,
            restore_database,
            retry_database,
            open_path,
            open_voucher_window,
            take_voucher_request,
            open_voucher_list_window,
            open_trial_balance_window,
            open_account_groups_window,
            open_account_definition_window,
            open_bulk_voucher_delete_window,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
