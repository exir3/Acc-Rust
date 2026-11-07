//! Balance-sheet print document.

use crate::report::layout::*;
use crate::vouchers::BalanceSheetReport;

/* ── Balance sheet ──────────────────────────────────────────── */

fn balance_section(
    title: &str,
    rows: &[crate::vouchers::AmountRow],
    unclassified: &[crate::vouchers::AmountRow],
    unclassified_title: &str,
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
    if !unclassified.is_empty() {
        body.push_str(&format!(
            r#"<tr class="rpt-unclassified-head"><td colspan="3">{title}</td></tr>"#,
            title = html(unclassified_title),
        ));
        for row in unclassified {
            body.push_str(&format!(
                r#"<tr class="rpt-unclassified"><td class="code">{code}</td><td class="title">{name}</td><td class="num">{amount}</td></tr>"#,
                code = code(&row.account_code),
                name = html(&row.account_name),
                amount = money(row.amount),
            ));
        }
    }
    if rows.is_empty() && unclassified.is_empty() {
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
        fs_line: "BS.EQ.RE".to_string(),
    });

    let unclassified_note = if report.unclassified_debit_total != 0
        || report.unclassified_credit_total != 0
    {
        r#"<div class="rpt-note">مانده‌های طبقه‌بندی‌نشده در ردیف جداگانه نمایش داده و در جمع دارایی‌ها/بدهی‌ها لحاظ شده‌اند؛ تا تعیین سرفصل، ترازنامه را تأییدشده تلقی نکنید.</div>"#
    } else {
        ""
    };

    let body = format!(
        r#"{head}
{assets}
{liabilities}
{equity}
<div class="rpt-total net"><span>جمع دارایی‌ها</span><span>{assets_total}</span></div>
<div class="rpt-total net"><span>جمع بدهی‌ها و حقوق مالکانه</span><span>{total}</span></div>
<div class="rpt-note">{balance_note}</div>
{unclassified_note}
{integrity}"#,
        head = head(
            "ترازنامه",
            meta,
            &format!("در تاریخ {}", date(&report.as_of))
        ),
        assets = balance_section(
            "دارایی‌ها",
            &report.assets,
            &report.unclassified_debit,
            "طبقه‌بندی‌نشده (مانده بدهکار)",
            report.assets_total,
            "جمع دارایی‌ها"
        ),
        liabilities = balance_section(
            "بدهی‌ها",
            &report.liabilities,
            &report.unclassified_credit,
            "طبقه‌بندی‌نشده (مانده بستانکار)",
            report.liabilities_total,
            "جمع بدهی‌ها"
        ),
        equity = balance_section(
            "حقوق مالکانه",
            &equity_rows,
            &[],
            "",
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
        unclassified_note = unclassified_note,
        integrity = integrity_panel(&report.integrity),
    );

    Ok(page("ترازنامه", &body, meta, false, "ترازنامه.html"))
}
