//! Bulk voucher import from parsed Excel workbooks.

use crate::db::{self, audit, Database};
use crate::vouchers::model::{
    VoucherLineInput, MAX_AMOUNT, MAX_DESCRIPTION, MAX_LINES, MAX_LINE_DESCRIPTION,
    MAX_VOUCHER_NUMBER,
};
use crate::vouchers::storage::voucher_snapshot;
use crate::vouchers::validation::{
    assert_voucher_type_matches_lines, checked_sum, normalize_date,
    normalize_voucher_line_accounts, validate_description, validate_voucher_line_accounts,
    validate_voucher_type,
};
use rusqlite::{params, OptionalExtension};
use std::collections::HashSet;

/// Deterministic FNV-1a fingerprint of the parsed workbook. It identifies a
/// retry of the same import without adding a hashing dependency.
struct PayloadHasher {
    state: u64,
}

impl PayloadHasher {
    fn new() -> Self {
        Self {
            state: 0xcbf2_9ce4_8422_2325,
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.state ^= u64::from(*byte);
            self.state = self.state.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn text(&mut self, value: &str) {
        self.write(value.as_bytes());
        self.write(&[0]);
    }

    fn number(&mut self, value: i64) {
        self.text(&value.to_string());
    }

    fn flag(&mut self, value: bool) {
        self.write(&[u8::from(value)]);
    }

    fn finish(self) -> String {
        format!("{:016x}", self.state)
    }
}

fn import_payload_hash(import: &crate::excel::BulkVoucherImport) -> String {
    let mut hasher = PayloadHasher::new();
    hasher.text(&import.file_name);
    hasher.text(&import.sheet_name);
    for account in &import.accounts {
        hasher.text(&account.code);
        hasher.text(&account.name);
        hasher.text(&account.parent_code);
        hasher.text(&account.group_code);
        hasher.number(account.level);
        hasher.text(&account.nature);
        hasher.flag(account.nature_is_explicit);
        hasher.flag(account.group_is_inferred);
    }
    for voucher in &import.vouchers {
        hasher.number(voucher.id.unwrap_or(0));
        hasher.number(voucher.number.unwrap_or(0));
        hasher.text(&voucher.date);
        hasher.text(&voucher.voucher_type);
        hasher.text(&voucher.description);
        for line in &voucher.lines {
            hasher.number(line.id.unwrap_or(0));
            hasher.text(&line.account_code);
            hasher.text(line.detail1_code.as_deref().unwrap_or(""));
            hasher.text(&line.description);
            hasher.number(line.debit);
            hasher.number(line.credit);
        }
    }
    hasher.finish()
}

impl Database {
    pub fn preview_bulk_import(
        &self,
        import: &crate::excel::BulkVoucherImport,
    ) -> Result<crate::excel::BulkVoucherPreview, String> {
        let file_name = import.file_name.clone();
        let sheet_name = import.sheet_name.clone();
        let account_rows = import.accounts.clone();
        let account_codes: HashSet<String> = account_rows
            .iter()
            .map(|account| db::normalize_code(&account.code))
            .collect();
        let referenced_codes: HashSet<String> = import
            .vouchers
            .iter()
            .flat_map(|voucher| voucher.lines.iter())
            .flat_map(|line| {
                [
                    db::normalize_code(&line.account_code),
                    line.detail1_code
                        .as_deref()
                        .map(db::normalize_code)
                        .unwrap_or_default(),
                ]
            })
            .filter(|code| !code.is_empty())
            .collect();
        let numbers: Vec<i64> = import
            .vouchers
            .iter()
            .filter_map(|voucher| voucher.number)
            .collect();
        let voucher_count = import.vouchers.len();
        let line_count = import
            .vouchers
            .iter()
            .map(|voucher| voucher.lines.len())
            .sum();
        let account_count = account_codes.len();
        // Accounts whose group was guessed from the code prefix, not read from
        // the workbook, are reported to the user before import and recorded in
        // the audit entry afterwards.
        let inferred_group_mappings: Vec<crate::excel::InferredGroupMapping> = account_rows
            .iter()
            .filter(|account| account.group_is_inferred)
            .map(|account| crate::excel::InferredGroupMapping {
                account_code: db::normalize_code(&account.code),
                account_name: account.name.clone(),
                level: account.level,
                group_code: db::normalize_code(&account.group_code),
            })
            .collect();
        let inferred_group_count = inferred_group_mappings.len();

        self.with_conn(move |conn| {
            // Validate all groups and existing account structures during preview,
            // not only after the user presses the final import button.
            let mut missing_groups: std::collections::BTreeMap<String, Vec<String>> =
                std::collections::BTreeMap::new();
            for account in &account_rows {
                let group_code = db::normalize_code(&account.group_code);
                let exists = conn
                    .query_row(
                        "SELECT 1 FROM account_groups WHERE code = ?1",
                        params![group_code],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(db::db_error)?
                    .is_some();
                if !exists {
                    missing_groups
                        .entry(group_code)
                        .or_default()
                        .push(account.code.clone());
                }
            }
            if !missing_groups.is_empty() {
                let detail = missing_groups
                    .iter()
                    .map(|(group, codes)| {
                        let shown = codes
                            .iter()
                            .take(5)
                            .map(db::to_fa_digits)
                            .collect::<Vec<_>>()
                            .join("، ");
                        let more = if codes.len() > 5 {
                            format!(" و {} حساب دیگر", db::to_fa_digits(codes.len() - 5))
                        } else {
                            String::new()
                        };
                        format!("گروه «{}» برای حساب‌های {shown}{more}", db::to_fa_digits(group))
                    })
                    .collect::<Vec<_>>()
                    .join(" | ");
                return Err(format!(
                    "پیش از ورود اسناد، گروه حساب‌های نامعتبر را اصلاح کنید: {detail}"
                ));
            }

            let mut new_account_count = 0;
            for account in &account_rows {
                let code = db::normalize_code(&account.code);
                let group_code = db::normalize_code(&account.group_code);
                let parent_code = db::normalize_code(&account.parent_code);
                let existing = conn
                    .query_row(
                        "SELECT level, group_code, COALESCE(parent_code, ''), nature
                           FROM accounts WHERE code = ?1",
                        params![code],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(db::db_error)?;

                if let Some((level, existing_group, existing_parent, existing_nature)) = existing {
                    if level != account.level
                        || existing_group != group_code
                        || existing_parent != parent_code
                    {
                        return Err(format!(
                            "حساب {} از قبل با گروه، سطح یا حساب بالاتر متفاوت ثبت شده است.",
                            db::to_fa_digits(&code)
                        ));
                    }
                    if account.nature_is_explicit && existing_nature != account.nature {
                        return Err(format!(
                            "ماهیت حساب {} با مقدار واردشده در فایل اکسل متفاوت است.",
                            db::to_fa_digits(&code)
                        ));
                    }
                } else {
                    if !matches!(account.nature.as_str(), "debtor" | "creditor" | "dual") {
                        return Err(format!(
                            "ماهیت حساب جدید {} مشخص نیست؛ برای گروه‌های سایر درآمدها و هزینه‌ها، ستون ماهیت مربوط به حساب را با مقدار بدهکار، بستانکار یا دوگانه تکمیل کنید.",
                            db::to_fa_digits(&code)
                        ));
                    }
                    new_account_count += 1;
                    if account.level > 1 && !parent_code.is_empty() {
                        let parent = conn
                            .query_row(
                                "SELECT level, group_code, is_active FROM accounts WHERE code = ?1",
                                params![parent_code],
                                |row| {
                                    Ok((
                                        row.get::<_, i64>(0)?,
                                        row.get::<_, String>(1)?,
                                        row.get::<_, i64>(2)?,
                                    ))
                                },
                            )
                            .optional()
                            .map_err(db::db_error)?;
                        if let Some((parent_level, parent_group, parent_is_active)) = parent {
                            if parent_level != account.level - 1 || parent_group != group_code {
                                return Err(format!(
                                    "گروه یا سطح حساب بالاتر {} با حساب {} سازگار نیست.",
                                    db::to_fa_digits(&parent_code),
                                    db::to_fa_digits(&code)
                                ));
                            }
                            if parent_is_active == 0 {
                                return Err(format!(
                                    "حساب بالاتر {} غیرفعال است؛ ابتدا آن را فعال کنید.",
                                    db::to_fa_digits(&parent_code)
                                ));
                            }
                        } else if !account_codes.contains(&parent_code) {
                            return Err(format!(
                                "حساب بالاتر {} برای حساب {} در فایل یا پایگاه داده پیدا نشد.",
                                db::to_fa_digits(&parent_code),
                                db::to_fa_digits(&code)
                            ));
                        }
                    }
                }
            }

            // Base and detail references must each exist in the live chart or
            // be declared in this workbook. Pair compatibility is validated
            // after all missing accounts have been created in the import tx.
            for code in &referenced_codes {
                let exists = conn.query_row(
                    "SELECT 1 FROM accounts WHERE code = ?1", params![code], |_| Ok(())
                ).optional().map_err(db::db_error)?.is_some();
                if !exists && !account_codes.contains(code) {
                    return Err(format!(
                        "حساب ردیف سند با کد {} در فایل یا پایگاه داده پیدا نشد.",
                        db::to_fa_digits(code)
                    ));
                }
            }

            let mut conflicting_voucher_numbers = Vec::new();
            for number in numbers {
                let exists = conn
                    .query_row(
                        "SELECT 1 FROM vouchers WHERE number = ?1",
                        params![number],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(db::db_error)?
                    .is_some();
                if exists {
                    conflicting_voucher_numbers.push(number);
                }
            }
            conflicting_voucher_numbers.sort_unstable();
            conflicting_voucher_numbers.dedup();
            Ok(crate::excel::BulkVoucherPreview {
                file_name,
                sheet_name,
                voucher_count,
                line_count,
                account_count,
                new_account_count,
                number_conflict_count: conflicting_voucher_numbers.len(),
                conflicting_voucher_numbers,
                inferred_group_count,
                inferred_group_mappings,
            })
        })
    }

    pub fn import_vouchers(
        &self,
        import: crate::excel::BulkVoucherImport,
        renumber_conflicts: bool,
    ) -> Result<usize, String> {
        if import.vouchers.is_empty() {
            return Err("فایلی برای ورود وجود ندارد.".to_string());
        }

        let payload_hash = import_payload_hash(&import);
        let file_name = import.file_name.clone();
        let sheet_name = import.sheet_name.clone();

        let mut prepared = Vec::with_capacity(import.vouchers.len());
        let mut numbers = HashSet::new();
        for input in import.vouchers {
            let date = normalize_date(&input.date, "تاریخ سند")?;
            let voucher_type = validate_voucher_type(&input.voucher_type)?;
            let description = validate_description(&input.description, "شرح سند", MAX_DESCRIPTION)?;
            let number = input.number.unwrap_or(0);
            if number <= 0 || number > MAX_VOUCHER_NUMBER || !numbers.insert(number) {
                return Err(format!(
                    "شماره سند {} نامعتبر یا تکراری است.",
                    db::to_fa_digits(number)
                ));
            }
            if input.lines.len() > MAX_LINES {
                return Err(format!(
                    "تعداد ردیف‌های سند {} از حد مجاز بیشتر است.",
                    db::to_fa_digits(number)
                ));
            }

            let mut lines = Vec::with_capacity(input.lines.len());
            for line in input.lines {
                let account_code = db::normalize_code(&line.account_code);
                let line_description =
                    validate_description(&line.description, "شرح ردیف", MAX_LINE_DESCRIPTION)?;
                if account_code.is_empty() {
                    return Err(format!(
                        "کد حساب در سند {} خالی است.",
                        db::to_fa_digits(number)
                    ));
                }
                if line.debit < 0
                    || line.credit < 0
                    || line.debit > MAX_AMOUNT
                    || line.credit > MAX_AMOUNT
                {
                    return Err(format!(
                        "مبلغ سند {} خارج از محدوده مجاز است.",
                        db::to_fa_digits(number)
                    ));
                }
                if (line.debit == 0 && line.credit == 0) || (line.debit > 0 && line.credit > 0) {
                    return Err(format!(
                        "مبلغ ردیف سند {} نامعتبر است.",
                        db::to_fa_digits(number)
                    ));
                }
                lines.push(VoucherLineInput {
                    id: line.id,
                    account_code,
                    detail1_code: line
                        .detail1_code
                        .as_deref()
                        .map(db::normalize_code)
                        .filter(|v| !v.is_empty()),
                    description: line_description,
                    debit: line.debit,
                    credit: line.credit,
                });
            }
            if lines.len() < 2 {
                return Err(format!(
                    "سند {} باید حداقل دو ردیف داشته باشد.",
                    db::to_fa_digits(number)
                ));
            }
            let total_debit = checked_sum(lines.iter().map(|line| line.debit), "جمع بدهکار سند")?;
            let total_credit =
                checked_sum(lines.iter().map(|line| line.credit), "جمع بستانکار سند")?;
            if total_debit == 0 || total_debit != total_credit {
                return Err(format!("سند {} تراز نیست.", db::to_fa_digits(number)));
            }
            prepared.push((number, date, voucher_type, description, lines));
        }

        let mut accounts = import.accounts;
        accounts.sort_by_key(|account| account.level);
        let inferred_group_mappings: Vec<crate::excel::InferredGroupMapping> = accounts
            .iter()
            .filter(|account| account.group_is_inferred)
            .map(|account| crate::excel::InferredGroupMapping {
                account_code: db::normalize_code(&account.code),
                account_name: account.name.clone(),
                level: account.level,
                group_code: db::normalize_code(&account.group_code),
            })
            .collect();
        let inferred_group_count = inferred_group_mappings.len();
        let imported_count = prepared.len();
        self.with_conn(move |conn| {
            let tx = conn.transaction().map_err(db::db_error)?;
            let previous = tx
                .query_row(
                    "SELECT voucher_count FROM import_batches WHERE payload_hash = ?1",
                    params![&payload_hash],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(db::db_error)?;
            if let Some(count) = previous {
                let count = usize::try_from(count)
                    .map_err(|_| "تعداد سندهای دسته ورودی معتبر نیست.".to_string())?;
                return Ok(count);
            }
            let mut imported_numbers = Vec::with_capacity(prepared.len());
            let incoming_numbers: HashSet<i64> = prepared.iter().map(|item| item.0).collect();
            let mut used_numbers = HashSet::new();
            {
                let mut statement = tx.prepare("SELECT number FROM vouchers").map_err(db::db_error)?;
                let rows = statement.query_map([], |row| row.get::<_, i64>(0)).map_err(db::db_error)?;
                for row in rows {
                    used_numbers.insert(row.map_err(db::db_error)?);
                }
            }
            let mut next_number = used_numbers.iter().copied().max().unwrap_or(0);
            for account in accounts {
                let code = db::normalize_code(&account.code);
                let name = account.name.split_whitespace().collect::<Vec<_>>().join(" ");
                let parent_code = db::normalize_code(&account.parent_code);
                let group_code = db::normalize_code(&account.group_code);
                if code.is_empty()
                    || code.len() > 50
                    || !code.chars().all(|ch| ch.is_ascii_digit())
                    || name.is_empty()
                    || name.chars().count() > 255
                    || !(1..=db::MAX_ACCOUNT_LEVEL).contains(&account.level)
                {
                    return Err("اطلاعات یکی از حساب‌های فایل اکسل معتبر نیست.".to_string());
                }
                let existing = tx
                    .query_row(
                        "SELECT level, group_code, COALESCE(parent_code, ''), nature
                           FROM accounts WHERE code = ?1",
                        params![code],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(db::db_error)?;
                if let Some((level, existing_group, existing_parent, existing_nature)) = existing {
                    if level != account.level
                        || existing_group != group_code
                        || existing_parent != parent_code
                    {
                        return Err(format!(
                            "حساب {} از قبل با گروه، سطح یا حساب بالاتر متفاوت ثبت شده است.",
                            db::to_fa_digits(&code)
                        ));
                    }
                    if account.nature_is_explicit && existing_nature != account.nature {
                        return Err(format!(
                            "ماهیت حساب {} با مقدار واردشده در فایل اکسل متفاوت است.",
                            db::to_fa_digits(&code)
                        ));
                    }
                    continue;
                }
                if !matches!(account.nature.as_str(), "debtor" | "creditor" | "dual") {
                    return Err(format!(
                        "ماهیت حساب جدید {} مشخص نیست؛ برای گروه‌های سایر درآمدها و هزینه‌ها، ستون ماهیت مربوط به حساب را تکمیل کنید.",
                        db::to_fa_digits(&code)
                    ));
                }
                let group_exists = tx
                    .query_row("SELECT 1 FROM account_groups WHERE code = ?1", params![group_code], |_| Ok(()))
                    .optional()
                    .map_err(db::db_error)?
                    .is_some();
                if !group_exists {
                    return Err(format!("گروه حساب {} برای حساب {} پیدا نشد.", group_code, code));
                }

                let parent_name = if account.level == 1 {
                    if !parent_code.is_empty() {
                        return Err("حساب کل فایل اکسل نباید حساب بالاتر داشته باشد.".to_string());
                    }
                    String::new()
                } else {
                    if parent_code.is_empty() || code == parent_code {
                        return Err(format!("ساختار حساب {} در فایل اکسل معتبر نیست.", db::to_fa_digits(&code)));
                    }
                    let parent = tx
                        .query_row(
                            "SELECT name, level, group_code, is_active FROM accounts WHERE code = ?1",
                            params![parent_code],
                            |row| {
                                Ok((
                                    row.get::<_, String>(0)?,
                                    row.get::<_, i64>(1)?,
                                    row.get::<_, String>(2)?,
                                    row.get::<_, i64>(3)?,
                                ))
                            },
                        )
                        .optional()
                        .map_err(db::db_error)?
                        .ok_or_else(|| format!("حساب بالاتر {} برای {} پیدا نشد.", db::to_fa_digits(&parent_code), db::to_fa_digits(&code)))?;
                    if parent.1 != account.level - 1 || parent.2 != group_code {
                        return Err(format!("گروه یا سطح حساب بالاتر {} با حساب {} سازگار نیست.", db::to_fa_digits(&parent_code), db::to_fa_digits(&code)));
                    }
                    if parent.3 == 0 {
                        return Err(format!(
                            "حساب بالاتر {} غیرفعال است؛ ابتدا آن را فعال کنید.",
                            db::to_fa_digits(&parent_code)
                        ));
                    }
                    parent.0
                };
                let parent_code_value = if parent_code.is_empty() { None } else { Some(parent_code) };
                tx.execute(
                    "INSERT INTO accounts (code, name, level, group_code, nature, parent_code, parent_name, is_postable) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![code, name, account.level, group_code, account.nature, parent_code_value, parent_name, i64::from(account.level <= 2)],
                )
                .map_err(db::db_error)?;
            }

            for (number, date, voucher_type, description, mut lines) in prepared {
                normalize_voucher_line_accounts(&tx, &mut lines)?;
                validate_voucher_line_accounts(&tx, None, &lines)?;
                assert_voucher_type_matches_lines(&tx, &voucher_type, &lines)?;
                let mut stored_number = number;
                if used_numbers.contains(&number) {
                    if !renumber_conflicts {
                        return Err(format!("شماره سند {} از قبل در برنامه وجود دارد.", db::to_fa_digits(number)));
                    }
                    loop {
                        next_number = next_number.checked_add(1)
                            .ok_or_else(|| "شماره سند بعدی از محدوده عددی مجاز بیشتر است.".to_string())?;
                        if next_number > MAX_VOUCHER_NUMBER {
                            return Err(format!(
                                "شماره سند بعدی از حداکثر {} بیشتر است.",
                                db::to_fa_digits(MAX_VOUCHER_NUMBER.to_string())
                            ));
                        }
                        if !used_numbers.contains(&next_number) && !incoming_numbers.contains(&next_number) {
                            stored_number = next_number;
                            break;
                        }
                    }
                }
                used_numbers.insert(stored_number);
                let fiscal_year_id = db::ensure_open_period(&tx, &date)?;
                tx.execute(
                    "INSERT INTO vouchers (number, date, type, description, fiscal_year_id)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![stored_number, date, voucher_type, description, fiscal_year_id],
                )
                .map_err(db::db_error)?;
                let voucher_id = tx.last_insert_rowid();
                for (index, line) in lines.iter().enumerate() {
                    tx.execute(
                        "INSERT INTO voucher_lines (voucher_id, row_no, account_id, account_code, detail1_code, description, debit, credit)
                         SELECT ?1, ?2, a.id, ?3, ?4, ?5, ?6, ?7 FROM accounts a WHERE a.code = ?3",
                        params![voucher_id, index as i64 + 1, line.account_code, line.detail1_code, line.description, line.debit, line.credit],
                    )
                    .map_err(db::db_error)?;
                }
                imported_numbers.push(stored_number);
                let snapshot = voucher_snapshot(&tx, voucher_id)?;
                audit::log(
                    &tx,
                    "create",
                    "voucher",
                    Some(voucher_id),
                    None,
                    Some(&snapshot.to_string()),
                    "ورود گروهی از فایل اکسل",
                )?;
            }
            let numbers_text = imported_numbers
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            tx.execute(
                "INSERT INTO import_batches
                    (payload_hash, file_name, sheet_name, voucher_count, imported_numbers)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    &payload_hash,
                    &file_name,
                    &sheet_name,
                    imported_count as i64,
                    numbers_text
                ],
            )
            .map_err(db::db_error)?;
            let after = serde_json::json!({
                "voucherCount": imported_count,
                "numbers": imported_numbers,
                "inferredGroupCount": inferred_group_count,
                "inferredGroupMappings": inferred_group_mappings,
            });
            let mut details = format!(
                "ورود گروهی {} سند از فایل «{}»، برگه «{}».",
                db::to_fa_digits(imported_count),
                file_name,
                sheet_name
            );
            if inferred_group_count > 0 {
                details.push_str(&format!(
                    " گروه {} حساب بدون کد گروه در فایل و بر اساس پیشوند کد حدس زده شد؛ جزئیات در فیلد پس از عملیات ثبت شده است.",
                    db::to_fa_digits(inferred_group_count)
                ));
            }
            audit::log(
                &tx,
                "import",
                "voucher",
                None,
                None,
                Some(&after.to_string()),
                &details,
            )?;
            tx.commit().map_err(db::db_error)?;
            Ok(imported_count)
        })
    }
}
