//! Accounting-health checks attached to the financial statements.
//!
//! The rules mirror `runIntegrityChecks` in the WebApp statement engine. This
//! module only inspects data; it never changes statement figures.

use std::collections::HashMap;

use rusqlite::{params, Connection, ToSql};

use crate::db::classification::{self, AccountClassificationRow, EffectiveClassification};
use crate::db::fs_catalog::{is_fs_line_code, FsStatement, UNCLASSIFIED};
use crate::db::{self, Database};
use crate::vouchers::model::{
    IntegrityCounts, IntegrityIssue, IntegrityReport, ReportScope, CLOSING_FINAL,
    CLOSING_TEMPORARY, INTEGRITY_ERROR, INTEGRITY_INFO, INTEGRITY_WARNING, VOUCHER_TYPES,
};

const SAMPLE_LIMIT: usize = 20;

#[derive(Debug, Clone, Copy, Default)]
struct Pair {
    debit: i64,
    credit: i64,
}

impl Pair {
    fn net(self) -> Result<i64, String> {
        self.debit
            .checked_sub(self.credit)
            .ok_or_else(|| "مانده حساب از محدوده عددی مجاز بیشتر است.".to_string())
    }
}

#[derive(Debug, Clone, Copy)]
enum PairBasis {
    /// Opening + normal vouchers, without closing vouchers (statement basis).
    Statement,
    /// Opening vouchers only.
    Opening,
    /// Every voucher except opening vouchers.
    Ledger,
    /// Normal and temporary-closing vouchers, without opening/final closing.
    PreClosing,
    /// Final closing vouchers only.
    Closing,
}

fn load_pairs(
    conn: &Connection,
    from: &str,
    to: &str,
    scope: ReportScope,
    basis: PairBasis,
) -> Result<HashMap<String, Pair>, String> {
    let scope_sql = scope.sql("v");
    let (predicate, extra): (&str, Vec<&dyn ToSql>) = match basis {
        PairBasis::Statement => (
            "v.type NOT IN (?3, ?4)",
            vec![&CLOSING_TEMPORARY, &CLOSING_FINAL],
        ),
        PairBasis::Opening => ("v.type = ?3", vec![&VOUCHER_TYPES[1]]),
        PairBasis::Ledger => ("v.type <> ?3", vec![&VOUCHER_TYPES[1]]),
        PairBasis::PreClosing => (
            "v.type NOT IN (?3, ?4)",
            vec![&VOUCHER_TYPES[1], &CLOSING_FINAL],
        ),
        PairBasis::Closing => ("v.type = ?3", vec![&CLOSING_FINAL]),
    };
    let sql = format!(
        "SELECT COALESCE(NULLIF(l.detail1_code, ''), l.account_code) AS code,
                COALESCE(SUM(l.debit), 0),
                COALESCE(SUM(l.credit), 0)
           FROM voucher_lines l
           JOIN vouchers v ON v.id = l.voucher_id
          WHERE (?1 = '' OR v.date >= ?1)
            AND (?2 = '' OR v.date <= ?2)
            AND {scope_sql}
            AND {predicate}
          GROUP BY 1"
    );

    let mut values: Vec<&dyn ToSql> = vec![&from, &to];
    values.extend(extra);
    let mut stmt = conn.prepare(&sql).map_err(db::db_error)?;
    let rows = stmt
        .query_map(values.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                Pair {
                    debit: row.get(1)?,
                    credit: row.get(2)?,
                },
            ))
        })
        .map_err(db::db_error)?;

    let mut pairs = HashMap::new();
    for row in rows {
        let (code, pair) = row.map_err(db::db_error)?;
        pairs.insert(code, pair);
    }
    Ok(pairs)
}

fn has_closing_voucher(
    conn: &Connection,
    to: &str,
    scope: ReportScope,
    permanent_only: bool,
) -> Result<bool, String> {
    let scope_sql = scope.sql("v");
    let predicate = if permanent_only {
        "v.type = ?2"
    } else {
        "v.type IN (?2, ?3)"
    };
    let sql = format!(
        "SELECT EXISTS(
            SELECT 1 FROM vouchers v
             WHERE (?1 = '' OR v.date <= ?1)
               AND {scope_sql}
               AND {predicate}
        )"
    );
    let mut values: Vec<&dyn ToSql> = vec![&to, &CLOSING_FINAL];
    if !permanent_only {
        values.push(&CLOSING_TEMPORARY);
    }
    conn.query_row(&sql, values.as_slice(), |row| row.get(0))
        .map_err(db::db_error)
}

fn sample(codes: &[String]) -> Vec<String> {
    codes.iter().take(SAMPLE_LIMIT).cloned().collect()
}

fn issue(
    issues: &mut Vec<IntegrityIssue>,
    code: &str,
    severity: &str,
    message: String,
    count: Option<i64>,
    amount: Option<i64>,
    accounts: Vec<String>,
) {
    issues.push(IntegrityIssue {
        code: code.to_string(),
        severity: severity.to_string(),
        message,
        count,
        amount,
        accounts: sample(&accounts),
    });
}

/// Runs the same accounting-health checks as the WebApp engine.
///
/// `from`/`to` follow the statement range (`from` is empty for an as-of
/// balance sheet), and `cumulative` selects opening + turnover instead of a
/// period-only movement. `balance_variance` is supplied only by the balance
/// sheet, once its totals are known.
pub(crate) fn run_integrity_checks(
    conn: &Connection,
    from: &str,
    to: &str,
    cumulative: bool,
    scope: ReportScope,
    balance_variance: Option<i64>,
) -> Result<IntegrityReport, String> {
    let accounts = classification::load_account_classifications_as_of(conn, to)?;
    let by_code = classification::load_classifications_as_of(conn, to)?;
    let effective: HashMap<String, EffectiveClassification> = accounts
        .iter()
        .map(|account| {
            (
                account.code.clone(),
                classification::resolve_account(account, &by_code),
            )
        })
        .collect();

    // Structural chart checks (a contra reference, for example) describe the
    // live chart, not a historical snapshot. They must still catch a row that
    // was changed outside the normal write path, so they read current rows.
    let current_accounts = classification::load_account_classifications(conn)?;
    let current_by_code: HashMap<String, AccountClassificationRow> = current_accounts
        .iter()
        .map(|account| (account.code.clone(), account.clone()))
        .collect();
    let current_effective: HashMap<String, EffectiveClassification> = current_accounts
        .iter()
        .map(|account| {
            (
                account.code.clone(),
                classification::resolve_account(account, &current_by_code),
            )
        })
        .collect();

    let (range_from, range_to) = if cumulative { ("", to) } else { (from, to) };
    let raw = load_pairs(conn, range_from, range_to, scope, PairBasis::Statement)?;
    let opening = load_pairs(conn, "", to, scope, PairBasis::Opening)?;
    let ledger = load_pairs(conn, "", to, scope, PairBasis::Ledger)?;
    let pre_closing = load_pairs(conn, "", to, scope, PairBasis::PreClosing)?;
    let closing = load_pairs(conn, "", to, scope, PairBasis::Closing)?;
    let has_closing = has_closing_voucher(conn, to, scope, false)?;
    let has_permanent_closing = has_closing_voucher(conn, to, scope, true)?;

    let pair_of =
        |map: &HashMap<String, Pair>, code: &str| map.get(code).copied().unwrap_or_default();

    let mut issues: Vec<IntegrityIssue> = Vec::new();

    // 1) Trial balance: total debit equals total credit on the statement basis.
    let mut total_debit = 0i64;
    let mut total_credit = 0i64;
    for account in &accounts {
        let pair = pair_of(&raw, &account.code);
        total_debit = total_debit
            .checked_add(pair.debit)
            .ok_or_else(|| "جمع بدهکار تراز آزمایشی از محدوده عددی مجاز بیشتر است.".to_string())?;
        total_credit = total_credit.checked_add(pair.credit).ok_or_else(|| {
            "جمع بستانکار تراز آزمایشی از محدوده عددی مجاز بیشتر است.".to_string()
        })?;
    }
    if total_debit != total_credit {
        let difference = total_debit
            .checked_sub(total_credit)
            .ok_or_else(|| "اختلاف تراز آزمایشی از محدوده عددی مجاز بیشتر است.".to_string())?;
        issue(
            &mut issues,
            "trial_balance",
            INTEGRITY_ERROR,
            format!(
                "تراز آزمایشی برقرار نیست: جمع بدهکار {} و جمع بستانکار {} (اختلاف {}).",
                db::to_fa_digits(total_debit),
                db::to_fa_digits(total_credit),
                db::to_fa_digits(difference.abs())
            ),
            None,
            Some(difference),
            Vec::new(),
        );
    }

    // 2) Accounts with a balance but no valid statement line.
    let mut unclassified: Vec<String> = Vec::new();
    let mut unclassified_amount = 0i64;
    for account in &accounts {
        let Some(classification) = effective.get(&account.code) else {
            continue;
        };
        if classification.fs_line != UNCLASSIFIED {
            continue;
        }
        let raw_net = pair_of(&raw, &account.code).net()?;
        if raw_net == 0 {
            continue;
        }
        unclassified.push(account.code.clone());
        unclassified_amount = unclassified_amount.checked_add(raw_net).ok_or_else(|| {
            "جمع مانده حساب‌های بدون سرفصل از محدوده عددی مجاز بیشتر است.".to_string()
        })?;
    }
    if !unclassified.is_empty() {
        issue(
            &mut issues,
            "unclassified_with_balance",
            INTEGRITY_ERROR,
            format!(
                "{} حساب دارای مانده سرفصل صورت مالی ندارد؛ در صفحه «بررسی طبقه‌بندی» تعیین کنید. تا آن زمان مبلغشان در ردیف «طبقه‌بندی‌نشده» می‌آید.",
                db::to_fa_digits(unclassified.len() as i64)
            ),
            Some(unclassified.len() as i64),
            Some(unclassified_amount),
            unclassified,
        );
    }

    // 3) Group type is incompatible with the effective statement line.
    let mut mismatched: Vec<String> = Vec::new();
    for account in &accounts {
        let Some(classification) = effective.get(&account.code) else {
            continue;
        };
        if !is_fs_line_code(&classification.fs_line) || account.group_type.is_empty() {
            continue;
        }
        if !classification::group_type_matches_fs_line(&account.group_type, &classification.fs_line)
        {
            mismatched.push(account.code.clone());
        }
    }
    if !mismatched.is_empty() {
        issue(
            &mut issues,
            "group_type_mismatch",
            INTEGRITY_WARNING,
            format!(
                "{} حساب سرفصلی ناسازگار با نوع گروهش دارد.",
                db::to_fa_digits(mismatched.len() as i64)
            ),
            Some(mismatched.len() as i64),
            None,
            mismatched,
        );
    }

    // 4) Invalid contra account: own nature/kind incompatible with the line,
    // missing/self target, different line, or a target that is itself a contra
    // account.
    let mut contra_problems: Vec<String> = Vec::new();
    for account in &current_accounts {
        if account.contra_of.is_empty() {
            continue;
        }
        let Some(own) = current_effective.get(&account.code) else {
            contra_problems.push(account.code.clone());
            continue;
        };
        if !is_fs_line_code(&own.fs_line)
            || !classification::is_contra_nature(&account.nature, &own.fs_line)
            || !classification::contra_kind_matches_fs_line(&account.contra_kind, &own.fs_line)
        {
            contra_problems.push(account.code.clone());
            continue;
        }
        let Some(target) = current_by_code.get(&account.contra_of) else {
            contra_problems.push(account.code.clone());
            continue;
        };
        if target.code == account.code {
            contra_problems.push(account.code.clone());
            continue;
        }
        let target_effective = classification::resolve_account(target, &current_by_code);
        if target_effective.fs_line != own.fs_line
            || !target.contra_of.is_empty()
            || classification::is_contra_nature(&target.nature, &target_effective.fs_line)
        {
            contra_problems.push(account.code.clone());
        }
    }
    if !contra_problems.is_empty() {
        issue(
            &mut issues,
            "invalid_contra",
            INTEGRITY_ERROR,
            format!(
                "{} حساب کاهنده ماهیت/نوع سازگار با سرفصل یا مقصد معتبر (هم‌سرفصل و غیرکاهنده) ندارد.",
                db::to_fa_digits(contra_problems.len() as i64)
            ),
            Some(contra_problems.len() as i64),
            None,
            contra_problems,
        );
    }

    // 5) Temporary accounts must start each period with a zero balance.
    let mut temporary_opening: Vec<String> = Vec::new();
    for account in &accounts {
        let Some(classification) = effective.get(&account.code) else {
            continue;
        };
        if classification.statement != FsStatement::IncomeStatement {
            continue;
        }
        if pair_of(&opening, &account.code).net()? != 0 {
            temporary_opening.push(account.code.clone());
        }
    }
    if !temporary_opening.is_empty() {
        issue(
            &mut issues,
            "temporary_opening_balance",
            INTEGRITY_ERROR,
            format!(
                "{} حساب موقت در سند افتتاحیه مانده دارد (حساب‌های سود و زیانی باید در ابتدای دوره صفر باشند).",
                db::to_fa_digits(temporary_opening.len() as i64)
            ),
            Some(temporary_opening.len() as i64),
            None,
            temporary_opening,
        );
    }

    // 6) The balance sheet must balance.
    if let Some(variance) = balance_variance {
        if variance != 0 {
            issue(
                &mut issues,
                "balance_sheet_not_balanced",
                INTEGRITY_ERROR,
                format!(
                    "جمع دارایی‌ها با جمع بدهی‌ها و حقوق مالکانه برابر نیست (اختلاف {}).",
                    db::to_fa_digits(variance.abs())
                ),
                None,
                Some(variance),
                Vec::new(),
            );
        }
    }

    // 7) After a closing voucher, every temporary account must be zero.
    if has_closing {
        let mut not_closed: Vec<String> = Vec::new();
        for account in &accounts {
            let Some(classification) = effective.get(&account.code) else {
                continue;
            };
            if classification.statement != FsStatement::IncomeStatement {
                continue;
            }
            let balance = pair_of(&opening, &account.code)
                .net()?
                .checked_add(pair_of(&ledger, &account.code).net()?)
                .ok_or_else(|| "جمع مانده حساب موقت از محدوده عددی مجاز بیشتر است.".to_string())?;
            if balance != 0 {
                not_closed.push(account.code.clone());
            }
        }
        if !not_closed.is_empty() {
            issue(
                &mut issues,
                "temporary_not_closed",
                INTEGRITY_ERROR,
                format!(
                    "{} حساب موقت پس از بستن سال مانده‌دار است (سند بستن حساب‌های موقت باید همه حساب‌های سود و زیانی را صفر کند).",
                    db::to_fa_digits(not_closed.len() as i64)
                ),
                Some(not_closed.len() as i64),
                None,
                not_closed,
            );
        }
    }

    // 8) A dual/split account has movement on both sides in the same period.
    let mut both_sides: Vec<String> = Vec::new();
    for account in &accounts {
        if account.nature != "dual" && !account.split_by_sign {
            continue;
        }
        let pair = pair_of(&raw, &account.code);
        if pair.debit > 0 && pair.credit > 0 {
            both_sides.push(account.code.clone());
        }
    }
    if !both_sides.is_empty() {
        issue(
            &mut issues,
            "dual_both_sides",
            INTEGRITY_INFO,
            format!(
                "{} حساب دوگانه/تفکیک‌شده در هر دو سمت گردش دارد.",
                db::to_fa_digits(both_sides.len() as i64)
            ),
            Some(both_sides.len() as i64),
            None,
            both_sides,
        );
    }

    // 9) Closing vouchers must cancel the balances they are meant to close.
    let mut closing_mismatch: Vec<String> = Vec::new();
    if has_closing {
        for account in &accounts {
            let Some(classification) = effective.get(&account.code) else {
                continue;
            };
            let relevant = classification.statement == FsStatement::IncomeStatement
                || (has_permanent_closing && classification.statement == FsStatement::BalanceSheet);
            if !relevant {
                continue;
            }
            let before = pair_of(&opening, &account.code)
                .net()?
                .checked_add(pair_of(&pre_closing, &account.code).net()?)
                .ok_or_else(|| {
                    "جمع مانده پیش از بستن از محدوده عددی مجاز بیشتر است.".to_string()
                })?;
            let closed = pair_of(&closing, &account.code).net()?;
            if before
                .checked_add(closed)
                .ok_or_else(|| "جمع مانده بستن از محدوده عددی مجاز بیشتر است.".to_string())?
                != 0
            {
                closing_mismatch.push(account.code.clone());
            }
        }
    }
    let mut income_summary: Vec<String> = Vec::new();
    for account in &accounts {
        if account.system_role != "income_summary" {
            continue;
        }
        let summary_balance = if has_closing {
            pair_of(&opening, &account.code)
                .net()?
                .checked_add(pair_of(&pre_closing, &account.code).net()?)
                .ok_or_else(|| {
                    "جمع مانده خلاصه سود و زیان از محدوده عددی مجاز بیشتر است.".to_string()
                })?
        } else {
            pair_of(&raw, &account.code).net()?
        };
        if summary_balance != 0 {
            income_summary.push(account.code.clone());
        }
    }
    if !closing_mismatch.is_empty() || !income_summary.is_empty() {
        let mut parts: Vec<String> = Vec::new();
        if !closing_mismatch.is_empty() {
            parts.push(format!(
                "{} حساب پس از بستن مربوط به آن هنوز مانده دارد",
                db::to_fa_digits(closing_mismatch.len() as i64)
            ));
        }
        if !income_summary.is_empty() {
            parts.push(format!(
                "حساب خلاصه سود و زیان ({}) به سود (زیان) انباشته منتقل نشده و مانده دارد",
                sample(&income_summary).join("، ")
            ));
        }
        issue(
            &mut issues,
            "closing_mismatch",
            if has_closing {
                INTEGRITY_ERROR
            } else {
                INTEGRITY_WARNING
            },
            format!("{}.", parts.join("؛ ")),
            Some((closing_mismatch.len() + income_summary.len()) as i64),
            None,
            closing_mismatch.into_iter().chain(income_summary).collect(),
        );
    }

    // 10) A permanent voucher must belong to a usable fiscal year. Temporary
    // vouchers are drafts and are allowed to sit outside the configured years
    // until they are finalized.
    let mut voucher_without_year: Vec<String> = Vec::new();
    {
        let sql = format!(
            "SELECT v.number, v.date
               FROM vouchers v
              WHERE v.is_temporary = 0
                AND {}
                AND (?1 = '' OR v.date >= ?1)
                AND v.date <= ?2
                AND (v.fiscal_year_id IS NULL
                     OR NOT EXISTS (SELECT 1 FROM fiscal_years f WHERE f.id = v.fiscal_year_id))
              ORDER BY v.date, v.number",
            scope.sql("v")
        );
        let mut statement = conn.prepare(&sql).map_err(db::db_error)?;
        let rows = statement
            .query_map(params![range_from, range_to], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db::db_error)?;
        for row in rows {
            let (number, date) = row.map_err(db::db_error)?;
            voucher_without_year.push(format!("شماره {} ({})", db::to_fa_digits(number), date));
        }
    }
    if !voucher_without_year.is_empty() {
        issue(
            &mut issues,
            "voucher_without_fiscal_year",
            INTEGRITY_ERROR,
            format!(
                "{} سند دائمی به سال مالی معتبر متصل نیست: {}.",
                db::to_fa_digits(voucher_without_year.len() as i64),
                sample(&voucher_without_year).join("، ")
            ),
            Some(voucher_without_year.len() as i64),
            None,
            Vec::new(),
        );
    }

    // 11) The voucher date must fall inside the linked fiscal year. This is a
    // data-integrity check for rows written outside the normal save path; the
    // save path itself rejects such a pair.
    let mut date_outside_year: Vec<String> = Vec::new();
    {
        let sql = format!(
            "SELECT v.number, v.date, f.title, f.start_date, f.end_date
               FROM vouchers v
               JOIN fiscal_years f ON f.id = v.fiscal_year_id
              WHERE {}
                AND (?1 = '' OR v.date >= ?1)
                AND v.date <= ?2
                AND (v.date < f.start_date OR v.date > f.end_date)
              ORDER BY v.date, v.number",
            scope.sql("v")
        );
        let mut statement = conn.prepare(&sql).map_err(db::db_error)?;
        let rows = statement
            .query_map(params![range_from, range_to], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(db::db_error)?;
        for row in rows {
            let (number, date, title, start, end) = row.map_err(db::db_error)?;
            date_outside_year.push(format!(
                "شماره {} ({}) خارج از «{}» ({} تا {})",
                db::to_fa_digits(number),
                date,
                title,
                start,
                end
            ));
        }
    }
    if !date_outside_year.is_empty() {
        issue(
            &mut issues,
            "voucher_date_outside_fiscal_year",
            INTEGRITY_ERROR,
            format!(
                "{} سند تاریخی خارج از بازه سال مالی متصل به خود دارد: {}.",
                db::to_fa_digits(date_outside_year.len() as i64),
                sample(&date_outside_year).join("، ")
            ),
            Some(date_outside_year.len() as i64),
            None,
            Vec::new(),
        );
    }

    // 12) Legacy level-4 rows stay readable but cannot be edited as a normal
    // account/detail pair. The explicit migration tool in settings resolves
    // them; until then the row is reported so it cannot be forgotten.
    let mut legacy_pending: Vec<String> = Vec::new();
    {
        let sql = format!(
            "SELECT v.number, l.account_code
               FROM voucher_lines l
               JOIN vouchers v ON v.id = l.voucher_id
              WHERE l.legacy_level4_pending = 1
                AND {}
                AND (?1 = '' OR v.date >= ?1)
                AND v.date <= ?2
              ORDER BY v.date, v.number, l.row_no",
            scope.sql("v")
        );
        let mut statement = conn.prepare(&sql).map_err(db::db_error)?;
        let rows = statement
            .query_map(params![range_from, range_to], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db::db_error)?;
        for row in rows {
            let (number, code) = row.map_err(db::db_error)?;
            legacy_pending.push(format!(
                "شماره {} / حساب {}",
                db::to_fa_digits(number),
                code
            ));
        }
    }
    if !legacy_pending.is_empty() {
        issue(
            &mut issues,
            "legacy_level4_pending",
            INTEGRITY_WARNING,
            format!(
                "{} ردیف سند به حساب سطح چهار قدیمی اشاره می‌کند که هنوز مهاجرت نشده است؛ برای یکسان‌سازی تفصیلی‌ها، ابزار «مهاجرت حساب‌های سطح چهار» را در تنظیمات اجرا کنید: {}.",
                db::to_fa_digits(legacy_pending.len() as i64),
                sample(&legacy_pending).join("، ")
            ),
            Some(legacy_pending.len() as i64),
            None,
            Vec::new(),
        );
    }

    let mut counts = IntegrityCounts::default();
    for finding in &issues {
        match finding.severity.as_str() {
            INTEGRITY_ERROR => counts.error += 1,
            INTEGRITY_WARNING => counts.warning += 1,
            INTEGRITY_INFO => counts.info += 1,
            _ => {}
        }
    }
    Ok(IntegrityReport {
        ok: counts.error == 0,
        counts,
        issues,
    })
}

impl Database {
    /// Integrity checks for a saved book, shared by report commands that need
    /// to block or disclose an unhealthy book before printing.
    pub(crate) fn integrity_report(
        &self,
        from: &str,
        to: &str,
        cumulative: bool,
        scope: ReportScope,
    ) -> Result<IntegrityReport, String> {
        self.with_conn(|conn| run_integrity_checks(conn, from, to, cumulative, scope, None))
    }

    /// Integrity checks for a saved book, exposed to tests.
    #[cfg(test)]
    pub(crate) fn integrity_report_for_test(
        &self,
        from: &str,
        to: &str,
        cumulative: bool,
        scope: ReportScope,
    ) -> Result<IntegrityReport, String> {
        self.integrity_report(from, to, cumulative, scope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::memory_database;
    use crate::vouchers::model::VoucherInput;
    use crate::vouchers::VoucherLineInput;

    fn line(account_code: &str, debit: i64, credit: i64) -> VoucherLineInput {
        VoucherLineInput {
            id: None,
            account_code: account_code.to_string(),
            detail1_code: None,
            description: String::new(),
            debit,
            credit,
        }
    }

    fn voucher(date: &str, voucher_type: &str, lines: Vec<VoucherLineInput>) -> VoucherInput {
        VoucherInput {
            id: None,
            number: None,
            date: date.to_string(),
            voucher_type: voucher_type.to_string(),
            description: String::new(),
            fiscal_year_id: None,
            lines,
        }
    }

    #[test]
    fn healthy_book_has_no_errors() {
        let db = memory_database();
        db.save_voucher(voucher(
            "1405/01/10",
            "",
            vec![line("110101", 1_000, 0), line("410101", 0, 1_000)],
        ))
        .unwrap();

        let report = db
            .integrity_report_for_test("1405/01/01", "1405/12/29", false, ReportScope::Working)
            .unwrap();
        assert!(report.ok, "{:?}", report.issues);
        assert_eq!(report.counts.error, 0);
    }

    #[test]
    fn group_type_mismatch_is_reported_as_warning() {
        let db = memory_database();
        db.create_account_group("90", "گروه ناسازگار", "دارایی", "revenue")
            .unwrap();
        db.create_account(
            "9001",
            "درآمد در گروه دارایی",
            1,
            "90",
            "creditor",
            "",
            true,
        )
        .unwrap();

        let report = db
            .integrity_report_for_test("1405/01/01", "1405/12/29", false, ReportScope::Working)
            .unwrap();
        assert!(report.ok, "a warning must not fail the report");
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "group_type_mismatch"));
    }

    #[test]
    fn invalid_contra_target_is_reported_as_error() {
        let db = memory_database();
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE accounts SET contra_of = 'missing-target', contra_kind = 'other' WHERE code = '110101'",
                [],
            )
            .map_err(db::db_error)?;
            Ok(())
        })
        .unwrap();

        let report = db
            .integrity_report_for_test("1405/01/01", "1405/12/29", false, ReportScope::Working)
            .unwrap();
        assert!(!report.ok);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "invalid_contra"));
    }

    #[test]
    fn permanent_voucher_without_fiscal_year_is_reported_as_error() {
        let db = memory_database();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO vouchers (number, date, type, is_temporary, fiscal_year_id)
                 VALUES (9001, '1405/01/10', 'سند حسابداری عادی', 0, NULL)",
                [],
            )
            .map_err(db::db_error)?;
            Ok(())
        })
        .unwrap();

        let report = db
            .integrity_report_for_test("1405/01/01", "1405/12/29", false, ReportScope::Working)
            .unwrap();
        assert!(!report.ok);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "voucher_without_fiscal_year"));
    }

    #[test]
    fn voucher_date_outside_linked_fiscal_year_is_reported_as_error() {
        let db = memory_database();
        let year = db
            .create_fiscal_year("سال نیمه", "1405/01/01", "1405/06/31", "open")
            .unwrap();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO vouchers (number, date, type, is_temporary, fiscal_year_id)
                 VALUES (9002, '1405/08/10', 'سند حسابداری عادی', 0, ?1)",
                rusqlite::params![year.id],
            )
            .map_err(db::db_error)?;
            Ok(())
        })
        .unwrap();

        let report = db
            .integrity_report_for_test("1405/01/01", "1405/12/29", false, ReportScope::Working)
            .unwrap();
        assert!(!report.ok);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "voucher_date_outside_fiscal_year"));
    }

    #[test]
    fn legacy_level4_pending_lines_are_reported_as_warning() {
        let db = memory_database();
        db.save_voucher(voucher(
            "1405/01/10",
            "",
            vec![line("110101", 1_000, 0), line("410101", 0, 1_000)],
        ))
        .unwrap();
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE voucher_lines
                    SET account_id = NULL, legacy_level4_pending = 1
                  WHERE account_code = '110101'",
                [],
            )
            .map_err(db::db_error)?;
            Ok(())
        })
        .unwrap();

        let report = db
            .integrity_report_for_test("1405/01/01", "1405/12/29", false, ReportScope::Working)
            .unwrap();
        assert!(
            report.ok,
            "a pending legacy row is a warning: {:?}",
            report.issues
        );
        assert!(report.counts.warning >= 1);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "legacy_level4_pending"));
    }

    #[test]
    fn dual_movement_on_both_sides_is_informational() {
        let db = memory_database();
        db.save_voucher(voucher(
            "1405/01/10",
            "",
            vec![
                line("110101", 100, 0),
                line("410101", 0, 100),
                line("410101", 40, 0),
                line("110101", 0, 40),
            ],
        ))
        .unwrap();

        let report = db
            .integrity_report_for_test("1405/01/01", "1405/12/29", false, ReportScope::Working)
            .unwrap();
        // 410101 is creditor-nature, not dual, so this book has no dual account.
        assert!(!report
            .issues
            .iter()
            .any(|issue| issue.code == "dual_both_sides"));

        db.create_account("1901", "حساب دوگانه", 1, "1", "dual", "", true)
            .unwrap();
        db.save_voucher(voucher(
            "1405/01/11",
            "",
            vec![
                line("1901", 50, 0),
                line("1901", 0, 20),
                line("110101", 0, 30),
            ],
        ))
        .unwrap();
        let report = db
            .integrity_report_for_test("1405/01/01", "1405/12/29", false, ReportScope::Working)
            .unwrap();
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "dual_both_sides"));
    }
}
