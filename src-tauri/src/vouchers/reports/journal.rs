//! Journal report: every line of every voucher in a date/text filter.

use crate::db::{self, Database};
use crate::vouchers::model::{JournalReport, JournalRow, ReportScope};
use crate::vouchers::query::{page_window, search_predicate, search_terms};
use crate::vouchers::validation::{check_range, normalize_filter_date};
use rusqlite::params;

impl Database {
    pub fn get_journal(&self, from: &str, to: &str, search: &str) -> Result<JournalReport, String> {
        self.get_journal_scoped(from, to, search, ReportScope::Working)
    }

    pub fn get_journal_scoped(
        &self,
        from: &str,
        to: &str,
        search: &str,
        scope: ReportScope,
    ) -> Result<JournalReport, String> {
        self.get_journal_page_scoped(from, to, search, None, None, scope)
    }

    /// Returns one page of journal rows while keeping the totals and voucher
    /// count for the whole filter.
    pub fn get_journal_page(
        &self,
        from: &str,
        to: &str,
        search: &str,
        offset: Option<i64>,
        limit: Option<i64>,
    ) -> Result<JournalReport, String> {
        self.get_journal_page_scoped(from, to, search, offset, limit, ReportScope::Working)
    }

    pub fn get_journal_page_scoped(
        &self,
        from: &str,
        to: &str,
        search: &str,
        offset: Option<i64>,
        limit: Option<i64>,
        scope: ReportScope,
    ) -> Result<JournalReport, String> {
        let from = normalize_filter_date(from, "تاریخ شروع")?;
        let to = normalize_filter_date(to, "تاریخ پایان")?;
        check_range(&from, &to)?;
        let terms = search_terms(search);
        let search = search.trim().to_string();
        let page = page_window(offset, limit)?;
        let scope_sql = scope.sql();

        self.with_conn(|conn| {
            let where_clause = format!(
                "WHERE (?1 = '' OR v.date >= ?1)
                    AND (?2 = '' OR v.date <= ?2)
                    AND {scope_sql}
                    AND {}",
                search_predicate()
            );
            let totals_sql = format!(
                "SELECT COUNT(*),
                        COUNT(DISTINCT v.id),
                        COALESCE(SUM(l.debit), 0),
                        COALESCE(SUM(l.credit), 0)
                   FROM voucher_lines l
                   JOIN vouchers v ON v.id = l.voucher_id
                   JOIN accounts a ON a.code = COALESCE(NULLIF(l.detail1_code, ''), l.account_code)
                 {where_clause}"
            );
            let (row_count, voucher_count, total_debit, total_credit) = conn
                .query_row(
                    &totals_sql,
                    params![from, to, terms.folded, terms.raw],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    },
                )
                .map_err(db::db_error)?;

            let mut sql = format!(
                "SELECT v.id, v.number, v.date, v.type, v.description,
                        l.row_no, COALESCE(NULLIF(l.detail1_code, ''), l.account_code), a.name, l.description,
                        l.debit, l.credit
                   FROM voucher_lines l
                   JOIN vouchers v ON v.id = l.voucher_id
                   JOIN accounts a ON a.code = COALESCE(NULLIF(l.detail1_code, ''), l.account_code)
                 {where_clause}
                  ORDER BY v.date, v.number, l.row_no"
            );
            if page.is_some() {
                sql.push_str(" LIMIT ?5 OFFSET ?6");
            }
            let mut stmt = conn.prepare(&sql).map_err(db::db_error)?;
            let map_row = |row: &rusqlite::Row<'_>| {
                Ok(JournalRow {
                    voucher_id: row.get(0)?,
                    number: row.get(1)?,
                    date: row.get(2)?,
                    voucher_type: row.get(3)?,
                    voucher_description: row.get(4)?,
                    row_no: row.get(5)?,
                    account_code: row.get(6)?,
                    account_name: row.get(7)?,
                    line_description: row.get(8)?,
                    debit: row.get(9)?,
                    credit: row.get(10)?,
                })
            };
            let rows = if let Some((limit, offset)) = page {
                stmt.query_map(
                    params![from, to, terms.folded, terms.raw, limit, offset],
                    map_row,
                )
                .map_err(db::db_error)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db::db_error)?
            } else {
                stmt.query_map(params![from, to, terms.folded, terms.raw], map_row)
                    .map_err(db::db_error)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(db::db_error)?
            };

            Ok(JournalReport {
                from,
                to,
                search,
                rows,
                total_debit,
                total_credit,
                row_count,
                voucher_count,
            })
        })
    }
}
