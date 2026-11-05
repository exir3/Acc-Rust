//! Voucher commands.

use tauri::{AppHandle, Emitter, State, WebviewWindow};

use crate::commands::state::{
    lock, notify_accounts_changed, notify_vouchers_changed, require_window, AppState,
};
use crate::excel;
use crate::vouchers::{self, VoucherInput};

/* ── Vouchers ───────────────────────────────────────────────── */

#[tauri::command(async)]
pub(crate) fn validate_voucher_date(date: String, window: WebviewWindow) -> Result<String, String> {
    require_window(&window, &["main", "voucher", "bulk-voucher-delete"])?;
    vouchers::validate_voucher_date(&date)
}

#[tauri::command(async)]
pub(crate) fn next_voucher_number(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    require_window(&window, &["main", "voucher"])?;
    state.database.next_voucher_number()
}

#[tauri::command(async)]
pub(crate) fn adjacent_voucher_number(
    number: i64,
    direction: String,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<i64>, String> {
    require_window(&window, &["main", "voucher"])?;
    state.database.adjacent_voucher_number(number, &direction)
}

#[tauri::command(async)]
pub(crate) fn get_voucher_count(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    require_window(&window, &["main", "voucher-list", "trial-balance"])?;
    state.database.voucher_count()
}

#[tauri::command(async)]
pub(crate) fn list_vouchers(
    from: String,
    to: String,
    search: String,
    offset: Option<i64>,
    limit: Option<i64>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<vouchers::VoucherList, String> {
    require_window(
        &window,
        &["main", "voucher", "voucher-list", "trial-balance"],
    )?;
    state.database.list_vouchers_page(
        &from,
        &to,
        &search,
        offset,
        limit.or(Some(vouchers::DEFAULT_VOUCHER_PAGE_SIZE)),
    )
}

#[tauri::command(async)]
pub(crate) fn get_voucher(
    number: i64,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<vouchers::Voucher>, String> {
    require_window(&window, &["main", "voucher", "voucher-list"])?;
    state.database.get_voucher(number)
}

#[tauri::command(async)]
pub(crate) fn voucher_counters(
    number: Option<i64>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<vouchers::VoucherCounters, String> {
    require_window(&window, &["main", "voucher"])?;
    state.database.voucher_counters(number)
}

#[tauri::command(async)]
pub(crate) fn transfer_voucher_lines(
    input: vouchers::VoucherTransferInput,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<vouchers::VoucherTransferResult, String> {
    require_window(&window, &["main", "voucher"])?;
    let result = state.database.transfer_voucher_lines(input)?;
    notify_vouchers_changed(&app);
    Ok(result)
}

#[tauri::command(async)]
pub(crate) fn save_voucher(
    input: VoucherInput,
    expected_updated_at: Option<String>,
    idempotency_key: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<vouchers::Voucher, String> {
    require_window(&window, &["main", "voucher"])?;
    let saved = state.database.save_voucher_idempotent(
        input,
        expected_updated_at.as_deref(),
        idempotency_key.as_deref(),
    )?;
    notify_vouchers_changed(&app);
    Ok(saved)
}

#[tauri::command(async)]
pub(crate) fn set_voucher_permanent(
    id: i64,
    expected_updated_at: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<vouchers::Voucher, String> {
    require_window(&window, &["main", "voucher"])?;
    let permanent = state
        .database
        .set_voucher_permanent(id, expected_updated_at.as_deref())?;
    notify_vouchers_changed(&app);
    Ok(permanent)
}

#[tauri::command(async)]
pub(crate) fn delete_voucher(
    id: i64,
    expected_updated_at: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher", "voucher-list"])?;
    state
        .database
        .delete_voucher(id, expected_updated_at.as_deref())?;
    notify_vouchers_changed(&app);
    Ok(())
}

#[tauri::command(async)]
pub(crate) fn export_voucher_excel(
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
pub(crate) fn import_voucher_excel(
    window: WebviewWindow,
) -> Result<Option<Vec<vouchers::VoucherLineInput>>, String> {
    require_window(&window, &["main", "voucher"])?;
    excel::import_voucher()
}

#[tauri::command(async)]
pub(crate) fn analyze_bulk_voucher_excel(
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
pub(crate) fn commit_bulk_voucher_excel(
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
    notify_vouchers_changed(&app);
    Ok(imported_count)
}

#[tauri::command(async)]
pub(crate) fn preview_voucher_reorganization(
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
pub(crate) fn apply_voucher_reorganization(
    operation: String,
    plan_token: String,
    app: AppHandle,
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
    notify_vouchers_changed(&app);
    Ok(result)
}

#[tauri::command(async)]
pub(crate) fn preview_bulk_voucher_delete(
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
pub(crate) fn apply_bulk_voucher_delete(
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
