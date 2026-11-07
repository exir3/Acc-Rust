//! Shared stylesheet, escaping helpers and page scaffolding.

pub(crate) use crate::db::to_fa_digits;
pub(crate) use crate::report::model::{PrintDoc, ReportMeta};
use crate::vouchers::{format_amount, IntegrityReport};
use base64::Engine;

/* ── Shared pieces ──────────────────────────────────────────── */

const BASE_STYLE: &str = r#"
@page { size: A4 portrait; margin: 14mm 12mm; }
@media print {
  html, body { background: #fff !important; -webkit-print-color-adjust: exact; print-color-adjust: exact; }
  .report-doc { padding: 0 !important; }
  thead { display: table-header-group; }
  tr { page-break-inside: avoid; }
}
* { box-sizing: border-box; }
html, body { margin: 0; padding: 0; background: #fff; }
.report-doc {
  font-family: "Vazirmatn FD", "B Nazanin", Vazirmatn, Tahoma, Arial, sans-serif;
  font-size: 12.5px; color: #16243a; line-height: 1.7; padding: 6px 10px;
  direction: rtl;
}
.rpt-head {
  border-bottom: 2.5px solid #123a6b; padding-bottom: 8px; margin-bottom: 10px;
}
.rpt-company { font-size: 16px; font-weight: 700; color: #123a6b; text-align: center; }
.rpt-title { font-size: 17px; font-weight: 700; color: #123a6b; text-align: center; margin: 2px 0 0; }
.rpt-meta {
  display: flex; justify-content: space-between; gap: 12px; flex-wrap: wrap;
  color: #4d5d77; font-size: 11.5px; margin-top: 5px;
}
.rpt-meta b { color: #16243a; }
.rpt-table { width: 100%; border-collapse: collapse; margin-top: 4px; }
.rpt-table th {
  background: #123a6b; color: #fff; border: 1px solid #0b2a52;
  padding: 4px 5px; font-size: 11.5px; text-align: center; font-weight: 600;
}
.rpt-table td { border: 1px solid #c5d2e3; padding: 4px 5px; font-size: 11.5px; }
.rpt-table tbody tr:nth-child(even) td { background: #f7faff; }
.rpt-table .num { text-align: center; direction: ltr; font-variant-numeric: tabular-nums; white-space: nowrap; }
.rpt-table .code { text-align: center; direction: ltr; font-variant-numeric: tabular-nums; white-space: nowrap; }
.rpt-table .title { text-align: right; }
.rpt-table tfoot td { background: #e8f0fb; font-weight: 700; border-color: #9fb7d6; }
.rpt-section { margin-top: 12px; }
.rpt-section h2 {
  font-size: 13px; color: #123a6b; margin: 0 0 4px; padding: 4px 8px;
  background: #eef4fd; border-right: 4px solid #1f5fd0;
}
.rpt-total { display: flex; justify-content: space-between; gap: 12px; padding: 5px 8px; background: #f3f7fc; border: 1px solid #d7e2f0; font-weight: 700; margin-top: 4px; }
.rpt-total.net { background: #e8f1ff; border-color: #b9cff0; color: #123a6b; font-size: 13px; }
.rpt-empty { text-align: center; color: #7c8ba4; padding: 22px 8px; border: 1px dashed #c5d2e3; margin-top: 8px; }
.rpt-foot { margin-top: 14px; padding-top: 6px; border-top: 1px solid #dbe4f0; color: #7c8ba4; font-size: 10.5px; display: flex; justify-content: space-between; }
.rpt-note { color: #7c8ba4; font-size: 10.5px; margin-top: 6px; }
.rpt-table tr.rpt-unclassified td { background: #fff8e6; color: #7a4a00; }
.rpt-table tr.rpt-unclassified-head td { background: #fdf1d8; color: #7a4a00; font-weight: 700; }
.rpt-table tr.rpt-profit-row td { background: #eef4fd; font-weight: 700; }
.rpt-table tr.rpt-supplementary td { background: #f7faff; color: #4d5d77; font-style: italic; }
.rpt-table tr.rpt-total-row td { background: #e8f0fb; font-weight: 700; border-color: #9fb7d6; }
.rpt-table tr.rpt-retained-row td { background: #e8f1ff; color: #123a6b; font-weight: 700; }
.rpt-table tr.rpt-account-row td { background: #fbfcfe; color: #4d5d77; font-size: 10.5px; }
.rpt-health { margin-top: 12px; border: 1px solid #d7e2f0; border-right: 4px solid #1f5fd0; padding: 6px 8px; font-size: 11px; }
.rpt-health[data-ok="true"] { border-right-color: #0d7f7a; background: #f0fbf9; color: #0d5f5a; }
.rpt-health[data-ok="false"] { border-right-color: #b42318; background: #fff5f5; color: #5c1a15; }
.rpt-health h3 { margin: 0 0 4px; font-size: 11.5px; }
.rpt-issue { margin: 3px 0; }
.rpt-issue[data-severity="error"] { color: #b42318; }
.rpt-issue[data-severity="warning"] { color: #9a5b00; }
.rpt-issue[data-severity="info"] { color: #1a4fa0; }
.rpt-issue-accounts { color: #7c8ba4; font-size: 10px; }
"#;

pub(crate) fn portable_base_style() -> String {
    let regular = base64::engine::general_purpose::STANDARD
        .encode(include_bytes!("../../../ui/fonts/Vazirmatn-FD-Regular.ttf"));
    let bold = base64::engine::general_purpose::STANDARD
        .encode(include_bytes!("../../../ui/fonts/Vazirmatn-FD-Bold.ttf"));
    format!(
        r#"
@font-face {{ font-family: "Vazirmatn FD"; src: url(data:font/ttf;base64,{regular}) format("truetype"); font-weight: 400; }}
@font-face {{ font-family: "Vazirmatn FD"; src: url(data:font/ttf;base64,{bold}) format("truetype"); font-weight: 700; }}
{BASE_STYLE}
"#
    )
}

pub(crate) fn html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub(crate) fn money(value: i64) -> String {
    if value == 0 {
        "۰".to_string()
    } else {
        to_fa_digits(format_amount(value))
    }
}

/// Adds two report amounts without silently clamping an overflow to a made-up
/// number; a printed report must never show a value the data does not have.
pub(crate) fn checked_total(left: i64, right: i64, label: &str) -> Result<i64, String> {
    left.checked_add(right)
        .ok_or_else(|| format!("{label} از محدوده عددی مجاز بیشتر است."))
}

pub(crate) fn date(value: &str) -> String {
    if value.trim().is_empty() {
        "—".to_string()
    } else {
        html(&to_fa_digits(value))
    }
}

pub(crate) fn code(value: &str) -> String {
    html(&to_fa_digits(value))
}

pub(crate) fn period(report_from: &str, report_to: &str) -> String {
    match (report_from.trim().is_empty(), report_to.trim().is_empty()) {
        (true, true) => "از ابتدا تا انتها".to_string(),
        (true, false) => format!("تا {}", date(report_to)),
        (false, true) => format!("از {}", date(report_from)),
        (false, false) => format!("از {} تا {}", date(report_from), date(report_to)),
    }
}

/// Print-side summary of the accounting-health checks attached to a
/// statement. Every finding is listed with its severity so a clean report is
/// still explicit about the checks that ran.
pub(crate) fn integrity_panel(integrity: &IntegrityReport) -> String {
    let status = if integrity.ok {
        "بدون خطا"
    } else {
        "نیازمند بررسی"
    };
    let mut issues = String::new();
    for issue in &integrity.issues {
        let severity = match issue.severity.as_str() {
            "error" => "خطا",
            "warning" => "هشدار",
            _ => "اطلاع",
        };
        let accounts = if issue.accounts.is_empty() {
            String::new()
        } else {
            format!(
                r#"<div class="rpt-issue-accounts">نمونه حساب‌ها: {}</div>"#,
                html(&issue.accounts.join("، "))
            )
        };
        issues.push_str(&format!(
            r#"<div class="rpt-issue" data-severity="{severity_key}"><b>{severity}:</b> {message}{accounts}</div>"#,
            severity_key = html(&issue.severity),
            severity = severity,
            message = html(&issue.message),
            accounts = accounts,
        ));
    }
    format!(
        r#"<div class="rpt-health" data-ok="{ok}">
<h3>کنترل سلامت صورت‌های مالی — {status}</h3>
<div>{errors} خطا، {warnings} هشدار، {info} اطلاع</div>
{issues}
</div>"#,
        ok = integrity.ok,
        status = status,
        errors = to_fa_digits(integrity.counts.error.to_string()),
        warnings = to_fa_digits(integrity.counts.warning.to_string()),
        info = to_fa_digits(integrity.counts.info.to_string()),
        issues = issues,
    )
}

pub(crate) fn page(
    title: &str,
    body: &str,
    meta: &ReportMeta,
    landscape: bool,
    file_name: &str,
) -> PrintDoc {
    let base_style = portable_base_style();
    let (width, height, page_css) = if landscape {
        (
            297.0,
            210.0,
            "@page { size: A4 landscape; margin: 10mm 9mm; }",
        )
    } else {
        (
            210.0,
            297.0,
            "@page { size: A4 portrait; margin: 14mm 12mm; }",
        )
    };
    let today = if meta.show_print_date_time {
        format!(
            "<span>تاریخ تهیه: {today}</span>",
            today = date(&meta.today)
        )
    } else {
        String::new()
    };
    let html_doc = format!(
        r#"<!DOCTYPE html>
<html dir="rtl" lang="fa">
<head>
<meta charset="utf-8">
<title>{title} — {company}</title>
<style>{base_style}
{page_css}
</style>
</head>
<body>
<div class="report-doc">
{body}
<div class="rpt-foot"><span>نرم‌افزار حسابداری</span>{today}</div>
</div>
</body>
</html>"#,
        title = html(title),
        company = meta.company(),
        body = body,
        today = today,
        page_css = page_css,
    );

    PrintDoc {
        html: html_doc,
        title: title.to_string(),
        subtitle: format!(
            "{} — سال مالی {}",
            if meta.company.trim().is_empty() {
                "واحد اقتصادی"
            } else {
                meta.company.trim()
            },
            if meta.fiscal_year.trim().is_empty() {
                "—"
            } else {
                meta.fiscal_year.trim()
            }
        ),
        file_name: file_name.to_string(),
        paper_width_mm: width,
        paper_height_mm: height,
        margin_top_mm: if landscape { 10.0 } else { 14.0 },
        margin_right_mm: if landscape { 9.0 } else { 12.0 },
        margin_bottom_mm: if landscape { 10.0 } else { 14.0 },
        margin_left_mm: if landscape { 9.0 } else { 12.0 },
    }
}

pub(crate) fn head(title: &str, meta: &ReportMeta, period_text: &str) -> String {
    let period_cell = if period_text.is_empty() {
        String::new()
    } else {
        format!("<span>دوره: <b>{period_text}</b></span>")
    };
    let today_cell = if meta.show_print_date_time {
        format!(
            "<span>تاریخ تهیه: <b>{today}</b></span>",
            today = date(&meta.today)
        )
    } else {
        String::new()
    };
    format!(
        r#"<div class="rpt-head">
  <div class="rpt-company">{company}</div>
  <div class="rpt-title">{title}</div>
  <div class="rpt-meta">
    {period}
    <span>سال مالی: <b>{year}</b></span>
    {today_cell}
  </div>
</div>"#,
        company = meta.company(),
        title = html(title),
        period = period_cell,
        year = meta.year(),
        today_cell = today_cell,
    )
}
