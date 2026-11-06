//! Database backup, restore and report-file commands.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use tauri::{AppHandle, Emitter, State, WebviewWindow};

use crate::commands::state::{
    lock, notify_accounts_changed, require_main_or_preview_window, require_window, AppState,
    RestoreSelection,
};
use crate::commands::windows::open_external;
use crate::db;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RestoreBackupChoice {
    token: String,
    encrypted: bool,
    file_name: String,
}

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
pub(crate) fn backup_database_encrypted(
    window: WebviewWindow,
    state: State<'_, AppState>,
    password: String,
) -> Result<Option<String>, String> {
    require_window(&window, &["main"])?;
    db::validate_portable_password(&password)?;
    let stamp = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("زمان سیستم معتبر نیست: {error}"))?
        .as_nanos();
    let suggested = format!("hesabdari-backup-{stamp}.{}", db::PORTABLE_EXTENSION);
    let Some(path) = rfd::FileDialog::new()
        .set_title("ذخیرهٔ پشتیبان رمزنگاری‌شده")
        .set_file_name(&suggested)
        .add_filter("پشتیبان رمزنگاری‌شدهٔ هسابداری", &[db::PORTABLE_EXTENSION])
        .save_file()
    else {
        return Ok(None);
    };
    let path = if path.extension().is_none() {
        path.with_extension(db::PORTABLE_EXTENSION)
    } else {
        path
    };
    state.database.backup_encrypted_to(&path, &password)?;
    Ok(Some(path.to_string_lossy().to_string()))
}

/// Opens the backup picker and binds the chosen file to an opaque token and a
/// hash. The renderer never supplies a path for restore.
#[tauri::command(async)]
pub(crate) fn choose_restore_backup(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<RestoreBackupChoice>, String> {
    require_window(&window, &["main"])?;
    let Some(path) = rfd::FileDialog::new()
        .set_title("انتخاب پشتیبان برای بازیابی")
        .add_filter(
            "پشتیبان هسابداری",
            &[db::PORTABLE_EXTENSION, "sqlite3", "db"],
        )
        .pick_file()
    else {
        return Ok(None);
    };
    let path = std::fs::canonicalize(&path)
        .map_err(|error| format!("مسیر فایل پشتیبان معتبر نیست: {error}"))?;
    let encrypted = db::is_encrypted_backup(&path)?;
    let file_sha256 = db::sha256_file(&path)?;
    let token = db::random_hex(16)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("backup")
        .to_string();
    *lock(&state.restore_selection) = Some(RestoreSelection {
        token: token.clone(),
        path,
        encrypted,
        file_sha256,
        created_at: SystemTime::now(),
    });
    Ok(Some(RestoreBackupChoice {
        token,
        encrypted,
        file_name,
    }))
}

#[tauri::command(async)]
pub(crate) fn restore_database_file(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    token: String,
    password: Option<String>,
) -> Result<String, String> {
    require_window(&window, &["main"])?;
    let selection = lock(&state.restore_selection)
        .take()
        .ok_or_else(|| "فایل پشتیبان انتخاب‌شده منقضی شده است؛ دوباره انتخاب کنید.".to_string())?;
    if selection.token != token {
        return Err("توکن بازیابی معتبر نیست؛ دوباره فایل را انتخاب کنید.".to_string());
    }
    if selection
        .created_at
        .elapsed()
        .map(|elapsed| elapsed > Duration::from_secs(15 * 60))
        .unwrap_or(true)
    {
        return Err("انتخاب فایل پشتیبان منقضی شده است؛ دوباره فایل را انتخاب کنید.".to_string());
    }
    let current_hash = db::sha256_file(&selection.path)?;
    if current_hash != selection.file_sha256 {
        return Err("فایل پشتیبان پس از انتخاب تغییر کرده است و بازیابی انجام نشد.".to_string());
    }

    let database_path = PathBuf::from(state.database.path_string());
    let safety_path = if selection.encrypted {
        let password = password
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "این پشتیبان رمزنگاری‌شده است؛ رمز عبور را وارد کنید.".to_string())?;
        state
            .database
            .restore_encrypted_from(&selection.path, &password)?
    } else {
        state.database.restore_from(&selection.path)?
    };
    let _ = db::prune_restore_safety_files(&database_path);
    notify_accounts_changed(&app);
    let _ = app.emit("database-restored", ());
    Ok(safety_path.to_string_lossy().to_string())
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
/// only ask for an HTML file this session wrote through `save_report_html`,
/// never for an arbitrary program or file.
#[tauri::command]
pub(crate) fn open_path(
    path: String,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
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
    let canonical = std::fs::canonicalize(target)
        .map_err(|_| "فقط فایل HTML ذخیره‌شده از گزارش‌ها قابل باز شدن است.".to_string())?;
    if !lock(&state.report_files).contains(&canonical) {
        return Err("فقط فایل HTML ذخیره‌شده از گزارش‌ها قابل باز شدن است.".to_string());
    }
    open_external(&path)
}

/* ── Fiscal years ───────────────────────────────────────────── */

#[tauri::command(async)]
pub(crate) fn list_fiscal_years(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::FiscalYearList, String> {
    require_window(&window, &["main"])?;
    state.database.list_fiscal_years()
}

#[tauri::command(async)]
pub(crate) fn create_fiscal_year(
    title: String,
    start_date: String,
    end_date: String,
    status: Option<String>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::FiscalYear, String> {
    require_window(&window, &["main"])?;
    state.database.create_fiscal_year(
        &title,
        &start_date,
        &end_date,
        status.as_deref().unwrap_or("open"),
    )
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_fiscal_year(
    id: i64,
    title: String,
    start_date: String,
    end_date: String,
    status: String,
    reason: Option<String>,
    confirm: Option<bool>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::FiscalYear, String> {
    require_window(&window, &["main"])?;
    state.database.update_fiscal_year(
        id,
        &title,
        &start_date,
        &end_date,
        &status,
        reason.as_deref(),
        confirm.unwrap_or(false),
    )
}

#[tauri::command(async)]
pub(crate) fn set_fiscal_year_status(
    id: i64,
    status: String,
    reason: Option<String>,
    confirm: Option<bool>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::FiscalYear, String> {
    require_window(&window, &["main"])?;
    state
        .database
        .set_fiscal_year_status(id, &status, reason.as_deref(), confirm.unwrap_or(false))
}

#[tauri::command(async)]
pub(crate) fn delete_fiscal_year(
    id: i64,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main"])?;
    state.database.delete_fiscal_year(id)
}

/* ── Audit history ─────────────────────────────────────────── */

#[tauri::command(async)]
pub(crate) fn get_audit_logs(
    limit: Option<i64>,
    offset: Option<i64>,
    entity_type: Option<String>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::AuditLogList, String> {
    require_window(&window, &["main"])?;
    state
        .database
        .list_audit_logs(limit, offset, entity_type.as_deref())
}

/* ── Legacy level-4 migration ──────────────────────────────── */

#[tauri::command(async)]
pub(crate) fn analyze_legacy_level4(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::LegacyLevel4Analysis, String> {
    require_window(&window, &["main"])?;
    state.database.analyze_legacy_level4()
}

/// Runs the explicit level-4 conversion. The caller must confirm; the
/// migration writes and validates its own backup before changing any row.
#[tauri::command(async)]
pub(crate) fn apply_legacy_level4_migration(
    confirm: bool,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::LegacyLevel4MigrationResult, String> {
    require_window(&window, &["main"])?;
    if !confirm {
        return Err("برای اجرای مهاجرت سطح ۴، تأیید صریح لازم است.".to_string());
    }
    let database_path = std::path::PathBuf::from(state.database.path_string());
    let backup_path = db::legacy_migration_backup_path(&database_path);
    let result = state.database.apply_legacy_level4_migration(&backup_path)?;
    notify_accounts_changed(&app);
    let _ = app.emit("database-restored", ());
    Ok(result)
}

#[tauri::command(async)]
pub(crate) fn preview_legacy_level4_resolution(
    account_code: String,
    detail1_code: String,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::LegacyLevel4ResolutionPreview, String> {
    require_window(&window, &["main"])?;
    state
        .database
        .preview_legacy_level4_resolution(&account_code, &detail1_code)
}

/// Applies an operator-chosen mapping for one ambiguous level-4 account. The
/// command writes its own backup before touching any line.
#[tauri::command(async)]
pub(crate) fn resolve_legacy_level4_account(
    account_code: String,
    detail1_code: String,
    confirm: bool,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::LegacyLevel4MigrationResult, String> {
    require_window(&window, &["main"])?;
    if !confirm {
        return Err("برای تبدیل حساب سطح ۴، تأیید صریح لازم است.".to_string());
    }
    let database_path = std::path::PathBuf::from(state.database.path_string());
    let backup_path = db::legacy_migration_backup_path(&database_path);
    let result =
        state
            .database
            .resolve_legacy_level4_account(&account_code, &detail1_code, &backup_path)?;
    notify_accounts_changed(&app);
    let _ = app.emit("database-restored", ());
    Ok(result)
}
