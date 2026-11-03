//! Ledger report for one account and all of its descendants.

use crate::db::{self, Database};
use crate::vouchers::model::{LedgerReport, LedgerRow};
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
        let code = db::normalize_code(account_code);
        if code.is_empty() {
            return Err("برای گزارش دفتر کل، حساب را انتخاب کنید.".to_string());
        }
        let from = normalize_filter_date(from, "تاریخ شروع")?;
        let to = normalize_filter_date(to, "تاریخ پایان")?;
        check_range(&from, &to)?;

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

            // Vouchers are only posted to leaf accounts, so the ledger of a
            // parent account is the combined activity of all its descendants.
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

            let mut opening_stmt = conn
                .prepare(&format!(
                    "{TREE}
                     SELECT l.debit, l.credit
                       FROM voucher_lines l
                       JOIN vouchers v ON v.id = l.voucher_id
                      WHERE l.account_code IN (SELECT code FROM tree)
                        AND (?2 <> '' AND v.date < ?2)"
                ))
                .map_err(db::db_error)?;
            let opening_rows = opening_stmt
                .query_map(params![code, from], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
                })
                .map_err(db::db_error)?;
            let mut opening_debit = 0i64;
            let mut opening_credit = 0i64;
            for row in opening_rows {
                let (debit, credit) = row.map_err(db::db_error)?;
                opening_debit = checked_add(opening_debit, debit, "بدهکار افتتاحیه دفتر کل")?;
                opening_credit = checked_add(opening_credit, credit, "بستانکار افتتاحیه دفتر کل")?;
            }

            let mut stmt = conn
                .prepare(&format!(
                    "{TREE}
                     SELECT v.id, v.number, v.date, v.type, v.description,
                            l.description, l.debit, l.credit, l.account_code, a.name
                       FROM voucher_lines l
                       JOIN vouchers v ON v.id = l.voucher_id
                       JOIN accounts a ON a.code = l.account_code
                      WHERE l.account_code IN (SELECT code FROM tree)
                        AND (?2 = '' OR v.date >= ?2)
                        AND (?3 = '' OR v.date <= ?3)
                      ORDER BY v.date, v.number, l.row_no"
                ))
                .map_err(db::db_error)?;

            let raw_rows = stmt
                .query_map(params![code, from, to], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                    ))
                })
                .map_err(db::db_error)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db::db_error)?;

            let opening_net = checked_sub(opening_debit, opening_credit, "مانده افتتاحیه دفتر کل")?;
            let mut running = opening_net;
            let mut total_debit = 0i64;
            let mut total_credit = 0i64;
            let mut rows = Vec::with_capacity(raw_rows.len());
            for (
                voucher_id,
                number,
                date,
                voucher_type,
                voucher_description,
                line_description,
                debit,
                credit,
                row_account_code,
                row_account_name,
            ) in raw_rows
            {
                running = checked_add(
                    running,
                    checked_sub(debit, credit, "گردش ردیف دفتر کل")?,
                    "مانده جاری دفتر کل",
                )?;
                total_debit = checked_add(total_debit, debit, "جمع بدهکار دفتر کل")?;
                total_credit = checked_add(total_credit, credit, "جمع بستانکار دفتر کل")?;
                rows.push(LedgerRow {
                    voucher_id,
                    account_code: row_account_code,
                    account_name: row_account_name,
                    number,
                    date,
                    voucher_type,
                    voucher_description,
                    line_description,
                    debit,
                    credit,
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
                opening_credit: checked_neg(opening_net, "مانده بستانکار افتتاحیه دفتر کل")?.max(0),
                total_debit,
                total_credit,
                closing_debit: closing.max(0),
                closing_credit: checked_neg(closing, "مانده بستانکار پایان دوره دفتر کل")?.max(0),
                rows,
            })
        })
    }
}
