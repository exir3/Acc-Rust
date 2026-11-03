//! Self-contained printable HTML documents for the accounting reports.
//!
//! The in-app preview window extracts the `<style>` element from each
//! document, so the screen view and the printed page share one layout.

use crate::db::to_fa_digits;
use crate::vouchers::{
    format_amount, BalanceSheetReport, IncomeStatementReport, JournalReport, LedgerReport,
    TrialBalanceReport, Voucher,
};
use base64::Engine;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintDoc {
    pub html: String,
    pub title: String,
    pub subtitle: String,
    pub file_name: String,
    pub paper_width_mm: f64,
    pub paper_height_mm: f64,
    pub margin_top_mm: f64,
    pub margin_right_mm: f64,
    pub margin_bottom_mm: f64,
    pub margin_left_mm: f64,
}

#[derive(Debug, Clone)]
pub struct ReportMeta {
    pub company: String,
    pub fiscal_year: String,
    pub today: String,
    /// Set by the trial-balance print command when the user unchecks
    /// "نمایش ساعت و تاریخ چاپ".
    pub show_print_date_time: bool,
}

impl ReportMeta {
    fn company(&self) -> String {
        let value = self.company.trim();
        if value.is_empty() {
            "واحد اقتصادی".to_string()
        } else {
            html(value)
        }
    }

    fn year(&self) -> String {
        let value = self.fiscal_year.trim();
        if value.is_empty() {
            "—".to_string()
        } else {
            html(value)
        }
    }
}

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
"#;

fn portable_base_style() -> String {
    let regular = base64::engine::general_purpose::STANDARD
        .encode(include_bytes!("../../ui/fonts/Vazirmatn-FD-Regular.ttf"));
    let bold = base64::engine::general_purpose::STANDARD
        .encode(include_bytes!("../../ui/fonts/Vazirmatn-FD-Bold.ttf"));
    format!(
        r#"
@font-face {{ font-family: "Vazirmatn FD"; src: url(data:font/ttf;base64,{regular}) format("truetype"); font-weight: 400; }}
@font-face {{ font-family: "Vazirmatn FD"; src: url(data:font/ttf;base64,{bold}) format("truetype"); font-weight: 700; }}
{BASE_STYLE}
"#
    )
}

fn html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn money(value: i64) -> String {
    if value == 0 {
        "۰".to_string()
    } else {
        to_fa_digits(format_amount(value))
    }
}

/// Adds two report amounts without silently clamping an overflow to a made-up
/// number; a printed report must never show a value the data does not have.
fn checked_total(left: i64, right: i64, label: &str) -> Result<i64, String> {
    left.checked_add(right)
        .ok_or_else(|| format!("{label} از محدوده عددی مجاز بیشتر است."))
}

fn date(value: &str) -> String {
    if value.trim().is_empty() {
        "—".to_string()
    } else {
        html(&to_fa_digits(value))
    }
}

fn code(value: &str) -> String {
    html(&to_fa_digits(value))
}

fn period(report_from: &str, report_to: &str) -> String {
    match (report_from.trim().is_empty(), report_to.trim().is_empty()) {
        (true, true) => "از ابتدا تا انتها".to_string(),
        (true, false) => format!("تا {}", date(report_to)),
        (false, true) => format!("از {}", date(report_from)),
        (false, false) => format!("از {} تا {}", date(report_from), date(report_to)),
    }
}

fn page(title: &str, body: &str, meta: &ReportMeta, landscape: bool, file_name: &str) -> PrintDoc {
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
            "{} — سال مالی تنظیم‌شده {}",
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

fn head(title: &str, meta: &ReportMeta, period_text: &str) -> String {
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
    <span>سال مالی (تنظیمات): <b>{year}</b></span>
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

/* ── Trial balance ──────────────────────────────────────────── */

pub fn trial_balance_print_doc(
    report: &TrialBalanceReport,
    meta: &ReportMeta,
    balance_type: &str,
    multi_level: bool,
    show_account_code: bool,
    show_account_name: bool,
) -> Result<PrintDoc, String> {
    let is_four_column = balance_type == "چهار ستونی";
    let show_opening = balance_type == "شش ستونی" || balance_type == "هشت ستونی";
    let show_accumulated = balance_type == "هشت ستونی";
    let amount_columns = if show_accumulated {
        8
    } else if show_opening {
        6
    } else {
        4
    };
    let identity_columns = 1 + usize::from(show_account_code) + usize::from(show_account_name);

    let code_head = if show_account_code {
        r#"<th rowspan="2" style="width:78px">کد حساب</th>"#
    } else {
        ""
    };
    let name_head = if show_account_name {
        r#"<th rowspan="2">عنوان حساب</th>"#
    } else {
        ""
    };

    let turnover_debit_title = if is_four_column {
        "جمع بدهکار"
    } else {
        "گردش بدهکار"
    };
    let turnover_credit_title = if is_four_column {
        "جمع بستانکار"
    } else {
        "گردش بستانکار"
    };

    let opening_header = if show_opening {
        r#"<th colspan="2">مانده افتتاحیه</th>"#
    } else {
        ""
    };
    let accumulated_header = if show_accumulated {
        r#"<th colspan="2">جمع دوره</th>"#
    } else {
        ""
    };
    let opening_cells = if show_opening {
        r#"<th style="width:84px">بدهکار</th><th style="width:84px">بستانکار</th>"#
    } else {
        ""
    };
    let accumulated_cells = if show_accumulated {
        r#"<th style="width:84px">بدهکار</th><th style="width:84px">بستانکار</th>"#
    } else {
        ""
    };

    let mut rows = String::new();
    for row in &report.rows {
        let indent = if multi_level {
            ((row.level - 1).max(0) as usize) * 16
        } else {
            0
        };
        // Four-column mode folds the opening balance into the turnover
        // columns, exactly like the WebApp report.
        let (turnover_debit, turnover_credit) = if is_four_column {
            (
                checked_total(row.opening_debit, row.period_debit, "جمع گردش بدهکار")?,
                checked_total(row.opening_credit, row.period_credit, "جمع گردش بستانکار")?,
            )
        } else {
            (row.period_debit, row.period_credit)
        };
        let opening = if show_opening {
            format!(
                r#"<td class="num">{}</td><td class="num">{}</td>"#,
                money(row.opening_debit),
                money(row.opening_credit)
            )
        } else {
            String::new()
        };
        let accumulated = if show_accumulated {
            format!(
                r#"<td class="num">{}</td><td class="num">{}</td>"#,
                money(checked_total(
                    row.opening_debit,
                    row.period_debit,
                    "جمع مانده بدهکار"
                )?),
                money(checked_total(
                    row.opening_credit,
                    row.period_credit,
                    "جمع مانده بستانکار"
                )?)
            )
        } else {
            String::new()
        };
        let name_style = if indent > 0 {
            format!(r#" style="padding-right:{indent}px""#)
        } else {
            String::new()
        };
        let code_cell = if show_account_code {
            format!(r#"<td class="code">{}</td>"#, code(&row.account_code))
        } else {
            String::new()
        };
        let name_cell = if show_account_name {
            format!(
                r#"<td class="title"{name_style}>{}</td>"#,
                html(&row.account_name)
            )
        } else {
            String::new()
        };
        rows.push_str(&format!(
            r#"<tr>
{code_cell}{name_cell}<td class="title">{group}</td>
{opening}<td class="num">{pd}</td><td class="num">{pc}</td>{accumulated}
<td class="num">{cd}</td><td class="num">{cc}</td>
</tr>"#,
            code_cell = code_cell,
            name_cell = name_cell,
            group = html(&row.group_name),
            opening = opening,
            pd = money(turnover_debit),
            pc = money(turnover_credit),
            accumulated = accumulated,
            cd = money(row.closing_debit),
            cc = money(row.closing_credit),
        ));
    }
    if rows.is_empty() {
        rows.push_str(&format!(
            "<tr><td colspan=\"{}\" class=\"rpt-empty\">حسابی با گردش یا مانده در این بازه وجود ندارد.</td></tr>",
            identity_columns + amount_columns
        ));
    }

    let opening_total = if show_opening {
        format!(
            r#"<td class="num">{}</td><td class="num">{}</td>"#,
            money(report.opening_debit),
            money(report.opening_credit)
        )
    } else {
        String::new()
    };
    let (total_turnover_debit, total_turnover_credit) = if is_four_column {
        (
            checked_total(report.opening_debit, report.period_debit, "جمع گردش بدهکار")?,
            checked_total(
                report.opening_credit,
                report.period_credit,
                "جمع گردش بستانکار",
            )?,
        )
    } else {
        (report.period_debit, report.period_credit)
    };

    let accumulated_total = if show_accumulated {
        format!(
            r#"<td class="num">{}</td><td class="num">{}</td>"#,
            money(checked_total(
                report.opening_debit,
                report.period_debit,
                "جمع مانده بدهکار",
            )?),
            money(checked_total(
                report.opening_credit,
                report.period_credit,
                "جمع مانده بستانکار",
            )?)
        )
    } else {
        String::new()
    };

    let body = format!(
        r#"{head}
<table class="rpt-table">
<thead>
<tr>
{code_head}{name_head}
<th rowspan="2">گروه</th>
{opening_header}
<th colspan="2">{turnover_title}</th>
{accumulated_header}
<th colspan="2">مانده اختتامیه</th>
</tr>
<tr>{opening_cells}
<th style="width:84px">{turnover_debit_title}</th><th style="width:84px">{turnover_credit_title}</th>
{accumulated_cells}
<th style="width:84px">بدهکار</th><th style="width:84px">بستانکار</th></tr>
</thead>
<tbody>{rows}</tbody>
<tfoot><tr>
<td colspan="{identity_columns}" class="title">جمع کل</td>
{opening_total}
<td class="num">{pd}</td><td class="num">{pc}</td>
{accumulated_total}
<td class="num">{cd}</td><td class="num">{cc}</td>
</tr></tfoot>
</table>"#,
        head = head("تراز آزمایشی", meta, &period(&report.from, &report.to)),
        code_head = code_head,
        name_head = name_head,
        identity_columns = identity_columns,
        opening_header = opening_header,
        turnover_title = if is_four_column {
            "جمع دوره"
        } else {
            "گردش دوره"
        },
        accumulated_header = accumulated_header,
        opening_cells = opening_cells,
        turnover_debit_title = turnover_debit_title,
        turnover_credit_title = turnover_credit_title,
        accumulated_cells = accumulated_cells,
        rows = rows,
        opening_total = opening_total,
        pd = money(total_turnover_debit),
        pc = money(total_turnover_credit),
        accumulated_total = accumulated_total,
        cd = money(report.closing_debit),
        cc = money(report.closing_credit),
    );

    Ok(page("تراز آزمایشی", &body, meta, true, "تراز-آزمایشی.html"))
}

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
{non_operating_revenues}
{non_operating_expenses}
<div class="rpt-total"><span>سود (زیان) عملیاتی</span><span>{operating}</span></div>
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
        gross = money(report.gross_profit),
        operating = money(report.operating_profit),
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

/* ── Balance sheet ──────────────────────────────────────────── */

fn balance_section(
    title: &str,
    rows: &[crate::vouchers::AmountRow],
    total: i64,
    total_label: &str,
) -> String {
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
<tfoot><tr><td colspan="2" class="title">{total_label}</td><td class="num">{total}</td></tr></tfoot>
</table>
</div>"#,
        title = html(title),
        body = body,
        total_label = html(total_label),
        total = money(total),
    )
}

pub fn balance_sheet_print_doc(
    report: &BalanceSheetReport,
    meta: &ReportMeta,
) -> Result<PrintDoc, String> {
    let equity_total = report
        .equity_total
        .checked_add(report.profit)
        .ok_or_else(|| "جمع حقوق مالکانه از محدوده عددی مجاز بیشتر است.".to_string())?;
    let mut equity_rows = report.equity.clone();
    equity_rows.push(crate::vouchers::AmountRow {
        account_code: String::new(),
        account_name: "سود (زیان) انباشته دوره".to_string(),
        amount: report.profit,
    });

    let body = format!(
        r#"{head}
{assets}
{liabilities}
{equity}
<div class="rpt-total net"><span>جمع دارایی‌ها</span><span>{assets_total}</span></div>
<div class="rpt-total net"><span>جمع بدهی‌ها و حقوق مالکانه</span><span>{total}</span></div>
<div class="rpt-note">{balance_note}</div>"#,
        head = head(
            "ترازنامه",
            meta,
            &format!("در تاریخ {}", date(&report.as_of))
        ),
        assets = balance_section(
            "دارایی‌ها",
            &report.assets,
            report.assets_total,
            "جمع دارایی‌ها"
        ),
        liabilities = balance_section(
            "بدهی‌ها",
            &report.liabilities,
            report.liabilities_total,
            "جمع بدهی‌ها"
        ),
        equity = balance_section(
            "حقوق مالکانه",
            &equity_rows,
            equity_total,
            "جمع حقوق مالکانه"
        ),
        assets_total = money(report.assets_total),
        total = money(report.total_liabilities_equity),
        balance_note = if report.balanced {
            "ترازنامه متوازن است."
        } else {
            "توجه: جمع دارایی‌ها با جمع بدهی‌ها و حقوق مالکانه برابر نیست."
        },
    );

    Ok(page("ترازنامه", &body, meta, false, "ترازنامه.html"))
}

/* ── Voucher ────────────────────────────────────────────────── */

pub fn voucher_print_doc(voucher: &Voucher, meta: &ReportMeta) -> PrintDoc {
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
        number = to_fa_digits(voucher.number),
        date = date(&voucher.date),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balance_sheet_print_rejects_equity_total_overflow() {
        let report = BalanceSheetReport {
            as_of: "1405/01/01".to_string(),
            assets: Vec::new(),
            assets_total: 0,
            liabilities: Vec::new(),
            liabilities_total: 0,
            equity: Vec::new(),
            equity_total: i64::MAX,
            profit: 1,
            total_liabilities_equity: 0,
            balanced: false,
        };
        let meta = ReportMeta {
            company: String::new(),
            fiscal_year: String::new(),
            today: String::new(),
            show_print_date_time: true,
        };

        assert!(balance_sheet_print_doc(&report, &meta)
            .unwrap_err()
            .contains("محدوده عددی"));
    }

    #[test]
    fn print_documents_escape_dates_codes_and_descriptions() {
        let voucher = Voucher {
            id: 1,
            number: 1,
            date: "1405/01/01<script>alert(1)</script>".to_string(),
            updated_at: "1405-01-01 00:00:00.000".to_string(),
            voucher_type: "<b>نوع</b>".to_string(),
            description: "<img src=x onerror=alert(1)>".to_string(),
            lines: vec![crate::vouchers::VoucherLine {
                id: 1,
                row_no: 1,
                account_code: "<svg/onload=alert(1)>".to_string(),
                account_name: "<script>alert(2)</script>".to_string(),
                description: "\"quoted\" & <tag>".to_string(),
                debit: 100,
                credit: 0,
            }],
            total_debit: 100,
            total_credit: 0,
        };
        let meta = ReportMeta {
            company: "شرکت <script>".to_string(),
            fiscal_year: "1405".to_string(),
            today: "1405/01/01".to_string(),
            show_print_date_time: true,
        };

        let doc = voucher_print_doc(&voucher, &meta);
        assert!(!doc.html.contains("<script>"));
        assert!(!doc.html.contains("<svg/onload"));
        assert!(!doc.html.contains("<img src=x"));
        assert!(doc.html.contains("&lt;script&gt;"));
        assert!(doc.html.contains("۱۴۰۵/۰۱/۰۱&lt;script&gt;"));
    }

    #[test]
    fn trial_balance_print_hides_optional_identity_and_date_columns() {
        let report = TrialBalanceReport {
            from: "1405/01/01".to_string(),
            to: "1405/01/31".to_string(),
            rows: vec![crate::vouchers::TrialBalanceRow {
                account_code: "1101".to_string(),
                account_name: "صندوق".to_string(),
                group_name: "دارایی‌های جاری".to_string(),
                level: 1,
                opening_debit: 100,
                opening_credit: 0,
                period_debit: 25,
                period_credit: 5,
                closing_debit: 120,
                closing_credit: 0,
            }],
            opening_debit: 100,
            opening_credit: 0,
            period_debit: 25,
            period_credit: 5,
            closing_debit: 120,
            closing_credit: 0,
        };
        let meta = ReportMeta {
            company: "شرکت آزمایشی".to_string(),
            fiscal_year: "1405".to_string(),
            today: "1405/02/01".to_string(),
            show_print_date_time: false,
        };

        let minimal = trial_balance_print_doc(&report, &meta, "شش ستونی", false, false, false)
            .expect("minimal trial balance print doc");
        assert!(!minimal.html.contains("کد حساب"));
        assert!(!minimal.html.contains("عنوان حساب"));
        assert!(!minimal.html.contains("تاریخ تهیه"));
        assert!(minimal.html.contains("گروه"));
        assert!(minimal.html.contains("جمع کل"));

        let full = trial_balance_print_doc(&report, &meta, "شش ستونی", false, true, true)
            .expect("full trial balance print doc");
        assert!(full.html.contains("کد حساب"));
        assert!(full.html.contains("عنوان حساب"));
    }

    #[test]
    fn portable_print_style_embeds_regular_and_bold_fonts() {
        let style = portable_base_style();
        assert_eq!(style.matches("data:font/ttf;base64,").count(), 2);
        assert!(style.contains("font-weight: 400"));
        assert!(style.contains("font-weight: 700"));
    }
}
