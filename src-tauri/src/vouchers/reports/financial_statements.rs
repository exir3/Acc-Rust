//! Income statement and balance sheet builders.
//!
//! Both statements are driven by each account's effective `fs_line`, not by
//! its group type alone. The contribution amount already applies the WebApp
//! rules for contra accounts and `split_by_sign`, so a misclassified account
//! stays visible instead of silently making the statement unbalanced.

use crate::db::fs_catalog::{FsSection, UNCLASSIFIED};
use crate::db::Database;
use crate::vouchers::accounting::account_activities;
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

fn row(activity: &crate::vouchers::accounting::AccountActivity, amount: i64) -> AmountRow {
    AmountRow {
        account_code: activity.account_code.clone(),
        account_name: activity.account_name.clone(),
        amount,
        fs_line: activity.contribution_line.clone(),
    }
}

fn build_income_statement(
    from: String,
    to: String,
    activities: Vec<crate::vouchers::accounting::AccountActivity>,
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
        let amount = activity.contribution_amount;
        if amount == 0 {
            continue;
        }
        if activity.statement() == crate::db::fs_catalog::FsStatement::BalanceSheet
            || activity.statement() == crate::db::fs_catalog::FsStatement::Memorandum
        {
            continue;
        }
        match activity.contribution_line.as_str() {
            "IS.REV" => revenues.push(row(&activity, amount)),
            "IS.COGS" => cogs.push(row(&activity, amount)),
            "IS.SELL" | "IS.ADM" => expenses.push(row(&activity, amount)),
            "IS.OOI" => other_operating_revenues.push(row(&activity, amount)),
            "IS.OOE" => other_operating_expenses.push(row(&activity, amount)),
            "IS.FIN" => finance_costs.push(row(&activity, amount)),
            "IS.NOI" => {
                if amount < 0 {
                    non_operating_expenses.push(row(
                        &activity,
                        checked_neg(amount, "مبلغ هزینه غیرعملیاتی")?,
                    ));
                } else {
                    non_operating_revenues.push(row(&activity, amount));
                }
            }
            "IS.TAX" => income_taxes.push(row(&activity, amount)),
            UNCLASSIFIED => {
                // Unclassified balances belong to the balance-sheet review
                // bucket, not to a guessed income-statement line.
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
    activities: Vec<crate::vouchers::accounting::AccountActivity>,
) -> Result<BalanceSheetReport, String> {
    let mut assets = Vec::new();
    let mut liabilities = Vec::new();
    let mut equity = Vec::new();
    let mut profit = 0i64;

    for activity in activities {
        let amount = activity.contribution_amount;
        if amount == 0 {
            continue;
        }
        let line = activity.contribution_line.as_str();
        let section = crate::db::fs_catalog::section_of(line);
        match section {
            FsSection::NonCurrentAsset | FsSection::CurrentAsset => {
                assets.push(row(&activity, amount));
            }
            FsSection::NonCurrentLiability | FsSection::CurrentLiability => {
                liabilities.push(row(&activity, amount));
            }
            FsSection::Equity => equity.push(row(&activity, amount)),
            FsSection::Revenue | FsSection::OtherOperating | FsSection::OtherNonOperating => {
                // A negative amount here is a contra-revenue item and reduces
                // the period result instead of being dropped.
                if line == "IS.OOE" || line == "IS.COGS" {
                    profit = checked_sub(profit, amount, "سود ترازنامه")?;
                } else {
                    profit = checked_add(profit, amount, "سود ترازنامه")?;
                }
            }
            FsSection::Cogs
            | FsSection::SellingExpense
            | FsSection::AdminExpense
            | FsSection::FinanceCost
            | FsSection::IncomeTax => {
                profit = checked_sub(profit, amount, "سود ترازنامه")?;
            }
            FsSection::Memorandum => {}
            FsSection::Unclassified => {
                // Keep the accounting equation intact and visible instead of
                // making the sheet silently unbalanced.
                if amount > 0 {
                    assets.push(row(&activity, amount));
                } else {
                    liabilities.push(row(&activity, checked_neg(amount, "مانده طبقه‌بندی‌نشده")?));
                }
            }
        }
    }

    let assets_total = checked_sum(assets.iter().map(|r| r.amount), "جمع دارایی ترازنامه")?;
    let liabilities_total = checked_sum(liabilities.iter().map(|r| r.amount), "جمع بدهی ترازنامه")?;
    let equity_total = checked_sum(equity.iter().map(|r| r.amount), "جمع سرمایه ترازنامه")?;
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
