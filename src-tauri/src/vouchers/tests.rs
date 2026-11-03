use super::*;
use crate::db;
use crate::vouchers::accounting::{classify, AccountClass};
use crate::vouchers::model::{serialize_exact_amount, CLOSING_FINAL, CLOSING_TEMPORARY};
use crate::vouchers::validation::{checked_add, checked_neg, checked_sub, checked_sum};
use rusqlite::params;
use serde::Serialize;

fn line(account_code: &str, debit: i64, credit: i64) -> VoucherLineInput {
    VoucherLineInput {
        id: None,
        account_code: account_code.to_string(),
        description: String::new(),
        debit,
        credit,
    }
}

fn voucher(date: &str, lines: Vec<VoucherLineInput>) -> VoucherInput {
    VoucherInput {
        id: None,
        number: None,
        date: date.to_string(),
        voucher_type: String::new(),
        description: "سند آزمایشی".to_string(),
        lines,
    }
}

fn cash_sale(date: &str, amount: i64) -> VoucherInput {
    voucher(
        date,
        vec![line("110101", amount, 0), line("410101", 0, amount)],
    )
}

#[test]
fn serializes_large_amounts_as_exact_decimal_strings() {
    #[derive(Serialize)]
    struct AmountValue {
        #[serde(serialize_with = "serialize_exact_amount")]
        amount: i64,
    }

    assert_eq!(
        serde_json::to_string(&AmountValue {
            amount: 9_007_199_254_740_991,
        })
        .unwrap(),
        "{\"amount\":9007199254740991}"
    );
    assert_eq!(
        serde_json::to_string(&AmountValue {
            amount: 9_007_199_254_740_992,
        })
        .unwrap(),
        "{\"amount\":\"9007199254740992\"}"
    );
}

#[test]
fn validates_and_normalizes_jalali_voucher_dates() {
    assert_eq!(validate_voucher_date("۱۴۰۵/۱/۱").unwrap(), "1405/01/01");
    assert!(validate_voucher_date("1405/13/01").is_err());
    assert!(validate_voucher_date("1405/02/32").is_err());
}

#[test]
fn validates_jalali_leap_day_and_supported_year_range() {
    assert_eq!(validate_voucher_date("۱۴۰۳/۱۲/۳۰").unwrap(), "1403/12/30");
    assert!(validate_voucher_date("۱۴۰۴/۱۲/۳۰").is_err());
    assert_eq!(validate_voucher_date("۱۴۰۴/۱۲/۲۹").unwrap(), "1404/12/29");
    assert!(validate_voucher_date("0000/01/01").is_err());
    assert!(validate_voucher_date("3178/01/01").is_err());
}

#[test]
fn voucher_list_separates_book_count_from_filtered_count() {
    let db = db::memory_database();
    db.save_voucher(cash_sale("1405/01/10", 1_000)).unwrap();
    db.save_voucher(cash_sale("1405/02/10", 2_000)).unwrap();

    let by_date = db.list_vouchers("۱۴۰۵/۰۲/۰۱", "۱۴۰۵/۰۲/۳۱", "").unwrap();
    assert_eq!(by_date.vouchers.len(), 1);
    assert_eq!(by_date.total_count, 2);
    assert_eq!(by_date.matched_count, 1);

    let by_search = db.list_vouchers("", "", "۲").unwrap();
    assert_eq!(by_search.vouchers.len(), 1);
    assert_eq!(by_search.vouchers[0].number, 2);
    assert_eq!(by_search.total_count, 2);
    assert_eq!(by_search.matched_count, 1);
}

#[test]
fn editing_voucher_without_number_preserves_its_number() {
    let db = db::memory_database();
    let saved = db.save_voucher(cash_sale("1405/01/10", 1_000)).unwrap();
    let mut edited = cash_sale("1405/01/11", 2_000);
    edited.id = Some(saved.id);

    let updated = db.save_voucher(edited).unwrap();

    assert_eq!(updated.number, saved.number);
    assert_eq!(updated.date, "1405/01/11");
    assert_eq!(updated.total_debit, 2_000);
}

#[test]
fn refuses_posting_to_parent_even_if_legacy_flag_allows_it() {
    let db = db::memory_database();
    db.with_conn(|conn| {
        conn.execute(
            "UPDATE accounts SET is_postable = 1 WHERE code = '110201'",
            [],
        )
        .map_err(db::db_error)?;
        Ok(())
    })
    .unwrap();

    let result = db.save_voucher(voucher(
        "1405/01/10",
        vec![line("110201", 1_000, 0), line("410101", 0, 1_000)],
    ));

    assert!(result.unwrap_err().contains("قابل ثبت نیست"));
}

#[test]
fn refuses_transferring_legacy_rows_posted_to_parent_accounts() {
    let db = db::memory_database();
    let saved = db.save_voucher(cash_sale("1405/01/10", 1_000)).unwrap();
    let first_line_id = saved.lines[0].id;
    let line_ids = saved.lines.iter().map(|line| line.id).collect();
    db.with_conn(|conn| {
        conn.execute(
            "UPDATE accounts SET is_postable = 1 WHERE code = '110201'",
            [],
        )
        .map_err(db::db_error)?;
        conn.execute(
            "UPDATE voucher_lines SET account_code = '110201' WHERE id = ?1",
            params![first_line_id],
        )
        .map_err(db::db_error)?;
        Ok(())
    })
    .unwrap();

    let error = db
        .transfer_voucher_lines(VoucherTransferInput {
            source_voucher_id: saved.id,
            source_voucher_number: saved.number,
            target_voucher_number: 2,
            line_ids,
            mode: "copy".to_string(),
        })
        .unwrap_err();

    assert!(error.contains("قابل ثبت نیست"));
}

#[test]
fn transfer_cannot_put_income_lines_into_a_final_closing_voucher() {
    let db = db::memory_database();
    let source = db.save_voucher(cash_sale("1405/01/10", 1_000)).unwrap();
    let mut target = voucher(
        "1405/01/10",
        vec![line("310101", 1_000, 0), line("110101", 0, 1_000)],
    );
    target.number = Some(2);
    target.voucher_type = CLOSING_FINAL.to_string();
    db.save_voucher(target).unwrap();

    let error = db
        .transfer_voucher_lines(VoucherTransferInput {
            source_voucher_id: source.id,
            source_voucher_number: source.number,
            target_voucher_number: 2,
            line_ids: source.lines.iter().map(|line| line.id).collect(),
            mode: "copy".to_string(),
        })
        .unwrap_err();

    assert!(error.contains("سازگار نیست"));
    assert_eq!(db.get_voucher(2).unwrap().unwrap().lines.len(), 2);
}

#[test]
fn transfer_rejects_target_numbers_above_the_safe_limit() {
    let db = db::memory_database();
    let error = db
        .transfer_voucher_lines(VoucherTransferInput {
            source_voucher_id: 1,
            source_voucher_number: 1,
            target_voucher_number: MAX_VOUCHER_NUMBER + 1,
            line_ids: Vec::new(),
            mode: "copy".to_string(),
        })
        .unwrap_err();

    assert!(error.contains("شماره سند مقصد"));
}

#[test]
fn adjacent_voucher_numbers_skip_gaps() {
    let db = db::memory_database();
    db.save_voucher(cash_sale("1405/01/10", 1_000)).unwrap();
    let mut third = cash_sale("1405/01/12", 3_000);
    third.number = Some(3);
    db.save_voucher(third).unwrap();

    assert_eq!(db.adjacent_voucher_number(1, "next").unwrap(), Some(3));
    assert_eq!(db.adjacent_voucher_number(3, "previous").unwrap(), Some(1));
    assert_eq!(db.adjacent_voucher_number(3, "next").unwrap(), None);
}

#[test]
fn saves_balanced_voucher_and_builds_reports() {
    let db = db::memory_database();
    let saved = db.save_voucher(cash_sale("1405/01/10", 1_000_000)).unwrap();

    assert_eq!(saved.number, 1);
    assert_eq!(saved.voucher_type, "سند حسابداری عادی");
    assert_eq!(saved.lines.len(), 2);
    assert_eq!(saved.lines[0].account_name, "صندوق مرکزی");
    assert_eq!(saved.total_debit, 1_000_000);
    assert_eq!(saved.total_credit, 1_000_000);

    let list = db.list_vouchers("", "", "").unwrap();
    assert_eq!(list.vouchers.len(), 1);
    assert_eq!(list.total_debit, 1_000_000);

    let journal = db.get_journal("", "", "").unwrap();
    assert_eq!(journal.rows.len(), 2);
    assert_eq!(journal.voucher_count, 1);
    assert_eq!(journal.total_debit, journal.total_credit);

    let trial = db.get_trial_balance("1405/01/01", "1405/01/31").unwrap();
    let cash = trial
        .rows
        .iter()
        .find(|r| r.account_code == "110101")
        .unwrap();
    assert_eq!(cash.period_debit, 1_000_000);
    assert_eq!(cash.closing_debit, 1_000_000);
    let revenue = trial
        .rows
        .iter()
        .find(|r| r.account_code == "410101")
        .unwrap();
    assert_eq!(revenue.period_credit, 1_000_000);
    assert_eq!(revenue.closing_credit, 1_000_000);
    assert_eq!(trial.closing_debit, trial.closing_credit);

    // With no start date there is no opening balance; everything is period
    // activity. This guards against counting the same lines twice.
    let all_trial = db.get_trial_balance("", "").unwrap();
    let cash_all = all_trial
        .rows
        .iter()
        .find(|r| r.account_code == "110101")
        .unwrap();
    assert_eq!(cash_all.opening_debit, 0);
    assert_eq!(cash_all.period_debit, 1_000_000);
    assert_eq!(cash_all.closing_debit, 1_000_000);

    let ledger = db.get_ledger("110101", "", "").unwrap();
    assert_eq!(ledger.account_name, "صندوق مرکزی");
    assert_eq!(ledger.rows.len(), 1);
    assert_eq!(ledger.total_debit, 1_000_000);
    assert_eq!(ledger.closing_debit, 1_000_000);
    assert_eq!(ledger.rows[0].running_debit, 1_000_000);

    let income = db.get_income_statement("1405/01/01", "1405/01/31").unwrap();
    assert_eq!(income.revenue_total, 1_000_000);
    assert_eq!(income.cogs_total, 0);
    assert_eq!(income.net_profit, 1_000_000);

    let balance = db.get_balance_sheet("1405/01/31").unwrap();
    assert_eq!(balance.assets_total, 1_000_000);
    assert_eq!(balance.profit, 1_000_000);
    assert_eq!(balance.total_liabilities_equity, 1_000_000);
    assert!(balance.balanced);
}

#[test]
fn non_operating_accounts_change_net_profit_not_operating_profit() {
    let db = db::memory_database();
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO accounts (code, name, level, group_code, nature, is_postable)
               VALUES ('1305', 'درآمد عملیاتی آزمایشی', 1, '10', 'dual', 1),
                   ('1405', 'هزینه عملیاتی آزمایشی', 1, '10', 'dual', 1),
                   ('1105', 'درآمد غیرعملیاتی آزمایشی', 1, '11', 'dual', 1),
                   ('1205', 'هزینه غیرعملیاتی آزمایشی', 1, '11', 'dual', 1)",
            [],
        )
        .map_err(db::db_error)?;
        Ok(())
    })
    .unwrap();

    db.save_voucher(voucher(
        "1405/01/10",
        vec![line("110101", 1_000, 0), line("410101", 0, 1_000)],
    ))
    .unwrap();
    db.save_voucher(voucher(
        "1405/01/11",
        vec![line("610101", 200, 0), line("110101", 0, 200)],
    ))
    .unwrap();
    db.save_voucher(voucher(
        "1405/01/12",
        vec![line("110101", 40, 0), line("1305", 0, 40)],
    ))
    .unwrap();
    db.save_voucher(voucher(
        "1405/01/13",
        vec![line("1405", 10, 0), line("110101", 0, 10)],
    ))
    .unwrap();
    db.save_voucher(voucher(
        "1405/01/14",
        vec![line("110101", 100, 0), line("1105", 0, 100)],
    ))
    .unwrap();
    db.save_voucher(voucher(
        "1405/01/15",
        vec![line("1205", 50, 0), line("110101", 0, 50)],
    ))
    .unwrap();

    let report = db.get_income_statement("1405/01/01", "1405/01/31").unwrap();
    assert_eq!(report.revenue_total, 1_000);
    assert_eq!(report.gross_profit, 1_000);
    assert_eq!(report.other_operating_revenue_total, 40);
    assert_eq!(report.other_operating_expense_total, 10);
    assert_eq!(report.operating_profit, 830);
    assert_eq!(report.non_operating_revenue_total, 100);
    assert_eq!(report.non_operating_expense_total, 50);
    assert_eq!(report.net_profit, 880);

    let balance = db.get_balance_sheet("1405/01/31").unwrap();
    assert_eq!(balance.assets_total, 880);
    assert_eq!(balance.profit, report.net_profit);
    assert!(balance.balanced);
}

#[test]
fn legacy_detail2_is_not_selectable_but_its_detail1_parent_remains_postable() {
    let db = db::memory_database();
    db.create_account("991", "کل آزمایشی", 1, "1", "debtor", "", false)
        .unwrap();
    db.create_account("99101", "معین آزمایشی", 2, "1", "debtor", "991", true)
        .unwrap();
    let detail1 = db
        .create_account(
            "9910101",
            "تفصیلی ۱ آزمایشی",
            3,
            "1",
            "debtor",
            "99101",
            true,
        )
        .unwrap();
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO accounts (code, name, level, group_code, nature, parent_code, parent_name, is_postable) VALUES ('991010101', 'تفصیلی ۲ قدیمی', 4, '1', 'debtor', '9910101', 'تفصیلی ۱ آزمایشی', 1)",
            [],
        )
        .map_err(db::db_error)?;
        conn.execute(
            "UPDATE accounts SET is_postable = 0, postable_before_children = 1 WHERE code = '9910101'",
            [],
        )
        .map_err(db::db_error)?;
        Ok(())
    }).unwrap();

    let accounts = db.list_accounts().unwrap().accounts;
    assert!(
        accounts
            .iter()
            .find(|a| a.code == detail1.code)
            .unwrap()
            .is_postable
    );
    assert!(
        !accounts
            .iter()
            .find(|a| a.code == "991010101")
            .unwrap()
            .is_postable
    );

    let saved = db
        .save_voucher(voucher(
            "1405/01/01",
            vec![line("9910101", 100, 0), line("110101", 0, 100)],
        ))
        .unwrap();
    assert_eq!(saved.lines[0].account_code, "9910101");
    let rejected = db.save_voucher(voucher(
        "1405/01/02",
        vec![line("991010101", 100, 0), line("110101", 0, 100)],
    ));
    assert!(rejected.is_err());
}

#[test]
fn an_existing_voucher_can_be_saved_without_replacing_its_legacy_detail2_code() {
    let db = db::memory_database();
    db.create_account("992", "کل آزمایشی", 1, "1", "debtor", "", false)
        .unwrap();
    db.create_account("99201", "معین آزمایشی", 2, "1", "debtor", "992", true)
        .unwrap();
    db.create_account(
        "9920101",
        "تفصیلی ۱ آزمایشی",
        3,
        "1",
        "debtor",
        "99201",
        true,
    )
    .unwrap();
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO accounts (code, name, level, group_code, nature, parent_code, parent_name, is_postable) VALUES ('992010101', 'تفصیلی ۲ قدیمی', 4, '1', 'debtor', '9920101', 'تفصیلی ۱ آزمایشی', 1)",
            [],
        ).map_err(db::db_error)?;
        conn.execute(
            "INSERT INTO vouchers (number, date, type, description) VALUES (777, '1405/01/01', ?1, 'سند قدیمی')",
            params![VOUCHER_TYPES[0]],
        ).map_err(db::db_error)?;
        let voucher_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO voucher_lines (voucher_id, row_no, account_code, description, debit, credit) VALUES (?1, 1, '992010101', '', 100, 0)",
            params![voucher_id],
        ).map_err(db::db_error)?;
        conn.execute(
            "INSERT INTO voucher_lines (voucher_id, row_no, account_code, description, debit, credit) VALUES (?1, 2, '110101', '', 0, 100)",
            params![voucher_id],
        ).map_err(db::db_error)?;
        Ok(())
    }).unwrap();
    let old = db.get_voucher(777).unwrap().unwrap();
    let input = VoucherInput {
        id: Some(old.id),
        number: Some(old.number),
        date: old.date.clone(),
        voucher_type: old.voucher_type.clone(),
        description: old.description.clone(),
        lines: old
            .lines
            .iter()
            .map(|line| VoucherLineInput {
                id: Some(line.id),
                account_code: line.account_code.clone(),
                description: line.description.clone(),
                debit: line.debit,
                credit: line.credit,
            })
            .collect(),
    };
    let saved = db
        .save_voucher_if_unchanged(input, Some(&old.updated_at))
        .unwrap();
    assert_eq!(saved.lines[0].account_code, "992010101");
}

#[test]
fn saves_voucher_to_a_new_detail1_account() {
    let db = db::memory_database();
    db.create_account("991", "کل آزمایشی", 1, "1", "debtor", "", false)
        .unwrap();
    db.create_account("99101", "معین آزمایشی", 2, "1", "debtor", "991", true)
        .unwrap();
    let detail1 = db
        .create_account(
            "9910101",
            "تفصیلی ۱ آزمایشی",
            3,
            "1",
            "debtor",
            "99101",
            true,
        )
        .unwrap();
    assert_eq!(detail1.level, 3);
    assert!(detail1.is_postable);

    let saved = db
        .save_voucher(voucher(
            "1405/01/01",
            vec![line("9910101", 100, 0), line("110101", 0, 100)],
        ))
        .unwrap();
    assert_eq!(saved.lines[0].account_code, "9910101");
}

#[test]
fn bulk_import_is_atomic_and_supports_large_vouchers() {
    let db = db::memory_database();
    let source_accounts = |general_code: &str, subsidiary_code: &str| {
        vec![
            crate::excel::ImportedAccount {
                code: general_code.to_string(),
                name: "حساب کل آزمایشی".to_string(),
                parent_code: String::new(),
                group_code: "1".to_string(),
                level: 1,
                nature: "debtor".to_string(),
                nature_is_explicit: false,
            },
            crate::excel::ImportedAccount {
                code: subsidiary_code.to_string(),
                name: "حساب معین آزمایشی".to_string(),
                parent_code: general_code.to_string(),
                group_code: "1".to_string(),
                level: 2,
                nature: "debtor".to_string(),
                nature_is_explicit: false,
            },
        ]
    };
    let make_import = |number, general_code: &str, subsidiary_code: &str, lines| {
        crate::excel::BulkVoucherImport {
            file_name: "آزمایشی.xlsx".to_string(),
            sheet_name: "اسناد".to_string(),
            accounts: source_accounts(general_code, subsidiary_code),
            vouchers: vec![VoucherInput {
                id: None,
                number: Some(number),
                date: "1405/01/01".to_string(),
                voucher_type: VOUCHER_TYPES[0].to_string(),
                description: String::new(),
                lines,
            }],
        }
    };

    let mut large_lines = vec![line("901101", 618, 0)];
    large_lines.extend((0..618).map(|_| line("110101", 0, 1)));
    assert_eq!(
        db.import_vouchers(make_import(1, "901", "901101", large_lines), false)
            .unwrap(),
        1
    );
    assert_eq!(db.get_voucher(1).unwrap().unwrap().lines.len(), 619);

    let conflicting = make_import(
        1,
        "902",
        "902101",
        vec![line("902101", 100, 0), line("110101", 0, 100)],
    );
    assert!(db.import_vouchers(conflicting.clone(), false).is_err());
    assert!(!db
        .list_accounts()
        .unwrap()
        .accounts
        .iter()
        .any(|account| account.code == "902"));

    assert_eq!(db.import_vouchers(conflicting, true).unwrap(), 1);
    assert!(db.get_voucher(2).unwrap().is_some());
    assert!(db
        .list_accounts()
        .unwrap()
        .accounts
        .iter()
        .any(|account| account.code == "902101"));
}

#[test]
fn bulk_import_supports_detail1_as_a_postable_leaf() {
    let db = db::memory_database();
    let import = crate::excel::BulkVoucherImport {
        file_name: "detail1.xlsx".to_string(),
        sheet_name: "اسناد".to_string(),
        accounts: vec![
            crate::excel::ImportedAccount {
                code: "991".to_string(),
                name: "گروه آزمایشی کل".to_string(),
                parent_code: String::new(),
                group_code: "1".to_string(),
                level: 1,
                nature: "debtor".to_string(),
                nature_is_explicit: false,
            },
            crate::excel::ImportedAccount {
                code: "99101".to_string(),
                name: "حساب معین آزمایشی".to_string(),
                parent_code: "991".to_string(),
                group_code: "1".to_string(),
                level: 2,
                nature: "debtor".to_string(),
                nature_is_explicit: false,
            },
            crate::excel::ImportedAccount {
                code: "9910101".to_string(),
                name: "حساب تفصیلی ۱ آزمایشی".to_string(),
                parent_code: "99101".to_string(),
                group_code: "1".to_string(),
                level: 3,
                nature: "debtor".to_string(),
                nature_is_explicit: false,
            },
        ],
        vouchers: vec![VoucherInput {
            id: None,
            number: Some(1),
            date: "1405/01/01".to_string(),
            voucher_type: VOUCHER_TYPES[0].to_string(),
            description: String::new(),
            lines: vec![line("9910101", 250, 0), line("110101", 0, 250)],
        }],
    };

    assert_eq!(db.import_vouchers(import, false).unwrap(), 1);
    let accounts = db.list_accounts().unwrap().accounts;
    let detail1 = accounts
        .iter()
        .find(|account| account.code == "9910101")
        .unwrap();
    let subsidiary = accounts
        .iter()
        .find(|account| account.code == "99101")
        .unwrap();
    assert_eq!(detail1.level, 3);
    assert!(detail1.is_postable);
    assert!(!subsidiary.is_postable);
    let saved = db.get_voucher(1).unwrap().unwrap();
    assert_eq!(saved.lines[0].account_code, "9910101");
}

#[test]
fn bulk_import_rejects_inactive_existing_parent_accounts() {
    let db = db::memory_database();
    db.with_conn(|conn| {
        conn.execute("UPDATE accounts SET is_active = 0 WHERE code = '1101'", [])
            .map_err(db::db_error)?;
        Ok(())
    })
    .unwrap();

    let import = crate::excel::BulkVoucherImport {
        file_name: "inactive-parent.xlsx".to_string(),
        sheet_name: "اسناد".to_string(),
        accounts: vec![
            crate::excel::ImportedAccount {
                code: "1101".to_string(),
                name: "صندوق".to_string(),
                parent_code: String::new(),
                group_code: "1".to_string(),
                level: 1,
                nature: "debtor".to_string(),
                nature_is_explicit: false,
            },
            crate::excel::ImportedAccount {
                code: "110199".to_string(),
                name: "صندوق جدید".to_string(),
                parent_code: "1101".to_string(),
                group_code: "1".to_string(),
                level: 2,
                nature: "debtor".to_string(),
                nature_is_explicit: false,
            },
        ],
        vouchers: vec![VoucherInput {
            id: None,
            number: Some(1),
            date: "1405/01/01".to_string(),
            voucher_type: VOUCHER_TYPES[0].to_string(),
            description: String::new(),
            lines: vec![line("110199", 100, 0), line("410101", 0, 100)],
        }],
    };

    let preview_error = db.preview_bulk_import(&import).unwrap_err();
    assert!(preview_error.contains("غیرفعال"));
    let error = db.import_vouchers(import, false).unwrap_err();
    assert!(error.contains("غیرفعال"));
    assert!(db.get_voucher(1).unwrap().is_none());
    assert!(!db
        .list_accounts()
        .unwrap()
        .accounts
        .iter()
        .any(|account| account.code == "110199"));
}

#[test]
fn bulk_import_preview_rejects_posted_account_with_imported_child() {
    let db = db::memory_database();
    let import = crate::excel::BulkVoucherImport {
        file_name: "mixed-level.xlsx".to_string(),
        sheet_name: "اسناد".to_string(),
        accounts: vec![
            crate::excel::ImportedAccount {
                code: "991".to_string(),
                name: "گروه آزمایشی کل".to_string(),
                parent_code: String::new(),
                group_code: "1".to_string(),
                level: 1,
                nature: "debtor".to_string(),
                nature_is_explicit: false,
            },
            crate::excel::ImportedAccount {
                code: "99101".to_string(),
                name: "حساب معین آزمایشی".to_string(),
                parent_code: "991".to_string(),
                group_code: "1".to_string(),
                level: 2,
                nature: "debtor".to_string(),
                nature_is_explicit: false,
            },
            crate::excel::ImportedAccount {
                code: "9910101".to_string(),
                name: "تفصیلی آزمایشی".to_string(),
                parent_code: "99101".to_string(),
                group_code: "1".to_string(),
                level: 3,
                nature: "debtor".to_string(),
                nature_is_explicit: false,
            },
        ],
        vouchers: vec![VoucherInput {
            id: None,
            number: Some(1),
            date: "1405/01/01".to_string(),
            voucher_type: VOUCHER_TYPES[0].to_string(),
            description: String::new(),
            lines: vec![line("99101", 100, 0), line("410101", 0, 100)],
        }],
    };

    let error = db.preview_bulk_import(&import).unwrap_err();
    assert!(error.contains("زیرحساب"));
    assert!(error.contains("99101"));
}

#[test]
fn bulk_import_preview_requires_nature_for_new_other_group_children() {
    let db = db::memory_database();
    let import = crate::excel::BulkVoucherImport {
        file_name: "other-group.xlsx".to_string(),
        sheet_name: "اسناد".to_string(),
        accounts: vec![
            crate::excel::ImportedAccount {
                code: "7201".to_string(),
                name: "سایر درآمدها و هزینه‌های عملیاتی جدید".to_string(),
                parent_code: String::new(),
                group_code: "10".to_string(),
                level: 1,
                nature: "dual".to_string(),
                nature_is_explicit: false,
            },
            crate::excel::ImportedAccount {
                code: "720101".to_string(),
                name: "حساب فرعی جدید".to_string(),
                parent_code: "7201".to_string(),
                group_code: "10".to_string(),
                level: 2,
                nature: "unknown".to_string(),
                nature_is_explicit: false,
            },
        ],
        vouchers: vec![VoucherInput {
            id: None,
            number: Some(1),
            date: "1405/01/01".to_string(),
            voucher_type: VOUCHER_TYPES[0].to_string(),
            description: String::new(),
            lines: vec![line("720101", 100, 0), line("410101", 0, 100)],
        }],
    };

    let error = db.preview_bulk_import(&import).unwrap_err();
    assert!(error.contains("ماهیت"));
}

#[test]
fn bulk_import_rejects_legacy_postable_parent_accounts() {
    let db = db::memory_database();
    db.with_conn(|conn| {
        conn.execute(
            "UPDATE accounts SET is_postable = 1 WHERE code = '1102'",
            [],
        )
        .map_err(db::db_error)?;
        Ok(())
    })
    .unwrap();

    let import = crate::excel::BulkVoucherImport {
        file_name: "legacy.xlsx".to_string(),
        sheet_name: "اسناد".to_string(),
        accounts: Vec::new(),
        vouchers: vec![VoucherInput {
            id: None,
            number: Some(1),
            date: "1405/01/01".to_string(),
            voucher_type: VOUCHER_TYPES[0].to_string(),
            description: String::new(),
            lines: vec![line("1102", 100, 0), line("410101", 0, 100)],
        }],
    };

    let error = db.import_vouchers(import, false).unwrap_err();
    assert!(error.contains("قابل ثبت"));
    assert!(db.get_voucher(1).unwrap().is_none());
}

#[test]
fn reorganize_preview_merges_then_sorts_by_date() {
    let db = db::memory_database();
    db.save_voucher(cash_sale("1405/01/02", 1_000)).unwrap();
    db.save_voucher(cash_sale("1405/01/01", 2_000)).unwrap();
    let mut source = cash_sale("1405/01/02", 500);
    source.description = "شرح سند مبدأ".to_string();
    db.save_voucher(source).unwrap();
    let mut opening = voucher(
        "1405/01/02",
        vec![line("110101", 750, 0), line("310101", 0, 750)],
    );
    opening.voucher_type = VOUCHER_TYPES[1].to_string();
    db.save_voucher(opening).unwrap();

    let preview = db
        .preview_voucher_reorganization("merge-then-sort")
        .unwrap();
    assert_eq!(preview.total_vouchers, 4);
    assert_eq!(preview.total_vouchers_after, 3);
    assert_eq!(preview.merge_groups.len(), 1);
    assert_eq!(preview.merge_groups[0].source_voucher_numbers, vec![1, 3]);
    assert_eq!(preview.merge_groups[0].line_count, 4);
    assert_eq!(preview.number_changes.len(), 3);

    let result = db
        .apply_voucher_reorganization("merge-then-sort", &preview.plan_token)
        .unwrap();
    assert_eq!(result.merged_group_count, 1);
    assert_eq!(result.removed_voucher_count, 1);
    assert_eq!(result.renumbered_count, 3);
    assert_eq!(db.get_voucher(1).unwrap().unwrap().date, "1405/01/01");
    let merged = db.get_voucher(2).unwrap().unwrap();
    assert_eq!(merged.date, "1405/01/02");
    assert_eq!(merged.lines.len(), 4);
    assert_eq!(merged.total_debit, 1_500);
    assert!(merged
        .description
        .contains("ادغام از سند شماره 3: شرح سند مبدأ"));
    assert_eq!(
        db.get_voucher(3).unwrap().unwrap().voucher_type,
        VOUCHER_TYPES[1]
    );
    assert!(db.get_voucher(4).unwrap().is_none());
}

#[test]
fn reorganize_does_not_merge_groups_over_the_form_line_limit() {
    let db = db::memory_database();
    let large_voucher = || {
        let mut lines = Vec::with_capacity(501);
        lines.push(line("110101", 251, 0));
        lines.extend((0..249).map(|_| line("110101", 1, 0)));
        lines.push(line("410101", 0, 250));
        lines.extend((0..250).map(|_| line("410101", 0, 1)));
        voucher("1405/01/02", lines)
    };
    db.save_voucher(large_voucher()).unwrap();
    db.save_voucher(large_voucher()).unwrap();

    let preview = db.preview_voucher_reorganization("merge").unwrap();
    assert!(preview.merge_groups.is_empty());
    assert_eq!(preview.unbalanced_groups.len(), 1);
    assert_eq!(preview.unbalanced_groups[0].line_count, 1_002);
    assert!(preview.unbalanced_groups[0].reason.contains("ظرفیت"));

    db.apply_voucher_reorganization("merge", &preview.plan_token)
        .unwrap();
    assert!(db.get_voucher(1).unwrap().is_some());
    assert!(db.get_voucher(2).unwrap().is_some());
}

#[test]
fn reorganize_rejects_a_stale_preview() {
    let db = db::memory_database();
    db.save_voucher(cash_sale("1405/01/02", 1_000)).unwrap();
    let preview = db.preview_voucher_reorganization("sort").unwrap();
    db.save_voucher(cash_sale("1405/01/01", 500)).unwrap();

    assert!(db
        .apply_voucher_reorganization("sort", &preview.plan_token)
        .unwrap_err()
        .contains("تغییر کرده‌اند"));
    assert_eq!(db.list_vouchers("", "", "").unwrap().total_count, 2);
}

fn bulk_delete_input(scope: &str) -> BulkVoucherDeleteInput {
    BulkVoucherDeleteInput {
        scope: scope.to_string(),
        from_date: String::new(),
        to_date: String::new(),
        from_number: None,
        to_number: None,
    }
}

#[test]
fn bulk_delete_previews_and_deletes_a_date_range() {
    let db = db::memory_database();
    db.save_voucher(cash_sale("1405/01/05", 1_000)).unwrap();
    db.save_voucher(cash_sale("1405/02/05", 2_000)).unwrap();
    db.save_voucher(cash_sale("1405/03/05", 3_000)).unwrap();

    let input = BulkVoucherDeleteInput {
        from_date: "۱۴۰۵/۰۲/۰۱".to_string(),
        to_date: "1405/02/31".to_string(),
        ..bulk_delete_input("dateRange")
    };
    let preview = db.preview_bulk_voucher_delete(&input).unwrap();
    assert_eq!(preview.scope, "dateRange");
    assert_eq!(preview.total_matched, 1);
    assert_eq!(preview.deletable_count, 1);
    assert_eq!(preview.locked_count, 0);
    assert_eq!(preview.deletable_sample[0].voucher_number, 2);
    assert_eq!(preview.date_range.as_ref().unwrap().from, "1405/02/01");

    let result = db
        .apply_bulk_voucher_delete(&input, &preview.plan_token, false)
        .unwrap();
    assert_eq!(result.deleted_count, 1);
    assert_eq!(result.locked_count, 0);
    assert!(result.message.contains("۱ سند"));
    assert_eq!(db.list_vouchers("", "", "").unwrap().total_count, 2);
    assert!(db.get_voucher(2).unwrap().is_none());
    let remaining_lines: i64 = db
        .with_conn(|conn| {
            conn.query_row("SELECT COUNT(*) FROM voucher_lines", [], |row| row.get(0))
                .map_err(db::db_error)
        })
        .unwrap();
    assert_eq!(remaining_lines, 4);
}

#[test]
fn bulk_delete_handles_number_range_and_all_scopes() {
    let db = db::memory_database();
    db.save_voucher(cash_sale("1405/01/05", 1_000)).unwrap();
    db.save_voucher(cash_sale("1405/01/06", 2_000)).unwrap();
    db.save_voucher(cash_sale("1405/01/07", 3_000)).unwrap();

    let number_input = BulkVoucherDeleteInput {
        from_number: Some(2),
        to_number: Some(3),
        ..bulk_delete_input("numberRange")
    };
    let number_preview = db.preview_bulk_voucher_delete(&number_input).unwrap();
    assert_eq!(number_preview.total_matched, 2);
    assert_eq!(number_preview.number_range.as_ref().unwrap().from, 2);
    db.apply_bulk_voucher_delete(&number_input, &number_preview.plan_token, false)
        .unwrap();
    assert_eq!(db.list_vouchers("", "", "").unwrap().total_count, 1);

    let all_input = bulk_delete_input("all");
    let all_preview = db.preview_bulk_voucher_delete(&all_input).unwrap();
    assert_eq!(all_preview.total_matched, 1);
    let result = db
        .apply_bulk_voucher_delete(&all_input, &all_preview.plan_token, false)
        .unwrap();
    assert_eq!(result.deleted_count, 1);
    assert_eq!(db.list_vouchers("", "", "").unwrap().total_count, 0);
}

#[test]
fn bulk_delete_rejects_a_stale_preview() {
    let db = db::memory_database();
    db.save_voucher(cash_sale("1405/01/05", 1_000)).unwrap();
    let preview = db
        .preview_bulk_voucher_delete(&bulk_delete_input("all"))
        .unwrap();
    db.save_voucher(cash_sale("1405/01/06", 2_000)).unwrap();

    assert!(db
        .apply_bulk_voucher_delete(&bulk_delete_input("all"), &preview.plan_token, false)
        .unwrap_err()
        .contains("تغییر کرده‌اند"));
    assert_eq!(db.list_vouchers("", "", "").unwrap().total_count, 2);
}

#[test]
fn bulk_delete_validates_ranges_and_scope() {
    let db = db::memory_database();

    let reversed_dates = BulkVoucherDeleteInput {
        from_date: "1405/02/01".to_string(),
        to_date: "1405/01/01".to_string(),
        ..bulk_delete_input("dateRange")
    };
    assert!(db
        .preview_bulk_voucher_delete(&reversed_dates)
        .unwrap_err()
        .contains("بعد از تاریخ پایان"));

    let reversed_numbers = BulkVoucherDeleteInput {
        from_number: Some(10),
        to_number: Some(2),
        ..bulk_delete_input("numberRange")
    };
    assert!(db
        .preview_bulk_voucher_delete(&reversed_numbers)
        .unwrap_err()
        .contains("بزرگ‌تر"));

    assert!(db
        .preview_bulk_voucher_delete(&bulk_delete_input("unknown"))
        .unwrap_err()
        .contains("معتبر نیست"));
}

#[test]
fn bulk_delete_backup_failure_keeps_vouchers() {
    let db = db::memory_database();
    db.save_voucher(cash_sale("1405/01/05", 1_000)).unwrap();
    let input = bulk_delete_input("all");
    let preview = db.preview_bulk_voucher_delete(&input).unwrap();
    assert_eq!(preview.backup_directory, "");

    let error = db
        .apply_bulk_voucher_delete(&input, &preview.plan_token, true)
        .unwrap_err();
    assert!(error.contains("پشتیبان"));
    assert_eq!(db.list_vouchers("", "", "").unwrap().total_count, 1);
}

#[test]
fn rejects_unbalanced_and_invalid_vouchers() {
    let db = db::memory_database();

    let unbalanced = db
        .save_voucher(voucher(
            "1405/01/10",
            vec![line("110101", 100_000, 0), line("410101", 0, 90_000)],
        ))
        .unwrap_err();
    assert!(unbalanced.contains("تراز نیست"));

    let non_postable = db
        .save_voucher(voucher(
            "1405/01/10",
            vec![line("1101", 100_000, 0), line("410101", 0, 100_000)],
        ))
        .unwrap_err();
    assert!(non_postable.contains("قابل ثبت نیست"));

    let unknown = db
        .save_voucher(voucher(
            "1405/01/10",
            vec![line("999999", 100_000, 0), line("410101", 0, 100_000)],
        ))
        .unwrap_err();
    assert!(unknown.contains("یافت نشد"));

    let single = db
        .save_voucher(voucher("1405/01/10", vec![line("110101", 100_000, 0)]))
        .unwrap_err();
    assert!(single.contains("حداقل دو ردیف"));

    let both_sides = db
        .save_voucher(voucher(
            "1405/01/10",
            vec![line("110101", 100_000, 50_000), line("410101", 0, 50_000)],
        ))
        .unwrap_err();
    assert!(both_sides.contains("فقط یکی"));

    let zero = db
        .save_voucher(voucher(
            "1405/01/10",
            vec![line("110101", 0, 0), line("410101", 0, 0)],
        ))
        .unwrap_err();
    assert!(zero.contains("مبلغ بدهکار یا بستانکار"));

    let negative = db
        .save_voucher(voucher(
            "1405/01/10",
            vec![line("110101", -1, 0), line("410101", 0, -1)],
        ))
        .unwrap_err();
    assert!(negative.contains("منفی"));

    let bad_date = db
        .save_voucher(cash_sale("1405/13/01", 100_000))
        .unwrap_err();
    assert!(bad_date.contains("تاریخ"));

    let mut invalid_type = cash_sale("1405/01/10", 100_000);
    invalid_type.voucher_type = "سند نامعتبر".to_string();
    assert!(db.save_voucher(invalid_type).is_err());

    assert!(db.list_vouchers("", "", "").unwrap().vouchers.is_empty());
}

#[test]
fn updates_existing_voucher_without_duplicating_it() {
    let db = db::memory_database();
    let saved = db.save_voucher(cash_sale("1405/01/10", 500_000)).unwrap();

    let updated = db
        .save_voucher(VoucherInput {
            id: Some(saved.id),
            number: Some(saved.number),
            date: "1405/01/11".to_string(),
            voucher_type: "سند حسابداری افتتاحیه".to_string(),
            description: "اصلاح سند".to_string(),
            lines: vec![line("110101", 800_000, 0), line("310101", 0, 800_000)],
        })
        .unwrap();

    assert_eq!(updated.id, saved.id);
    assert_eq!(updated.number, saved.number);
    assert_eq!(updated.date, "1405/01/11");
    assert_eq!(updated.voucher_type, "سند حسابداری افتتاحیه");
    assert_eq!(updated.total_debit, 800_000);

    let list = db.list_vouchers("", "", "").unwrap();
    assert_eq!(list.vouchers.len(), 1);
    assert_eq!(list.vouchers[0].description, "اصلاح سند");
    assert_eq!(list.vouchers[0].line_count, 2);

    let second = db.save_voucher(cash_sale("1405/01/12", 200_000)).unwrap();
    assert_eq!(second.number, 2);

    let duplicate = db
        .save_voucher(VoucherInput {
            id: Some(second.id),
            number: Some(1),
            date: "1405/01/12".to_string(),
            voucher_type: String::new(),
            description: String::new(),
            lines: vec![line("110101", 200_000, 0), line("410101", 0, 200_000)],
        })
        .unwrap_err();
    assert!(duplicate.contains("تکراری"));

    let missing = db
        .save_voucher(VoucherInput {
            id: Some(9999),
            number: None,
            date: "1405/01/12".to_string(),
            voucher_type: String::new(),
            description: String::new(),
            lines: vec![line("110101", 200_000, 0), line("410101", 0, 200_000)],
        })
        .unwrap_err();
    assert!(missing.contains("یافت نشد"));
}

#[test]
fn stale_voucher_version_cannot_overwrite_a_newer_edit() {
    let db = db::memory_database();
    let saved = db.save_voucher(cash_sale("1405/01/10", 500_000)).unwrap();
    let mut first_edit = cash_sale("1405/01/11", 600_000);
    first_edit.id = Some(saved.id);
    let updated = db
        .save_voucher_if_unchanged(first_edit, Some(&saved.updated_at))
        .unwrap();

    let mut stale_edit = cash_sale("1405/01/12", 700_000);
    stale_edit.id = Some(saved.id);
    let error = db
        .save_voucher_if_unchanged(stale_edit, Some(&saved.updated_at))
        .unwrap_err();

    assert!(error.contains("پس از باز شدن تغییر کرده"));
    assert_eq!(
        db.get_voucher(saved.number).unwrap().unwrap().date,
        updated.date
    );
}

#[test]
fn deletes_voucher_and_its_lines() {
    let db = db::memory_database();
    let saved = db.save_voucher(cash_sale("1405/01/10", 300_000)).unwrap();
    assert_eq!(
        db.get_voucher(saved.number).unwrap().unwrap().lines.len(),
        2
    );

    db.delete_voucher(saved.id, Some(&saved.updated_at))
        .unwrap();
    assert!(db.get_voucher(saved.number).unwrap().is_none());
    assert!(db.list_vouchers("", "", "").unwrap().vouchers.is_empty());
    assert!(db.get_journal("", "", "").unwrap().rows.is_empty());
    assert!(db
        .delete_voucher(saved.id, Some(&saved.updated_at))
        .is_err());
}

#[test]
fn delete_rejects_a_stale_voucher_version() {
    let db = db::memory_database();
    let saved = db.save_voucher(cash_sale("1405/01/10", 300_000)).unwrap();

    let error = db
        .delete_voucher(saved.id, Some("2000-01-01 00:00:00.000"))
        .unwrap_err();
    assert!(error.contains("تغییر کرده"));
    assert!(db.get_voucher(saved.number).unwrap().is_some());

    db.delete_voucher(saved.id, Some(&saved.updated_at))
        .unwrap();
    assert!(db.get_voucher(saved.number).unwrap().is_none());
}

#[test]
fn rejects_a_voucher_type_that_contradicts_its_accounts() {
    let db = db::memory_database();

    let mut opening = cash_sale("1405/01/10", 100_000);
    opening.voucher_type = VOUCHER_TYPES[1].to_string();
    let error = db.save_voucher(opening).unwrap_err();
    assert!(error.contains("سازگار نیست"));

    let mut closing = voucher(
        "1405/01/10",
        vec![line("110101", 100_000, 0), line("310101", 0, 100_000)],
    );
    closing.voucher_type = CLOSING_TEMPORARY.to_string();
    let error = db.save_voucher(closing).unwrap_err();
    assert!(error.contains("سازگار نیست"));
}

#[test]
fn trial_balance_splits_opening_and_period_activity() {
    let db = db::memory_database();
    db.save_voucher(cash_sale("1405/01/05", 500_000)).unwrap();
    db.save_voucher(cash_sale("1405/02/05", 300_000)).unwrap();

    let trial = db.get_trial_balance("1405/02/01", "1405/02/31").unwrap();
    let cash = trial
        .rows
        .iter()
        .find(|r| r.account_code == "110101")
        .unwrap();
    assert_eq!(cash.opening_debit, 500_000);
    assert_eq!(cash.period_debit, 300_000);
    assert_eq!(cash.closing_debit, 800_000);

    let revenue = trial
        .rows
        .iter()
        .find(|r| r.account_code == "410101")
        .unwrap();
    assert_eq!(revenue.opening_credit, 500_000);
    assert_eq!(revenue.period_credit, 300_000);
    assert_eq!(revenue.closing_credit, 800_000);
    assert_eq!(trial.opening_debit, trial.opening_credit);
    assert_eq!(trial.closing_debit, trial.closing_credit);

    let ledger = db.get_ledger("110101", "1405/02/01", "1405/02/31").unwrap();
    assert_eq!(ledger.opening_debit, 500_000);
    assert_eq!(ledger.total_debit, 300_000);
    assert_eq!(ledger.closing_debit, 800_000);

    let income = db.get_income_statement("1405/02/01", "1405/02/31").unwrap();
    assert_eq!(income.revenue_total, 300_000);

    let balance = db.get_balance_sheet("1405/02/31").unwrap();
    assert_eq!(balance.assets_total, 800_000);
    assert!(balance.balanced);
}

#[test]
fn trial_balance_rolls_up_level_one_and_filters_code_range() {
    let db = db::memory_database();
    db.save_voucher(cash_sale("1405/01/05", 500_000)).unwrap();
    db.save_voucher(cash_sale("1405/01/06", 300_000)).unwrap();

    // Default view: only level-1 rows, each showing its whole branch.
    let level_one = db
        .get_trial_balance_filtered("1405/01/01", "1405/01/31", "", "", false, false)
        .unwrap();
    assert!(level_one.rows.iter().all(|row| row.level == 1));
    let cash_parent = level_one
        .rows
        .iter()
        .find(|row| row.account_code == "1101")
        .unwrap();
    assert_eq!(cash_parent.period_debit, 800_000);
    assert_eq!(cash_parent.closing_debit, 800_000);
    let revenue_parent = level_one
        .rows
        .iter()
        .find(|row| row.account_code == "4101")
        .unwrap();
    assert_eq!(revenue_parent.period_credit, 800_000);
    assert_eq!(level_one.period_debit, level_one.period_credit);

    // A code range searches every level, but a shown parent keeps its
    // children out of the totals so nothing is counted twice.
    let range = db
        .get_trial_balance_filtered("1405/01/01", "1405/01/31", "1101", "1101", false, false)
        .unwrap();
    assert!(range
        .rows
        .iter()
        .all(|row| row.account_code.starts_with("1101")));
    assert_eq!(range.rows.len(), 2);
    assert_eq!(range.period_debit, 800_000);
    assert_eq!(range.period_credit, 0);
}

#[test]
fn voucher_save_rejects_inactive_accounts_but_keeps_saved_lines_editable() {
    let db = db::memory_database();
    let inactive = db
        .create_account_active(
            "110202001",
            "بانک غیرفعال",
            3,
            "1",
            "debtor",
            "110202",
            true,
            false,
        )
        .unwrap();
    let error = db
        .save_voucher(voucher(
            "1405/01/05",
            vec![line(&inactive.code, 1_000, 0), line("410101", 0, 1_000)],
        ))
        .unwrap_err();
    assert!(error.contains("غیرفعال"));

    // A line saved before its account was deactivated stays editable.
    let saved = db.save_voucher(cash_sale("1405/01/05", 500_000)).unwrap();
    let cash = db
        .list_accounts()
        .unwrap()
        .accounts
        .into_iter()
        .find(|account| account.code == "110101")
        .unwrap();
    db.set_account_active(cash.id, false).unwrap();
    let mut edited = cash_sale("1405/01/05", 500_000);
    edited.id = Some(saved.id);
    // The stable id of each saved row is what preserves the exception; a
    // line without an id is treated as a new posting.
    for line in &mut edited.lines {
        line.id = saved
            .lines
            .iter()
            .find(|saved_line| saved_line.account_code == line.account_code)
            .map(|saved_line| saved_line.id);
    }
    db.save_voucher(edited).unwrap();

    // Adding another row with the same inactive account must still fail.
    // Re-read the voucher because saving replaced its line ids.
    let saved = db.get_voucher(saved.number).unwrap().unwrap();
    let mut added = cash_sale("1405/01/05", 500_000);
    added.id = Some(saved.id);
    for line in &mut added.lines {
        line.id = saved
            .lines
            .iter()
            .find(|saved_line| saved_line.account_code == line.account_code)
            .map(|saved_line| saved_line.id);
    }
    added.lines.push(line(&cash.code, 1_000, 0));
    added.lines.push(line("410101", 0, 1_000));
    let error = db.save_voucher(added).unwrap_err();
    assert!(error.contains("غیرفعال"), "{error}");
}

#[test]
fn saved_line_ids_cannot_be_reused_and_export_checks_new_account_postings() {
    let db = db::memory_database();
    let saved = db.save_voucher(cash_sale("1405/01/05", 500_000)).unwrap();
    let cash = db
        .list_accounts()
        .unwrap()
        .accounts
        .into_iter()
        .find(|account| account.code == "110101")
        .unwrap();
    db.set_account_active(cash.id, false).unwrap();

    // Reusing one saved row id twice used to copy its historical inactive
    // account exception to an additional posting.
    let mut duplicated = cash_sale("1405/01/05", 500_000);
    duplicated.id = Some(saved.id);
    for row in &mut duplicated.lines {
        row.id = saved
            .lines
            .iter()
            .find(|saved_row| saved_row.account_code == row.account_code)
            .map(|saved_row| saved_row.id);
    }
    let mut extra_cash = duplicated
        .lines
        .iter()
        .find(|row| row.account_code == cash.code)
        .unwrap()
        .clone();
    extra_cash.debit = 100_000;
    duplicated
        .lines
        .iter_mut()
        .find(|row| row.account_code == "410101")
        .unwrap()
        .credit = 600_000;
    duplicated.lines.push(extra_cash);

    let error = db.save_voucher(duplicated.clone()).unwrap_err();
    assert!(error.contains("تکرار"), "{error}");
    let export_error = db
        .validate_voucher_accounts(Some(saved.id), &duplicated.lines)
        .unwrap_err();
    assert!(export_error.contains("تکرار"), "{export_error}");

    // Even when original rows retain their ids, an extra row cannot post
    // to the inactive account merely because that code exists in the voucher.
    let mut new_posting = cash_sale("1405/01/05", 500_000);
    new_posting.id = Some(saved.id);
    for row in &mut new_posting.lines {
        row.id = saved
            .lines
            .iter()
            .find(|saved_row| saved_row.account_code == row.account_code)
            .map(|saved_row| saved_row.id);
    }
    let mut new_cash = line(&cash.code, 100_000, 0);
    new_cash.id = None;
    new_posting
        .lines
        .iter_mut()
        .find(|row| row.account_code == "410101")
        .unwrap()
        .credit = 600_000;
    new_posting.lines.push(new_cash);
    let error = db
        .validate_voucher_accounts(Some(saved.id), &new_posting.lines)
        .unwrap_err();
    assert!(error.contains("غیرفعال"), "{error}");

    // A non-positive voucher id must not silently create a new document.
    let mut invalid_id = cash_sale("1405/01/05", 1_000);
    invalid_id.id = Some(-1);
    let error = db.save_voucher(invalid_id).unwrap_err();
    assert!(error.contains("شناسه سند نامعتبر"), "{error}");
    assert_eq!(db.voucher_count().unwrap(), 1);
}

#[test]
fn filters_vouchers_by_date_and_text() {
    let db = db::memory_database();
    let mut first = cash_sale("1405/01/10", 100_000);
    first.description = "فروش نقدی".to_string();
    db.save_voucher(first).unwrap();
    let mut second = cash_sale("1405/02/10", 250_000);
    second.description = "دریافت از مشتری".to_string();
    db.save_voucher(second).unwrap();

    assert_eq!(
        db.list_vouchers("1405/02/01", "1405/02/31", "")
            .unwrap()
            .vouchers
            .len(),
        1
    );
    assert_eq!(db.list_vouchers("", "", "نقدی").unwrap().vouchers.len(), 1);
    assert_eq!(
        db.list_vouchers("", "", "دریافت").unwrap().vouchers.len(),
        1
    );
    assert_eq!(
        db.list_vouchers("", "", "ناموجود").unwrap().vouchers.len(),
        0
    );
    // Account names are searchable too.
    assert_eq!(db.list_vouchers("", "", "صندوق").unwrap().vouchers.len(), 2);

    let journal = db.get_journal("", "", "دریافت").unwrap();
    assert_eq!(journal.rows.len(), 2);
    assert_eq!(journal.voucher_count, 1);
}

#[test]
fn classifies_accounts_by_group_or_code() {
    assert_eq!(classify("دارایی", "9999"), AccountClass::Asset);
    assert_eq!(classify("بدهی", "9999"), AccountClass::Liability);
    assert_eq!(classify("سرمایه", "9999"), AccountClass::Equity);
    assert_eq!(classify("درآمد", "9999"), AccountClass::Income);
    assert_eq!(classify("هزینه", "9999"), AccountClass::Expense);
    assert_eq!(classify("سایر", "1101"), AccountClass::Asset);
    assert_eq!(classify("سایر", "2101"), AccountClass::Liability);
    assert_eq!(classify("سایر", "3101"), AccountClass::Equity);
    assert_eq!(classify("سایر", "4101"), AccountClass::Income);
    assert_eq!(classify("سایر", "5101"), AccountClass::Expense);
    assert_eq!(classify("سایر", "9101"), AccountClass::Expense);
}

#[test]
fn income_statement_classifies_cogs_by_group_not_code_prefix() {
    let db = db::memory_database();
    db.create_account("5102", "بهای تمام‌شده", 1, "9", "debtor", "", false)
        .unwrap();
    db.create_account("510201", "مواد مصرفی", 2, "9", "debtor", "5102", true)
        .unwrap();
    db.create_account("5111", "هزینه‌های عملیاتی", 1, "8", "debtor", "", false)
        .unwrap();
    db.create_account("511101", "اجاره", 2, "8", "debtor", "5111", true)
        .unwrap();

    db.save_voucher(voucher(
        "1405/01/10",
        vec![
            line("510201", 100, 0),
            line("511101", 40, 0),
            line("110101", 0, 140),
        ],
    ))
    .unwrap();

    let income = db.get_income_statement("1405/01/01", "1405/12/29").unwrap();
    assert_eq!(income.cogs_total, 100);
    assert_eq!(income.cogs.len(), 1);
    assert_eq!(income.cogs[0].account_code, "510201");
    assert_eq!(income.expense_total, 40);
    assert_eq!(income.expenses.len(), 1);
    assert_eq!(income.expenses[0].account_code, "511101");
    assert_eq!(income.gross_profit, -100);
    assert_eq!(income.net_profit, -140);
}

#[test]
fn aggregate_arithmetic_reports_overflow_instead_of_wrapping() {
    assert!(checked_add(i64::MAX, 1, "آزمون").is_err());
    assert!(checked_sub(i64::MIN, 1, "آزمون").is_err());
    assert!(checked_neg(i64::MIN, "آزمون").is_err());
    assert!(checked_sum([i64::MAX, 1], "آزمون").is_err());
    assert_eq!(checked_sum([1, 2, 3], "آزمون").unwrap(), 6);
}

#[test]
fn formats_amounts_with_grouping() {
    assert_eq!(format_amount(0), "0");
    assert_eq!(format_amount(999), "999");
    assert_eq!(format_amount(1_000), "1,000");
    assert_eq!(format_amount(1_234_567), "1,234,567");
    assert_eq!(format_amount(-1_500), "-1,500");
    assert_eq!(format_amount(i64::MIN), "-9,223,372,036,854,775,808");
}
#[test]
fn closing_vouchers_do_not_hide_income_statement_figures() {
    let db = db::memory_database();
    db.save_voucher(cash_sale("1405/01/10", 1_000_000)).unwrap();

    // Closing the temporary accounts moves the revenue into equity.
    let mut closing = voucher(
        "1405/12/29",
        vec![line("410101", 1_000_000, 0), line("310101", 0, 1_000_000)],
    );
    closing.voucher_type = "سند بستن حساب‌های موقت".to_string();
    db.save_voucher(closing).unwrap();

    let income = db.get_income_statement("1405/01/01", "1405/12/29").unwrap();
    assert_eq!(income.revenue_total, 1_000_000);
    assert_eq!(income.net_profit, 1_000_000);

    let balance = db.get_balance_sheet("1405/12/29").unwrap();
    assert_eq!(balance.assets_total, 1_000_000);
    assert_eq!(balance.equity_total, 1_000_000);
    assert_eq!(balance.profit, 0);
    assert!(balance.balanced);

    // The trial balance still shows every voucher, closing ones included.
    let trial = db.get_trial_balance("", "").unwrap();
    let revenue = trial
        .rows
        .iter()
        .find(|r| r.account_code == "410101")
        .unwrap();
    assert_eq!(revenue.closing_debit + revenue.closing_credit, 0);
    assert_eq!(trial.closing_debit, trial.closing_credit);

    // After the final closing voucher the balance sheet still reads the
    // books as they stood before it.
    let mut final_closing = voucher(
        "1405/12/29",
        vec![line("310101", 1_000_000, 0), line("110101", 0, 1_000_000)],
    );
    final_closing.voucher_type = "سند حسابداری اختتامیه".to_string();
    db.save_voucher(final_closing).unwrap();
    let balance = db.get_balance_sheet("1405/12/29").unwrap();
    assert_eq!(balance.assets_total, 1_000_000);
    assert!(balance.balanced);
}

#[test]
fn opening_voucher_cancels_the_previous_final_closing() {
    let db = db::memory_database();
    db.save_voucher(cash_sale("1405/01/10", 1_000_000)).unwrap();

    let mut closing = voucher(
        "1405/12/29",
        vec![line("410101", 1_000_000, 0), line("310101", 0, 1_000_000)],
    );
    closing.voucher_type = CLOSING_TEMPORARY.to_string();
    db.save_voucher(closing).unwrap();

    let mut final_closing = voucher(
        "1405/12/29",
        vec![line("310101", 1_000_000, 0), line("110101", 0, 1_000_000)],
    );
    final_closing.voucher_type = CLOSING_FINAL.to_string();
    db.save_voucher(final_closing).unwrap();

    // Before the next year is opened the statement still shows the books
    // as they stood before the final closing voucher.
    let before_opening = db.get_balance_sheet("1405/12/29").unwrap();
    assert_eq!(before_opening.assets_total, 1_000_000);
    assert_eq!(before_opening.equity_total, 1_000_000);
    assert!(before_opening.balanced);

    let mut opening = voucher(
        "1406/01/01",
        vec![line("110101", 1_000_000, 0), line("310101", 0, 1_000_000)],
    );
    opening.voucher_type = "سند حسابداری افتتاحیه".to_string();
    db.save_voucher(opening).unwrap();

    // With the opening voucher present, the closing and opening pair is
    // included and cancels out instead of doubling the balances.
    let after_opening = db.get_balance_sheet("1406/06/01").unwrap();
    assert_eq!(after_opening.assets_total, 1_000_000);
    assert_eq!(after_opening.equity_total, 1_000_000);
    assert_eq!(after_opening.profit, 0);
    assert!(after_opening.balanced);
}

#[test]
fn ledger_of_parent_account_includes_sub_accounts() {
    let db = db::memory_database();
    db.save_voucher(voucher(
        "1405/01/10",
        vec![line("110201001", 100_000, 0), line("410101", 0, 100_000)],
    ))
    .unwrap();
    db.save_voucher(voucher(
        "1405/01/11",
        vec![line("110201002", 50_000, 0), line("410101", 0, 50_000)],
    ))
    .unwrap();

    let parent = db.get_ledger("110201", "", "").unwrap();
    assert!(parent.includes_children);
    assert_eq!(parent.rows.len(), 2);
    assert_eq!(parent.total_debit, 150_000);
    assert_eq!(parent.closing_debit, 150_000);
    assert_eq!(parent.rows[0].account_code, "110201001");
    assert_eq!(parent.rows[1].running_debit, 150_000);

    // A grandparent reaches the leaf accounts through the middle level.
    let top = db.get_ledger("1102", "", "").unwrap();
    assert_eq!(top.total_debit, 150_000);

    let leaf = db.get_ledger("110201001", "", "").unwrap();
    assert!(!leaf.includes_children);
    assert_eq!(leaf.total_debit, 100_000);

    // Opening balance of the parent covers its sub-accounts as well.
    let later = db.get_ledger("110201", "1405/01/11", "").unwrap();
    assert_eq!(later.opening_debit, 100_000);
    assert_eq!(later.total_debit, 50_000);
}

#[test]
fn search_treats_wildcards_literally_and_folds_digits() {
    let db = db::memory_database();
    let mut first = cash_sale("1405/01/10", 100_000);
    first.description = "تخفیف 50% مشتری".to_string();
    db.save_voucher(first).unwrap();
    let mut second = cash_sale("1405/01/11", 100_000);
    second.description = "تخفیف 5000 ریالی".to_string();
    db.save_voucher(second).unwrap();

    assert_eq!(db.list_vouchers("", "", "50%").unwrap().vouchers.len(), 1);
    assert_eq!(db.list_vouchers("", "", "۵۰%").unwrap().vouchers.len(), 1);
    assert_eq!(db.list_vouchers("", "", "5_00").unwrap().vouchers.len(), 0);
    assert_eq!(db.list_vouchers("", "", "%").unwrap().vouchers.len(), 1);
    // Voucher numbers can be searched with Persian digits.
    assert_eq!(db.list_vouchers("", "", "۲").unwrap().vouchers.len(), 1);
    // Arabic yeh/kaf match the Persian letters stored in names.
    assert_eq!(db.list_vouchers("", "", "مشتري").unwrap().vouchers.len(), 1);
    assert_eq!(db.get_journal("", "", "50%").unwrap().voucher_count, 1);
}

#[test]
fn list_reports_total_count_and_rejects_reversed_range() {
    let db = db::memory_database();
    db.save_voucher(cash_sale("1405/01/10", 1_000)).unwrap();
    db.save_voucher(cash_sale("1405/02/10", 2_000)).unwrap();

    let list = db.list_vouchers("1405/02/01", "", "").unwrap();
    assert_eq!(list.vouchers.len(), 1);
    assert_eq!(list.total_count, 2);
    assert_eq!(list.matched_count, 1);

    assert!(db.list_vouchers("1405/03/01", "1405/02/01", "").is_err());
    assert!(db.get_journal("1405/03/01", "1405/02/01", "").is_err());
    assert!(db.get_ledger("110101", "1405/03/01", "1405/02/01").is_err());
    assert!(db.get_trial_balance("1405/03/01", "1405/02/01").is_err());
    assert!(db.get_income_statement("1405/03/01", "1405/02/01").is_err());
}

#[test]
fn paged_lists_keep_totals_for_the_whole_filter() {
    let db = db::memory_database();
    db.save_voucher(cash_sale("1405/01/10", 1_000)).unwrap();
    db.save_voucher(cash_sale("1405/01/11", 2_000)).unwrap();
    db.save_voucher(cash_sale("1405/01/12", 3_000)).unwrap();

    let page = db.list_vouchers_page("", "", "", Some(1), Some(1)).unwrap();
    assert_eq!(page.vouchers.len(), 1);
    assert_eq!(page.total_count, 3);
    assert_eq!(page.matched_count, 3);
    assert_eq!(page.total_debit, 6_000);
    assert_eq!(page.total_credit, 6_000);

    let journal = db.get_journal_page("", "", "", Some(0), Some(2)).unwrap();
    assert_eq!(journal.rows.len(), 2);
    assert_eq!(journal.row_count, 6);
    assert_eq!(journal.voucher_count, 3);
    assert_eq!(journal.total_debit, 6_000);
    assert_eq!(journal.total_credit, 6_000);

    assert!(db
        .list_vouchers_page("", "", "", Some(-1), Some(10))
        .is_err());
    assert!(db.list_vouchers_page("", "", "", Some(0), Some(0)).is_err());
    assert!(db.list_vouchers_page("", "", "", Some(10), None).is_err());
}

#[test]
fn rejects_amounts_above_the_safe_limit() {
    let db = db::memory_database();
    let err = db
        .save_voucher(cash_sale("1405/01/10", MAX_AMOUNT + 1))
        .unwrap_err();
    assert!(err.contains("بیشتر از"));
    assert!(db.save_voucher(cash_sale("1405/01/10", MAX_AMOUNT)).is_ok());
}

#[test]
fn voucher_number_limit_matches_javascript_safe_integer() {
    let db = db::memory_database();
    let mut accepted = cash_sale("1405/01/10", 1_000);
    accepted.number = Some(MAX_VOUCHER_NUMBER);
    assert!(db.save_voucher(accepted).is_ok());

    let mut rejected = cash_sale("1405/01/10", 1_000);
    rejected.number = Some(MAX_VOUCHER_NUMBER + 1);
    assert!(db.save_voucher(rejected).unwrap_err().contains("حداکثر"));
}
