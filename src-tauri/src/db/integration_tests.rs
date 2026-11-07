//! Real on-disk SQLite integration tests.
//!
//! These tests call the same public database API that the Tauri commands call,
//! but against a temporary `.sqlite3` file instead of an in-memory database.
//! They cover the backend half of the UI → Tauri → Rust → SQLite → response
//! chain: account hierarchy, voucher lifecycle, permanent immutability, closed
//! fiscal years, backup/restore and two connections editing the same file. The
//! UI half is covered by the Playwright specs and the Windows smoke test.

use super::*;
use crate::vouchers::{VoucherInput, VoucherLineInput};

fn line(account_code: &str, debit: i64, credit: i64) -> VoucherLineInput {
    VoucherLineInput {
        id: None,
        account_code: account_code.to_string(),
        detail1_code: None,
        description: String::new(),
        debit,
        credit,
    }
}

fn voucher(number: i64, date: &str, lines: Vec<VoucherLineInput>) -> VoucherInput {
    VoucherInput {
        id: None,
        number: Some(number),
        date: date.to_string(),
        voucher_type: String::new(),
        description: "سند آزمون یکپارچه".to_string(),
        fiscal_year_id: None,
        lines,
    }
}

fn cash_sale(number: i64, date: &str, amount: i64) -> VoucherInput {
    voucher(
        number,
        date,
        vec![line("110101", amount, 0), line("410101", 0, amount)],
    )
}

fn unbalanced(number: i64, date: &str, debit: i64, credit: i64) -> VoucherInput {
    voucher(
        number,
        date,
        vec![line("110101", debit, 0), line("410101", 0, credit)],
    )
}

#[test]
fn real_file_account_hierarchy_and_voucher_lifecycle() {
    let directory = test_temp_dir("integration-flow");
    let active_path = directory.join(DB_FILE_NAME);
    let db = file_database(&active_path);

    // Group, general (کل), subsidiary (معین) and detail (تفصیلی ۱).
    db.create_account("9001", "کل آزمون", 1, "1", "debtor", "", false)
        .unwrap();
    db.create_account("9001001", "معین آزمون", 2, "1", "debtor", "9001", false)
        .unwrap();
    db.create_account(
        "9001001001",
        "تفصیلی آزمون",
        3,
        "1",
        "debtor",
        "9001001",
        true,
    )
    .unwrap();
    let accounts = db.list_accounts().unwrap().accounts;
    let detail = accounts
        .iter()
        .find(|account| account.code == "9001001001")
        .expect("detail account");
    assert_eq!(detail.level, 3);
    assert_eq!(detail.parent_code, "9001001");
    assert!(detail.is_active);

    // A balanced voucher is accepted and starts as temporary.
    let saved = db.save_voucher(cash_sale(1, "1405/01/10", 1_000)).unwrap();
    assert!(saved.is_temporary);
    assert_eq!(saved.total_debit, 1_000);

    // An unbalanced voucher is rejected and nothing is written.
    let error = db
        .save_voucher(unbalanced(2, "1405/01/11", 1_000, 900))
        .unwrap_err();
    assert!(
        error.contains("تراز") || error.contains("بدهکار") || error.contains("بستانکار"),
        "unexpected error: {error}"
    );
    assert!(db.get_voucher(2).unwrap().is_none());

    // A temporary voucher can be read, edited and deleted.
    let mut edit = cash_sale(1, "1405/01/12", 2_000);
    edit.id = Some(saved.id);
    let updated = db
        .save_voucher_if_unchanged(edit, Some(&saved.updated_at))
        .unwrap();
    assert_eq!(db.get_voucher(1).unwrap().unwrap().date, updated.date);
    db.delete_voucher(updated.id, Some(&updated.updated_at))
        .unwrap();
    assert!(db.get_voucher(1).unwrap().is_none());

    // A permanent voucher rejects both editing and deletion.
    let saved = db.save_voucher(cash_sale(3, "1405/01/13", 1_500)).unwrap();
    let permanent = db
        .set_voucher_permanent(saved.id, Some(&saved.updated_at))
        .unwrap();
    assert!(!permanent.is_temporary);
    let mut edit = cash_sale(3, "1405/01/14", 1_700);
    edit.id = Some(permanent.id);
    let edit_error = db
        .save_voucher_if_unchanged(edit, Some(&permanent.updated_at))
        .unwrap_err();
    assert!(edit_error.contains("دائمی"), "unexpected: {edit_error}");
    let delete_error = db
        .delete_voucher(permanent.id, Some(&permanent.updated_at))
        .unwrap_err();
    assert!(delete_error.contains("دائمی"), "unexpected: {delete_error}");
    assert!(db.get_voucher(3).unwrap().is_some());

    drop(db);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn real_file_closed_year_and_two_connections_share_version_checks() {
    let directory = test_temp_dir("integration-concurrency");
    let active_path = directory.join(DB_FILE_NAME);
    let db = file_database(&active_path);
    let saved = db.save_voucher(cash_sale(1, "1405/01/10", 1_000)).unwrap();

    // Two windows open the same file. The first edit wins; the second edit
    // still holds the old version token and must be rejected.
    let second_window = file_database(&active_path);
    let mut first_edit = cash_sale(1, "1405/01/20", 2_000);
    first_edit.id = Some(saved.id);
    let updated = second_window
        .save_voucher_if_unchanged(first_edit, Some(&saved.updated_at))
        .unwrap();
    assert_eq!(updated.date, "1405/01/20");

    let mut stale_edit = cash_sale(1, "1405/01/21", 3_000);
    stale_edit.id = Some(saved.id);
    let stale_error = db
        .save_voucher_if_unchanged(stale_edit, Some(&saved.updated_at))
        .unwrap_err();
    assert!(
        stale_error.contains("تغییر کرده"),
        "unexpected: {stale_error}"
    );
    assert_eq!(
        db.get_voucher(1).unwrap().unwrap().date,
        "1405/01/20".to_string()
    );

    // A closed fiscal year rejects a voucher even when another year would
    // accept the same date.
    let year = db
        .create_fiscal_year("سال ۱۴۰۶", "1406/01/01", "1406/12/29", "open")
        .unwrap();
    db.set_fiscal_year_status(year.id, "closed", None, false)
        .unwrap();
    let mut blocked = cash_sale(2, "1406/02/01", 1_000);
    blocked.fiscal_year_id = Some(year.id);
    let year_error = db.save_voucher(blocked).unwrap_err();
    assert!(
        year_error.contains("بسته") || year_error.contains("نهایی"),
        "unexpected: {year_error}"
    );
    assert_eq!(db.list_vouchers("", "", "").unwrap().total_count, 1);

    drop(second_window);
    drop(db);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn real_file_backup_and_restore_preserve_counts_and_report_totals() {
    let directory = test_temp_dir("integration-backup");
    let active_path = directory.join(DB_FILE_NAME);
    let backup_path = directory.join("snapshot.sqlite3");
    let db = file_database(&active_path);

    let saved = db.save_voucher(cash_sale(1, "1405/01/10", 1_000)).unwrap();
    db.set_voucher_permanent(saved.id, Some(&saved.updated_at))
        .unwrap();
    let temporary = db.save_voucher(cash_sale(2, "1405/01/11", 250)).unwrap();

    let before_count = db.list_vouchers("", "", "").unwrap().total_count;
    let before = db.get_trial_balance("1405/01/01", "1405/12/29").unwrap();

    db.backup_to(&backup_path).unwrap();
    db.delete_voucher(temporary.id, Some(&temporary.updated_at))
        .unwrap();
    assert_eq!(db.list_vouchers("", "", "").unwrap().total_count, 1);

    let safety_path = db.restore_from(&backup_path).unwrap();
    assert!(safety_path.exists());

    let after_count = db.list_vouchers("", "", "").unwrap().total_count;
    let after = db.get_trial_balance("1405/01/01", "1405/12/29").unwrap();
    assert_eq!(after_count, before_count);
    assert_eq!(after.period_debit, before.period_debit);
    assert_eq!(after.period_credit, before.period_credit);
    assert_eq!(after.closing_debit, before.closing_debit);
    assert_eq!(after.closing_credit, before.closing_credit);
    assert!(db.get_voucher(temporary.number).unwrap().is_some());

    drop(db);
    let _ = std::fs::remove_dir_all(&directory);
}
