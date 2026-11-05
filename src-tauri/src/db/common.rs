//! Small database-wide constants and error/format helpers.

pub const DB_FILE_NAME: &str = "hesabdari.sqlite3";
/// Version physically described by the base `SCHEMA` string. Additive
/// migrations then bring a fresh or restored database up to
/// `CURRENT_SCHEMA_VERSION`. Keeping the two constants separate is intentional:
/// a fresh database must still run the v5 voucher-pair migration.
pub(crate) const BASE_SCHEMA_VERSION: i64 = 4;
pub(crate) const CURRENT_SCHEMA_VERSION: i64 = 8;
// A pre-delete snapshot must preserve legacy vouchers exactly as they existed,
// even when an old document already violates the current balancing rules.
pub(crate) const BULK_DELETE_SNAPSHOT_KEY: &str = "backup_snapshot_kind";
pub(crate) const BULK_DELETE_SNAPSHOT_VALUE: &str = "bulk-delete-preimage-v1";
pub(crate) const LEGACY_DB_FILE_NAME: &str = "audit-analyzer.sqlite3";
pub(crate) const LEGACY_IDENTIFIER: &str = "ir.audit.account-analyzer";
/// Built-in groups whose account nature determines whether a row is income or expense.
pub(crate) const OTHER_OPERATING_GROUP_CODE: &str = "10";
pub(crate) const OTHER_NON_OPERATING_GROUP_CODE: &str = "11";

pub(crate) fn today_jalali() -> String {
    let d = jalali_calendar::JalaliDate::today();
    format!("{:04}/{:02}/{:02}", d.year(), d.month(), d.day())
}

pub(crate) fn db_error(error: rusqlite::Error) -> String {
    // SQLite raises this when SUM() of integer columns leaves i64. The
    // numbers are not corrupted, but the raw English message is unhelpful.
    if error.to_string().contains("integer overflow") {
        return "جمع مبالغ از محدوده عددی مجاز بیشتر است؛ بازه گزارش را کوچک‌تر کنید."
            .to_string();
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
