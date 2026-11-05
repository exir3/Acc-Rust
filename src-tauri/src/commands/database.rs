//! Database backup, restore and report-file commands.

use tauri::{AppHandle, Emitter, State, WebviewWindow};

use crate::commands::state::{
    notify_accounts_changed, require_main_or_preview_window, require_window, AppState,
};
use crate::commands::windows::open_external;
use crate::db;

#[tauri::command(async)]
pub(crate) fn backup_database(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    require_window(&window, &["main"])?;
    let database_path = std::path::PathBuf::from(state.database.path_string());
    let backup_dir = database_path
        .parent()
        .ok_or_else(|| "مسیر پوشه پایگاه داده پیدا نشد.".to_string())?
        .join("backups");
    std::fs::create_dir_all(&backup_dir)
        .map_err(|error| format!("ساخت پوشه پشتیبان ناموفق بود: {error}"))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("زمان سیستم معتبر نیست: {error}"))?
        .as_nanos();
    let path = backup_dir.join(format!("hesabdari-backup-{stamp}.sqlite3"));
    state.database.backup_to(&path)?;
    // Retention is best-effort: a successful backup must not be reported as a
    // failure just because Windows kept an old file locked.
    let _ = db::prune_backup_files(&backup_dir);
    Ok(Some(path.to_string_lossy().to_string()))
}

#[tauri::command(async)]
pub(crate) fn restore_database(
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
    let database_path = std::path::PathBuf::from(state.database.path_string());
    let safety_path = state.database.restore_from(&path)?;
    let _ = db::prune_restore_safety_files(&database_path);
    notify_accounts_changed(&app);
    let _ = app.emit("database-restored", ());
    Ok(Some(safety_path.to_string_lossy().to_string()))
}

/// Reopens the database after a failed restore left the application blocked.
/// A failed reopen keeps the block in place so an uncertain file is never used.
#[tauri::command(async)]
pub(crate) fn retry_database(
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
pub(crate) fn open_path(path: String, window: WebviewWindow) -> Result<(), String> {
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
