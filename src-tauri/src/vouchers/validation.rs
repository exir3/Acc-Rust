//! Validation and normalization for voucher input.
//!
//! Saving, Excel import and export all use the same account-eligibility
//! checks so they cannot disagree about which postings are allowed.

use crate::db::{self, normalize_digits, Database};
use crate::vouchers::accounting::{classify, AccountClass};
use crate::vouchers::model::{VoucherLineInput, CLOSING_FINAL, CLOSING_TEMPORARY, VOUCHER_TYPES};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{BTreeMap, HashSet};

/* ── Validation helpers ─────────────────────────────────────── */

/// Validates and normalizes a user-entered Jalali date for immediate form feedback.
/// Saving still calls `normalize_date` again, so this command is not a security boundary.
pub fn validate_voucher_date(input: &str) -> Result<String, String> {
    normalize_date(input, "تاریخ سند")
}

pub(crate) fn checked_add(left: i64, right: i64, label: &str) -> Result<i64, String> {
    left.checked_add(right)
        .ok_or_else(|| format!("مقدار محاسبه‌شده در بخش «{label}» از محدوده عددی مجاز بیشتر است."))
}

pub(crate) fn checked_sub(left: i64, right: i64, label: &str) -> Result<i64, String> {
    left.checked_sub(right)
        .ok_or_else(|| format!("مقدار محاسبه‌شده در بخش «{label}» از محدوده عددی مجاز بیشتر است."))
}

pub(crate) fn checked_neg(value: i64, label: &str) -> Result<i64, String> {
    value
        .checked_neg()
        .ok_or_else(|| format!("مقدار محاسبه‌شده در بخش «{label}» از محدوده عددی مجاز بیشتر است."))
}

pub(crate) fn checked_sum(
    values: impl IntoIterator<Item = i64>,
    label: &str,
) -> Result<i64, String> {
    values
        .into_iter()
        .try_fold(0i64, |total, value| checked_add(total, value, label))
}

pub(crate) fn normalize_date(input: &str, label: &str) -> Result<String, String> {
    let text = normalize_digits(input).trim().replace('-', "/");
    if text.is_empty() {
        return Err(format!("{label} الزامی است."));
    }
    let parts: Vec<&str> = text.split('/').collect();
    if parts.len() != 3 {
        return Err(format!("{label} باید به شکل ۱۴۰۵/۰۱/۰۱ وارد شود."));
    }
    let year = parts[0]
        .parse::<i32>()
        .map_err(|_| format!("{label} معتبر نیست."))?;
    let month = parts[1]
        .parse::<u32>()
        .map_err(|_| format!("{label} معتبر نیست."))?;
    let day = parts[2]
        .parse::<u32>()
        .map_err(|_| format!("{label} معتبر نیست."))?;
    // Keep the supported calendar range aligned with WebApp's shared
    // Jalali utilities (years 1 through 3177).
    if !(1..=3177).contains(&year) {
        return Err(format!("{label} خارج از بازه پشتیبانی تقویم شمسی است."));
    }
    jalali_calendar::JalaliDate::new(year, month, day)
        .map_err(|_| format!("{label} یک تاریخ شمسی معتبر نیست."))?;
    Ok(format!("{year:04}/{month:02}/{day:02}"))
}

/// Empty text means "unbounded"; anything else must be a real Jalali date.
pub fn normalize_filter_date(input: &str, label: &str) -> Result<String, String> {
    if input.trim().is_empty() {
        return Ok(String::new());
    }
    normalize_date(input, label)
}

pub(crate) fn check_range(from: &str, to: &str) -> Result<(), String> {
    if !from.is_empty() && !to.is_empty() && from > to {
        return Err("تاریخ شروع نمی‌تواند بعد از تاریخ پایان باشد.".to_string());
    }
    Ok(())
}

pub(crate) fn validate_description(input: &str, label: &str, max: usize) -> Result<String, String> {
    let value = input.trim().to_string();
    if value.chars().count() > max {
        return Err(format!(
            "{label} نمی‌تواند بیشتر از {} کاراکتر باشد.",
            db::to_fa_digits(max)
        ));
    }
    Ok(value)
}

pub(crate) fn validate_voucher_type(input: &str) -> Result<String, String> {
    let value = input.trim();
    if value.is_empty() {
        return Ok(VOUCHER_TYPES[0].to_string());
    }
    if VOUCHER_TYPES.contains(&value) {
        Ok(value.to_string())
    } else {
        Err("نوع سند انتخاب‌شده معتبر نیست.".to_string())
    }
}

pub(crate) fn assert_postable_leaf(
    conn: &Connection,
    account_code: &str,
) -> Result<String, String> {
    let account = conn
        .query_row(
            "SELECT name, level,
                    CASE
                        WHEN level > 3 THEN 0
                        WHEN level = 3 AND EXISTS(
                            SELECT 1 FROM accounts child
                             WHERE child.parent_code = accounts.code AND child.level > 3
                        ) AND NOT EXISTS(
                            SELECT 1 FROM accounts child
                             WHERE child.parent_code = accounts.code AND child.level <= 3
                        ) THEN COALESCE(postable_before_children, 1)
                        ELSE is_postable
                    END,
                    EXISTS(
                        SELECT 1 FROM accounts child
                         WHERE child.parent_code = accounts.code AND child.level <= 3
                    )
               FROM accounts WHERE code = ?1",
            params![account_code],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, bool>(3)?,
                ))
            },
        )
        .optional()
        .map_err(db::db_error)?
        .ok_or_else(|| format!("حساب با کد {} یافت نشد.", db::to_fa_digits(account_code)))?;
    if account.1 > db::MAX_ACCOUNT_LEVEL || account.2 == 0 || account.3 {
        return Err(format!(
            "حساب «{}» قابل ثبت نیست؛ فقط حساب‌های معین و تفصیلی ۱ که زیرحساب فعال ندارند قابل ثبت هستند. تفصیلی ۲ فقط برای سوابق قدیمی نگهداری می‌شود.",
            account.0
        ));
    }
    Ok(account.0)
}

pub(crate) fn assert_active_account(conn: &Connection, account_code: &str) -> Result<(), String> {
    let account = conn
        .query_row(
            "SELECT name, is_active FROM accounts WHERE code = ?1",
            params![account_code],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(db::db_error)?
        .ok_or_else(|| format!("حساب با کد {} یافت نشد.", db::to_fa_digits(account_code)))?;
    if account.1 == 0 {
        return Err(format!(
            "حساب «{}» غیرفعال است و برای ردیف جدید سند قابل انتخاب نیست.",
            account.0
        ));
    }
    Ok(())
}

pub(crate) fn assert_existing_legacy_account(
    conn: &Connection,
    account_code: &str,
) -> Result<String, String> {
    conn.query_row(
        "SELECT name FROM accounts WHERE code = ?1 AND level > ?2
          AND NOT EXISTS (SELECT 1 FROM accounts child WHERE child.parent_code = accounts.code)",
        params![account_code, db::MAX_ACCOUNT_LEVEL],
        |row| row.get(0),
    )
    .optional()
    .map_err(db::db_error)?
    .ok_or_else(|| {
        format!(
            "حساب قدیمی {} دیگر قابل استفاده نیست؛ برای ثبت جدید، معین یا تفصیلی ۱ را انتخاب کنید.",
            db::to_fa_digits(account_code)
        )
    })
}

/// The single source of truth for checking accounts referenced by voucher rows.
/// Historical exceptions are bound to a stable row id, never merely to an
/// account code; this keeps save and Excel export behavior identical.
pub(crate) fn validate_voucher_line_accounts(
    conn: &Connection,
    voucher_id: Option<i64>,
    lines: &[VoucherLineInput],
) -> Result<(), String> {
    let saved_lines: BTreeMap<i64, String> = if let Some(id) = voucher_id.filter(|id| *id > 0) {
        let mut statement = conn
            .prepare("SELECT id, account_code FROM voucher_lines WHERE voucher_id = ?1")
            .map_err(db::db_error)?;
        let rows = statement
            .query_map(params![id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db::db_error)?
            .collect::<rusqlite::Result<BTreeMap<_, _>>>()
            .map_err(db::db_error)?;
        rows
    } else {
        BTreeMap::new()
    };

    let mut used_line_ids = HashSet::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        let account_code = db::normalize_code(&line.account_code);
        if account_code.is_empty() {
            return Err(format!(
                "کد حساب ردیف {} الزامی است.",
                db::to_fa_digits(index + 1)
            ));
        }

        let saved_line = match line.id {
            Some(line_id) => {
                if !used_line_ids.insert(line_id) {
                    return Err(format!(
                        "شناسه ردیف ذخیره‌شده در ردیف {} تکرار شده است؛ سند را دوباره بارگذاری کنید.",
                        db::to_fa_digits(index + 1)
                    ));
                }
                let saved_code = saved_lines.get(&line_id).ok_or_else(|| {
                    format!(
                        "ردیف {} به این سند تعلق ندارد؛ سند را دوباره بارگذاری کنید.",
                        db::to_fa_digits(index + 1)
                    )
                })?;
                // Changing the account makes this a new posting. The historical
                // exception remains valid only when both id and account match.
                (saved_code == &account_code).then_some(saved_code)
            }
            None => None,
        };

        if let Err(error) = assert_postable_leaf(conn, &account_code) {
            if saved_line.is_none() {
                return Err(error);
            }
            assert_existing_legacy_account(conn, &account_code)?;
        }
        if saved_line.is_none() {
            assert_active_account(conn, &account_code)?;
        }
    }
    Ok(())
}

/// A voucher type is not a free label: the closing/opening types decide which
/// reports count the voucher. Reject lines whose accounts contradict the type.
pub(crate) fn assert_voucher_type_matches_lines(
    conn: &Connection,
    voucher_type: &str,
    lines: &[VoucherLineInput],
) -> Result<(), String> {
    let (forbidden, description): (&[AccountClass], &str) = if voucher_type == CLOSING_TEMPORARY {
        (
            &[AccountClass::Asset, AccountClass::Liability],
            "حساب‌های دارایی یا بدهی",
        )
    } else if voucher_type == CLOSING_FINAL || voucher_type == VOUCHER_TYPES[1] {
        (
            &[AccountClass::Income, AccountClass::Expense],
            "حساب‌های درآمد یا هزینه",
        )
    } else {
        return Ok(());
    };

    for line in lines {
        let class = account_class_for_type_rule(conn, &line.account_code)?;
        if forbidden.contains(&class) {
            return Err(format!(
                "نوع سند «{voucher_type}» با ردیف حساب {} سازگار نیست؛ این نوع سند نباید شامل {} باشد.",
                db::to_fa_digits(&line.account_code),
                description
            ));
        }
    }
    Ok(())
}

fn account_class_for_type_rule(
    conn: &Connection,
    account_code: &str,
) -> Result<AccountClass, String> {
    let (group_code, group_type, nature) = conn
        .query_row(
            "SELECT g.code, g.type, a.nature
               FROM accounts a
               JOIN account_groups g ON g.code = a.group_code
              WHERE a.code = ?1",
            params![account_code],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(db::db_error)?;
    if group_code == "10" || group_code == "11" {
        return Ok(if nature == "creditor" {
            AccountClass::Income
        } else {
            AccountClass::Expense
        });
    }
    Ok(classify(&group_type, account_code))
}

pub fn format_amount(value: i64) -> String {
    let negative = value < 0;
    let digits = value.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if negative {
        out.insert(0, '-');
    }
    out
}

impl Database {
    /// Applies the same row-identity and account-eligibility checks as saving.
    /// This prevents an Excel export from containing postings that could not
    /// be saved, including new rows for inactive or legacy accounts.
    pub fn validate_voucher_accounts(
        &self,
        voucher_id: Option<i64>,
        lines: &[VoucherLineInput],
    ) -> Result<(), String> {
        self.with_conn(|conn| validate_voucher_line_accounts(conn, voucher_id, lines))
    }
}
