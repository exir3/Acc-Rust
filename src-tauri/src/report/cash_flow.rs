//! Statement of cash flows (indirect method) print document.

use crate::report::layout::*;
use crate::vouchers::{CashFlowReport, CashFlowSection};

/* ── Statement of cash flows ────────────────────────────────── */

fn cash_section(section: &CashFlowSection) -> String {
    let mut rows = String::new();
    for line in &section.lines {
        let note = line
            .note
            .as_deref()
            .map(|note| format!(r#"<div class="rpt-note">{}</div>"#, html(note)))
            .unwrap_or_default();
        rows.push_str(&format!(
            r#"<tr><td class="title">{label}{note}</td><td class="num">{amount}</td></tr>"#,
            label = html(&line.label),
            note = note,
            amount = money(line.amount),
        ));
    }
    if section.lines.is_empty() {
        rows.push_str("<tr><td colspan=\"2\" class=\"rpt-empty\">جریان نقدی در این بخش ثبت نشده است.</td></tr>");
    }
    format!(
        r#"<div class="rpt-section">
<h2>{title}</h2>
<table class="rpt-table">
<thead><tr><th>شرح</th><th style="width:150px">مبلغ</th></tr></thead>
<tbody>{rows}</tbody>
<tfoot><tr><td class="title">جمع {title}</td><td class="num">{total}</td></tr></tfoot>
</table>
</div>"#,
        title = html(&section.title),
        rows = rows,
        total = money(section.total),
    )
}

fn unresolved_section(report: &CashFlowReport) -> String {
    if report.unresolved_vouchers.is_empty() {
        return String::new();
    }
    let rows = report
        .unresolved_vouchers
        .iter()
        .map(|voucher| {
            format!(
                r#"<tr><td class="code">{number}</td><td class="num">{amount}</td><td class="title">{reason}</td></tr>"#,
                number = to_fa_digits(voucher.voucher_number),
                amount = money(voucher.cash_movement),
                reason = html(&voucher.reason),
            )
        })
        .collect::<String>();
    format!(
        r#"<div class="rpt-section">
<h2>اسناد دارای گردش نقد طبقه‌بندی‌نشده</h2>
<table class="rpt-table">
<thead><tr><th style="width:90px">شماره سند</th><th style="width:130px">گردش نقد</th><th>علت</th></tr></thead>
<tbody>{rows}</tbody>
</table>
<div class="rpt-note">مبلغ این اسناد به هیچ فعالیتی نسبت داده نشده است؛ تا تعیین سرفصل طرف حساب، صورت جریان نقد کامل نیست.</div>
</div>"#,
        rows = rows,
    )
}

pub fn cash_flow_print_doc(report: &CashFlowReport, meta: &ReportMeta) -> PrintDoc {
    let sections = report.sections.iter().map(cash_section).collect::<String>();
    let status = if report.classification_complete {
        "همهٔ گردش‌های نقد طبقه‌بندی شده و جمع بخش‌ها با تغییر موجودی نقد تطبیق دارد."
    } else {
        "صورت جریان نقد کامل نیست؛ اسناد طبقه‌بندی‌نشده یا مغایرت تطبیق در کنترل سلامت فهرست شده است."
    };
    let variance_note = if report.variance == 0 {
        String::new()
    } else {
        format!(
            r#"<div class="rpt-total"><span>مغایرت تطبیق (تغییر واقعی − جمع بخش‌ها)</span><span>{}</span></div>"#,
            money(report.variance)
        )
    };
    let body = format!(
        r#"{head}
{sections}
<div class="rpt-total"><span>تغییر خالص موجودی نقد</span><span>{net_change}</span></div>
{variance_note}
<div class="rpt-section">
<table class="rpt-table">
<thead><tr><th>تطبیق موجودی نقد</th><th style="width:150px">مبلغ</th></tr></thead>
<tbody>
<tr><td class="title">موجودی نقد ابتدای دوره</td><td class="num">{opening}</td></tr>
<tr><td class="title">تغییر خالص دوره</td><td class="num">{net_change}</td></tr>
<tr><td class="title">موجودی نقد پایان دوره</td><td class="num">{closing}</td></tr>
</tbody>
</table>
</div>
<div class="rpt-note">{status}</div>
{unresolved}
{integrity}"#,
        head = head(
            "صورت جریان‌های نقدی (روش غیرمستقیم)",
            meta,
            &format!("تا تاریخ {}", date(&report.to_date))
        ),
        sections = sections,
        net_change = money(report.net_change),
        variance_note = variance_note,
        opening = money(report.opening_cash),
        closing = money(report.closing_cash),
        status = html(status),
        unresolved = unresolved_section(report),
        integrity = integrity_panel(&report.integrity),
    );

    page(
        "صورت جریان‌های نقدی",
        &body,
        meta,
        false,
        "صورت-جریان-های-نقدی.html",
    )
}
