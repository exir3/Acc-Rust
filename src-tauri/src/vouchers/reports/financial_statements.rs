//! Income statement and balance sheet builders.
//!
//! Both statements are driven by each account's effective `fs_line`, not by
//! its group type alone. The contribution amount already applies the WebApp
//! rules for contra accounts and `split_by_sign`, so a misclassified account
//! stays visible instead of silently making the statement unbalanced.
//!
//! Accounts without a valid statement line are never guessed into assets,
//! liabilities or an income-statement section. Their debit and credit balances
//! are reported separately, and an integrity report explains why the statement
//! needs classification work before it can be trusted.

use crate::db::fs_catalog::{section_of, FsSection, UNCLASSIFIED};
use crate::db::Database;
use crate::vouchers::accounting::account_activities;
use crate::vouchers::model::{
    AmountRow, BalanceSheetReport, IncomeStatementReport, IntegrityReport, ReportScope,
    CLOSING_FINAL, CLOSING_TEMPORARY,
};
use crate::vouchers::validation::{
    check_range, checked_add, checked_neg, checked_sub, checked_sum, normalize_date,
    normalize_filter_date,
};

use super::integrity::run_integrity_checks;

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
            let mut report = build_income_statement(from, to, activities)?;
            report.integrity =
                run_integrity_checks(conn, &report.from, &report.to, false, scope, None)?;
            Ok(report)
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
            let mut report = build_balance_sheet(as_of.clone(), activities)?;
            let variance = checked_sub(
                report.assets_total,
                report.total_liabilities_equity,
                "مغایرت ترازنامه",
            )?;
            report.integrity = run_integrity_checks(conn, "", &as_of, true, scope, Some(variance))?;
            Ok(report)
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
    let mut unclassified_debit = Vec::new();
    let mut unclassified_credit = Vec::new();

    for activity in activities {
        let amount = activity.contribution_amount;
        if amount == 0 {
            continue;
        }
        // An unclassified balance has no reliable statement side. It is shown
        // below net profit instead of being guessed into a revenue or expense
        // line, so the period result stays aligned with the balance sheet.
        if activity.contribution_line == UNCLASSIFIED {
            if amount > 0 {
                unclassified_debit.push(row(&activity, amount));
            } else {
                unclassified_credit
                    .push(row(&activity, checked_neg(amount, "مانده طبقه‌بندی‌نشده")?));
            }
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
    let unclassified_debit_total = checked_sum(
        unclassified_debit.iter().map(|r| r.amount),
        "جمع مانده بدهکار طبقه‌بندی‌نشده",
    )?;
    let unclassified_credit_total = checked_sum(
        unclassified_credit.iter().map(|r| r.amount),
        "جمع مانده بستانکار طبقه‌بندی‌نشده",
    )?;
    let unclassified_total = checked_sub(
        unclassified_credit_total,
        unclassified_debit_total,
        "خالص مانده طبقه‌بندی‌نشده",
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
        unclassified_debit,
        unclassified_debit_total,
        unclassified_credit,
        unclassified_credit_total,
        unclassified_total,
        integrity: IntegrityReport::default(),
    })
}

fn build_balance_sheet(
    as_of: String,
    activities: Vec<crate::vouchers::accounting::AccountActivity>,
) -> Result<BalanceSheetReport, String> {
    let mut assets = Vec::new();
    let mut liabilities = Vec::new();
    let mut equity = Vec::new();
    let mut unclassified_debit = Vec::new();
    let mut unclassified_credit = Vec::new();
    let mut profit = 0i64;

    for activity in activities {
        let amount = activity.contribution_amount;
        if amount == 0 {
            continue;
        }
        let line = activity.contribution_line.as_str();
        // Keep unclassified balances in their own review buckets. Their sign
        // only says which side of the ledger they came from; it does not prove
        // they are an asset or a liability.
        if line == UNCLASSIFIED {
            if amount > 0 {
                unclassified_debit.push(row(&activity, amount));
            } else {
                unclassified_credit
                    .push(row(&activity, checked_neg(amount, "مانده طبقه‌بندی‌نشده")?));
            }
            continue;
        }
        let section = section_of(line);
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
            FsSection::Memorandum | FsSection::Unclassified => {}
        }
    }

    let classified_assets_total =
        checked_sum(assets.iter().map(|r| r.amount), "جمع دارایی ترازنامه")?;
    let unclassified_debit_total = checked_sum(
        unclassified_debit.iter().map(|r| r.amount),
        "جمع مانده بدهکار طبقه‌بندی‌نشده",
    )?;
    let assets_total = checked_add(
        classified_assets_total,
        unclassified_debit_total,
        "جمع دارایی ترازنامه با طبقه‌بندی‌نشده",
    )?;
    let classified_liabilities_total =
        checked_sum(liabilities.iter().map(|r| r.amount), "جمع بدهی ترازنامه")?;
    let unclassified_credit_total = checked_sum(
        unclassified_credit.iter().map(|r| r.amount),
        "جمع مانده بستانکار طبقه‌بندی‌نشده",
    )?;
    let liabilities_total = checked_add(
        classified_liabilities_total,
        unclassified_credit_total,
        "جمع بدهی ترازنامه با طبقه‌بندی‌نشده",
    )?;
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
        unclassified_debit,
        unclassified_debit_total,
        liabilities,
        liabilities_total,
        unclassified_credit,
        unclassified_credit_total,
        equity,
        equity_total,
        profit,
        total_liabilities_equity,
        balanced: assets_total == total_liabilities_equity,
        integrity: IntegrityReport::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::classification::EffectiveClassification;
    use crate::db::fs_catalog::{FsNormalSide, FsPermanence, FsStatement};
    use crate::vouchers::accounting::AccountActivity;

    fn activity(
        code: &str,
        name: &str,
        line: &str,
        amount: i64,
        statement: FsStatement,
    ) -> AccountActivity {
        AccountActivity {
            account_code: code.to_string(),
            account_name: name.to_string(),
            effective: EffectiveClassification {
                fs_line: line.to_string(),
                source: "manual".to_string(),
                section: section_of(line),
                statement,
                normal_side: FsNormalSide::Debit,
                permanence: FsPermanence::Permanent,
                contra_of: String::new(),
                opposite_fs_line: String::new(),
                split_by_sign: false,
                system_role: String::new(),
            },
            contribution_line: line.to_string(),
            contribution_amount: amount,
        }
    }

    #[test]
    fn balance_sheet_keeps_unclassified_balances_in_separate_buckets() {
        let report = build_balance_sheet(
            "1405/01/01".to_string(),
            vec![
                activity("1101", "نقد", "BS.CA.CASH", 50, FsStatement::BalanceSheet),
                activity(
                    "9001",
                    "بدهکار ناشناخته",
                    UNCLASSIFIED,
                    100,
                    FsStatement::BalanceSheet,
                ),
                activity(
                    "9002",
                    "بستانکار ناشناخته",
                    UNCLASSIFIED,
                    -30,
                    FsStatement::BalanceSheet,
                ),
            ],
        )
        .unwrap();

        assert_eq!(report.assets.len(), 1);
        assert_eq!(report.assets[0].account_code, "1101");
        assert_eq!(report.unclassified_debit.len(), 1);
        assert_eq!(report.unclassified_debit[0].amount, 100);
        assert_eq!(report.unclassified_credit.len(), 1);
        assert_eq!(report.unclassified_credit[0].amount, 30);
        // Unclassified balances stay in the totals, but not in the classified
        // lists; the audit can see exactly where they went.
        assert_eq!(report.assets_total, 150);
        assert_eq!(report.liabilities_total, 30);
        assert!(!report.balanced);
        // The test only checks the classification split; integrity is attached
        // by the database method.
        assert!(report.integrity.ok);
    }

    #[test]
    fn income_statement_reports_unclassified_below_net_profit() {
        let report = build_income_statement(
            "1405/01/01".to_string(),
            "1405/12/29".to_string(),
            vec![
                activity("4101", "فروش", "IS.REV", 100, FsStatement::IncomeStatement),
                activity(
                    "9001",
                    "بدهکار ناشناخته",
                    UNCLASSIFIED,
                    25,
                    FsStatement::BalanceSheet,
                ),
                activity(
                    "9002",
                    "بستانکار ناشناخته",
                    UNCLASSIFIED,
                    -40,
                    FsStatement::BalanceSheet,
                ),
            ],
        )
        .unwrap();

        assert_eq!(report.revenue_total, 100);
        assert_eq!(report.net_profit, 100);
        assert_eq!(report.unclassified_debit_total, 25);
        assert_eq!(report.unclassified_credit_total, 40);
        assert_eq!(report.unclassified_total, 15);
        assert_eq!(report.unclassified_debit.len(), 1);
        assert_eq!(report.unclassified_credit.len(), 1);
    }
}
