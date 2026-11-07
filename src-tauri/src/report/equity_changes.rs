//! Statement of changes in equity print document.

use crate::report::layout::*;
use crate::vouchers::EquityChangesReport;

/* ── Statement of changes in equity ─────────────────────────── */

pub fn equity_changes_print_doc(report: &EquityChangesReport, meta: &ReportMeta) -> PrintDoc {
    let mut rows = String::new();
    for row in &report.rows {
        if row.kind == "total" {
            continue;
        }
        let class = if row.kind == "profit" {
            r#" class="rpt-profit-row""#
        } else {
            ""
        };
        rows.push_str(&format!(
            r#"<tr{class}><td class="title">{title}</td><td class="num">{opening}</td><td class="num">{change}</td><td class="num">{closing}</td></tr>"#,
            class = class,
            title = html(&row.title),
            opening = money(row.opening),
            change = money(row.change),
            closing = money(row.closing),
        ));
    }

    let body = format!(
        r#"{head}
<div class="rpt-section">
<table class="rpt-table">
<thead><tr><th>شرح</th><th style="width:130px">مانده ابتدایی</th><th style="width:130px">تغییر دوره</th><th style="width:130px">مانده پایانی</th></tr></thead>
<tbody>{rows}</tbody>
</table>
</div>
<div class="rpt-total"><span>تغییر کل حقوق مالکانه</span><span>{change}</span></div>
<div class="rpt-total net"><span>جمع حقوق مالکانه در پایان دوره</span><span>{closing}</span></div>
<div class="rpt-note">مانده ابتدایی از آخرین سند افتتاحیه تا تاریخ گزارش گرفته می‌شود و سود خالص دوره از تاریخ همان سند محاسبه شده است؛ اسناد افتتاحیهٔ قبلی دوباره جمع نمی‌شوند.</div>
{integrity}"#,
        head = head(
            "صورت تغییرات حقوق مالکانه",
            meta,
            &format!("تا تاریخ {}", date(&report.to_date))
        ),
        rows = rows,
        change = money(report.totals.change),
        closing = money(report.totals.closing),
        integrity = integrity_panel(&report.integrity),
    );

    page(
        "صورت تغییرات حقوق مالکانه",
        &body,
        meta,
        false,
        "صورت-تغییرات-حقوق-مالکانه.html",
    )
}
