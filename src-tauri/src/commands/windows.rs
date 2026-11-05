//! Native child-window commands and preview-window management.

use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::commands::state::{lock, require_window, AppState};
use crate::report;

/* ── Windows ────────────────────────────────────────────────── */

pub(crate) fn stamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

static PREVIEW_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Keeps a single preview window open: stores the new document and rebuilds
/// the window so it always shows the latest report.
pub(crate) fn show_preview_window(app: &AppHandle, doc: report::PrintDoc) -> Result<(), String> {
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
pub(crate) fn open_voucher_window(
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
pub(crate) fn open_voucher_list_window(
    app: AppHandle,
    caller: WebviewWindow,
) -> Result<(), String> {
    require_window(&caller, &["main"])?;
    if let Some(window) = app.get_webview_window("voucher-list") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    let mut builder =
        WebviewWindowBuilder::new(&app, "voucher-list", WebviewUrl::App("index.html".into()))
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
pub(crate) fn open_trial_balance_window(
    app: AppHandle,
    caller: WebviewWindow,
) -> Result<(), String> {
    require_window(&caller, &["main"])?;
    if let Some(window) = app.get_webview_window("trial-balance") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    let mut builder =
        WebviewWindowBuilder::new(&app, "trial-balance", WebviewUrl::App("index.html".into()))
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
pub(crate) fn take_voucher_request(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Option<i64> {
    if require_window(&window, &["voucher"]).is_err() {
        return None;
    }
    lock(&state.pending_voucher).take().flatten()
}

/// Opens (or focuses) the account-group form in a normal OS window.
#[tauri::command(async)]
pub(crate) fn open_account_groups_window(
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
pub(crate) fn open_account_definition_window(
    app: AppHandle,
    caller: WebviewWindow,
) -> Result<(), String> {
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
            (logical.width * 0.70).clamp(900.0, 1000.0),
            (logical.height * 0.78).clamp(640.0, 720.0),
        );
    } else {
        builder = builder.inner_size(960.0, 680.0);
    }

    if let Some(main) = app.get_webview_window("main") {
        builder = builder
            .parent(&main)
            .map_err(|e| format!("تعیین پنجره اصلی به‌عنوان مالک ناموفق بود: {e}"))?;
    }

    let window = builder
        .min_inner_size(840.0, 600.0)
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
pub(crate) fn open_bulk_voucher_delete_window(
    app: AppHandle,
    caller: WebviewWindow,
) -> Result<(), String> {
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

pub(crate) fn open_external(path: &str) -> Result<(), String> {
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
