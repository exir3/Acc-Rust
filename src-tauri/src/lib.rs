//! Tauri backend for the accounting application.
//!
//! Accounts, vouchers and every figure shown by the UI live in one local
//! SQLite database. The commands below are the only way the webview can read
//! or change that data.

pub mod db;
pub mod excel;
pub mod report;
pub mod vouchers;

use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use vouchers::VoucherInput;

/// A poisoned lock only means another thread panicked while holding it; the
/// guarded value is plain data, so keep going instead of panicking too.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Default)]
pub struct AppState {
    /// Latest report waiting to be rendered by the preview window.
    print_doc: Mutex<Option<report::PrintDoc>>,
    /// Voucher number requested by the main window the next time the
    /// voucher form asks for one. `Some(None)` means "start a blank voucher".
    pending_voucher: Mutex<Option<Option<i64>>>,
    /// Local SQLite database.
    database: db::Database,
    /// Parsed workbook awaiting the user's bulk-import confirmation.
    bulk_voucher_import: Mutex<Option<excel::BulkVoucherImport>>,
    /// Reorganization preview awaiting explicit confirmation.
    reorganization_preview: Mutex<Option<(String, String)>>,
}

fn report_meta(state: &State<'_, AppState>) -> Result<report::ReportMeta, String> {
    let settings = state.database.get_settings()?;
    Ok(report::ReportMeta {
        company: settings.company_name,
        fiscal_year: settings.fiscal_year,
        today: settings.today,
    })
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
    state: State<'_, AppState>,
) -> Result<db::AccountGroup, String> {
    state
        .database
        .create_account_group(&code, &name, &group_type)
}

#[tauri::command(async)]
fn update_account_group(
    id: i64,
    code: String,
    name: String,
    group_type: String,
    state: State<'_, AppState>,
) -> Result<db::AccountGroup, String> {
    state
        .database
        .update_account_group(id, &code, &name, &group_type)
}

#[tauri::command(async)]
fn delete_account_group(id: i64, state: State<'_, AppState>) -> Result<(), String> {
    state.database.delete_account_group(id)
}

/* ── Chart of accounts ──────────────────────────────────────── */

#[tauri::command(async)]
fn list_accounts(state: State<'_, AppState>) -> Result<db::AccountList, String> {
    state.database.list_accounts()
}

#[tauri::command(async)]
fn create_account(
    code: String,
    name: String,
    level: i64,
    group_code: String,
    nature: String,
    parent_code: String,
    is_postable: bool,
    state: State<'_, AppState>,
) -> Result<db::Account, String> {
    state.database.create_account(
        &code,
        &name,
        level,
        &group_code,
        &nature,
        &parent_code,
        is_postable,
    )
}

#[tauri::command(async)]
fn update_account(
    id: i64,
    name: String,
    group_code: String,
    nature: String,
    parent_code: String,
    is_postable: bool,
    state: State<'_, AppState>,
) -> Result<db::Account, String> {
    state
        .database
        .update_account(id, &name, &group_code, &nature, &parent_code, is_postable)
}

#[tauri::command(async)]
fn delete_account(id: i64, state: State<'_, AppState>) -> Result<(), String> {
    state.database.delete_account(id)
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
    state: State<'_, AppState>,
) -> Result<db::Settings, String> {
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
fn list_vouchers(
    from: String,
    to: String,
    search: String,
    state: State<'_, AppState>,
) -> Result<vouchers::VoucherList, String> {
    state.database.list_vouchers(&from, &to, &search)
}

#[tauri::command(async)]
fn get_voucher(
    number: i64,
    state: State<'_, AppState>,
) -> Result<Option<vouchers::Voucher>, String> {
    state.database.get_voucher(number)
}

#[tauri::command(async)]
fn save_voucher(
    input: VoucherInput,
    state: State<'_, AppState>,
) -> Result<vouchers::Voucher, String> {
    state.database.save_voucher(input)
}

#[tauri::command(async)]
fn delete_voucher(id: i64, state: State<'_, AppState>) -> Result<(), String> {
    state.database.delete_voucher(id)
}

#[tauri::command(async)]
fn export_voucher_excel(input: VoucherInput) -> Result<Option<String>, String> {
    excel::export_voucher(&input)
}

#[tauri::command(async)]
fn import_voucher_excel() -> Result<Option<Vec<vouchers::VoucherLineInput>>, String> {
    excel::import_voucher()
}

#[tauri::command(async)]
fn analyze_bulk_voucher_excel(
    state: State<'_, AppState>,
) -> Result<Option<excel::BulkVoucherPreview>, String> {
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
    state: State<'_, AppState>,
) -> Result<usize, String> {
    let import = lock(&state.bulk_voucher_import)
        .clone()
        .ok_or_else(|| "ابتدا فایل اکسل را انتخاب و تحلیل کنید.".to_string())?;
    let imported_count = state.database.import_vouchers(import, renumber_conflicts)?;
    *lock(&state.bulk_voucher_import) = None;
    Ok(imported_count)
}

#[tauri::command(async)]
fn preview_voucher_reorganization(
    operation: String,
    state: State<'_, AppState>,
) -> Result<vouchers::VoucherReorganizationPreview, String> {
    *lock(&state.reorganization_preview) = None;
    let preview = state.database.preview_voucher_reorganization(&operation)?;
    *lock(&state.reorganization_preview) = Some((operation, preview.plan_token.clone()));
    Ok(preview)
}

#[tauri::command(async)]
fn apply_voucher_reorganization(
    operation: String,
    plan_token: String,
    state: State<'_, AppState>,
) -> Result<vouchers::VoucherReorganizationResult, String> {
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

/* ── Reports ────────────────────────────────────────────────── */

#[tauri::command(async)]
fn get_journal(
    from: String,
    to: String,
    search: String,
    state: State<'_, AppState>,
) -> Result<vouchers::JournalReport, String> {
    state.database.get_journal(&from, &to, &search)
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
    state: State<'_, AppState>,
) -> Result<vouchers::TrialBalanceReport, String> {
    state.database.get_trial_balance(&from, &to)
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

#[tauri::command(async)]
fn get_dashboard(state: State<'_, AppState>) -> Result<vouchers::Dashboard, String> {
    state.database.get_dashboard()
}

/* ── Print preview ──────────────────────────────────────────── */

#[tauri::command(async)]
fn open_journal_preview(
    from: String,
    to: String,
    search: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
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
    state: State<'_, AppState>,
) -> Result<(), String> {
    let report = state.database.get_ledger(&account_code, &from, &to)?;
    let meta = report_meta(&state)?;
    show_preview_window(&app, report::ledger_print_doc(&report, &meta))
}

#[tauri::command(async)]
fn open_trial_balance_preview(
    from: String,
    to: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let report = state.database.get_trial_balance(&from, &to)?;
    let meta = report_meta(&state)?;
    show_preview_window(&app, report::trial_balance_print_doc(&report, &meta))
}

#[tauri::command(async)]
fn open_income_statement_preview(
    from: String,
    to: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let report = state.database.get_income_statement(&from, &to)?;
    let meta = report_meta(&state)?;
    show_preview_window(&app, report::income_statement_print_doc(&report, &meta))
}

#[tauri::command(async)]
fn open_balance_sheet_preview(
    as_of: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let report = state.database.get_balance_sheet(&as_of)?;
    let meta = report_meta(&state)?;
    show_preview_window(&app, report::balance_sheet_print_doc(&report, &meta))
}

#[tauri::command(async)]
fn open_voucher_preview(
    number: i64,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let voucher = state
        .database
        .get_voucher(number)?
        .ok_or_else(|| "برای چاپ، ابتدا سند را ذخیره کنید.".to_string())?;
    let meta = report_meta(&state)?;
    show_preview_window(&app, report::voucher_print_doc(&voucher, &meta))
}

/// The preview window asks for the document it should render.
#[tauri::command]
fn get_print_doc(state: State<'_, AppState>) -> Result<report::PrintDoc, String> {
    lock(&state.print_doc)
        .clone()
        .ok_or_else(|| "گزارشی برای پیش‌نمایش آماده نیست.".to_string())
}

/// Saves the standalone (browser-printable) HTML, used by the preview toolbar.
#[tauri::command]
fn save_report_html(default_name: String, html: String) -> Result<Option<String>, String> {
    let Some(picked) = rfd::FileDialog::new()
        .set_title("ذخیره گزارش قابل چاپ")
        .set_file_name(default_name)
        .add_filter("HTML", &["html"])
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

/// Opens a report the user saved from the preview window. The webview may
/// only ask for an existing HTML file, never for an arbitrary program.
#[tauri::command]
fn open_path(path: String) -> Result<(), String> {
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

/// Keeps a single preview window open: stores the new document and rebuilds
/// the window so it always shows the latest report.
fn show_preview_window(app: &AppHandle, doc: report::PrintDoc) -> Result<(), String> {
    let state = app.state::<AppState>();
    *lock(&state.print_doc) = Some(doc);

    for (label, window) in app.webview_windows() {
        if label.starts_with("print-preview") {
            let _ = window.close();
        }
    }

    let label = format!("print-preview-{}", stamp());
    let mut builder = WebviewWindowBuilder::new(app, label, WebviewUrl::App("print.html".into()))
        .title("پیش‌نمایش چاپ — نرم‌افزار حسابداری")
        .inner_size(1180.0, 900.0)
        .min_inner_size(720.0, 520.0)
        .center()
        .resizable(true);

    if let Some(main) = app.get_webview_window("main") {
        builder = builder
            .parent(&main)
            .map_err(|e| format!("تعیین پنجره اصلی به‌عنوان مالک ناموفق بود: {e}"))?;
    }

    let window = builder
        .build()
        .map_err(|e| format!("باز کردن پنجره پیش‌نمایش ناموفق بود: {e}"))?;
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
    state: State<'_, AppState>,
) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("voucher") {
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
            .resizable(true);

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

/// Called once by the voucher window to learn which voucher it should open.
#[tauri::command]
fn take_voucher_request(state: State<'_, AppState>) -> Option<i64> {
    lock(&state.pending_voucher).take().flatten()
}

/// Opens (or focuses) the account-group form in a normal OS window.
#[tauri::command(async)]
fn open_account_groups_window(app: AppHandle) -> Result<(), String> {
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
        .setup(|app| {
            let (connection, path) = db::initialize(app.handle()).map_err(std::io::Error::other)?;
            app.state::<AppState>()
                .database
                .attach(connection, path);
            Ok(())
        })
        .on_window_event(|window, event| {
            // Child forms must not outlive the main window.
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { .. } = event {
                    for label in ["voucher", "account-groups"] {
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
            delete_account,
            get_settings,
            save_settings,
            validate_voucher_date,
            next_voucher_number,
            list_vouchers,
            get_voucher,
            save_voucher,
            delete_voucher,
            export_voucher_excel,
            import_voucher_excel,
            analyze_bulk_voucher_excel,
            commit_bulk_voucher_excel,
            preview_voucher_reorganization,
            apply_voucher_reorganization,
            get_journal,
            get_ledger,
            get_trial_balance,
            get_income_statement,
            get_balance_sheet,
            get_dashboard,
            open_journal_preview,
            open_ledger_preview,
            open_trial_balance_preview,
            open_income_statement_preview,
            open_balance_sheet_preview,
            open_voucher_preview,
            get_print_doc,
            save_report_html,
            open_path,
            open_voucher_window,
            take_voucher_request,
            open_account_groups_window,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
