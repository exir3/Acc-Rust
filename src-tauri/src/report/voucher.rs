//! Single-voucher print document.

use crate::report::layout::*;
use crate::vouchers::{IntegrityReport, Voucher};

/* ── Voucher ────────────────────────────────────────────────── */

pub fn voucher_print_doc(
    voucher: &Voucher,
    meta: &ReportMeta,
    integrity: &IntegrityReport,
) -> PrintDoc {
    let mut rows = String::new();
    for line in &voucher.lines {
        rows.push_str(&format!(
            r#"<tr>
<td class="code">{row}</td>
<td class="code">{code}</td>
<td class="title">{name}</td>
<td class="title">{description}</td>
<td class="num">{debit}</td>
<td class="num">{credit}</td>
</tr>"#,
            row = to_fa_digits(line.row_no),
            code = code(&line.account_code),
            name = html(&line.account_name),
            description = html(&line.description),
            debit = money(line.debit),
            credit = money(line.credit),
        ));
    }

    let body = format!(
        r#"{head}
{integrity_warning}
<div class="rpt-note">شماره سند: <b>{number}</b> &nbsp;|&nbsp; تاریخ: <b>{date}</b> &nbsp;|&nbsp; نوع: <b>{kind}</b></div>
<table class="rpt-table" style="margin-top:8px">
<thead><tr>
<th style="width:48px">ردیف</th><th style="width:95px">کد حساب</th><th>عنوان حساب</th>
<th>شرح</th><th style="width:110px">بدهکار</th><th style="width:110px">بستانکار</th>
</tr></thead>
<tbody>{rows}</tbody>
<tfoot><tr>
<td colspan="4" class="title">جمع سند</td>
<td class="num">{debit}</td><td class="num">{credit}</td>
</tr></tfoot>
</table>
<div class="rpt-note">شرح سند: {description}</div>"#,
        head = head("سند حسابداری", meta, ""),
        integrity_warning = integrity_warning(integrity),
        number = to_fa_digits(voucher.number),
        date = html(&date(&voucher.date)),
        kind = html(&voucher.voucher_type),
        rows = rows,
        debit = money(voucher.total_debit),
        credit = money(voucher.total_credit),
        description = html(&voucher.description),
    );

    page(
        "سند حسابداری",
        &body,
        meta,
        false,
        &format!("سند-{}.html", voucher.number),
    )
}
