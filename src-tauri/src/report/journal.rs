//! Journal (day book) print document.

use crate::report::layout::*;
use crate::vouchers::JournalReport;

/* ── Journal ────────────────────────────────────────────────── */

pub fn journal_print_doc(report: &JournalReport, meta: &ReportMeta) -> PrintDoc {
    let mut rows = String::new();
    for row in &report.rows {
        let account = format!("{} — {}", code(&row.account_code), html(&row.account_name));
        rows.push_str(&format!(
            r#"<tr>
<td class="code">{date}</td>
<td class="code">{number}</td>
<td class="title">{account}</td>
<td class="title">{description}</td>
<td class="num">{debit}</td>
<td class="num">{credit}</td>
</tr>"#,
            date = date(&row.date),
            number = to_fa_digits(row.number),
            account = account,
            description = html(if row.line_description.trim().is_empty() {
                &row.voucher_description
            } else {
                &row.line_description
            }),
            debit = money(row.debit),
            credit = money(row.credit),
        ));
    }
    if rows.is_empty() {
        rows.push_str(
            "<tr><td colspan=\"6\" class=\"rpt-empty\">سندی در این بازه ثبت نشده است.</td></tr>",
        );
    }

    let body = format!(
        r#"{head}
<table class="rpt-table">
<thead><tr>
<th style="width:78px">تاریخ</th><th style="width:58px">شماره سند</th>
<th>حساب</th><th>شرح</th><th style="width:105px">بدهکار</th><th style="width:105px">بستانکار</th>
</tr></thead>
<tbody>{rows}</tbody>
<tfoot><tr>
<td colspan="4" class="title">جمع کل ({count} سند)</td>
<td class="num">{debit}</td><td class="num">{credit}</td>
</tr></tfoot>
</table>"#,
        head = head("دفتر روزنامه", meta, &period(&report.from, &report.to)),
        rows = rows,
        count = to_fa_digits(report.voucher_count),
        debit = money(report.total_debit),
        credit = money(report.total_credit),
    );

    page("دفتر روزنامه", &body, meta, true, "دفتر-روزنامه.html")
}
