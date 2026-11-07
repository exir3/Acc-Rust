//! Profit-appropriation statement print document.

use crate::report::layout::*;
use crate::vouchers::AppropriationReport;

/* ── Statement of profit appropriation ──────────────────────── */

pub fn appropriation_print_doc(report: &AppropriationReport, meta: &ReportMeta) -> PrintDoc {
    let mut rows = String::new();
    for row in &report.rows {
        let class = match row.kind.as_str() {
            "total" => r#" class="rpt-total-row""#,
            "retained" => r#" class="rpt-retained-row""#,
            _ => "",
        };
        rows.push_str(&format!(
            r#"<tr{class}><td class="title">{label}</td><td class="num">{amount}</td></tr>"#,
            class = class,
            label = html(&row.label),
            amount = money(row.amount),
        ));
        for account in &row.accounts {
            rows.push_str(&format!(
                r#"<tr class="rpt-account-row"><td class="title">حساب {code} — {name}</td><td class="num">{amount}</td></tr>"#,
                code = code(&account.account_code),
                name = html(&account.account_name),
                amount = money(account.amount),
            ));
        }
    }

    let body = format!(
        r#"{head}
<div class="rpt-section">
<table class="rpt-table">
<thead><tr><th>شرح</th><th style="width:160px">مبلغ</th></tr></thead>
<tbody>{rows}</tbody>
</table>
</div>
<div class="rpt-total"><span>جمع تخصیص‌های شناسایی‌شده</span><span>{appropriated}</span></div>
<div class="rpt-total net"><span>ماندهٔ محاسباتی پس از تخصیص</span><span>{retained}</span></div>
<div class="rpt-note">فقط انتقال‌های مستقل و متوازن از سود انباشته به اندوخته یا سود سهام شناسایی می‌شوند؛ پرداخت سود، آورده و ثبت‌های مرکب در مبلغ تخصیص وارد نمی‌شوند و در کنترل سلامت فهرست می‌شوند.</div>
{integrity}"#,
        head = head(
            "صورت تخصیص سود",
            meta,
            &format!("تا تاریخ {}", date(&report.to_date))
        ),
        rows = rows,
        appropriated = money(report.totals.appropriated),
        retained = money(report.totals.retained),
        integrity = integrity_panel(&report.integrity),
    );

    page("صورت تخصیص سود", &body, meta, false, "صورت-تخصیص-سود.html")
}
