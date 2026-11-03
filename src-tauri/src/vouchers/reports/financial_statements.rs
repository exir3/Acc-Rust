//! Income statement and balance sheet builders.

use crate::db::Database;
use crate::vouchers::accounting::{
    account_activities, classify_activity, AccountActivity, AccountClass,
};
use crate::vouchers::model::{
    AmountRow, BalanceSheetReport, IncomeStatementReport, CLOSING_FINAL, CLOSING_TEMPORARY,
};
use crate::vouchers::validation::{
    check_range, checked_add, checked_neg, checked_sub, checked_sum, normalize_date,
    normalize_filter_date,
};

impl Database {
    pub fn get_income_statement(
        &self,
        from: &str,
        to: &str,
    ) -> Result<IncomeStatementReport, String> {
        let from = normalize_filter_date(from, "تاریخ شروع")?;
        let to = normalize_filter_date(to, "تاریخ پایان")?;
        check_range(&from, &to)?;

        self.with_conn(|conn| {
            // Closing vouchers move income and expenses to equity; leaving
            // them in would show every closed period as having no activity.
            let activities =
                account_activities(conn, &from, &to, false, &[CLOSING_TEMPORARY, CLOSING_FINAL])?;
            build_income_statement(from, to, activities)
        })
    }

    pub fn get_balance_sheet(&self, as_of: &str) -> Result<BalanceSheetReport, String> {
        let as_of = normalize_date(as_of, "تاریخ ترازنامه")?;
        self.with_conn(|conn| {
            // A final closing voucher without a later opening voucher zeroes
            // every balance-sheet account, so the statement is prepared from
            // the books before it. `account_activities` applies that rule.
            let activities = account_activities(conn, "", &as_of, true, &[])?;
            build_balance_sheet(as_of, activities)
        })
    }
}

fn build_income_statement(
    from: String,
    to: String,
    activities: Vec<AccountActivity>,
) -> Result<IncomeStatementReport, String> {
    let mut revenues = Vec::new();
    let mut cogs = Vec::new();
    let mut other_operating_revenues = Vec::new();
    let mut other_operating_expenses = Vec::new();
    let mut expenses = Vec::new();
    let mut non_operating_revenues = Vec::new();
    let mut non_operating_expenses = Vec::new();

    for activity in activities {
        let net = checked_sub(
            activity.debit,
            activity.credit,
            "مانده حساب صورت سود و زیان",
        )?;
        if net == 0 {
            continue;
        }
        if activity.group_code == "10" || activity.group_code == "11" {
            let amount = if net < 0 {
                checked_neg(net, "مبلغ درآمد گروه‌های سایر")?
            } else {
                net
            };
            let row = AmountRow {
                account_code: activity.account_code,
                account_name: activity.account_name,
                amount,
            };
            match (activity.group_code.as_str(), net < 0) {
                ("10", true) => other_operating_revenues.push(row),
                ("10", false) => other_operating_expenses.push(row),
                ("11", true) => non_operating_revenues.push(row),
                ("11", false) => non_operating_expenses.push(row),
                _ => unreachable!(),
            }
            continue;
        }
        match classify_activity(&activity, net) {
            AccountClass::Income => {
                let amount = checked_neg(net, "مبلغ درآمد")?;
                if amount != 0 {
                    let row = AmountRow {
                        account_code: activity.account_code,
                        account_name: activity.account_name,
                        amount,
                    };
                    revenues.push(row);
                }
            }
            AccountClass::Expense => {
                let amount = net;
                if amount == 0 {
                    continue;
                }
                let row = AmountRow {
                    account_code: activity.account_code.clone(),
                    account_name: activity.account_name,
                    amount,
                };
                if activity.group_code == "9" {
                    cogs.push(row);
                } else {
                    expenses.push(row);
                }
            }
            _ => {}
        }
    }

    let revenue_total = checked_sum(
        revenues.iter().map(|r| r.amount),
        "جمع درآمد صورت سود و زیان",
    )?;
    let cogs_total = checked_sum(cogs.iter().map(|r| r.amount), "جمع بهای تمام‌شده")?;
    let other_operating_revenue_total = checked_sum(
        other_operating_revenues.iter().map(|r| r.amount),
        "جمع سایر درآمدهای عملیاتی",
    )?;
    let other_operating_expense_total = checked_sum(
        other_operating_expenses.iter().map(|r| r.amount),
        "جمع سایر هزینه‌های عملیاتی",
    )?;
    let expense_total = checked_sum(
        expenses.iter().map(|r| r.amount),
        "جمع هزینه صورت سود و زیان",
    )?;
    let non_operating_revenue_total = checked_sum(
        non_operating_revenues.iter().map(|r| r.amount),
        "جمع درآمدهای غیرعملیاتی",
    )?;
    let non_operating_expense_total = checked_sum(
        non_operating_expenses.iter().map(|r| r.amount),
        "جمع هزینه‌های غیرعملیاتی",
    )?;
    let gross_profit = checked_sub(revenue_total, cogs_total, "سود ناخالص")?;
    let operating_profit = checked_sub(
        checked_add(
            gross_profit,
            other_operating_revenue_total,
            "سود عملیاتی پس از سایر درآمدها",
        )?,
        checked_add(
            expense_total,
            other_operating_expense_total,
            "هزینه‌های عملیاتی",
        )?,
        "سود عملیاتی",
    )?;
    let net_profit = checked_sub(
        checked_add(
            operating_profit,
            non_operating_revenue_total,
            "سود پیش از هزینه‌های غیرعملیاتی",
        )?,
        non_operating_expense_total,
        "سود خالص",
    )?;
    Ok(IncomeStatementReport {
        from,
        to,
        revenues,
        revenue_total,
        cogs,
        cogs_total,
        other_operating_revenues,
        other_operating_revenue_total,
        other_operating_expenses,
        other_operating_expense_total,
        expenses,
        expense_total,
        non_operating_revenues,
        non_operating_revenue_total,
        non_operating_expenses,
        non_operating_expense_total,
        gross_profit,
        operating_profit,
        net_profit,
    })
}

fn build_balance_sheet(
    as_of: String,
    activities: Vec<AccountActivity>,
) -> Result<BalanceSheetReport, String> {
    let mut assets = Vec::new();
    let mut liabilities = Vec::new();
    let mut equity = Vec::new();
    let mut revenue = 0;
    let mut expenses = 0;

    for activity in activities {
        let net = checked_sub(activity.debit, activity.credit, "مانده حساب ترازنامه")?;
        if net == 0 {
            continue;
        }
        match classify_activity(&activity, net) {
            AccountClass::Asset => assets.push(AmountRow {
                account_code: activity.account_code,
                account_name: activity.account_name,
                amount: net,
            }),
            AccountClass::Liability => liabilities.push(AmountRow {
                account_code: activity.account_code,
                account_name: activity.account_name,
                amount: checked_neg(net, "مانده بدهی ترازنامه")?,
            }),
            AccountClass::Equity => equity.push(AmountRow {
                account_code: activity.account_code,
                account_name: activity.account_name,
                amount: checked_neg(net, "مانده سرمایه ترازنامه")?,
            }),
            AccountClass::Income => {
                revenue = checked_add(
                    revenue,
                    checked_neg(net, "درآمد ترازنامه")?,
                    "جمع درآمد ترازنامه",
                )?
            }
            AccountClass::Expense => expenses = checked_add(expenses, net, "جمع هزینه ترازنامه")?,
        }
    }

    let assets_total = checked_sum(assets.iter().map(|r| r.amount), "جمع دارایی ترازنامه")?;
    let liabilities_total = checked_sum(liabilities.iter().map(|r| r.amount), "جمع بدهی ترازنامه")?;
    let equity_total = checked_sum(equity.iter().map(|r| r.amount), "جمع سرمایه ترازنامه")?;
    let profit = checked_sub(revenue, expenses, "سود ترازنامه")?;
    let total_liabilities_equity = checked_add(
        checked_add(
            liabilities_total,
            equity_total,
            "جمع بدهی و سرمایه ترازنامه",
        )?,
        profit,
        "جمع بدهی و سرمایه با سود ترازنامه",
    )?;
    Ok(BalanceSheetReport {
        as_of,
        assets,
        assets_total,
        liabilities,
        liabilities_total,
        equity,
        equity_total,
        profit,
        total_liabilities_equity,
        balanced: assets_total == total_liabilities_equity,
    })
}
