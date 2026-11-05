//! Income statement and balance sheet builders.

use crate::db::{self, Database};
use crate::vouchers::accounting::{
    account_activities, classify_activity, section_of, AccountActivity, AccountClass,
    StatementSection,
};
use crate::vouchers::model::{
    AmountRow, BalanceSheetReport, IncomeStatementReport, ReportScope, CLOSING_FINAL,
    CLOSING_TEMPORARY,
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
        self.get_income_statement_scoped(from, to, ReportScope::Working)
    }

    pub fn get_income_statement_scoped(
        &self,
        from: &str,
        to: &str,
        scope: ReportScope,
    ) -> Result<IncomeStatementReport, String> {
        let from = normalize_filter_date(from, "تاریخ شروع")?;
        let to = normalize_filter_date(to, "تاریخ پایان")?;
        check_range(&from, &to)?;

        self.with_conn(|conn| {
            // Closing vouchers move income and expenses to equity; leaving
            // them in would show every closed period as having no activity.
            let activities = account_activities(
                conn,
                &from,
                &to,
                false,
                &[CLOSING_TEMPORARY, CLOSING_FINAL],
                scope,
            )?;
            build_income_statement(from, to, activities)
        })
    }

    pub fn get_balance_sheet(&self, as_of: &str) -> Result<BalanceSheetReport, String> {
        self.get_balance_sheet_scoped(as_of, ReportScope::Working)
    }

    pub fn get_balance_sheet_scoped(
        &self,
        as_of: &str,
        scope: ReportScope,
    ) -> Result<BalanceSheetReport, String> {
        let as_of = normalize_date(as_of, "تاریخ ترازنامه")?;
        self.with_conn(|conn| {
            // A final closing voucher without a later opening voucher zeroes
            // every balance-sheet account, so the statement is prepared from
            // the books before it. `account_activities` applies that rule.
            let activities = account_activities(conn, "", &as_of, true, &[], scope)?;
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
    let mut finance_costs = Vec::new();
    let mut income_taxes = Vec::new();

    for activity in activities {
        let net = checked_sub(
            activity.debit,
            activity.credit,
            "مانده حساب صورت سود و زیان",
        )?;
        if net == 0 {
            continue;
        }

        let section = section_of(&activity.default_section);
        if matches!(section, Some(StatementSection::Memorandum)) {
            continue;
        }
        match section {
            Some(StatementSection::OtherOperating) | Some(StatementSection::OtherNonOperating) => {
                let other_operating = matches!(section, Some(StatementSection::OtherOperating));
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
                match (other_operating, net < 0) {
                    (true, true) => other_operating_revenues.push(row),
                    (true, false) => other_operating_expenses.push(row),
                    (false, true) => non_operating_revenues.push(row),
                    (false, false) => non_operating_expenses.push(row),
                }
                continue;
            }
            Some(StatementSection::Revenue) => {
                revenues.push(AmountRow {
                    account_code: activity.account_code,
                    account_name: activity.account_name,
                    amount: checked_neg(net, "مبلغ درآمد")?,
                });
                continue;
            }
            Some(StatementSection::Cogs) => {
                cogs.push(AmountRow {
                    account_code: activity.account_code,
                    account_name: activity.account_name,
                    amount: net,
                });
                continue;
            }
            Some(StatementSection::SellingExpense) | Some(StatementSection::AdminExpense) => {
                expenses.push(AmountRow {
                    account_code: activity.account_code,
                    account_name: activity.account_name,
                    amount: net,
                });
                continue;
            }
            Some(StatementSection::FinanceCost) => {
                finance_costs.push(AmountRow {
                    account_code: activity.account_code,
                    account_name: activity.account_name,
                    amount: net,
                });
                continue;
            }
            Some(StatementSection::IncomeTax) => {
                income_taxes.push(AmountRow {
                    account_code: activity.account_code,
                    account_name: activity.account_name,
                    amount: net,
                });
                continue;
            }
            // Balance-sheet sections are skipped below by classification.
            Some(_) | None => {}
        }

        if activity.group_code == db::OTHER_OPERATING_GROUP_CODE
            || activity.group_code == db::OTHER_NON_OPERATING_GROUP_CODE
        {
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
                (db::OTHER_OPERATING_GROUP_CODE, true) => other_operating_revenues.push(row),
                (db::OTHER_OPERATING_GROUP_CODE, false) => other_operating_expenses.push(row),
                (db::OTHER_NON_OPERATING_GROUP_CODE, true) => non_operating_revenues.push(row),
                (db::OTHER_NON_OPERATING_GROUP_CODE, false) => non_operating_expenses.push(row),
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
    let finance_cost_total =
        checked_sum(finance_costs.iter().map(|r| r.amount), "جمع هزینه‌های مالی")?;
    let income_tax_total =
        checked_sum(income_taxes.iter().map(|r| r.amount), "جمع مالیات بر درآمد")?;
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
    let profit_before_tax = checked_sub(
        checked_add(
            operating_profit,
            non_operating_revenue_total,
            "سود پیش از هزینه‌های غیرعملیاتی",
        )?,
        checked_add(
            non_operating_expense_total,
            finance_cost_total,
            "هزینه‌های غیرعملیاتی و مالی",
        )?,
        "سود پیش از مالیات",
    )?;
    let net_profit = checked_sub(profit_before_tax, income_tax_total, "سود خالص")?;
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
        finance_costs,
        finance_cost_total,
        income_taxes,
        income_tax_total,
        gross_profit,
        operating_profit,
        profit_before_tax,
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
            AccountClass::Memorandum => {}
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
