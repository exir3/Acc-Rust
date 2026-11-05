//! Income-statement print document.

use crate::report::layout::*;
use crate::vouchers::IncomeStatementReport;

/* ── Income statement ───────────────────────────────────────── */

fn section(title: &str, rows: &[crate::vouchers::AmountRow], total: i64) -> String {
    let mut body = String::new();
    for row in rows {
        body.push_str(&format!(
            r#"<tr><td class="code">{code}</td><td class="title">{name}</td><td class="num">{amount}</td></tr>"#,
            code = code(&row.account_code),
            name = html(&row.account_name),
            amount = money(row.amount),
        ));
    }
    if rows.is_empty() {
        body.push_str("<tr><td colspan=\"3\" class=\"rpt-empty\">موردی ثبت نشده است.</td></tr>");
    }
    format!(
        r#"<div class="rpt-section">
<h2>{title}</h2>
<table class="rpt-table">
<thead><tr><th style="width:78px">کد</th><th>عنوان</th><th style="width:120px">مبلغ</th></tr></thead>
<tbody>{body}</tbody>
<tfoot><tr><td colspan="2" class="title">جمع {title}</td><td class="num">{total}</td></tr></tfoot>
</table>
</div>"#,
        title = html(title),
        body = body,
        total = money(total),
    )
}

pub fn income_statement_print_doc(report: &IncomeStatementReport, meta: &ReportMeta) -> PrintDoc {
    let body = format!(
        r#"{head}
{revenues}
{cogs}
<div class="rpt-total"><span>سود (زیان) ناخالص</span><span>{gross}</span></div>
{other_operating_revenues}
{other_operating_expenses}
{expenses}
<div class="rpt-total"><span>سود (زیان) عملیاتی</span><span>{operating}</span></div>
{non_operating_revenues}
{non_operating_expenses}
{finance_costs}
<div class="rpt-total"><span>سود (زیان) پیش از مالیات</span><span>{before_tax}</span></div>
{income_taxes}
<div class="rpt-total net"><span>سود (زیان) خالص دوره</span><span>{net}</span></div>"#,
        head = head("صورت سود و زیان", meta, &period(&report.from, &report.to)),
        revenues = section("درآمدها", &report.revenues, report.revenue_total),
        cogs = section("بهای تمام شده درآمدها", &report.cogs, report.cogs_total),
        other_operating_revenues = section(
            "سایر درآمدهای عملیاتی",
            &report.other_operating_revenues,
            report.other_operating_revenue_total
        ),
        other_operating_expenses = section(
            "سایر هزینه‌های عملیاتی",
            &report.other_operating_expenses,
            report.other_operating_expense_total
        ),
        expenses = section("هزینه‌های عملیاتی", &report.expenses, report.expense_total),
        non_operating_revenues = section(
            "درآمدهای غیرعملیاتی",
            &report.non_operating_revenues,
            report.non_operating_revenue_total
        ),
        non_operating_expenses = section(
            "هزینه‌های غیرعملیاتی",
            &report.non_operating_expenses,
            report.non_operating_expense_total
        ),
        finance_costs = section(
            "هزینه‌های مالی",
            &report.finance_costs,
            report.finance_cost_total
        ),
        income_taxes = section(
            "مالیات بر درآمد",
            &report.income_taxes,
            report.income_tax_total
        ),
        gross = money(report.gross_profit),
        operating = money(report.operating_profit),
        before_tax = money(report.profit_before_tax),
        net = money(report.net_profit),
    );

    page(
        "صورت سود و زیان",
        &body,
        meta,
        false,
        "صورت-سود-و-زیان.html",
    )
}
