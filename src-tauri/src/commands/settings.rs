//! Settings commands.

use tauri::{State, WebviewWindow};

use crate::commands::state::{require_window, AppState};
use crate::db;

/* ── Settings ───────────────────────────────────────────────── */

#[tauri::command(async)]
pub(crate) fn get_settings(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::Settings, String> {
    require_window(
        &window,
        &[
            "main",
            "voucher-list",
            "trial-balance",
            "voucher",
            "bulk-voucher-delete",
        ],
    )?;
    state.database.get_settings()
}

#[tauri::command(async)]
pub(crate) fn save_settings(
    company_name: String,
    fiscal_year: String,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::Settings, String> {
    require_window(&window, &["main"])?;
    state.database.save_settings(&company_name, &fiscal_year)
}
