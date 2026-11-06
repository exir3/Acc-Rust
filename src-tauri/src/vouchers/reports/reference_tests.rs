//! Reference-dataset regression tests for the financial statements.
//!
//! The dataset below is deliberately small but covers every statement section,
//! dual-nature groups, opening/temporary/final-closing vouchers, detail
//! accounts, working versus official scopes and a leap-year boundary. The
//! expected values live in `fixtures/report_reference.json` and are compared
//! exactly, so a classification change cannot silently alter a report.

use crate::db::{self, Database};
use crate::vouchers::model::{
    ReportScope, Voucher, VoucherInput, VoucherLineInput, CLOSING_FINAL, CLOSING_TEMPORARY,
};
use serde_json::{json, Value};

const OPENING_TYPE: &str = "سند حسابداری افتتاحیه";

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

fn detail_line(
    account_code: &str,
    detail1_code: &str,
    debit: i64,
    credit: i64,
) -> VoucherLineInput {
    VoucherLineInput {
        id: None,
        account_code: account_code.to_string(),
        detail1_code: Some(detail1_code.to_string()),
        description: String::new(),
        debit,
        credit,
    }
}

fn voucher(date: &str, voucher_type: &str, lines: Vec<VoucherLineInput>) -> VoucherInput {
    VoucherInput {
        id: None,
        number: None,
        date: date.to_string(),
        voucher_type: voucher_type.to_string(),
        description: format!("سند مرجع {date}"),
        fiscal_year_id: None,
        lines,
    }
}

fn save_permanent(db: &Database, input: VoucherInput) -> Voucher {
    let saved = db.save_voucher(input).unwrap();
    db.set_voucher_permanent(saved.id, Some(&saved.updated_at))
        .unwrap()
}

/// Builds the reference chart and book. Every voucher except the deliberately
/// temporary sale at 1405/02/15 is finalized so the official scope can be
/// compared with the working scope.
fn reference_database() -> Database {
    let db = db::memory_database();

    // Groups 20..33 exercise the explicit statement-section mapping. Groups 10
    // and 11 are the built-in dual groups and intentionally keep no section.
    for (code, name, group_type, section) in [
        ("20", "دارایی جاری مرجع", "دارایی", "current_asset"),
        ("21", "دارایی غیرجاری مرجع", "دارایی", "non_current_asset"),
        ("22", "بدهی جاری مرجع", "بدهی", "current_liability"),
        ("23", "بدهی غیرجاری مرجع", "بدهی", "non_current_liability"),
        ("24", "سرمایه مرجع", "سرمایه", "equity"),
        ("25", "درآمد مرجع", "درآمد", "revenue"),
        ("26", "بهای تمام‌شده مرجع", "هزینه", "cogs"),
        ("27", "هزینه فروش مرجع", "هزینه", "selling_expense"),
        ("28", "هزینه اداری مرجع", "هزینه", "admin_expense"),
        ("29", "سایر عملیاتی مرجع", "سایر", "other_operating"),
        ("30", "سایر غیرعملیاتی مرجع", "سایر", "other_non_operating"),
        ("31", "هزینه مالی مرجع", "هزینه", "finance_cost"),
        ("32", "مالیات مرجع", "هزینه", "income_tax"),
        ("33", "انتظامی مرجع", "سایر", "memorandum"),
    ] {
        db.create_account_group(code, name, group_type, section)
            .unwrap();
    }

    for (code, name, level, group_code, nature, parent_code) in [
        ("1010", "موجودی نقد", 1, "20", "debtor", ""),
        ("101001", "صندوق", 2, "20", "debtor", "1010"),
        ("1020", "حساب‌های دریافتنی", 1, "20", "debtor", ""),
        ("102001", "بدهکاران تجاری", 2, "20", "debtor", "1020"),
        (
            "102001001",
            "مشتری الف - شعبه شمال",
            3,
            "20",
            "debtor",
            "102001",
        ),
        ("1030", "موجودی کالا", 1, "20", "debtor", ""),
        ("1110", "دارایی ثابت", 1, "21", "debtor", ""),
        ("2010", "حساب‌های پرداختنی", 1, "22", "creditor", ""),
        ("201001", "بستانکاران تجاری", 2, "22", "creditor", "2010"),
        ("2020", "وام کوتاه‌مدت", 1, "22", "creditor", ""),
        ("2110", "وام بلندمدت", 1, "23", "creditor", ""),
        ("3010", "سرمایه", 1, "24", "creditor", ""),
        ("3020", "سود و زیان انباشته", 1, "24", "creditor", ""),
        ("4010", "فروش", 1, "25", "creditor", ""),
        ("4020", "برگشت از فروش", 1, "25", "debtor", ""),
        ("5010", "بهای تمام‌شده کالای فروش‌رفته", 1, "26", "debtor", ""),
        ("6010", "هزینه فروش", 1, "27", "debtor", ""),
        ("7010", "هزینه اداری", 1, "28", "debtor", ""),
        ("8010", "سایر درآمدهای عملیاتی", 1, "29", "creditor", ""),
        ("8020", "سایر هزینه‌های عملیاتی", 1, "29", "debtor", ""),
        ("9010", "درآمدهای غیرعملیاتی", 1, "30", "creditor", ""),
        ("9020", "هزینه‌های غیرعملیاتی", 1, "30", "debtor", ""),
        ("9510", "هزینه‌های مالی", 1, "31", "debtor", ""),
        ("9610", "مالیات بر درآمد", 1, "32", "debtor", ""),
        ("9710", "حساب انتظامی بدهکار", 1, "33", "dual", ""),
        ("9720", "حساب انتظامی بستانکار", 1, "33", "dual", ""),
        (
            "9810",
            "سایر هزینه عملیاتی (گروه دوگانه)",
            1,
            "10",
            "dual",
            "",
        ),
        (
            "9820",
            "سایر درآمد عملیاتی (گروه دوگانه)",
            1,
            "10",
            "dual",
            "",
        ),
        (
            "9910",
            "سایر درآمد غیرعملیاتی (گروه دوگانه)",
            1,
            "11",
            "dual",
            "",
        ),
    ] {
        db.create_account(code, name, level, group_code, nature, parent_code, true)
            .unwrap();
    }

    // Opening balances: 1,350,000 assets = 450,000 liabilities + 900,000 equity.
    save_permanent(
        &db,
        voucher(
            "1405/01/01",
            OPENING_TYPE,
            vec![
                line("101001", 500_000, 0),
                line("102001", 200_000, 0),
                line("1030", 250_000, 0),
                line("1110", 400_000, 0),
                line("201001", 0, 120_000),
                line("2020", 0, 80_000),
                line("2110", 0, 250_000),
                line("3010", 0, 900_000),
            ],
        ),
    );

    // January: sale through a level-3 detail account, cost of goods, selling and
    // administrative expenses.
    save_permanent(
        &db,
        voucher(
            "1405/01/15",
            "",
            vec![
                detail_line("102001", "102001001", 300_000, 0),
                line("4010", 0, 300_000),
            ],
        ),
    );
    save_permanent(
        &db,
        voucher(
            "1405/01/16",
            "",
            vec![line("5010", 180_000, 0), line("1030", 0, 180_000)],
        ),
    );
    save_permanent(
        &db,
        voucher(
            "1405/01/20",
            "",
            vec![line("6010", 30_000, 0), line("101001", 0, 30_000)],
        ),
    );
    save_permanent(
        &db,
        voucher(
            "1405/01/21",
            "",
            vec![line("7010", 45_000, 0), line("101001", 0, 45_000)],
        ),
    );

    // February: every remaining statement section, including the built-in
    // dual groups 10 and 11 with debit and credit balances.
    save_permanent(
        &db,
        voucher(
            "1405/02/01",
            "",
            vec![line("101001", 12_000, 0), line("8010", 0, 12_000)],
        ),
    );
    save_permanent(
        &db,
        voucher(
            "1405/02/02",
            "",
            vec![line("8020", 7_000, 0), line("101001", 0, 7_000)],
        ),
    );
    save_permanent(
        &db,
        voucher(
            "1405/02/03",
            "",
            vec![line("101001", 9_000, 0), line("9010", 0, 9_000)],
        ),
    );
    save_permanent(
        &db,
        voucher(
            "1405/02/04",
            "",
            vec![line("9020", 4_000, 0), line("101001", 0, 4_000)],
        ),
    );
    save_permanent(
        &db,
        voucher(
            "1405/02/05",
            "",
            vec![line("9510", 6_000, 0), line("101001", 0, 6_000)],
        ),
    );
    save_permanent(
        &db,
        voucher(
            "1405/02/06",
            "",
            vec![line("9610", 8_000, 0), line("101001", 0, 8_000)],
        ),
    );
    save_permanent(
        &db,
        voucher(
            "1405/02/07",
            "",
            vec![line("9710", 25_000, 0), line("9720", 0, 25_000)],
        ),
    );
    save_permanent(
        &db,
        voucher(
            "1405/02/08",
            "",
            vec![line("9810", 3_000, 0), line("101001", 0, 3_000)],
        ),
    );
    save_permanent(
        &db,
        voucher(
            "1405/02/09",
            "",
            vec![line("101001", 5_000, 0), line("9820", 0, 5_000)],
        ),
    );
    save_permanent(
        &db,
        voucher(
            "1405/02/10",
            "",
            vec![line("101001", 2_000, 0), line("9910", 0, 2_000)],
        ),
    );

    // Deliberately temporary draft sale. It must appear in working reports and
    // be excluded from official reports.
    db.save_voucher(voucher(
        "1405/02/15",
        "",
        vec![line("102001", 50_000, 0), line("4010", 0, 50_000)],
    ))
    .unwrap();

    // Contra-revenue: a debit balance in the revenue group reduces revenue.
    save_permanent(
        &db,
        voucher(
            "1405/02/20",
            "",
            vec![line("4020", 20_000, 0), line("102001", 0, 20_000)],
        ),
    );

    // Temporary closing closes the finalized income and expense balances to
    // retained earnings. The draft sale stays open, as it is not finalized.
    save_permanent(
        &db,
        voucher(
            "1405/12/29",
            CLOSING_TEMPORARY,
            vec![
                line("4010", 300_000, 0),
                line("8010", 12_000, 0),
                line("9820", 5_000, 0),
                line("9010", 9_000, 0),
                line("9910", 2_000, 0),
                line("4020", 0, 20_000),
                line("5010", 0, 180_000),
                line("6010", 0, 30_000),
                line("7010", 0, 45_000),
                line("8020", 0, 7_000),
                line("9810", 0, 3_000),
                line("9020", 0, 4_000),
                line("9510", 0, 6_000),
                line("9610", 0, 8_000),
                line("3020", 0, 25_000),
            ],
        ),
    );

    // Final closing zeroes the finalized balance-sheet accounts. It stays
    // hidden until the next opening voucher exists.
    save_permanent(
        &db,
        voucher(
            "1405/12/29",
            CLOSING_FINAL,
            vec![
                line("201001", 120_000, 0),
                line("2020", 80_000, 0),
                line("2110", 250_000, 0),
                line("3010", 900_000, 0),
                line("3020", 25_000, 0),
                line("101001", 0, 425_000),
                line("102001", 0, 180_000),
                detail_line("102001", "102001001", 0, 300_000),
                line("1030", 0, 70_000),
                line("1110", 0, 400_000),
            ],
        ),
    );

    // Next year's opening cancels the final closing instead of doubling it.
    save_permanent(
        &db,
        voucher(
            "1406/01/01",
            OPENING_TYPE,
            vec![
                line("101001", 425_000, 0),
                line("102001", 180_000, 0),
                detail_line("102001", "102001001", 300_000, 0),
                line("1030", 70_000, 0),
                line("1110", 400_000, 0),
                line("201001", 0, 120_000),
                line("2020", 0, 80_000),
                line("2110", 0, 250_000),
                line("3010", 0, 900_000),
                line("3020", 0, 25_000),
            ],
        ),
    );

    db
}

fn trial_snapshot(report: &crate::vouchers::model::TrialBalanceReport) -> Value {
    json!({
        "totals": {
            "openingDebit": report.opening_debit,
            "openingCredit": report.opening_credit,
            "periodDebit": report.period_debit,
            "periodCredit": report.period_credit,
            "closingDebit": report.closing_debit,
            "closingCredit": report.closing_credit,
        },
        "rows": report.rows.iter().map(|row| json!({
            "code": row.account_code,
            "level": row.level,
            "parent": row.parent_code,
            "openingDebit": row.opening_debit,
            "openingCredit": row.opening_credit,
            "periodDebit": row.period_debit,
            "periodCredit": row.period_credit,
            "closingDebit": row.closing_debit,
            "closingCredit": row.closing_credit,
        })).collect::<Vec<_>>(),
    })
}

fn amount_rows(rows: &[crate::vouchers::model::AmountRow]) -> Vec<Value> {
    rows.iter()
        .map(|row| json!({"code": row.account_code, "amount": row.amount}))
        .collect()
}

fn income_snapshot(report: &crate::vouchers::model::IncomeStatementReport) -> Value {
    json!({
        "totals": {
            "revenue": report.revenue_total,
            "cogs": report.cogs_total,
            "otherOperatingRevenue": report.other_operating_revenue_total,
            "otherOperatingExpense": report.other_operating_expense_total,
            "expenses": report.expense_total,
            "nonOperatingRevenue": report.non_operating_revenue_total,
            "nonOperatingExpense": report.non_operating_expense_total,
            "financeCost": report.finance_cost_total,
            "incomeTax": report.income_tax_total,
            "grossProfit": report.gross_profit,
            "operatingProfit": report.operating_profit,
            "profitBeforeTax": report.profit_before_tax,
            "netProfit": report.net_profit,
        },
        "revenues": amount_rows(&report.revenues),
        "cogs": amount_rows(&report.cogs),
        "otherOperatingRevenues": amount_rows(&report.other_operating_revenues),
        "otherOperatingExpenses": amount_rows(&report.other_operating_expenses),
        "expenses": amount_rows(&report.expenses),
        "nonOperatingRevenues": amount_rows(&report.non_operating_revenues),
        "nonOperatingExpenses": amount_rows(&report.non_operating_expenses),
        "financeCosts": amount_rows(&report.finance_costs),
        "incomeTaxes": amount_rows(&report.income_taxes),
    })
}

fn balance_snapshot(report: &crate::vouchers::model::BalanceSheetReport) -> Value {
    json!({
        "totals": {
            "assets": report.assets_total,
            "liabilities": report.liabilities_total,
            "equity": report.equity_total,
            "profit": report.profit,
            "totalLiabilitiesEquity": report.total_liabilities_equity,
            "balanced": report.balanced,
        },
        "assets": amount_rows(&report.assets),
        "liabilities": amount_rows(&report.liabilities),
        "equity": amount_rows(&report.equity),
    })
}

fn reference_snapshot() -> Value {
    let db = reference_database();

    let trial_working = db.get_trial_balance("1405/01/01", "1405/02/28").unwrap();
    let trial_february = db.get_trial_balance("1405/02/01", "1405/02/28").unwrap();
    let trial_official = db
        .get_trial_balance_filtered_scoped(
            "1405/01/01",
            "1405/02/28",
            "",
            "",
            true,
            true,
            ReportScope::Official,
        )
        .unwrap();

    let income_working = db
        .get_income_statement_scoped("1405/01/01", "1405/02/28", ReportScope::Working)
        .unwrap();
    let income_official = db
        .get_income_statement_scoped("1405/01/01", "1405/02/28", ReportScope::Official)
        .unwrap();
    let income_february = db
        .get_income_statement_scoped("1405/02/01", "1405/02/28", ReportScope::Working)
        .unwrap();

    let balance_working_before_closing = db
        .get_balance_sheet_scoped("1405/02/28", ReportScope::Working)
        .unwrap();
    let balance_official_before_closing = db
        .get_balance_sheet_scoped("1405/02/28", ReportScope::Official)
        .unwrap();
    let balance_working_after_closing = db
        .get_balance_sheet_scoped("1405/12/29", ReportScope::Working)
        .unwrap();
    // The final closing voucher is hidden until the next opening voucher, so
    // this report is the pre-final-closing position.
    let balance_official_after_closing = db
        .get_balance_sheet_scoped("1405/12/29", ReportScope::Official)
        .unwrap();
    let balance_official_after_opening = db
        .get_balance_sheet_scoped("1406/06/01", ReportScope::Official)
        .unwrap();
    let balance_working_after_opening = db
        .get_balance_sheet_scoped("1406/06/01", ReportScope::Working)
        .unwrap();

    let leap_db = db::memory_database();
    leap_db
        .save_voucher(voucher(
            "1403/12/30",
            "",
            vec![line("110101", 1_000, 0), line("410101", 0, 1_000)],
        ))
        .unwrap();
    let leap_trial = leap_db
        .get_trial_balance("1403/12/30", "1403/12/30")
        .unwrap();
    let leap_income = leap_db
        .get_income_statement("1403/12/29", "1403/12/30")
        .unwrap();
    let leap_balance = leap_db.get_balance_sheet("1403/12/30").unwrap();

    json!({
        "trialBalanceWorking": trial_snapshot(&trial_working),
        "trialBalanceFebruary": trial_snapshot(&trial_february),
        "trialBalanceOfficial": trial_snapshot(&trial_official),
        "incomeStatementWorking": income_snapshot(&income_working),
        "incomeStatementOfficial": income_snapshot(&income_official),
        "incomeStatementFebruary": income_snapshot(&income_february),
        "balanceSheetWorkingBeforeClosing": balance_snapshot(&balance_working_before_closing),
        "balanceSheetOfficialBeforeClosing": balance_snapshot(&balance_official_before_closing),
        "balanceSheetWorkingAtYearEnd": balance_snapshot(&balance_working_after_closing),
        "balanceSheetOfficialAtYearEnd": balance_snapshot(&balance_official_after_closing),
        "balanceSheetOfficialAfterOpening": balance_snapshot(&balance_official_after_opening),
        "balanceSheetWorkingAfterOpening": balance_snapshot(&balance_working_after_opening),
        "leapYear": {
            "trialPeriodDebit": leap_trial.period_debit,
            "trialPeriodCredit": leap_trial.period_credit,
            "trialClosingDebit": leap_trial.closing_debit,
            "trialClosingCredit": leap_trial.closing_credit,
            "incomeNetProfit": leap_income.net_profit,
            "balanceBalanced": leap_balance.balanced,
            "balanceAssets": leap_balance.assets_total,
        },
    })
}

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/report_reference.json")).unwrap()
}

#[test]
fn financial_reports_match_the_reference_fixture() {
    let actual = reference_snapshot();
    let expected = fixture();
    assert_eq!(
        actual, expected,
        "report output no longer matches the approved reference fixture; if the classification change is intentional, update \
         src-tauri/src/vouchers/reports/fixtures/report_reference.json deliberately"
    );
}

#[test]
#[ignore]
fn print_reference_snapshot() {
    println!(
        "{}",
        serde_json::to_string_pretty(&reference_snapshot()).unwrap()
    );
}
