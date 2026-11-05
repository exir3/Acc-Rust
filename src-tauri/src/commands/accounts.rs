//! Account-group and chart-of-accounts commands.

use tauri::{AppHandle, State, WebviewWindow};

use crate::commands::state::{notify_accounts_changed, require_window, AppState};
use crate::{db, excel};

/* ── Account groups ─────────────────────────────────────────── */

#[tauri::command(async)]
pub(crate) fn list_account_groups(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::AccountGroupList, String> {
    require_window(
        &window,
        &[
            "main",
            "voucher-list",
            "trial-balance",
            "voucher",
            "account-groups",
            "account-definition",
        ],
    )?;
    state.database.list_account_groups()
}

#[tauri::command(async)]
pub(crate) fn create_account_group(
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
pub(crate) fn update_account_group(
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
pub(crate) fn delete_account_group(
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
pub(crate) fn list_accounts(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::AccountList, String> {
    require_window(
        &window,
        &[
            "main",
            "voucher-list",
            "trial-balance",
            "voucher",
            "account-groups",
            "account-definition",
        ],
    )?;
    state.database.list_accounts()
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_account(
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
    require_window(&window, &["main", "voucher", "account-groups", "account-definition"])?;
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
pub(crate) fn update_account(
    id: i64,
    name: String,
    group_code: String,
    nature: String,
    parent_code: String,
    is_active: Option<bool>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::Account, String> {
    require_window(&window, &["main", "account-groups", "account-definition"])?;
    let account = if let Some(is_active) = is_active {
        state.database.update_account_with_active(
            id,
            &name,
            &group_code,
            &nature,
            &parent_code,
            is_active,
        )?
    } else {
        state
            .database
            .update_account(id, &name, &group_code, &nature, &parent_code)?
    };
    notify_accounts_changed(&app);
    Ok(account)
}

#[tauri::command(async)]
pub(crate) fn set_account_active(
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
pub(crate) fn delete_account(
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
pub(crate) fn delete_accounts_bulk(
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
pub(crate) fn export_account_excel(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<excel::AccountExportResult>, String> {
    require_window(&window, &["main", "account-groups", "account-definition"])?;
    excel::export_accounts(&state.database)
}

#[tauri::command(async)]
pub(crate) fn import_account_excel(
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
