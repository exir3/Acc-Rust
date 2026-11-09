//! Tauri backend for the accounting application.
//!
//! Accounts, vouchers and every figure shown by the UI live in one local
//! SQLite database. The commands in [`commands`] are the only way the webview
//! can read or change that data.

pub mod db;
pub mod excel;
pub mod report;
pub mod vouchers;

mod commands;
mod limits;

use commands::*;
use tauri::Manager;

/// Release builds abort on panic, so without a hook the only trace of a crash
/// can disappear with the process. Append the location and message to a temp
/// log before the default hook (or the abort) runs. Logging must never panic
/// itself, otherwise a broken filesystem would turn a report into a loop.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            })
            .unwrap_or_else(|| "unknown".to_string());
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("unnamed");
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|value| (*value).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic payload>".to_string());
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        let line = format!(
            "version={} unix_time={stamp} thread={thread_name} at {location}: {payload}\n",
            env!("CARGO_PKG_VERSION")
        );
        let log_path = std::env::temp_dir().join("hesabdari-panic.log");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
        {
            use std::io::Write;
            let _ = file.write_all(line.as_bytes());
        }
        default_hook(info);
    }));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_panic_hook();
    tauri::Builder::default()
        .manage(AppState::default())
        .setup(|app| match db::initialize(app.handle()) {
            Ok((connection, path)) => {
                app.state::<AppState>().database.attach(connection, path);
                Ok(())
            }
            Err(message) => {
                // A modal dialog is invisible to CI and to a post-mortem log.
                // Mirror the startup failure to stderr and a temp file so the
                // Windows smoke test and users can see the real cause.
                let log_path = std::env::temp_dir().join("hesabdari-startup-error.log");
                let log_line = format!(
                    "version={} data_dir_override={:?}\n{message}\n",
                    env!("CARGO_PKG_VERSION"),
                    std::env::var_os("HESABDARI_SMOKE_DATA_DIR")
                );
                let _ = std::fs::write(&log_path, log_line);
                eprintln!("[hesabdari] startup failed: {message}");
                eprintln!("[hesabdari] startup log: {}", log_path.display());
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
                    // Print preview windows are created with a dynamic
                    // `print-preview-*` label for every report, so they cannot
                    // be listed by name. Close all of them with the main window.
                    for (label, child) in window.app_handle().webview_windows() {
                        if label.starts_with("print-preview") {
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
            preview_account_group_classification_change,
            delete_account_group,
            list_accounts,
            list_fs_lines,
            create_account,
            update_account,
            set_account_active,
            delete_account,
            delete_accounts_bulk,
            export_account_excel,
            preview_account_excel_import,
            import_account_excel,
            get_settings,
            save_settings,
            validate_voucher_date,
            next_voucher_number,
            adjacent_voucher_number,
            list_vouchers,
            get_voucher_count,
            get_voucher,
            voucher_counters,
            transfer_voucher_lines,
            save_voucher,
            set_voucher_permanent,
            delete_voucher,
            export_voucher_excel,
            import_voucher_excel,
            analyze_bulk_voucher_excel,
            cancel_bulk_voucher_import,
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
            get_equity_changes_statement,
            get_cogs_statement,
            get_profit_appropriation_statement,
            get_cash_flow_statement,
            open_journal_preview,
            open_ledger_preview,
            open_trial_balance_preview,
            open_income_statement_preview,
            open_balance_sheet_preview,
            open_equity_changes_preview,
            open_cogs_preview,
            open_profit_appropriation_preview,
            open_cash_flow_preview,
            open_voucher_preview,
            get_print_doc,
            save_report_html,
            backup_database,
            backup_database_encrypted,
            choose_restore_backup,
            restore_database_file,
            retry_database,
            list_fiscal_years,
            create_fiscal_year,
            update_fiscal_year,
            set_fiscal_year_status,
            delete_fiscal_year,
            get_closing_preview,
            assign_closing_roles,
            execute_closing,
            reverse_closing,
            get_carry_forward_preview,
            execute_carry_forward,
            get_audit_logs,
            analyze_legacy_level4,
            apply_legacy_level4_migration,
            preview_legacy_level4_resolution,
            resolve_legacy_level4_account,
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
