//! Small database-wide constants and error/format helpers.

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

pub const DB_FILE_NAME: &str = "hesabdari.sqlite3";
/// Version physically described by the base `SCHEMA` string. Additive
/// migrations then bring a fresh or restored database up to
/// `CURRENT_SCHEMA_VERSION`. Keeping the two constants separate is intentional:
/// a fresh database must still run the v5 voucher-pair migration.
pub(crate) const BASE_SCHEMA_VERSION: i64 = 4;
pub(crate) const CURRENT_SCHEMA_VERSION: i64 = 14;
// A pre-delete snapshot must preserve legacy vouchers exactly as they existed,
// even when an old document already violates the current balancing rules.
pub(crate) const BULK_DELETE_SNAPSHOT_KEY: &str = "backup_snapshot_kind";
pub(crate) const BULK_DELETE_SNAPSHOT_VALUE: &str = "bulk-delete-preimage-v1";
// A legacy level-4 migration backup deliberately captures level-4 accounts
// whose parent hierarchy is inconsistent; the migration then resolves them.
pub(crate) const LEGACY_LEVEL4_SNAPSHOT_VALUE: &str = "legacy-level4-migration-v1";
pub(crate) const LEGACY_DB_FILE_NAME: &str = "audit-analyzer.sqlite3";
pub(crate) const LEGACY_IDENTIFIER: &str = "ir.audit.account-analyzer";

pub(crate) fn today_jalali() -> String {
    let d = jalali_calendar::JalaliDate::today();
    format!("{:04}/{:02}/{:02}", d.year(), d.month(), d.day())
}

pub(crate) fn db_error(error: rusqlite::Error) -> String {
    // SQLite raises this when SUM() of integer columns leaves i64. The
    // numbers are not corrupted, but the raw English message is unhelpful.
    if error.to_string().contains("integer overflow") {
        return "جمع مبالغ از محدوده عددی مجاز بیشتر است؛ بازه گزارش را کوچک‌تر کنید.".to_string();
    }
    if error.to_string().contains("voucher is permanent") {
        return "این سند دائمی است و تغییر، حذف یا انتقال ردیف‌های آن مجاز نیست.".to_string();
    }
    format!("خطا در پایگاه داده: {error}")
}

/// Converts Persian/Arabic digits to ASCII so codes work with either keyboard.
pub fn normalize_digits(input: &str) -> String {
    input
        .chars()
        .map(|ch| match ch {
            '\u{06F0}'..='\u{06F9}' => char::from(b'0' + (ch as u32 - 0x06F0) as u8),
            '\u{0660}'..='\u{0669}' => char::from(b'0' + (ch as u32 - 0x0660) as u8),
            _ => ch,
        })
        .collect()
}

pub(crate) fn normalize_code(input: &str) -> String {
    normalize_digits(input).trim().replace(' ', "")
}

/// Folds Persian/Arabic text to one canonical spelling. Arabic yeh, alef
/// maksura and Arabic kaf become their Persian forms, and invisible bidi
/// controls and tatweel are removed. Digits are intentionally left untouched;
/// callers that need numeric input use [`normalize_digits`].
pub(crate) fn normalize_fa_text(input: &str) -> String {
    input
        .chars()
        .filter(|ch| !is_invisible_fa_mark(*ch))
        .map(fold_fa_char)
        .collect()
}

fn is_invisible_fa_mark(ch: char) -> bool {
    matches!(
        ch,
        '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{0640}'
    )
}

fn fold_fa_char(ch: char) -> char {
    match ch {
        '\u{064A}' | '\u{0649}' => '\u{06CC}',
        '\u{0643}' => '\u{06A9}',
        other => other,
    }
}

/// SQL expression that applies [`normalize_fa_text`] to a column. The literals
/// come from the fixed mapping above, never from user input. Permanent voucher
/// rows cannot be updated by a migration, so search compares against this
/// expression instead of the raw stored text.
pub(crate) fn fa_norm_sql(column: &str) -> String {
    let mut expression = column.to_string();
    for (from, to) in [
        ('\u{064A}', '\u{06CC}'),
        ('\u{0649}', '\u{06CC}'),
        ('\u{0643}', '\u{06A9}'),
    ] {
        expression = format!("replace({expression}, '{from}', '{to}')");
    }
    for mark in [
        '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}',
        '\u{0640}',
    ] {
        expression = format!("replace({expression}, '{mark}', '')");
    }
    expression
}

pub(crate) fn to_fa_digits(value: impl ToString) -> String {
    value
        .to_string()
        .chars()
        .map(|ch| match ch {
            '0'..='9' => char::from_u32(0x06F0 + (ch as u32 - '0' as u32)).unwrap_or(ch),
            _ => ch,
        })
        .collect()
}

/// Cryptographically random bytes encoded as lowercase hexadecimal. Used for
/// preview tokens and snapshot manifest operation ids; these values are never
/// persisted as secret material themselves.
pub(crate) fn random_hex(byte_count: usize) -> Result<String, String> {
    let mut bytes = vec![0u8; byte_count];
    getrandom::fill(&mut bytes)
        .map_err(|error| format!("تولید مقدار تصادفی امن ناموفق بود: {error}"))?;
    Ok(to_hex(&bytes))
}

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push_str(&format!("{byte:02x}"));
    }
    value
}

pub(crate) fn from_hex(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) {
        return Err("مقدار هگزادسیمال نامعتبر است.".to_string());
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for index in (0..value.len()).step_by(2) {
        let pair = &value[index..index + 2];
        let byte = u8::from_str_radix(pair, 16)
            .map_err(|_| "مقدار هگزادسیمال نامعتبر است.".to_string())?;
        bytes.push(byte);
    }
    Ok(bytes)
}

/// SHA-256 of a file's bytes, encoded as lowercase hexadecimal. Used to bind a
/// preview or snapshot manifest to the exact file the user confirmed.
pub(crate) fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("باز کردن فایل برای محاسبه hash ناموفق بود: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("خواندن فایل برای محاسبه hash ناموفق بود: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(to_hex(&hasher.finalize()))
}
