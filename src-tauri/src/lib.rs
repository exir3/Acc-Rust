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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .setup(|app| match db::initialize(app.handle()) {
            Ok((connection, path)) => {
                app.state::<AppState>().database.attach(connection, path);
                Ok(())
            }
            Err(message) => {
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
            set_account_active,
            delete_account,
            delete_accounts_bulk,
            export_account_excel,
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
            open_journal_preview,
            open_ledger_preview,
            open_trial_balance_preview,
            open_income_statement_preview,
            open_balance_sheet_preview,
            open_voucher_preview,
            get_print_doc,
            save_report_html,
            backup_database,
            restore_database,
            retry_database,
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
