//! Shared SQL helpers for voucher filters, paging and single-voucher reads.

use crate::db::{self, normalize_digits};
use crate::vouchers::model::{Voucher, VoucherLine, VoucherSummary, MAX_PAGE_SIZE};
use crate::vouchers::validation::checked_sum;
use rusqlite::{params, Connection, OptionalExtension};

pub(crate) struct SearchTerms {
    /// Persian/Arabic digits and Arabic yeh/kaf folded to their usual forms.
    pub(crate) folded: String,
    /// What the user actually typed.
    pub(crate) raw: String,
}

fn escape_like(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

pub(crate) fn search_terms(input: &str) -> SearchTerms {
    let raw = input.trim();
    let folded: String = normalize_digits(raw)
        .chars()
        .map(|ch| match ch {
            '\u{064A}' | '\u{0649}' => '\u{06CC}',
            '\u{0643}' => '\u{06A9}',
            other => other,
        })
        .collect();
    SearchTerms {
        folded: escape_like(&folded),
        raw: escape_like(raw),
    }
}

/// SQL predicate matching a voucher against the search placeholder `p`
/// (`?3` or `?4`). Wildcards in the text are escaped by `search_terms`.
fn search_clause(p: &str) -> String {
    format!(
        "CAST(v.number AS TEXT) LIKE '%' || {p} || '%' ESCAPE '\\'
         OR v.description LIKE '%' || {p} || '%' ESCAPE '\\'
         OR v.type LIKE '%' || {p} || '%' ESCAPE '\\'
         OR EXISTS (
             SELECT 1
               FROM voucher_lines l2
               JOIN accounts a2 ON a2.code = l2.account_code
               LEFT JOIN accounts d2 ON d2.code = l2.detail1_code
              WHERE l2.voucher_id = v.id
                AND (a2.name LIKE '%' || {p} || '%' ESCAPE '\\'
                     OR l2.account_code LIKE '%' || {p} || '%' ESCAPE '\\'
                     OR d2.name LIKE '%' || {p} || '%' ESCAPE '\\'
                     OR COALESCE(l2.detail1_code, '') LIKE '%' || {p} || '%' ESCAPE '\\'
                     OR l2.description LIKE '%' || {p} || '%' ESCAPE '\\')
         )"
    )
}

pub(crate) fn search_predicate() -> String {
    format!(
        "(?3 = '' OR {} OR {})",
        search_clause("?3"),
        search_clause("?4")
    )
}

pub(crate) fn count(conn: &Connection, sql: &str) -> Result<i64, String> {
    conn.query_row(sql, [], |row| row.get(0))
        .map_err(db::db_error)
}

pub(crate) fn list_vouchers(
    conn: &Connection,
    from: &str,
    to: &str,
    terms: &SearchTerms,
    page: Option<(i64, i64)>,
) -> Result<Vec<VoucherSummary>, String> {
    let mut sql = format!(
        "SELECT v.id, v.number, v.date, v.updated_at, v.type, v.description, v.is_temporary,
                COALESCE(SUM(l.debit), 0), COALESCE(SUM(l.credit), 0), COUNT(l.id)
           FROM vouchers v
           LEFT JOIN voucher_lines l ON l.voucher_id = v.id
          WHERE (?1 = '' OR v.date >= ?1)
            AND (?2 = '' OR v.date <= ?2)
            AND {}
          GROUP BY v.id
          ORDER BY v.date DESC, v.number DESC",
        search_predicate()
    );
    if page.is_some() {
        sql.push_str(" LIMIT ?5 OFFSET ?6");
    }
    let mut stmt = conn.prepare(&sql).map_err(db::db_error)?;
    let map_row = |row: &rusqlite::Row<'_>| {
        Ok(VoucherSummary {
            id: row.get(0)?,
            number: row.get(1)?,
            date: row.get(2)?,
            updated_at: row.get(3)?,
            voucher_type: row.get(4)?,
            description: row.get(5)?,
            is_temporary: row.get::<_, i64>(6)? != 0,
            total_debit: row.get(7)?,
            total_credit: row.get(8)?,
            line_count: row.get(9)?,
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
    Ok(rows)
}

/// Accepts the optional paging arguments sent by the UI. No limit means the
/// caller wants the whole result, which reports and export paths rely on.
pub(crate) fn page_window(
    offset: Option<i64>,
    limit: Option<i64>,
) -> Result<Option<(i64, i64)>, String> {
    let Some(limit) = limit else {
        if offset.unwrap_or(0) != 0 {
            return Err("برای صفحه‌بندی، اندازهٔ صفحه را هم مشخص کنید.".to_string());
        }
        return Ok(None);
    };
    let offset = offset.unwrap_or(0);
    if offset < 0 {
        return Err("موقعیت شروع صفحه نمی‌تواند منفی باشد.".to_string());
    }
    if limit <= 0 || limit > MAX_PAGE_SIZE {
        return Err(format!(
            "تعداد ردیف‌های هر صفحه باید بین ۱ و {} باشد.",
            db::to_fa_digits(MAX_PAGE_SIZE.to_string())
        ));
    }
    Ok(Some((limit, offset)))
}

pub(crate) fn voucher_list_totals(
    conn: &Connection,
    from: &str,
    to: &str,
    terms: &SearchTerms,
) -> Result<(i64, i64, i64), String> {
    let sql = format!(
        "SELECT COUNT(DISTINCT v.id),
                COALESCE(SUM(l.debit), 0),
                COALESCE(SUM(l.credit), 0)
           FROM vouchers v
           LEFT JOIN voucher_lines l ON l.voucher_id = v.id
          WHERE (?1 = '' OR v.date >= ?1)
            AND (?2 = '' OR v.date <= ?2)
            AND {}",
        search_predicate()
    );
    conn.query_row(&sql, params![from, to, terms.folded, terms.raw], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })
    .map_err(db::db_error)
}

pub(crate) fn fetch_voucher_by_number(
    conn: &Connection,
    number: i64,
) -> Result<Option<Voucher>, String> {
    conn.query_row(
        "SELECT id FROM vouchers WHERE number = ?1",
        params![number],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map_err(db::db_error)?
    .map(|id| fetch_voucher_by_id(conn, id))
    .transpose()
}

pub(crate) fn fetch_voucher_by_id(conn: &Connection, id: i64) -> Result<Voucher, String> {
    let (number, date, updated_at, voucher_type, description, is_temporary) = conn
        .query_row(
            "SELECT number, date, updated_at, type, description, is_temporary FROM vouchers WHERE id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)? != 0,
                ))
            },
        )
        .optional()
        .map_err(db::db_error)?
        .ok_or_else(|| "سند حسابداری یافت نشد.".to_string())?;

    let mut stmt = conn
        .prepare(
            "SELECT l.id, l.row_no, l.account_code, a.name,
                    NULLIF(l.detail1_code,''), d.name, l.description, l.debit, l.credit
               FROM voucher_lines l
               JOIN accounts a ON a.code = l.account_code
               LEFT JOIN accounts d ON d.code = l.detail1_code
              WHERE l.voucher_id = ?1
              ORDER BY l.row_no",
        )
        .map_err(db::db_error)?;
    let lines = stmt
        .query_map(params![id], |row| {
            Ok(VoucherLine {
                id: row.get(0)?,
                row_no: row.get(1)?,
                account_code: row.get(2)?,
                account_name: row.get(3)?,
                detail1_code: row.get(4)?,
                detail1_name: row.get(5)?,
                description: row.get(6)?,
                debit: row.get(7)?,
                credit: row.get(8)?,
            })
        })
        .map_err(db::db_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db::db_error)?;

    let total_debit = checked_sum(lines.iter().map(|l| l.debit), "جمع بدهکار سند")?;
    let total_credit = checked_sum(lines.iter().map(|l| l.credit), "جمع بستانکار سند")?;
    Ok(Voucher {
        id,
        number,
        date,
        updated_at,
        is_temporary,
        voucher_type,
        description,
        lines,
        total_debit,
        total_credit,
    })
}
