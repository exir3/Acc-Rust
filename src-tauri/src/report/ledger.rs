//! Ledger print document.

use crate::report::layout::*;
use crate::vouchers::LedgerReport;

/* ── Ledger ─────────────────────────────────────────────────── */

pub fn ledger_print_doc(report: &LedgerReport, meta: &ReportMeta) -> PrintDoc {
    // A parent account's ledger mixes several sub-accounts, so each row
    // also names the account it belongs to.
    let wide = report.includes_children;
    let columns = if wide { 8 } else { 7 };

    let mut rows = String::new();
    for row in &report.rows {
        let account_cell = if wide {
            format!(
                "<td class=\"title\">{} — {}</td>\n",
                code(&row.account_code),
                html(&row.account_name)
            )
        } else {
            String::new()
        };
        rows.push_str(&format!(
            r#"<tr>
<td class="code">{date}</td>
<td class="code">{number}</td>
{account_cell}<td class="title">{description}</td>
<td class="num">{debit}</td>
<td class="num">{credit}</td>
<td class="num">{run_debit}</td>
<td class="num">{run_credit}</td>
</tr>"#,
            date = date(&row.date),
            number = to_fa_digits(row.number),
            account_cell = account_cell,
            description = html(if row.line_description.trim().is_empty() {
                &row.voucher_description
            } else {
                &row.line_description
            }),
            debit = money(row.debit),
            credit = money(row.credit),
            run_debit = money(row.running_debit),
            run_credit = money(row.running_credit),
        ));
    }
    if rows.is_empty() {
        rows.push_str(&format!(
            "<tr><td colspan=\"{columns}\" class=\"rpt-empty\">گردشی برای این حساب در این بازه وجود ندارد.</td></tr>"
        ));
    }

    let account_head = if wide {
        r#"<th style="width:150px">حساب</th>"#
    } else {
        ""
    };
    // Label columns: date, number, [account], description.
    let label_span = columns - 4;

    let body = format!(
        r#"{head}
<div class="rpt-note">حساب: <b>{code} — {name}</b> &nbsp;|&nbsp; گروه: <b>{group}</b>{children_note}</div>
<table class="rpt-table" style="margin-top:8px">
<thead><tr>
<th style="width:78px">تاریخ</th><th style="width:58px">شماره</th>{account_head}<th>شرح</th>
<th style="width:95px">بدهکار</th><th style="width:95px">بستانکار</th>
<th style="width:95px">مانده بدهکار</th><th style="width:95px">مانده بستانکار</th>
</tr></thead>
<tbody>
<tr>
<td class="title" colspan="{opening_span}"><b>مانده افتتاحیه</b></td>
<td class="num"><b>{opening_debit}</b></td><td class="num"><b>{opening_credit}</b></td>
</tr>
{rows}
</tbody>
<tfoot>
<tr>
<td class="title" colspan="{label_span}">گردش دوره و مانده اختتامیه</td>
<td class="num">{period_debit}</td><td class="num">{period_credit}</td>
<td class="num">{closing_debit}</td><td class="num">{closing_credit}</td>
</tr>
</tfoot>
</table>"#,
        head = head("دفتر کل حساب", meta, &period(&report.from, &report.to)),
        code = code(&report.account_code),
        name = html(&report.account_name),
        group = html(&report.group_name),
        children_note = if wide {
            " &nbsp;|&nbsp; <b>شامل گردش زیرحساب‌ها</b>"
        } else {
            ""
        },
        account_head = account_head,
        opening_span = columns - 2,
        label_span = label_span,
        opening_debit = money(report.opening_debit),
        opening_credit = money(report.opening_credit),
        rows = rows,
        period_debit = money(report.total_debit),
        period_credit = money(report.total_credit),
        closing_debit = money(report.closing_debit),
        closing_credit = money(report.closing_credit),
    );

    page(
        "دفتر کل حساب",
        &body,
        meta,
        wide,
        &format!("دفتر-کل-{}.html", report.account_code),
    )
}
