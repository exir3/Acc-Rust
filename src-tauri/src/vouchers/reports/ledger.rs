//! Ledger report for one account and all of its descendants.

use crate::db::{self, Database};
use crate::vouchers::model::{LedgerReport, LedgerRow, ReportScope};
use crate::vouchers::query::page_window;
use crate::vouchers::validation::{
    check_range, checked_add, checked_neg, checked_sub, normalize_filter_date,
};
use rusqlite::{params, OptionalExtension};

impl Database {
    pub fn get_ledger(
        &self,
        account_code: &str,
        from: &str,
        to: &str,
    ) -> Result<LedgerReport, String> {
        self.get_ledger_page(account_code, from, to, None, None)
    }

    pub fn get_ledger_scoped(
        &self,
        account_code: &str,
        from: &str,
        to: &str,
        scope: ReportScope,
    ) -> Result<LedgerReport, String> {
        self.get_ledger_page_scoped(account_code, from, to, None, None, scope)
    }

    /// Returns one page of ledger rows. Opening/closing balances and the debit
    /// and credit totals always describe the whole period, so a paged view
    /// still shows the same bottom line as the printed report.
    pub fn get_ledger_page(
        &self,
        account_code: &str,
        from: &str,
        to: &str,
        offset: Option<i64>,
        limit: Option<i64>,
    ) -> Result<LedgerReport, String> {
        self.get_ledger_page_scoped(account_code, from, to, offset, limit, ReportScope::Working)
    }

    pub fn get_ledger_page_scoped(
        &self,
        account_code: &str,
        from: &str,
        to: &str,
        offset: Option<i64>,
        limit: Option<i64>,
        scope: ReportScope,
    ) -> Result<LedgerReport, String> {
        let code = db::normalize_code(account_code);
        if code.is_empty() {
            return Err("برای گزارش دفتر کل، حساب را انتخاب کنید.".to_string());
        }
        let from = normalize_filter_date(from, "تاریخ شروع")?;
        let to = normalize_filter_date(to, "تاریخ پایان")?;
        check_range(&from, &to)?;
        let page = page_window(offset, limit)?;
        let scope_sql = scope.sql();

        self.with_conn(|conn| {
            let (account_name, group_name) = conn
                .query_row(
                    "SELECT a.name, g.name
                       FROM accounts a
                       JOIN account_groups g ON g.code = a.group_code
                      WHERE a.code = ?1",
                    params![code],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(db::db_error)?
                .ok_or_else(|| "حساب انتخاب‌شده یافت نشد.".to_string())?;

            // A line posts either to its base account or to its selected
            // level-3 detail. The tree therefore rolls up each line exactly once.
            const TREE: &str = "WITH RECURSIVE tree(code) AS (
                    SELECT code FROM accounts WHERE code = ?1
                    UNION ALL
                    SELECT a.code FROM accounts a JOIN tree t ON a.parent_code = t.code
                )";

            let includes_children: bool = conn
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM accounts WHERE parent_code = ?1)",
                    params![code],
                    |row| row.get(0),
                )
                .map_err(db::db_error)?;

            // Opening amounts are aggregated by SQLite; only two numbers cross
            // into Rust instead of every line before the start date.
            let (opening_debit, opening_credit) = conn
                .query_row(
                    &format!(
                        "{TREE}
                         SELECT COALESCE(SUM(l.debit), 0), COALESCE(SUM(l.credit), 0)
                           FROM voucher_lines l
                           JOIN vouchers v ON v.id = l.voucher_id
                          WHERE COALESCE(NULLIF(l.detail1_code, ''), l.account_code) IN (SELECT code FROM tree)
                            AND {scope_sql}
                            AND (?2 <> '' AND v.date < ?2)"
                    ),
                    params![code, from],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .map_err(db::db_error)?;
            let opening_net = checked_sub(opening_debit, opening_credit, "مانده افتتاحیه دفتر کل")?;

            let (row_count, total_debit, total_credit) = conn
                .query_row(
                    &format!(
                        "{TREE}
                         SELECT COUNT(*), COALESCE(SUM(l.debit), 0), COALESCE(SUM(l.credit), 0)
                           FROM voucher_lines l
                           JOIN vouchers v ON v.id = l.voucher_id
                          WHERE COALESCE(NULLIF(l.detail1_code, ''), l.account_code) IN (SELECT code FROM tree)
                            AND {scope_sql}
                            AND (?2 = '' OR v.date >= ?2)
                            AND (?3 = '' OR v.date <= ?3)"
                    ),
                    params![code, from, to],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )
                .map_err(db::db_error)?;

            // Rows before this page define the running balance it starts from.
            // The partial window is summed by SQLite, so a deep page does not
            // transfer every earlier line into Rust just to add it up.
            let mut running = opening_net;
            if let Some((_, offset)) = page {
                if offset > 0 {
                    let (prior_debit, prior_credit) = conn
                        .query_row(
                            &format!(
                                "{TREE}
                                 SELECT COALESCE(SUM(debit), 0), COALESCE(SUM(credit), 0)
                                   FROM (
                                        SELECT l.debit AS debit, l.credit AS credit
                                          FROM voucher_lines l
                                          JOIN vouchers v ON v.id = l.voucher_id
                                         WHERE COALESCE(NULLIF(l.detail1_code, ''), l.account_code) IN (SELECT code FROM tree)
                                           AND {scope_sql}
                                           AND (?2 = '' OR v.date >= ?2)
                                           AND (?3 = '' OR v.date <= ?3)
                                         ORDER BY v.date, v.number, l.row_no
                                         LIMIT ?4
                                   )"
                            ),
                            params![code, from, to, offset],
                            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                        )
                        .map_err(db::db_error)?;
                    running = checked_sub(
                        checked_add(running, prior_debit, "مانده پیش از صفحه دفتر کل")?,
                        prior_credit,
                        "مانده پیش از صفحه دفتر کل",
                    )?;
                }
            }

            let mut sql = format!(
                "{TREE}
                 SELECT v.id, v.number, v.date, v.type, v.description,
                        l.description, l.debit, l.credit, COALESCE(NULLIF(l.detail1_code, ''), l.account_code), a.name
                   FROM voucher_lines l
                   JOIN vouchers v ON v.id = l.voucher_id
                   JOIN accounts a ON a.code = COALESCE(NULLIF(l.detail1_code, ''), l.account_code)
                  WHERE COALESCE(NULLIF(l.detail1_code, ''), l.account_code) IN (SELECT code FROM tree)
                    AND {scope_sql}
                    AND (?2 = '' OR v.date >= ?2)
                    AND (?3 = '' OR v.date <= ?3)
                  ORDER BY v.date, v.number, l.row_no"
            );
            if page.is_some() {
                sql.push_str(" LIMIT ?4 OFFSET ?5");
            }
            let mut stmt = conn.prepare(&sql).map_err(db::db_error)?;
            let raw_rows = if let Some((limit, offset)) = page {
                stmt.query_map(params![code, from, to, limit, offset], map_ledger_row)
                    .map_err(db::db_error)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(db::db_error)?
            } else {
                stmt.query_map(params![code, from, to], map_ledger_row)
                    .map_err(db::db_error)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(db::db_error)?
            };

            let mut rows = Vec::with_capacity(raw_rows.len());
            for row in raw_rows {
                running = checked_add(
                    running,
                    checked_sub(row.6, row.7, "گردش ردیف دفتر کل")?,
                    "مانده جاری دفتر کل",
                )?;
                rows.push(LedgerRow {
                    voucher_id: row.0,
                    account_code: row.8,
                    account_name: row.9,
                    number: row.1,
                    date: row.2,
                    voucher_type: row.3,
                    voucher_description: row.4,
                    line_description: row.5,
                    debit: row.6,
                    credit: row.7,
                    running_debit: running.max(0),
                    running_credit: checked_neg(running, "مانده بستانکار جاری دفتر کل")?.max(0),
                });
            }

            let closing = checked_sub(
                checked_add(opening_net, total_debit, "مانده پایان دوره دفتر کل")?,
                total_credit,
                "مانده پایان دوره دفتر کل",
            )?;
            Ok(LedgerReport {
                account_code: code,
                account_name,
                group_name,
                includes_children,
                from,
                to,
                opening_debit: opening_net.max(0),
                opening_credit: checked_neg(opening_net, "مانده بستانکار افتتاحیه دفتر کل")?
                    .max(0),
                total_debit,
                total_credit,
                closing_debit: closing.max(0),
                closing_credit: checked_neg(closing, "مانده بستانکار پایان دوره دفتر کل")?.max(0),
                row_count,
                rows,
            })
        })
    }
}

type RawLedgerRow = (
    i64,
    i64,
    String,
    String,
    String,
    String,
    i64,
    i64,
    String,
    String,
);

fn map_ledger_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawLedgerRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
    ))
}
