//! Voucher persistence: paging, reading, saving, deleting and numbering.

use crate::db::{self, Database};
use crate::vouchers::model::{
    Voucher, VoucherInput, VoucherLineInput, VoucherList, MAX_AMOUNT, MAX_DESCRIPTION, MAX_LINES,
    MAX_LINE_DESCRIPTION, MAX_VOUCHER_NUMBER, TOUCHED_AT,
};
use crate::vouchers::query::{
    count, fetch_voucher_by_id, fetch_voucher_by_number, list_vouchers, page_window, search_terms,
    voucher_list_totals,
};
use crate::vouchers::validation::{
    assert_voucher_type_matches_lines, check_range, format_amount, normalize_date,
    normalize_filter_date, validate_description, normalize_voucher_line_accounts, validate_voucher_line_accounts,
    validate_voucher_type,
};
use rusqlite::{params, OptionalExtension};

impl Database {
    pub fn next_voucher_number(&self) -> Result<i64, String> {
        self.with_conn(|conn| {
            let maximum: i64 = conn
                .query_row("SELECT COALESCE(MAX(number), 0) FROM vouchers", [], |row| {
                    row.get(0)
                })
                .map_err(db::db_error)?;
            let next = maximum
                .checked_add(1)
                .ok_or_else(|| "شماره سند بعدی از محدوده مجاز خارج است.".to_string())?;
            if next > MAX_VOUCHER_NUMBER {
                return Err(format!(
                    "شماره سند بعدی از حداکثر {} بیشتر است؛ شماره سند جدیدی ثبت نمی‌شود.",
                    db::to_fa_digits(MAX_VOUCHER_NUMBER.to_string())
                ));
            }
            Ok(next)
        })
    }

    pub fn adjacent_voucher_number(
        &self,
        number: i64,
        direction: &str,
    ) -> Result<Option<i64>, String> {
        let query = match direction {
            "previous" => "SELECT MAX(number) FROM vouchers WHERE number < ?1",
            "next" => "SELECT MIN(number) FROM vouchers WHERE number > ?1",
            _ => return Err("جهت پیمایش سند معتبر نیست.".to_string()),
        };
        self.with_conn(|conn| {
            conn.query_row(query, params![number], |row| row.get(0))
                .map_err(db::db_error)
        })
    }

    pub fn voucher_count(&self) -> Result<i64, String> {
        self.with_conn(|conn| count(conn, "SELECT COUNT(*) FROM vouchers"))
    }

    pub fn list_vouchers(&self, from: &str, to: &str, search: &str) -> Result<VoucherList, String> {
        self.list_vouchers_page(from, to, search, None, None)
    }

    /// Returns one page of the voucher list. Both totals and `matched_count`
    /// are still calculated over every voucher matching the filter, not only
    /// the returned page.
    pub fn list_vouchers_page(
        &self,
        from: &str,
        to: &str,
        search: &str,
        offset: Option<i64>,
        limit: Option<i64>,
    ) -> Result<VoucherList, String> {
        let from = normalize_filter_date(from, "تاریخ شروع")?;
        let to = normalize_filter_date(to, "تاریخ پایان")?;
        check_range(&from, &to)?;
        let terms = search_terms(search);
        let page = page_window(offset, limit)?;

        let (vouchers, total_count, matched_count, total_debit, total_credit) =
            self.with_conn(|conn| {
                let vouchers = list_vouchers(conn, &from, &to, &terms, page)?;
                let total = count(conn, "SELECT COUNT(*) FROM vouchers")?;
                let (matched, debit, credit) = voucher_list_totals(conn, &from, &to, &terms)?;
                Ok((vouchers, total, matched, debit, credit))
            })?;
        Ok(VoucherList {
            vouchers,
            total_count,
            matched_count,
            total_debit,
            total_credit,
            database_path: self.path_string(),
        })
    }

    pub fn get_voucher(&self, number: i64) -> Result<Option<Voucher>, String> {
        if number <= 0 {
            return Ok(None);
        }
        self.with_conn(|conn| fetch_voucher_by_number(conn, number))
    }

    pub fn save_voucher(&self, input: VoucherInput) -> Result<Voucher, String> {
        self.save_voucher_inner(input, None, None)
    }

    pub fn save_voucher_if_unchanged(
        &self,
        input: VoucherInput,
        expected_updated_at: Option<&str>,
    ) -> Result<Voucher, String> {
        if input.id.is_some() && expected_updated_at.is_none() {
            return Err("نسخه سند مشخص نیست؛ سند را دوباره بارگذاری کنید.".to_string());
        }
        self.save_voucher_inner(input, expected_updated_at.map(str::to_string), None)
    }

    /// Saves a voucher with an optional durable idempotency key. A retry with
    /// the same key and same payload replays the original voucher; key reuse
    /// with a different payload is rejected.
    pub fn save_voucher_idempotent(
        &self,
        input: VoucherInput,
        expected_updated_at: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<Voucher, String> {
        if input.id.is_some() && expected_updated_at.is_none() {
            return Err("نسخه سند مشخص نیست؛ سند را دوباره بارگذاری کنید.".to_string());
        }

        let request = if let Some(key) = idempotency_key {
            let key = key.trim();
            if key.is_empty() || key.len() > 128 || key.chars().any(char::is_control) {
                return Err("کلید یکتای ذخیره معتبر نیست.".to_string());
            }
            let signature = serde_json::to_string(&(input.clone(), expected_updated_at))
                .map_err(|_| "ساخت امضای درخواست ذخیره ناموفق بود.".to_string())?;
            Some((key.to_string(), signature))
        } else {
            None
        };

        self.save_voucher_inner(input, expected_updated_at.map(str::to_string), request)
    }

    fn save_voucher_inner(
        &self,
        input: VoucherInput,
        expected_updated_at: Option<String>,
        idempotency: Option<(String, String)>,
    ) -> Result<Voucher, String> {
        let date = normalize_date(&input.date, "تاریخ سند")?;
        let voucher_type = validate_voucher_type(&input.voucher_type)?;
        let description = validate_description(&input.description, "شرح سند", MAX_DESCRIPTION)?;

        if input.lines.len() > MAX_LINES {
            return Err(format!(
                "تعداد ردیف‌های سند نمی‌تواند بیشتر از {} باشد.",
                db::to_fa_digits(MAX_LINES)
            ));
        }

        // Drop rows the user left entirely blank, then validate what remains.
        let mut lines: Vec<VoucherLineInput> = Vec::new();
        for line in input.lines {
            let account_code = db::normalize_code(&line.account_code);
            let line_description =
                validate_description(&line.description, "شرح ردیف", MAX_LINE_DESCRIPTION)?;
            let detail1_code = line.detail1_code.as_deref().map(db::normalize_code).filter(|v| !v.is_empty());
            let blank = account_code.is_empty()
                && detail1_code.is_none()
                && line_description.is_empty()
                && line.debit == 0
                && line.credit == 0;
            if blank {
                continue;
            }
            if account_code.is_empty() {
                return Err("برای هر ردیف، کد حساب الزامی است.".to_string());
            }
            if line.debit < 0 || line.credit < 0 {
                return Err("مبلغ بدهکار یا بستانکار نمی‌تواند منفی باشد.".to_string());
            }
            if line.debit > MAX_AMOUNT || line.credit > MAX_AMOUNT {
                return Err(format!(
                    "مبلغ هر ردیف نمی‌تواند بیشتر از {} ریال باشد.",
                    db::to_fa_digits(format_amount(MAX_AMOUNT))
                ));
            }
            if line.debit == 0 && line.credit == 0 {
                return Err("برای هر ردیف، مبلغ بدهکار یا بستانکار الزامی است.".to_string());
            }
            if line.debit > 0 && line.credit > 0 {
                return Err("در یک ردیف فقط یکی از مبالغ بدهکار یا بستانکار ثبت می‌شود.".to_string());
            }
            lines.push(VoucherLineInput {
                id: line.id,
                account_code,
                detail1_code,
                description: line_description,
                debit: line.debit,
                credit: line.credit,
            });
        }

        if lines.len() < 2 {
            return Err("سند حسابداری باید حداقل دو ردیف داشته باشد.".to_string());
        }

        // Each line is capped and the line count is capped, so these sums fit
        // in an i64; `checked_add` keeps that guarantee explicit.
        let sum = |pick: fn(&VoucherLineInput) -> i64| -> Result<i64, String> {
            lines.iter().try_fold(0i64, |acc, l| {
                acc.checked_add(pick(l))
                    .ok_or_else(|| "جمع مبالغ سند بیش از حد مجاز است.".to_string())
            })
        };
        let total_debit = sum(|l| l.debit)?;
        let total_credit = sum(|l| l.credit)?;
        if total_debit == 0 {
            return Err("مبلغ سند باید بزرگ‌تر از صفر باشد.".to_string());
        }
        if total_debit != total_credit {
            return Err(format!(
                "سند تراز نیست: جمع بدهکار {} و جمع بستانکار {} است.",
                db::to_fa_digits(format_amount(total_debit)),
                db::to_fa_digits(format_amount(total_credit))
            ));
        }

        if input.id.is_some_and(|id| id <= 0) {
            return Err("شناسه سند نامعتبر است.".to_string());
        }
        let id = input.id.unwrap_or(0);
        let requested_number = input.number.unwrap_or(0);
        if requested_number < 0 {
            return Err("شماره سند نمی‌تواند منفی باشد.".to_string());
        }

        self.with_conn(move |conn| {
            let tx = conn.transaction().map_err(db::db_error)?;

            if let Some((key, signature)) = &idempotency {
                let previous = tx
                    .query_row(
                        "SELECT payload_signature, voucher_id
                           FROM voucher_save_idempotency
                          WHERE idempotency_key = ?1",
                        params![key],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .optional()
                    .map_err(db::db_error)?;
                if let Some((previous_signature, voucher_id)) = previous {
                    if previous_signature != *signature {
                        return Err("کلید یکتای ذخیره قبلاً برای محتوای دیگری استفاده شده است.".to_string());
                    }
                    return fetch_voucher_by_id(&tx, voucher_id);
                }
            }

            // Convert legacy level-3-only payloads to the canonical pair, then
            // validate both the base account and its optional detail.
            normalize_voucher_line_accounts(&tx, &mut lines)?;
            // A saved inactive account is allowed only on its original row id
            // and unchanged base/detail pair.
            validate_voucher_line_accounts(&tx, (id > 0).then_some(id), &lines)?;
            assert_voucher_type_matches_lines(&tx, &voucher_type, &lines)?;

            let existing = if id > 0 {
                tx.query_row(
                    "SELECT number, updated_at, is_temporary FROM vouchers WHERE id = ?1",
                    params![id],
                    |row| Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)? != 0,
                    )),
                )
                .optional()
                .map_err(db::db_error)?
            } else {
                None
            };
            if id > 0 && existing.is_none() {
                return Err("سند حسابداری یافت نشد.".to_string());
            }
            if existing.as_ref().is_some_and(|(_, _, is_temporary)| !*is_temporary) {
                return Err("این سند دائمی است و قابل ویرایش نیست.".to_string());
            }
            if let (Some(expected), Some((_, current, _))) =
                (expected_updated_at.as_deref(), existing.as_ref())
            {
                if expected != current {
                    return Err(
                        "سند پس از باز شدن تغییر کرده است؛ دوباره بارگذاری کنید.".to_string()
                    );
                }
            }

            let existing_number = existing.as_ref().map(|(number, _, _)| *number);
            let number = match (requested_number, existing_number) {
                (number, _) if number > 0 => number,
                (_, Some(current_number)) => current_number,
                _ => tx
                    .query_row("SELECT COALESCE(MAX(number), 0) FROM vouchers", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .map_err(db::db_error)?
                    .checked_add(1)
                    .ok_or_else(|| "شماره سند بعدی از محدوده مجاز خارج است.".to_string())?,
            };
            if number <= 0 || number > MAX_VOUCHER_NUMBER {
                return Err(format!(
                    "شماره سند باید عدد صحیح و حداکثر {} باشد.",
                    db::to_fa_digits(MAX_VOUCHER_NUMBER.to_string())
                ));
            }

            let duplicate = tx
                .query_row(
                    "SELECT id FROM vouchers WHERE number = ?1 AND id <> ?2",
                    params![number, if id > 0 { id } else { 0 }],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(db::db_error)?;
            if duplicate.is_some() {
                return Err("شماره سند تکراری است؛ شماره دیگری انتخاب کنید.".to_string());
            }

            let voucher_id = if id > 0 {
                let updated = if let Some(expected) = expected_updated_at.as_deref() {
                    tx.execute(
                        &format!(
                            "UPDATE vouchers
                                SET number = ?1, date = ?2, type = ?3, description = ?4,
                                    updated_at = {TOUCHED_AT}
                              WHERE id = ?5 AND updated_at = ?6"
                        ),
                        params![number, date, voucher_type, description, id, expected],
                    )
                } else {
                    tx.execute(
                        &format!(
                            "UPDATE vouchers
                                SET number = ?1, date = ?2, type = ?3, description = ?4,
                                    updated_at = {TOUCHED_AT}
                              WHERE id = ?5"
                        ),
                        params![number, date, voucher_type, description, id],
                    )
                }
                .map_err(db::db_error)?;
                if updated == 0 && expected_updated_at.is_some() {
                    return Err(
                        "سند پس از باز شدن تغییر کرده است؛ دوباره بارگذاری کنید.".to_string()
                    );
                }
                tx.execute(
                    "DELETE FROM voucher_lines WHERE voucher_id = ?1",
                    params![id],
                )
                .map_err(db::db_error)?;
                id
            } else {
                tx.execute(
                    "INSERT INTO vouchers (number, date, type, description)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![number, date, voucher_type, description],
                )
                .map_err(db::db_error)?;
                tx.last_insert_rowid()
            };

            for (index, line) in lines.iter().enumerate() {
                let account_id: i64 = tx.query_row(
                    "SELECT id FROM accounts WHERE code = ?1", params![line.account_code], |row| row.get(0)
                ).map_err(db::db_error)?;
                tx.execute(
                    "INSERT INTO voucher_lines
                        (voucher_id, row_no, account_id, account_code, detail1_code, description, debit, credit)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        voucher_id, index as i64 + 1, account_id, line.account_code,
                        line.detail1_code, line.description, line.debit, line.credit
                    ],
                )
                .map_err(db::db_error)?;
            }

            if let Some((key, signature)) = &idempotency {
                tx.execute(
                    "DELETE FROM voucher_save_idempotency
                      WHERE created_at < datetime('now', 'localtime', '-30 days')",
                    [],
                ).map_err(db::db_error)?;
                tx.execute(
                    "INSERT INTO voucher_save_idempotency
                        (idempotency_key, payload_signature, voucher_id)
                     VALUES (?1, ?2, ?3)",
                    params![key, signature, voucher_id],
                ).map_err(db::db_error)?;
            }

            tx.commit().map_err(db::db_error)?;
            fetch_voucher_by_id(conn, voucher_id)
        })
    }

    /// Permanently finalizes a saved temporary voucher. The operation is
    /// one-way and uses the same optimistic concurrency token as regular saves.
    pub fn set_voucher_permanent(
        &self,
        id: i64,
        expected_updated_at: Option<&str>,
    ) -> Result<Voucher, String> {
        if id <= 0 {
            return Err("شناسه سند نامعتبر است.".to_string());
        }
        let expected = expected_updated_at
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "نسخه سند مشخص نیست؛ سند را دوباره بارگذاری کنید.".to_string())?
            .to_string();

        self.with_conn(move |conn| {
            let tx = conn.transaction().map_err(db::db_error)?;
            let current = tx
                .query_row(
                    "SELECT updated_at, is_temporary FROM vouchers WHERE id = ?1",
                    params![id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? != 0)),
                )
                .optional()
                .map_err(db::db_error)?
                .ok_or_else(|| "سند حسابداری یافت نشد.".to_string())?;

            if !current.1 {
                return Err("این سند قبلاً دائمی شده است.".to_string());
            }
            if current.0 != expected {
                return Err("سند پس از باز شدن تغییر کرده است؛ دوباره بارگذاری کنید.".to_string());
            }

            let affected = tx
                .execute(
                    &format!(
                        "UPDATE vouchers SET is_temporary = 0, updated_at = {TOUCHED_AT}
                         WHERE id = ?1 AND updated_at = ?2 AND is_temporary = 1"
                    ),
                    params![id, expected],
                )
                .map_err(db::db_error)?;
            if affected != 1 {
                return Err("دائمی‌کردن سند انجام نشد؛ سند را دوباره بارگذاری کنید.".to_string());
            }

            tx.commit().map_err(db::db_error)?;
            fetch_voucher_by_id(conn, id)
        })
    }

    pub fn delete_voucher(&self, id: i64, expected_updated_at: Option<&str>) -> Result<(), String> {
        if id <= 0 {
            return Err("شناسه سند نامعتبر است.".to_string());
        }
        let expected = expected_updated_at
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "نسخه سند مشخص نیست؛ سند را دوباره بارگذاری کنید.".to_string())?
            .to_string();
        self.with_conn(move |conn| {
            let current = conn
                .query_row(
                    "SELECT updated_at, is_temporary FROM vouchers WHERE id = ?1",
                    params![id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? != 0)),
                )
                .optional()
                .map_err(db::db_error)?;
            let Some((current, is_temporary)) = current else {
                return Err("سند حسابداری یافت نشد.".to_string());
            };
            if !is_temporary {
                return Err("این سند دائمی است و قابل حذف نیست.".to_string());
            }
            if current != expected {
                return Err(
                    "این سند پس از باز شدن تغییر کرده است؛ فهرست یا سند را دوباره بارگذاری کنید و حذف را تکرار کنید."
                        .to_string(),
                );
            }
            let affected = conn
                .execute(
                    "DELETE FROM vouchers WHERE id = ?1 AND updated_at = ?2",
                    params![id, expected],
                )
                .map_err(db::db_error)?;
            if affected == 0 {
                return Err(
                    "این سند پس از باز شدن تغییر کرده است؛ فهرست یا سند را دوباره بارگذاری کنید و حذف را تکرار کنید."
                        .to_string(),
                );
            }
            Ok(())
        })
    }
}
