//! Account activity and classification shared by validation and reports.

use crate::db::{self};
use crate::vouchers::model::{ReportScope, CLOSING_FINAL, VOUCHER_TYPES};
use rusqlite::{params, Connection};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub(crate) struct AccountActivity {
    pub(crate) account_code: String,
    pub(crate) account_name: String,
    pub(crate) group_code: String,
    pub(crate) group_type: String,
    pub(crate) default_section: String,
    pub(crate) debit: i64,
    pub(crate) credit: i64,
}

/// Per-account totals for a date range. Vouchers whose type is in
/// `skip_types` are left out of the totals.
///
/// A final closing voucher is also hidden while no opening voucher follows
/// it up to the report date. That keeps a statement prepared at the closing
/// date on the books as they stood before the close. Once the next year's
/// opening voucher exists, both vouchers are included and cancel each other,
/// so later periods are not counted twice.
pub(crate) fn account_activities(
    conn: &Connection,
    from: &str,
    to: &str,
    cumulative: bool,
    skip_types: &[&str],
    scope: ReportScope,
) -> Result<Vec<AccountActivity>, String> {
    let skip_a = skip_types.first().copied().unwrap_or("");
    let skip_b = skip_types.get(1).copied().unwrap_or(skip_a);
    let (from_value, to_value) = if cumulative {
        ("".to_string(), to.to_string())
    } else {
        (from.to_string(), to.to_string())
    };
    let scope_sql = scope.sql();

    // Step 1: totals per effective account, grouped in SQLite.
    //
    // The old query LEFT JOINed `accounts` to a nested join of every voucher
    // line. SQLite materialised that nested join and re-scanned it for each
    // account (cost ~ accounts x lines), and every line crossed into Rust.
    // Now one aggregated row per active account comes back and the chart of
    // accounts is combined with it through a HashMap. The filters are the
    // same as before, only moved from the join condition to WHERE (the join
    // was an inner join between lines and vouchers, so the result is equal).
    let mut totals: HashMap<String, (i64, i64)> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT COALESCE(NULLIF(l.detail1_code, ''), l.account_code) AS code,
                        COALESCE(SUM(l.debit), 0),
                        COALESCE(SUM(l.credit), 0)
                   FROM voucher_lines l
                   JOIN vouchers v ON v.id = l.voucher_id
                  WHERE (?1 = '' OR v.date >= ?1)
                    AND (?2 = '' OR v.date <= ?2)
                    AND {scope_sql}
                    AND v.type <> ?3
                    AND v.type <> ?4
                    AND NOT (
                          v.type = ?5
                          AND NOT EXISTS (
                              SELECT 1
                                FROM vouchers o
                               WHERE o.type = ?6
                                 AND {scope_sql}
                                 AND (?2 = '' OR o.date <= ?2)
                                 AND (o.date > v.date
                                      OR (o.date = v.date AND o.id > v.id))
                          )
                    )
                  GROUP BY 1"
            ))
            .map_err(db::db_error)?;
        let rows = stmt
            .query_map(
                params![
                    from_value,
                    to_value,
                    skip_a,
                    skip_b,
                    CLOSING_FINAL,
                    VOUCHER_TYPES[1]
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .map_err(db::db_error)?;
        for row in rows {
            let (code, debit, credit) = row.map_err(db::db_error)?;
            totals.insert(code, (debit, credit));
        }
    }

    // Step 2: every account of the chart (accounts without movement stay at
    // zero, exactly like the old LEFT JOIN), in code order.
    let mut stmt = conn
        .prepare(
            "SELECT a.code, a.name, g.code, g.type, g.default_section
               FROM accounts a
               JOIN account_groups g ON g.code = a.group_code
              ORDER BY a.code",
        )
        .map_err(db::db_error)?;
    let raw = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(db::db_error)?;

    let mut activities: Vec<AccountActivity> = Vec::with_capacity(totals.len());
    for row in raw {
        let (code, name, group_code, group_type, default_section) = row.map_err(db::db_error)?;
        let (debit, credit) = totals.get(&code).copied().unwrap_or((0, 0));
        activities.push(AccountActivity {
            account_code: code,
            account_name: name,
            group_code,
            group_type,
            default_section,
            debit,
            credit,
        });
    }
    Ok(activities)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccountClass {
    Asset,
    Liability,
    Equity,
    Income,
    Expense,
    Memorandum,
}

/// Financial-statement placement chosen for an account group. Keys match
/// `ACCOUNT_GROUP_SECTIONS`; an empty or unknown value keeps the legacy
/// group-type/code-prefix classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatementSection {
    CurrentAsset,
    NonCurrentAsset,
    Equity,
    CurrentLiability,
    NonCurrentLiability,
    Revenue,
    Cogs,
    SellingExpense,
    AdminExpense,
    OtherOperating,
    FinanceCost,
    OtherNonOperating,
    IncomeTax,
    Memorandum,
}

pub(crate) fn section_of(key: &str) -> Option<StatementSection> {
    REPORT_SECTIONS
        .iter()
        .find(|entry| entry.0 == key)
        .map(|entry| entry.1)
}

/// Statement class used by the UI classification matrix. Keeping the section
/// key, its Rust enum and its reported class in one table prevents the UI and
/// backend mappings from drifting apart; `report_section_mapping_matches_ui`
/// also checks the table against `ui/js/pages/settings.js`.
pub(crate) fn section_class(key: &str) -> Option<&'static str> {
    REPORT_SECTIONS
        .iter()
        .find(|entry| entry.0 == key)
        .map(|entry| entry.2)
}

pub(crate) const REPORT_SECTIONS: [(&str, StatementSection, &str); 14] = [
    ("current_asset", StatementSection::CurrentAsset, "asset"),
    (
        "non_current_asset",
        StatementSection::NonCurrentAsset,
        "asset",
    ),
    ("equity", StatementSection::Equity, "equity"),
    (
        "current_liability",
        StatementSection::CurrentLiability,
        "liability",
    ),
    (
        "non_current_liability",
        StatementSection::NonCurrentLiability,
        "liability",
    ),
    ("revenue", StatementSection::Revenue, "income"),
    ("cogs", StatementSection::Cogs, "expense"),
    (
        "selling_expense",
        StatementSection::SellingExpense,
        "expense",
    ),
    ("admin_expense", StatementSection::AdminExpense, "expense"),
    ("other_operating", StatementSection::OtherOperating, "dual"),
    ("finance_cost", StatementSection::FinanceCost, "expense"),
    (
        "other_non_operating",
        StatementSection::OtherNonOperating,
        "dual",
    ),
    ("income_tax", StatementSection::IncomeTax, "expense"),
    ("memorandum", StatementSection::Memorandum, "memorandum"),
];

pub(crate) const GROUP_TYPE_CLASSES: [(&str, &str); 6] = [
    ("دارایی", "asset"),
    ("بدهی", "liability"),
    ("سرمایه", "equity"),
    ("درآمد", "income"),
    ("هزینه", "expense"),
    ("سایر", "other"),
];

/// Group type is the primary classification. A group with a financial
/// statement section uses that section first, so assigning a section in the
/// chart of accounts actually changes report placement. «سایر» without a
/// section falls back to the conventional first digit of the account code
/// (1/2/3 balance sheet, 4 income, 5-9 expenses).
pub(crate) fn classify(group_type: &str, account_code: &str) -> AccountClass {
    match group_type_class(group_type) {
        Some("asset") => AccountClass::Asset,
        Some("liability") => AccountClass::Liability,
        Some("equity") => AccountClass::Equity,
        Some("income") => AccountClass::Income,
        Some("expense") => AccountClass::Expense,
        _ => match account_code.chars().next() {
            Some('1') => AccountClass::Asset,
            Some('2') => AccountClass::Liability,
            Some('3') => AccountClass::Equity,
            Some('4') => AccountClass::Income,
            _ => AccountClass::Expense,
        },
    }
}

pub(crate) fn group_type_class(group_type: &str) -> Option<&'static str> {
    GROUP_TYPE_CLASSES
        .iter()
        .find(|entry| entry.0 == group_type)
        .map(|entry| entry.1)
}

pub(crate) fn classify_activity(activity: &AccountActivity, net: i64) -> AccountClass {
    if let Some(class) = section_class(&activity.default_section) {
        return match class {
            "asset" => AccountClass::Asset,
            "liability" => AccountClass::Liability,
            "equity" => AccountClass::Equity,
            "income" => AccountClass::Income,
            "expense" => AccountClass::Expense,
            "memorandum" => AccountClass::Memorandum,
            "dual" => {
                if net < 0 {
                    AccountClass::Income
                } else {
                    AccountClass::Expense
                }
            }
            _ => return classify(&activity.group_type, &activity.account_code),
        };
    }
    if matches!(activity.group_code.as_str(), "10" | "11") {
        if net < 0 {
            AccountClass::Income
        } else {
            AccountClass::Expense
        }
    } else {
        classify(&activity.group_type, &activity.account_code)
    }
}
