//! Account-group and chart-of-accounts commands.

use tauri::{AppHandle, State, WebviewWindow};

use crate::commands::state::{lock, notify_accounts_changed, require_window, AppState};
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

// A Tauri command receives one argument per JS field, so this signature is
// fixed by the IPC contract rather than by internal style.
#[allow(clippy::too_many_arguments)]
#[tauri::command(async)]
pub(crate) fn update_account_group(
    id: i64,
    code: String,
    name: String,
    group_type: String,
    default_section: Option<String>,
    confirm_reclassification: Option<bool>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::AccountGroup, String> {
    require_window(&window, &["main", "account-groups"])?;
    let group = state.database.update_account_group_confirmed(
        id,
        &code,
        &name,
        &group_type,
        default_section.as_deref().unwrap_or(""),
        confirm_reclassification.unwrap_or(false),
    )?;
    notify_accounts_changed(&app);
    Ok(group)
}

#[tauri::command(async)]
pub(crate) fn preview_account_group_classification_change(
    id: i64,
    group_type: String,
    default_section: Option<String>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::AccountGroupClassificationImpact, String> {
    require_window(&window, &["main", "account-groups"])?;
    state.database.preview_account_group_classification_change(
        id,
        &group_type,
        default_section.as_deref().unwrap_or(""),
    )
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
pub(crate) fn list_fs_lines(window: WebviewWindow) -> Result<db::FsCatalog, String> {
    require_window(
        &window,
        &["main", "voucher", "account-groups", "account-definition"],
    )?;
    Ok(db::fs_catalog::fs_catalog())
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
    fs_line: Option<String>,
    contra_of: Option<String>,
    contra_kind: Option<String>,
    opposite_fs_line: Option<String>,
    split_by_sign: Option<bool>,
    system_role: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::Account, String> {
    require_window(
        &window,
        &["main", "voucher", "account-groups", "account-definition"],
    )?;
    let account = db::NewAccount::new(
        &code,
        &name,
        level,
        &group_code,
        &nature,
        &parent_code,
        is_postable,
    )
    .with_active(is_active.unwrap_or(true))
    .with_fs_line(fs_line.as_deref().unwrap_or(""))
    .with_classification(
        contra_of.as_deref().unwrap_or(""),
        contra_kind.as_deref().unwrap_or(""),
        opposite_fs_line.as_deref().unwrap_or(""),
        split_by_sign.unwrap_or(false),
        system_role.as_deref().unwrap_or(""),
    );
    let account = state
        .database
        .create_accounts_atomic(vec![account])?
        .pop()
        .ok_or_else(|| "ساخت حساب انجام نشد.".to_string())?;
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
    fs_line: Option<String>,
    contra_of: Option<String>,
    contra_kind: Option<String>,
    opposite_fs_line: Option<String>,
    split_by_sign: Option<bool>,
    system_role: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<db::Account, String> {
    require_window(&window, &["main", "account-groups", "account-definition"])?;
    let has_classification = fs_line.is_some()
        || contra_of.is_some()
        || contra_kind.is_some()
        || opposite_fs_line.is_some()
        || split_by_sign.is_some()
        || system_role.is_some();
    let account = if has_classification {
        state.database.update_account_with_classification(
            id,
            &name,
            &group_code,
            &nature,
            &parent_code,
            is_active,
            db::ClassificationUpdate {
                fs_line: fs_line.unwrap_or_default(),
                contra_of: contra_of.unwrap_or_default(),
                contra_kind: contra_kind.unwrap_or_default(),
                opposite_fs_line: opposite_fs_line.unwrap_or_default(),
                split_by_sign: split_by_sign.unwrap_or(false),
                system_role: system_role.unwrap_or_default(),
            },
        )?
    } else if let Some(is_active) = is_active {
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
pub(crate) fn preview_account_excel_import(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<excel::AccountImportPreview>, String> {
    require_window(&window, &["main", "account-groups", "account-definition"])?;
    let Some((preview, session)) = excel::preview_accounts(&state.database)? else {
        return Ok(None);
    };
    *lock(&state.account_import_preview) = Some(session);
    Ok(Some(preview))
}

#[tauri::command(async)]
pub(crate) fn import_account_excel(
    preview_token: String,
    mode: String,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<excel::AccountImportResult, String> {
    require_window(&window, &["main", "account-groups", "account-definition"])?;
    // Consume the session only when the token matches. A stale or forged token
    // cannot apply a plan that was never previewed in this process.
    let session = {
        let mut guard = lock(&state.account_import_preview);
        match guard.as_ref() {
            Some(session) if session.token == preview_token => guard.take(),
            _ => None,
        }
    };
    let Some(session) = session else {
        return Err(
            "پیش‌نمایش ورود اکسل معتبر نیست یا منقضی شده است؛ دوباره پیش‌نمایش بگیرید.".to_string(),
        );
    };
    let result = excel::import_accounts(&state.database, session, &mode)?;
    if result.created_count > 0 {
        notify_accounts_changed(&app);
    }
    Ok(result)
}
