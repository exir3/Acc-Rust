//! Trial-balance print document.

use crate::report::layout::*;
use crate::vouchers::TrialBalanceReport;

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
