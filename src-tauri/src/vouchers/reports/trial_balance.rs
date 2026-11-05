//! Hierarchical trial balance with opening, period and closing amounts.

use crate::db::{self, Database};
use crate::vouchers::model::{TrialBalanceReport, TrialBalanceRow};
use crate::vouchers::validation::{
    check_range, checked_add, checked_neg, checked_sub, checked_sum, normalize_filter_date,
};
use rusqlite::params;

struct TrialAccountRaw {
    code: String,
    name: String,
    group_name: String,
    level: i64,
    parent_code: String,
    opening_debit: i64,
    opening_credit: i64,
    period_debit: i64,
    period_credit: i64,
}

/// Adds all child amounts to their parent, recursively, and memoizes results.
fn roll_up_trial_amounts(
    code: &str,
    accounts: &[TrialAccountRaw],
    index_by_code: &std::collections::HashMap<String, usize>,
    children: &std::collections::HashMap<String, Vec<String>>,
    included: Option<&std::collections::HashSet<String>>,
    memo: &mut std::collections::HashMap<String, (i64, i64, i64, i64)>,
) -> Result<(i64, i64, i64, i64), String> {
    if let Some(amounts) = memo.get(code) {
        return Ok(*amounts);
    }
    let Some(&index) = index_by_code.get(code) else {
        return Ok((0, 0, 0, 0));
    };
    let account = &accounts[index];
    let mut total = if included.map_or(true, |codes| codes.contains(code)) {
        (
            account.opening_debit,
            account.opening_credit,
            account.period_debit,
            account.period_credit,
        )
    } else {
        (0, 0, 0, 0)
    };
    if let Some(child_codes) = children.get(code) {
        for child in child_codes {
            let amounts =
                roll_up_trial_amounts(child, accounts, index_by_code, children, included, memo)?;
            total.0 = checked_add(total.0, amounts.0, "جمع تراز آزمایشی")?;
            total.1 = checked_add(total.1, amounts.1, "جمع تراز آزمایشی")?;
            total.2 = checked_add(total.2, amounts.2, "جمع تراز آزمایشی")?;
            total.3 = checked_add(total.3, amounts.3, "جمع تراز آزمایشی")?;
        }
    }
    memo.insert(code.to_string(), total);
    Ok(total)
}

fn trial_code_path(
    code: &str,
    accounts: &[TrialAccountRaw],
    index_by_code: &std::collections::HashMap<String, usize>,
) -> Option<Vec<String>> {
    let mut path = Vec::new();
    let mut current = code;
    let mut guard = std::collections::HashSet::new();
    while !current.is_empty() && guard.insert(current.to_string()) {
        path.push(current.to_string());
        current = accounts
            .get(*index_by_code.get(current)?)
            .map(|account| account.parent_code.as_str())?;
    }
    path.reverse();
    Some(path)
}

fn compare_trial_codes(
    left: &str,
    right: &str,
    accounts: &[TrialAccountRaw],
    index_by_code: &std::collections::HashMap<String, usize>,
) -> std::cmp::Ordering {
    match (
        trial_code_path(left, accounts, index_by_code),
        trial_code_path(right, accounts, index_by_code),
    ) {
        (Some(left_path), Some(right_path)) => left_path.cmp(&right_path),
        _ => left.cmp(right),
    }
}

fn is_same_or_descendant(
    account: &TrialAccountRaw,
    ancestor_code: &str,
    accounts: &[TrialAccountRaw],
    index_by_code: &std::collections::HashMap<String, usize>,
) -> bool {
    if account.code == ancestor_code {
        return true;
    }
    let mut parent = account.parent_code.as_str();
    let mut guard = std::collections::HashSet::new();
    while !parent.is_empty() && guard.insert(parent.to_string()) {
        if parent == ancestor_code {
            return true;
        }
        parent = index_by_code
            .get(parent)
            .map(|&index| accounts[index].parent_code.as_str())
            .unwrap_or("");
    }
    false
}

fn normalize_code_filter(input: &str) -> String {
    db::normalize_code(input)
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect()
}

impl Database {
    pub fn get_trial_balance(&self, from: &str, to: &str) -> Result<TrialBalanceReport, String> {
        // Kept for callers and tests that predate the report filters. It shows
        // every level and every account of an active branch, including rows
        // whose closing balance is zero.
        self.get_trial_balance_filtered(from, to, "", "", true, true)
    }

    /// Hierarchical trial balance with the WebApp report filters.
    ///
    /// - `account_from` / `account_to` limit the code range; either side may be
    ///   empty. When a range is given every level is searched.
    /// - `show_zero` keeps accounts whose closing balance is zero.
    /// - `multi_level` shows the whole tree; otherwise only level-1 accounts.
    pub fn get_trial_balance_filtered(
        &self,
        from: &str,
        to: &str,
        account_from: &str,
        account_to: &str,
        show_zero: bool,
        multi_level: bool,
    ) -> Result<TrialBalanceReport, String> {
        let from = normalize_filter_date(from, "تاریخ شروع")?;
        let to = normalize_filter_date(to, "تاریخ پایان")?;
        check_range(&from, &to)?;
        let code_from = normalize_code_filter(account_from);
        let code_to = normalize_code_filter(account_to);
        let has_code_range = !code_from.is_empty() || !code_to.is_empty();

        self.with_conn(|conn| {
            // Aggregate in SQLite so only one row per account crosses into Rust.
            // Parent accounts are rolled up from these raw amounts below.
            let mut stmt = conn
                .prepare(
                    "SELECT a.code, a.name, g.name, a.level, COALESCE(a.parent_code, ''),
                            COALESCE(SUM(CASE WHEN ?1 <> '' AND v.date < ?1
                                              THEN l.debit END), 0) AS opening_debit,
                            COALESCE(SUM(CASE WHEN ?1 <> '' AND v.date < ?1
                                              THEN l.credit END), 0) AS opening_credit,
                            COALESCE(SUM(CASE WHEN (?1 = '' OR v.date >= ?1)
                                               AND (?2 = '' OR v.date <= ?2)
                                              THEN l.debit END), 0) AS period_debit,
                            COALESCE(SUM(CASE WHEN (?1 = '' OR v.date >= ?1)
                                               AND (?2 = '' OR v.date <= ?2)
                                              THEN l.credit END), 0) AS period_credit
                       FROM accounts a
                       JOIN account_groups g ON g.code = a.group_code
                       LEFT JOIN voucher_lines l ON COALESCE(NULLIF(l.detail1_code, ''), l.account_code) = a.code
                       LEFT JOIN vouchers v ON v.id = l.voucher_id
                      GROUP BY a.code
                      ORDER BY a.code",
                )
                .map_err(db::db_error)?;

            let raw = stmt
                .query_map(params![from, to], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                })
                .map_err(db::db_error)?;

            let mut accounts = Vec::new();
            for row in raw {
                let (code, name, group_name, level, parent_code, od, oc, pd, pc) =
                    row.map_err(db::db_error)?;
                accounts.push(TrialAccountRaw {
                    code,
                    name,
                    group_name,
                    level,
                    parent_code,
                    opening_debit: od,
                    opening_credit: oc,
                    period_debit: pd,
                    period_credit: pc,
                });
            }

            let mut index_by_code = std::collections::HashMap::new();
            for (index, account) in accounts.iter().enumerate() {
                index_by_code.insert(account.code.clone(), index);
            }
            let mut children: std::collections::HashMap<String, Vec<String>> =
                std::collections::HashMap::new();
            let mut has_direct_activity = Vec::with_capacity(accounts.len());
            for account in &accounts {
                if !account.parent_code.is_empty()
                    && index_by_code.contains_key(&account.parent_code)
                {
                    children
                        .entry(account.parent_code.clone())
                        .or_default()
                        .push(account.code.clone());
                }
                has_direct_activity.push(
                    account.opening_debit != 0
                        || account.opening_credit != 0
                        || account.period_debit != 0
                        || account.period_credit != 0,
                );
            }

            let mut selected_by_code = vec![!has_code_range; accounts.len()];
            let mut included_in_report = vec![!has_code_range; accounts.len()];
            if has_code_range {
                for (index, account) in accounts.iter().enumerate() {
                    let in_range = (code_from.is_empty()
                        || compare_trial_codes(
                            &account.code,
                            &code_from,
                            &accounts,
                            &index_by_code,
                        ) != std::cmp::Ordering::Less
                        || is_same_or_descendant(account, &code_from, &accounts, &index_by_code))
                        && (code_to.is_empty()
                            || compare_trial_codes(
                                &account.code,
                                &code_to,
                                &accounts,
                                &index_by_code,
                            ) != std::cmp::Ordering::Greater
                            || is_same_or_descendant(account, &code_to, &accounts, &index_by_code));
                    if !in_range {
                        continue;
                    }
                    selected_by_code[index] = true;
                    included_in_report[index] = true;

                    let mut parent = account.parent_code.as_str();
                    let mut guard = std::collections::HashSet::new();
                    while !parent.is_empty() && guard.insert(parent.to_string()) {
                        let Some(&parent_index) = index_by_code.get(parent) else {
                            break;
                        };
                        included_in_report[parent_index] = true;
                        parent = accounts[parent_index].parent_code.as_str();
                    }
                }
            }
            let selected_codes: std::collections::HashSet<String> = accounts
                .iter()
                .enumerate()
                .filter_map(|(index, account)| {
                    selected_by_code[index].then_some(account.code.clone())
                })
                .collect();

            // Roll every child's amounts into its parent so a level-1 row shows
            // the whole branch, just like the WebApp balance report.
            let mut rolled = std::collections::HashMap::new();
            for account in &accounts {
                let amounts = roll_up_trial_amounts(
                    &account.code,
                    &accounts,
                    &index_by_code,
                    &children,
                    if has_code_range {
                        Some(&selected_codes)
                    } else {
                        None
                    },
                    &mut rolled,
                )?;
                rolled.insert(account.code.clone(), amounts);
            }

            // The WebApp report shows a whole branch as soon as one of its
            // accounts has activity: mark every active account, its ancestors
            // and all of its descendants. Branches without activity stay out.
            let mut relevant = vec![false; accounts.len()];
            let mut queue: Vec<usize> = has_direct_activity
                .iter()
                .enumerate()
                .filter_map(|(index, active)| active.then_some(index))
                .collect();
            while let Some(index) = queue.pop() {
                if relevant[index] {
                    continue;
                }
                relevant[index] = true;
                let parent = accounts[index].parent_code.clone();
                if let Some(&parent_index) = index_by_code.get(&parent) {
                    queue.push(parent_index);
                }
                if let Some(child_codes) = children.get(&accounts[index].code) {
                    for child in child_codes {
                        if let Some(&child_index) = index_by_code.get(child) {
                            queue.push(child_index);
                        }
                    }
                }
            }

            let mut rows = Vec::new();
            for (index, account) in accounts.iter().enumerate() {
                if !relevant[index] {
                    continue;
                }
                if has_code_range && !included_in_report[index] {
                    continue;
                }
                if !has_code_range && !multi_level && account.level != 1 {
                    continue;
                }

                let (od, oc, pd, pc) = rolled.get(&account.code).copied().unwrap_or((0, 0, 0, 0));
                let opening_net = checked_sub(od, oc, "مانده افتتاحیه تراز آزمایشی")?;
                let closing_net = checked_sub(
                    checked_add(opening_net, pd, "مانده پایان دوره تراز آزمایشی")?,
                    pc,
                    "مانده پایان دوره تراز آزمایشی",
                )?;
                let closing_debit = closing_net.max(0);
                let closing_credit =
                    checked_neg(closing_net, "مانده بستانکار پایان دوره تراز آزمایشی")?.max(0);
                if !show_zero && closing_debit == 0 && closing_credit == 0 {
                    continue;
                }
                rows.push(TrialBalanceRow {
                    account_code: account.code.clone(),
                    account_name: account.name.clone(),
                    group_name: account.group_name.clone(),
                    parent_code: account.parent_code.clone(),
                    level: account.level,
                    opening_debit: opening_net.max(0),
                    opening_credit: checked_neg(
                        opening_net,
                        "مانده بستانکار افتتاحیه تراز آزمایشی",
                    )?
                    .max(0),
                    period_debit: pd,
                    period_credit: pc,
                    closing_debit,
                    closing_credit,
                });
            }

            // Totals come from the highest shown row of each branch; summing
            // every row would count children twice inside their parent.
            let shown: std::collections::HashSet<&str> =
                rows.iter().map(|row| row.account_code.as_str()).collect();
            let mut total_rows = Vec::new();
            for row in &rows {
                let mut parent = accounts[index_by_code[&row.account_code]]
                    .parent_code
                    .as_str();
                let mut has_shown_ancestor = false;
                let mut guard = std::collections::HashSet::new();
                while !parent.is_empty() && guard.insert(parent.to_string()) {
                    if shown.contains(parent) {
                        has_shown_ancestor = true;
                        break;
                    }
                    parent = index_by_code
                        .get(parent)
                        .map(|&index| accounts[index].parent_code.as_str())
                        .unwrap_or("");
                }
                if !has_shown_ancestor {
                    total_rows.push(row);
                }
            }

            let opening_debit = checked_sum(
                total_rows.iter().map(|r| r.opening_debit),
                "جمع بدهکار افتتاحیه تراز آزمایشی",
            )?;
            let opening_credit = checked_sum(
                total_rows.iter().map(|r| r.opening_credit),
                "جمع بستانکار افتتاحیه تراز آزمایشی",
            )?;
            let period_debit = checked_sum(
                total_rows.iter().map(|r| r.period_debit),
                "جمع بدهکار دوره تراز آزمایشی",
            )?;
            let period_credit = checked_sum(
                total_rows.iter().map(|r| r.period_credit),
                "جمع بستانکار دوره تراز آزمایشی",
            )?;
            let closing_debit = checked_sum(
                total_rows.iter().map(|r| r.closing_debit),
                "جمع بدهکار پایان دوره تراز آزمایشی",
            )?;
            let closing_credit = checked_sum(
                total_rows.iter().map(|r| r.closing_credit),
                "جمع بستانکار پایان دوره تراز آزمایشی",
            )?;

            Ok(TrialBalanceReport {
                from,
                to,
                rows,
                opening_debit,
                opening_credit,
                period_debit,
                period_credit,
                closing_debit,
                closing_credit,
            })
        })
    }
}
