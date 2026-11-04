//! Copying or moving persisted voucher rows between two vouchers.

use crate::db::{self, Database};
use crate::vouchers::model::{
    VoucherLineInput, VoucherTransferInput, VoucherTransferResult, MAX_AMOUNT, MAX_LINES,
    MAX_VOUCHER_NUMBER, TOUCHED_AT,
};
use crate::vouchers::query::fetch_voucher_by_id;
use crate::vouchers::validation::{
    assert_voucher_type_matches_lines, checked_add, checked_sum, validate_voucher_line_accounts,
};
use rusqlite::{params, OptionalExtension};
use std::collections::HashSet;

impl Database {
    pub fn transfer_voucher_lines(
        &self,
        input: VoucherTransferInput,
    ) -> Result<VoucherTransferResult, String> {
        if input.source_voucher_id <= 0 || input.source_voucher_number <= 0 {
            return Err("سند مبدأ معتبر نیست.".to_string());
        }
        if input.target_voucher_number <= 0 || input.target_voucher_number > MAX_VOUCHER_NUMBER {
            return Err(format!(
                "شماره سند مقصد باید بین ۱ و {} باشد.",
                db::to_fa_digits(MAX_VOUCHER_NUMBER.to_string())
            ));
        }
        let mode = input.mode.trim().to_lowercase();
        if mode != "copy" && mode != "move" {
            return Err("نوع عملیات کپی یا انتقال معتبر نیست.".to_string());
        }
        if input.source_voucher_number == input.target_voucher_number {
            return Err("شماره سند مقصد باید با سند مبدأ متفاوت باشد.".to_string());
        }
        if input.line_ids.is_empty() || input.line_ids.len() > MAX_LINES {
            return Err("حداقل یک ردیف معتبر برای عملیات انتخاب کنید.".to_string());
        }
        let mut unique_ids = HashSet::new();
        for id in &input.line_ids {
            if *id <= 0 || !unique_ids.insert(*id) {
                return Err("شناسه ردیف‌های انتخابی نامعتبر یا تکراری است.".to_string());
            }
        }
        let source_id = input.source_voucher_id;
        let source_number = input.source_voucher_number;
        let target_number = input.target_voucher_number;
        let line_ids = input.line_ids;
        let is_move = mode == "move";

        self.with_conn(move |conn| {
            let tx = conn.transaction().map_err(db::db_error)?;
            let (source_date, source_type) = tx
                .query_row(
                    "SELECT date, type FROM vouchers WHERE id = ?1 AND number = ?2",
                    params![source_id, source_number],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(db::db_error)?
                .ok_or_else(|| "سند مبدأ پس از بازخوانی پیدا نشد؛ فرم را تازه‌سازی کنید.".to_string())?;

            let source_lines = {
                let mut stmt = tx.prepare(
                    "SELECT id, row_no, account_code, NULLIF(detail1_code,''), description, debit, credit
                       FROM voucher_lines WHERE voucher_id = ?1 ORDER BY row_no, id"
                ).map_err(db::db_error)?;
                let rows = stmt.query_map(params![source_id], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        VoucherLineInput {
                            id: None,
                            account_code: row.get(2)?,
                            detail1_code: row.get(3)?,
                            description: row.get(4)?,
                            debit: row.get(5)?,
                            credit: row.get(6)?,
                        },
                    ))
                }).map_err(db::db_error)?
                  .collect::<rusqlite::Result<Vec<_>>>()
                  .map_err(db::db_error)?;
                rows
            };

            let requested: HashSet<i64> = line_ids.iter().copied().collect();
            let selected: Vec<_> = source_lines.iter()
                .filter(|(id, _, _)| requested.contains(id))
                .cloned()
                .collect();
            if selected.len() != line_ids.len() {
                return Err("برخی از ردیف‌های انتخابی دیگر در سند مبدأ وجود ندارند؛ سند را دوباره باز کنید.".to_string());
            }
            if selected.len() < 2 {
                return Err("برای کپی یا انتقال، حداقل دو ردیف لازم است.".to_string());
            }

            for (_, row_no, line) in &selected {
                if line.debit < 0 || line.credit < 0
                    || line.debit > MAX_AMOUNT || line.credit > MAX_AMOUNT
                    || (line.debit == 0 && line.credit == 0)
                    || (line.debit > 0 && line.credit > 0) {
                    return Err(format!(
                        "مبالغ ردیف {} معتبر نیست؛ ابتدا سند مبدأ را اصلاح کنید.",
                        db::to_fa_digits(*row_no)
                    ));
                }
                validate_voucher_line_accounts(&tx, None, std::slice::from_ref(line))
                    .map_err(|_| format!(
                        "حساب پایه یا تفصیلی ردیف {} دیگر معتبر/فعال نیست؛ انتقال متوقف شد.",
                        db::to_fa_digits(*row_no)
                    ))?;
            }

            let selected_debit = checked_sum(
                selected.iter().map(|(_, _, line)| line.debit),
                "جمع بدهکار ردیف‌های انتخابی",
            )?;
            let selected_credit = checked_sum(
                selected.iter().map(|(_, _, line)| line.credit),
                "جمع بستانکار ردیف‌های انتخابی",
            )?;
            if selected_debit <= 0 || selected_debit != selected_credit {
                return Err("مجموع ردیف‌های انتخابی تراز نیست؛ ردیف‌های بدهکار و بستانکار متناظر را با هم انتخاب کنید.".to_string());
            }

            if is_move {
                let remaining: Vec<_> = source_lines.iter()
                    .filter(|(id, _, _)| !requested.contains(id))
                    .collect();
                if remaining.len() < 2 {
                    return Err("انتقال انتخاب‌شده باعث می‌شود سند مبدأ کمتر از دو ردیف داشته باشد؛ عملیات انجام نشد.".to_string());
                }
                let remaining_debit = checked_sum(
                    remaining.iter().map(|(_, _, line)| line.debit),
                    "جمع بدهکار باقی‌مانده سند مبدأ",
                )?;
                let remaining_credit = checked_sum(
                    remaining.iter().map(|(_, _, line)| line.credit),
                    "جمع بستانکار باقی‌مانده سند مبدأ",
                )?;
                if remaining_debit <= 0 || remaining_debit != remaining_credit {
                    return Err("با انتقال این ردیف‌ها سند مبدأ ناتراز می‌شود؛ ردیف‌های متناظر را نیز انتخاب کنید یا از حالت کپی استفاده کنید.".to_string());
                }
            }

            let existing_target = tx
                .query_row(
                    "SELECT id, type FROM vouchers WHERE number = ?1",
                    params![target_number],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(db::db_error)?;
            if existing_target.as_ref().map(|(id, _)| *id) == Some(source_id) {
                return Err("سند مقصد نمی‌تواند همان سند مبدأ باشد.".to_string());
            }
            let target_type = existing_target
                .as_ref()
                .map(|(_, kind)| kind.clone())
                .unwrap_or_else(|| source_type.clone());

            // The selected lines join the target voucher, so they must agree
            // with the target's type even when they came from another type.
            let selected_lines: Vec<VoucherLineInput> = selected
                .iter()
                .map(|(_, _, line)| line.clone())
                .collect();
            assert_voucher_type_matches_lines(&tx, &target_type, &selected_lines)?;

            let (target_id, target_count, target_debit, target_credit, created_target) =
                if let Some((id, _)) = existing_target {
                    let totals = tx.query_row(
                        "SELECT COUNT(*), COALESCE(SUM(debit), 0), COALESCE(SUM(credit), 0)
                           FROM voucher_lines WHERE voucher_id = ?1",
                        params![id],
                        |row| Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                        )),
                    ).map_err(db::db_error)?;
                    (id, totals.0, totals.1, totals.2, false)
                } else {
                    tx.execute(
                        "INSERT INTO vouchers (number, date, type, description)
                         VALUES (?1, ?2, ?3, '')",
                        params![target_number, source_date, source_type],
                    ).map_err(db::db_error)?;
                    (tx.last_insert_rowid(), 0, 0, 0, true)
                };

            let new_target_count = target_count.checked_add(selected.len() as i64)
                .ok_or_else(|| "تعداد ردیف‌های مقصد از محدوده مجاز بیشتر است.".to_string())?;
            if new_target_count > MAX_LINES as i64 {
                return Err(format!("سند مقصد پس از عملیات بیش از {} ردیف خواهد داشت.", db::to_fa_digits(MAX_LINES)));
            }
            if new_target_count < 2 {
                return Err("سند مقصد باید حداقل دو ردیف داشته باشد.".to_string());
            }
            let new_target_debit = checked_add(target_debit, selected_debit, "جمع بدهکار سند مقصد")?;
            let new_target_credit = checked_add(target_credit, selected_credit, "جمع بستانکار سند مقصد")?;
            if new_target_debit <= 0 || new_target_debit != new_target_credit {
                return Err("افزودن ردیف‌های انتخابی باعث ناترازی سند مقصد می‌شود؛ ردیف‌های بدهکار و بستانکار متناظر را انتخاب کنید.".to_string());
            }

            let mut next_row: i64 = tx.query_row(
                "SELECT COALESCE(MAX(row_no), 0) FROM voucher_lines WHERE voucher_id = ?1",
                params![target_id],
                |row| row.get(0),
            ).map_err(db::db_error)?;
            for (_, _, line) in &selected {
                next_row = next_row.checked_add(1)
                    .ok_or_else(|| "شماره ردیف‌های سند مقصد از محدوده مجاز بیشتر است.".to_string())?;
                tx.execute(
                    "INSERT INTO voucher_lines (voucher_id, row_no, account_id, account_code, detail1_code, description, debit, credit)
                     SELECT ?1, ?2, a.id, ?3, ?4, ?5, ?6, ?7 FROM accounts a WHERE a.code = ?3",
                    params![target_id, next_row, &line.account_code, &line.detail1_code, &line.description, line.debit, line.credit],
                ).map_err(db::db_error)?;
            }

            if is_move {
                for id in &line_ids {
                    let changed = tx.execute(
                        "DELETE FROM voucher_lines WHERE id = ?1 AND voucher_id = ?2",
                        params![id, source_id],
                    ).map_err(db::db_error)?;
                    if changed != 1 {
                        return Err("یکی از ردیف‌های سند مبدأ هم‌زمان تغییر کرده است؛ عملیات لغو شد.".to_string());
                    }
                }
                tx.execute(
                    &format!("UPDATE vouchers SET updated_at = {TOUCHED_AT} WHERE id = ?1"),
                    params![source_id],
                ).map_err(db::db_error)?;
            }
            tx.execute(
                &format!("UPDATE vouchers SET updated_at = {TOUCHED_AT} WHERE id = ?1"),
                params![target_id],
            ).map_err(db::db_error)?;

            tx.commit().map_err(db::db_error)?;
            let target = fetch_voucher_by_id(conn, target_id)?;
            let source = if is_move { Some(fetch_voucher_by_id(conn, source_id)?) } else { None };
            Ok(VoucherTransferResult { mode, created_target, target, source })
        })
    }
}
