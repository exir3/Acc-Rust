//! Cost-of-goods-sold statement print document.

use crate::report::layout::*;
use crate::vouchers::CogsReport;

/* ── Statement of cost of goods sold ────────────────────────── */

pub fn cogs_print_doc(report: &CogsReport, meta: &ReportMeta) -> PrintDoc {
    let mut rows = String::new();
    for row in &report.rows {
        if row.kind == "total" {
            continue;
        }
        let class = if row.kind == "inventory" {
            r#" class="rpt-supplementary""#
        } else {
            ""
        };
        let code = row
            .code
            .as_deref()
            .map(code)
            .unwrap_or_else(|| "—".to_string());
        let note = row
            .note
            .as_deref()
            .map(|note| format!(r#"<div class="rpt-note">{}</div>"#, html(note)))
            .unwrap_or_default();
        rows.push_str(&format!(
            r#"<tr{class}><td class="code">{code}</td><td class="title">{label}{note}</td><td class="num">{amount}</td></tr>"#,
            class = class,
            code = code,
            label = html(&row.label),
            note = note,
            amount = money(row.amount),
        ));
    }

    let body = format!(
        r#"{head}
<div class="rpt-section">
<table class="rpt-table">
<thead><tr><th style="width:96px">کد</th><th>شرح</th><th style="width:140px">مبلغ</th></tr></thead>
<tbody>{rows}</tbody>
<tfoot><tr><td colspan="2" class="title">بهای تمام‌شدهٔ کالای فروش‌رفته</td><td class="num">{total}</td></tr></tfoot>
</table>
</div>
<div class="rpt-total net"><span>بهای تمام‌شدهٔ کالای فروش‌رفته</span><span>{total}</span></div>
<div class="rpt-note">{limitation}</div>
{integrity}"#,
        head = head(
            "صورت بهای تمام‌شده کالای فروش‌رفته",
            meta,
            &period(&report.from_date, &report.to_date)
        ),
        rows = rows,
        total = money(report.totals.cogs),
        limitation = html(&report.limitation),
        integrity = integrity_panel(&report.integrity),
    );

    page(
        "صورت بهای تمام‌شده کالای فروش‌رفته",
        &body,
        meta,
        false,
        "صورت-بهای-تمام-شده-کالای-فروش-رفته.html",
    )
}
