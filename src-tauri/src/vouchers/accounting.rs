//! Account activity and classification shared by validation and reports.

use crate::db::{self};
use crate::vouchers::model::{CLOSING_FINAL, VOUCHER_TYPES};
use crate::vouchers::validation::checked_add;
use rusqlite::{params, Connection};

#[derive(Debug, Clone)]
pub(crate) struct AccountActivity {
    pub(crate) account_code: String,
    pub(crate) account_name: String,
    pub(crate) group_code: String,
    pub(crate) group_type: String,
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
) -> Result<Vec<AccountActivity>, String> {
    let skip_a = skip_types.first().copied().unwrap_or("");
    let skip_b = skip_types.get(1).copied().unwrap_or(skip_a);
    let (from_value, to_value) = if cumulative {
        ("".to_string(), to.to_string())
    } else {
        (from.to_string(), to.to_string())
    };

    let mut stmt = conn
        .prepare(
            "SELECT a.code, a.name, g.code, g.type, l.debit, l.credit
               FROM accounts a
               JOIN account_groups g ON g.code = a.group_code
               LEFT JOIN (
                    voucher_lines l
                    JOIN vouchers v ON v.id = l.voucher_id
                     AND (?1 = '' OR v.date >= ?1)
                     AND (?2 = '' OR v.date <= ?2)
                     AND v.type <> ?3
                     AND v.type <> ?4
                     AND NOT (
                           v.type = ?5
                           AND NOT EXISTS (
                               SELECT 1
                                 FROM vouchers o
                                WHERE o.type = ?6
                                  AND (?2 = '' OR o.date <= ?2)
                                  AND (o.date > v.date
                                       OR (o.date = v.date AND o.id > v.id))
                           )
                     )
               ) ON COALESCE(NULLIF(l.detail1_code, ''), l.account_code) = a.code
              ORDER BY a.code",
        )
        .map_err(db::db_error)?;

    let raw = stmt
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
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                ))
            },
        )
        .map_err(db::db_error)?;

    let mut activities: Vec<AccountActivity> = Vec::new();
    for row in raw {
        let (code, name, group_code, group_type, debit, credit) = row.map_err(db::db_error)?;
        if let Some(last) = activities
            .last_mut()
            .filter(|item| item.account_code.as_str() == code.as_str())
        {
            if let Some(debit) = debit {
                last.debit = checked_add(last.debit, debit, "گردش بدهکار حساب")?;
            }
            if let Some(credit) = credit {
                last.credit = checked_add(last.credit, credit, "گردش بستانکار حساب")?;
            }
        } else {
            activities.push(AccountActivity {
                account_code: code,
                account_name: name,
                group_code,
                group_type,
                debit: debit.unwrap_or(0),
                credit: credit.unwrap_or(0),
            });
        }
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
}

/// Group type is the primary classification. «سایر» falls back to the
/// conventional first digit of the account code (1/2/3 balance sheet,
/// 4 income, 5-9 expenses).
pub(crate) fn classify(group_type: &str, account_code: &str) -> AccountClass {
    match group_type {
        "دارایی" => AccountClass::Asset,
        "بدهی" => AccountClass::Liability,
        "سرمایه" => AccountClass::Equity,
        "درآمد" => AccountClass::Income,
        "هزینه" => AccountClass::Expense,
        _ => match account_code.chars().next() {
            Some('1') => AccountClass::Asset,
            Some('2') => AccountClass::Liability,
            Some('3') => AccountClass::Equity,
            Some('4') => AccountClass::Income,
            _ => AccountClass::Expense,
        },
    }
}

pub(crate) fn classify_activity(activity: &AccountActivity, net: i64) -> AccountClass {
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
