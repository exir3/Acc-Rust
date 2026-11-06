//! Shared application state and window/notification helpers.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State, WebviewWindow};

use crate::{db, excel, report};

/// A poisoned lock only means another thread panicked while holding it; the
/// guarded value is plain data, so keep going instead of panicking too.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn notify_accounts_changed(app: &AppHandle) {
    let _ = app.emit("accounts-changed", ());
}

/// Tells every open window that vouchers changed, so a list or report in
/// another window refreshes instead of showing stale data.
pub(crate) fn notify_vouchers_changed(app: &AppHandle) {
    let _ = app.emit("vouchers-changed", ());
}

#[derive(Default)]
pub struct AppState {
    /// Reports waiting to be rendered, keyed by their preview-window label.
    pub(crate) print_docs: Mutex<HashMap<String, report::PrintDoc>>,
    /// Voucher number requested by the main window the next time the
    /// voucher form asks for one. `Some(None)` means "start a blank voucher".
    pub(crate) pending_voucher: Mutex<Option<Option<i64>>>,
    /// Local SQLite database.
    pub(crate) database: db::Database,
    /// Parsed workbook awaiting the user's bulk-import confirmation.
    pub(crate) bulk_voucher_import: Mutex<Option<excel::BulkVoucherImport>>,
    /// Parsed chart-of-accounts workbook awaiting the user's confirmation.
    /// The import command accepts only the token, so it cannot substitute a
    /// different path or file after the preview was shown.
    pub(crate) account_import_preview: Mutex<Option<excel::AccountImportSession>>,
    /// Set by the cancel command; the running parse or commit checks it and
    /// aborts, rolling back the import transaction.
    pub(crate) bulk_import_cancel: Arc<AtomicBool>,
    /// Reorganization preview awaiting explicit confirmation.
    pub(crate) reorganization_preview: Mutex<Option<(String, String)>>,
    /// Bulk-delete preview token awaiting explicit confirmation.
    pub(crate) bulk_delete_preview: Mutex<Option<String>>,
    /// Report files this session wrote, canonicalized. `open_path` may not
    /// open anything outside this allowlist.
    pub(crate) report_files: Mutex<HashSet<PathBuf>>,
}

pub(crate) fn report_meta(
    state: &State<'_, AppState>,
    from: &str,
    to: &str,
) -> Result<report::ReportMeta, String> {
    let settings = state.database.get_settings()?;
    let fiscal_year = state
        .database
        .fiscal_year_label_for_range(from, to)?
        .unwrap_or(settings.fiscal_year);
    Ok(report::ReportMeta {
        company: settings.company_name,
        fiscal_year,
        today: settings.today,
        show_print_date_time: true,
    })
}

/// Defense in depth: a compromised child window must not be able to run
/// commands that belong to another window.
pub(crate) fn require_window(window: &WebviewWindow, allowed: &[&str]) -> Result<(), String> {
    if allowed.contains(&window.label()) {
        Ok(())
    } else {
        Err("این عملیات از پنجره جاری مجاز نیست.".to_string())
    }
}

pub(crate) fn require_preview_window(window: &WebviewWindow) -> Result<(), String> {
    if window.label().starts_with("print-preview-") {
        Ok(())
    } else {
        Err("این عملیات فقط از پنجره پیش‌نمایش چاپ مجاز است.".to_string())
    }
}

pub(crate) fn require_main_or_preview_window(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "main" || window.label().starts_with("print-preview-") {
        Ok(())
    } else {
        Err("این عملیات از پنجره جاری مجاز نیست.".to_string())
    }
}
