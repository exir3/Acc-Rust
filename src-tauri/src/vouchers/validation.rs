//! Validation and normalization for voucher input.
//!
//! Saving, Excel import and export all use the same account-eligibility
//! checks so they cannot disagree about which postings are allowed.

use crate::db::fs_catalog::{section_of, FsNormalSide, FsSection};
use crate::db::{self, classification, normalize_digits, Database};
use crate::vouchers::accounting::{classify, AccountClass};
use crate::vouchers::model::{VoucherLineInput, CLOSING_FINAL, CLOSING_TEMPORARY, VOUCHER_TYPES};
use rusqlite::{params, Connection};
use std::collections::{BTreeMap, HashMap, HashSet};

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

#[derive(Debug)]
struct VoucherAccountRecord {
    name: String,
    level: i64,
    group_code: String,
    parent_code: String,
    is_postable: i64,
    is_active: i64,
}

fn load_voucher_accounts(
    conn: &Connection,
    codes: &[String],
) -> Result<HashMap<String, VoucherAccountRecord>, String> {
    if codes.is_empty() {
        return Ok(HashMap::new());
    }

    let placeholders = std::iter::repeat_n("?", codes.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "WITH RECURSIVE needed(code) AS (
             SELECT code FROM accounts WHERE code IN ({placeholders})
             UNION
             SELECT accounts.parent_code
               FROM accounts
               JOIN needed ON accounts.code = needed.code
              WHERE accounts.parent_code IS NOT NULL AND accounts.parent_code <> ''
         )
         SELECT code, name, level, group_code, COALESCE(parent_code, ''),
                is_postable, is_active
           FROM accounts
          WHERE code IN (SELECT code FROM needed)"
    );
    let mut statement = conn.prepare(&sql).map_err(db::db_error)?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(codes.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                VoucherAccountRecord {
                    name: row.get(1)?,
                    level: row.get(2)?,
                    group_code: row.get(3)?,
                    parent_code: row.get(4)?,
                    is_postable: row.get(5)?,
                    is_active: row.get(6)?,
                },
            ))
        })
        .map_err(db::db_error)?
        .collect::<rusqlite::Result<HashMap<_, _>>>()
        .map_err(db::db_error)?;
    Ok(rows)
}

fn assert_postable_leaf(
    accounts: &HashMap<String, VoucherAccountRecord>,
    account_code: &str,
) -> Result<String, String> {
    let account = accounts
        .get(account_code)
        .ok_or_else(|| format!("حساب با کد {} یافت نشد.", db::to_fa_digits(account_code)))?;
    if !(1..=2).contains(&account.level) || account.is_postable == 0 {
        return Err(format!(
            "حساب «{}» قابل ثبت نیست؛ حساب پایه باید از سطح کل یا معین و فعالِ ثبت باشد. تفصیلی سطح ۳ باید در ستون جداگانه انتخاب شود.",
            account.name
        ));
    }
    Ok(account.name.clone())
}

/// Checks the complete parent chain (existence, level, group, activity and cycles).
fn assert_active_account(
    accounts: &HashMap<String, VoucherAccountRecord>,
    account_code: &str,
) -> Result<(), String> {
    let mut current = account_code.to_string();
    let mut seen = HashSet::new();
    loop {
        if !seen.insert(current.clone()) {
            return Err("چرخه‌ای در سلسله‌مراتب حساب‌ها شناسایی شد.".to_string());
        }
        let row = accounts.get(&current).ok_or_else(|| {
            format!(
                "حساب {} یا یکی از والدهای آن یافت نشد.",
                db::to_fa_digits(&current)
            )
        })?;
        if row.is_active == 0 {
            return Err(format!(
                "حساب «{}» یا یکی از حساب‌های بالاتر آن غیرفعال است.",
                row.name
            ));
        }
        if row.parent_code.is_empty() {
            if row.level != 1 {
                return Err(format!("حساب «{}» والد معتبر ندارد.", row.name));
            }
            break;
        }
        let parent = accounts.get(&row.parent_code).ok_or_else(|| {
            format!(
                "حساب بالاتر {} یافت نشد.",
                db::to_fa_digits(&row.parent_code)
            )
        })?;
        if parent.level != row.level - 1 || parent.group_code != row.group_code {
            return Err(format!(
                "سطح یا گروه حساب بالاتر «{}» با ساختار حساب سازگار نیست.",
                db::to_fa_digits(&row.parent_code)
            ));
        }
        current = row.parent_code.clone();
    }
    Ok(())
}

/// Converts legacy clients that send a level-3 account in accountCode into
/// the canonical base-account + detail1Code pair.
pub(crate) fn normalize_voucher_line_accounts(
    conn: &Connection,
    lines: &mut [VoucherLineInput],
) -> Result<(), String> {
    let mut codes = Vec::with_capacity(lines.len() * 2);
    for (index, line) in lines.iter_mut().enumerate() {
        line.account_code = db::normalize_code(&line.account_code);
        let explicit_detail = line
            .detail1_code
            .as_deref()
            .map(db::normalize_code)
            .unwrap_or_default();
        if line.account_code.is_empty() {
            return Err(format!(
                "کد حساب ردیف {} الزامی است.",
                db::to_fa_digits(index + 1)
            ));
        }
        codes.push(line.account_code.clone());
        if !explicit_detail.is_empty() {
            codes.push(explicit_detail);
        }
    }
    let accounts = load_voucher_accounts(conn, &codes)?;

    for (index, line) in lines.iter_mut().enumerate() {
        let explicit_detail = line
            .detail1_code
            .as_deref()
            .map(db::normalize_code)
            .unwrap_or_default();
        let account = accounts.get(&line.account_code).ok_or_else(|| {
            format!(
                "حساب ردیف {} با کد {} یافت نشد.",
                db::to_fa_digits(index + 1),
                db::to_fa_digits(&line.account_code)
            )
        })?;
        if account.level == 3 {
            if !explicit_detail.is_empty() {
                return Err(format!(
                    "ردیف {} هم حساب سطح ۳ و هم تفصیلی جداگانه دارد؛ حساب پایه را انتخاب کنید.",
                    db::to_fa_digits(index + 1)
                ));
            }
            let Some(parent) = accounts.get(&account.parent_code) else {
                return Err(format!(
                    "تفصیلی ردیف {} والد معتبر ندارد.",
                    db::to_fa_digits(index + 1)
                ));
            };
            if parent.level != 2 || parent.group_code != account.group_code {
                return Err(format!(
                    "تفصیلی ردیف {} باید فرزند مستقیم یک حساب معین از همان گروه باشد.",
                    db::to_fa_digits(index + 1)
                ));
            }
            line.detail1_code = Some(line.account_code.clone());
            line.account_code = account.parent_code.clone();
        } else {
            if !explicit_detail.is_empty() {
                line.detail1_code = Some(explicit_detail);
            } else {
                line.detail1_code = None;
            }
        }
    }
    Ok(())
}

fn validate_base_detail_pair(
    accounts: &HashMap<String, VoucherAccountRecord>,
    base_code: &str,
    detail_code: Option<&str>,
) -> Result<(), String> {
    assert_postable_leaf(accounts, base_code)?;
    assert_active_account(accounts, base_code)?;
    let base = accounts
        .get(base_code)
        .ok_or_else(|| format!("حساب با کد {} یافت نشد.", db::to_fa_digits(base_code)))?;
    if let Some(detail) = detail_code.filter(|value| !value.is_empty()) {
        if base.level != 2 {
            return Err("انتخاب تفصیلی سطح ۱ فقط برای حساب معین مجاز است؛ حساب کل نمی‌تواند تفصیلی داشته باشد.".to_string());
        }
        let detail_row = accounts
            .get(detail)
            .ok_or_else(|| format!("حساب تفصیلی {} یافت نشد.", db::to_fa_digits(detail)))?;
        if detail_row.level != 3
            || detail_row.parent_code != base_code
            || detail_row.group_code != base.group_code
        {
            return Err(format!(
                "تفصیلی {} باید از سطح ۳ و فرزند مستقیم معین {} از همان گروه باشد.",
                db::to_fa_digits(detail),
                db::to_fa_digits(base_code)
            ));
        }
        if detail_row.is_active == 0 {
            return Err(format!(
                "حساب تفصیلی {} غیرفعال است.",
                db::to_fa_digits(detail)
            ));
        }
        assert_active_account(accounts, detail)?;
    }
    Ok(())
}

/// The single source of truth for account references in saved voucher rows.
/// Historical exceptions are permitted only for the original line id and
/// the exact same base/detail pair.
pub(crate) fn validate_voucher_line_accounts(
    conn: &Connection,
    voucher_id: Option<i64>,
    lines: &[VoucherLineInput],
) -> Result<(), String> {
    let codes = lines
        .iter()
        .flat_map(|line| {
            std::iter::once(db::normalize_code(&line.account_code)).chain(
                line.detail1_code
                    .as_deref()
                    .map(db::normalize_code)
                    .filter(|code| !code.is_empty()),
            )
        })
        .filter(|code| !code.is_empty())
        .collect::<Vec<_>>();
    let accounts = load_voucher_accounts(conn, &codes)?;

    let saved_lines: BTreeMap<i64, (String, Option<String>)> = if let Some(id) =
        voucher_id.filter(|id| *id > 0)
    {
        let mut statement = conn.prepare("SELECT id, account_code, NULLIF(detail1_code,'') FROM voucher_lines WHERE voucher_id = ?1").map_err(db::db_error)?;
        let rows = statement
            .query_map(params![id], |row| {
                Ok((row.get(0)?, (row.get(1)?, row.get(2)?)))
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
        let base = db::normalize_code(&line.account_code);
        let detail = line
            .detail1_code
            .as_deref()
            .map(db::normalize_code)
            .filter(|v| !v.is_empty());
        if base.is_empty() {
            return Err(format!(
                "کد حساب ردیف {} الزامی است.",
                db::to_fa_digits(index + 1)
            ));
        }
        let saved_line = match line.id {
            Some(line_id) => {
                if !used_line_ids.insert(line_id) {
                    return Err(format!("شناسه ردیف ذخیره‌شده در ردیف {} تکرار شده است؛ سند را دوباره بارگذاری کنید.", db::to_fa_digits(index + 1)));
                }
                let saved_pair = saved_lines.get(&line_id).ok_or_else(|| {
                    format!(
                        "ردیف {} به این سند تعلق ندارد؛ سند را دوباره بارگذاری کنید.",
                        db::to_fa_digits(index + 1)
                    )
                })?;
                (saved_pair.0 == base && saved_pair.1 == detail).then_some(saved_pair)
            }
            None => None,
        };
        if saved_line.is_none() {
            validate_base_detail_pair(&accounts, &base, detail.as_deref())
                .map_err(|error| format!("ردیف {}: {error}", db::to_fa_digits(index + 1)))?;
        }
    }
    Ok(())
}

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
        let effective_code = line
            .detail1_code
            .as_deref()
            .filter(|v| !v.is_empty())
            .unwrap_or(&line.account_code);
        let class = account_class_for_type_rule(conn, effective_code)?;
        if forbidden.contains(&class) {
            return Err(format!(
                "نوع سند «{voucher_type}» با ردیف حساب {} سازگار نیست؛ این نوع سند نباید شامل {} باشد.",
                db::to_fa_digits(effective_code),
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
    let map = classification::load_classifications(conn)?;
    let row = map.get(account_code).ok_or_else(|| {
        format!(
            "حساب {} برای تعیین نوع سند یافت نشد.",
            db::to_fa_digits(account_code)
        )
    })?;
    let effective = classification::resolve_account(row, &map);
    Ok(match section_of(&effective.fs_line) {
        FsSection::NonCurrentAsset | FsSection::CurrentAsset => AccountClass::Asset,
        FsSection::NonCurrentLiability | FsSection::CurrentLiability => AccountClass::Liability,
        FsSection::Equity => AccountClass::Equity,
        FsSection::Revenue => AccountClass::Income,
        FsSection::Cogs
        | FsSection::SellingExpense
        | FsSection::AdminExpense
        | FsSection::FinanceCost
        | FsSection::IncomeTax => AccountClass::Expense,
        FsSection::OtherOperating | FsSection::OtherNonOperating => {
            if effective.normal_side == FsNormalSide::Credit {
                AccountClass::Income
            } else {
                AccountClass::Expense
            }
        }
        FsSection::Memorandum => AccountClass::Memorandum,
        FsSection::Unclassified => classify(&row.group_type, account_code),
    })
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
        self.with_conn(|conn| {
            let mut normalized = lines.to_vec();
            normalize_voucher_line_accounts(conn, &mut normalized)?;
            validate_voucher_line_accounts(conn, voucher_id, &normalized)
        })
    }
}
