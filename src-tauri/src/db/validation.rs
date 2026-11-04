//! Input validation for account groups and accounts.

use crate::db::common::normalize_code;
use crate::db::model::{
    ACCOUNT_GROUP_SECTIONS, ACCOUNT_GROUP_TYPES, ACCOUNT_NATURES, MAX_ACCOUNT_LEVEL,
};

pub(crate) fn validate_group_code(input: &str) -> Result<String, String> {
    let code = normalize_code(input);
    if code.is_empty() {
        return Err("کد گروه الزامی است.".to_string());
    }
    if code.chars().count() > 50 {
        return Err("کد گروه نمی‌تواند بیشتر از ۵۰ کاراکتر باشد.".to_string());
    }
    if !code.chars().all(|c| c.is_ascii_digit()) || !code.chars().any(|c| c != '0') {
        return Err("کد گروه باید فقط شامل ارقام انگلیسی و بزرگ‌تر از صفر باشد.".to_string());
    }
    Ok(code)
}

pub(crate) fn validate_account_code(input: &str) -> Result<String, String> {
    let code = normalize_code(input);
    if code.is_empty() {
        return Err("کد حساب الزامی است.".to_string());
    }
    if code.chars().count() > 50 {
        return Err("کد حساب نمی‌تواند بیشتر از ۵۰ رقم باشد.".to_string());
    }
    if !code.chars().all(|c| c.is_ascii_digit()) {
        return Err("کد حساب باید فقط شامل ارقام باشد.".to_string());
    }
    if !code.chars().any(|c| c != '0') {
        return Err("کد حساب باید بزرگ‌تر از صفر باشد.".to_string());
    }
    Ok(code)
}

pub(crate) fn validate_name(input: &str, label: &str) -> Result<String, String> {
    let name = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err(format!("{label} الزامی است."));
    }
    if name.chars().count() > 255 {
        return Err(format!("{label} نمی‌تواند بیشتر از ۲۵۵ کاراکتر باشد."));
    }
    Ok(name)
}

pub(crate) fn validate_level(input: i64) -> Result<i64, String> {
    if (1..=MAX_ACCOUNT_LEVEL).contains(&input) {
        Ok(input)
    } else {
        Err("سطح حساب جدید باید یکی از سطوح کل، معین یا تفصیلی ۱ باشد.".to_string())
    }
}

pub(crate) fn validate_nature(input: &str) -> Result<String, String> {
    let value = input.trim();
    if ACCOUNT_NATURES.contains(&value) {
        Ok(value.to_string())
    } else {
        Err("ماهیت حساب انتخاب‌شده معتبر نیست.".to_string())
    }
}

pub(crate) fn validate_type(input: &str) -> Result<String, String> {
    let value = input.trim();
    if ACCOUNT_GROUP_TYPES.contains(&value) {
        Ok(value.to_string())
    } else {
        Err("نوع حساب انتخاب‌شده معتبر نیست.".to_string())
    }
}

pub(crate) fn validate_section(input: &str) -> Result<String, String> {
    let value = input.trim();
    if value.is_empty() || ACCOUNT_GROUP_SECTIONS.contains(&value) {
        Ok(value.to_string())
    } else {
        Err("بخش پیش‌فرض صورت مالی انتخاب‌شده معتبر نیست.".to_string())
    }
}
