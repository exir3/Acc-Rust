//! Shared Excel limits, file checks and cell parsers.

use crate::db;
use crate::vouchers::MAX_VOUCHER_NUMBER;
use calamine::Data;
use rust_xlsxwriter::{Format, Worksheet, XlsxError};
use std::path::Path;

pub(crate) const MAX_IMPORT_LINES: usize = 1000;
pub(crate) const MAX_LINE_DESCRIPTION: usize = 500;
pub(crate) const HEADER_SCAN_ROWS: usize = 30;
pub(crate) const MAX_BULK_IMPORT_ROWS: usize = 300_000;
pub(crate) const MAX_BULK_IMPORT_COLUMNS: usize = 40;
pub(crate) const MAX_VOUCHER_IMPORT_ROWS: usize = 20_000;
pub(crate) const MAX_ACCOUNT_IMPORT_ROWS: usize = 10_000;
pub(crate) const MAX_NUMERIC_ACCOUNT_CODE: i64 = 999_999_999_999_999;
pub(crate) const MAX_SHEET_COLUMNS: usize = 50;
pub(crate) const MAX_EXCEL_FILE_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const MAX_XLSX_ENTRY_BYTES: u64 = 128 * 1024 * 1024;
pub(crate) const MAX_XLSX_UNCOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;
pub(crate) const MAX_AMOUNT: i64 = 999_999_999_999_999;
pub(crate) const MAX_IMPORT_ERRORS: usize = 200;

/// Rejects an oversized workbook before calamine allocates its used range.
pub(crate) fn ensure_excel_file_size(path: &Path) -> Result<(), String> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("خواندن اطلاعات فایل اکسل ناموفق بود: {error}"))?;
    if !metadata.is_file() {
        return Err("مسیر انتخاب‌شده یک فایل نیست.".to_string());
    }
    if metadata.len() > MAX_EXCEL_FILE_BYTES {
        return Err(format!(
            "حجم فایل اکسل بیش از حد مجاز است (حداکثر {} مگابایت).",
            db::to_fa_digits(MAX_EXCEL_FILE_BYTES / (1024 * 1024))
        ));
    }
    ensure_xlsx_archive_budget(path)
}

/// Checks the declared size of every entry in the ZIP container before the
/// workbook is parsed. A small compressed file can otherwise expand into a
/// huge worksheet that is only rejected after it is already in memory.
pub(crate) fn ensure_xlsx_archive_budget(path: &Path) -> Result<(), String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("باز کردن فایل اکسل ناموفق بود: {error}"))?;
    let mut signature = [0u8; 4];
    let read = file.read(&mut signature).unwrap_or(0);
    drop(file);
    // Legacy .xls files are not ZIP containers; calamine handles them itself.
    if read < 4 || &signature[..2] != b"PK" {
        return Ok(());
    }

    let file = std::fs::File::open(path)
        .map_err(|error| format!("باز کردن فایل اکسل ناموفق بود: {error}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("ساختار فایل اکسل معتبر نیست: {error}"))?;
    let mut total = 0u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index_raw(index)
            .map_err(|error| format!("خواندن فهرست فایل اکسل ناموفق بود: {error}"))?;
        let size = entry.size();
        if size > MAX_XLSX_ENTRY_BYTES {
            return Err(format!(
                "یکی از بخش‌های فایل اکسل بیش از {} مگابایت باز می‌شود؛ فایل معتبر نیست.",
                db::to_fa_digits(MAX_XLSX_ENTRY_BYTES / (1024 * 1024))
            ));
        }
        total = total.saturating_add(size);
        if total > MAX_XLSX_UNCOMPRESSED_BYTES {
            return Err(format!(
                "حجم بازشده فایل اکسل بیش از حد مجاز است (حداکثر {} مگابایت).",
                db::to_fa_digits(MAX_XLSX_UNCOMPRESSED_BYTES / (1024 * 1024))
            ));
        }
    }
    Ok(())
}

pub(crate) fn ensure_sheet_dimensions(
    range: &calamine::Range<Data>,
    max_rows: usize,
    max_columns: usize,
) -> Result<(), String> {
    if range.height() > max_rows || range.width() > max_columns {
        return Err(format!(
            "ابعاد کاربرگ اکسل بیش از حد مجاز است (حداکثر {} ردیف و {} ستون).",
            db::to_fa_digits(max_rows),
            db::to_fa_digits(max_columns)
        ));
    }
    Ok(())
}

/* ── Shared helpers ─────────────────────────────────────────── */

pub(crate) fn xlsx_error(error: XlsxError) -> String {
    format!("ساخت فایل اکسل ناموفق بود: {error}")
}

pub(crate) fn normalize_header(value: &str) -> String {
    db::normalize_digits(value)
        .replace(['ي', 'ى'], "ی")
        .replace('ك', "ک")
        .replace(['\u{200c}', ' ', '\u{00a0}'], "")
        .replace(['۲', '٢'], "2")
        .trim()
        .to_string()
}

/// Normalizes Persian/Arabic digits and trims a cell that must contain a
/// numeric code. Unlike a digit filter, it never drops characters, so bad
/// input can be rejected instead of silently turning into another code.
pub(crate) fn code_text(value: &str) -> String {
    db::normalize_digits(value).trim().to_string()
}

pub(crate) fn parse_code(value: &str, label: &str) -> Result<String, String> {
    let code = code_text(value);
    if code.is_empty() {
        return Err(format!("{label} خالی است."));
    }
    if !code.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(format!(
            "{label} باید فقط شامل ارقام باشد؛ مقدار «{}» معتبر نیست.",
            value.trim()
        ));
    }
    Ok(code)
}

pub(crate) fn parse_account_code_cell(value: &str, label: &str) -> Result<String, String> {
    let code = parse_code(value, label)?;
    if code.chars().count() > 50 {
        return Err(format!("{label} نمی‌تواند بیشتر از ۵۰ رقم باشد."));
    }
    Ok(code)
}

pub(crate) fn parse_account_code_data_cell(cell: &Data, label: &str) -> Result<String, String> {
    match cell {
        // Numeric cells are common in accounting workbooks. Accept only exact,
        // non-negative integers within Excel's 15-digit precision limit.
        // Codes with leading zeroes or more than 15 digits must be text cells.
        Data::Int(value) if (0..=MAX_NUMERIC_ACCOUNT_CODE).contains(value) => {
            parse_account_code_cell(&value.to_string(), label)
        }
        Data::Float(value)
            if value.is_finite()
                && value.fract() == 0.0
                && (0.0..=MAX_NUMERIC_ACCOUNT_CODE as f64).contains(value) =>
        {
            parse_account_code_cell(&format!("{value:.0}"), label)
        }
        Data::Int(_) | Data::Float(_) => Err(format!(
            "{label} باید عدد صحیح و حداکثر ۱۵ رقمی باشد؛ برای حفظ صفرهای ابتدایی یا کدهای طولانی‌تر، سلول را به‌صورت متن ذخیره کنید."
        )),
        _ => parse_account_code_cell(&cell_text(cell), label),
    }
}

pub(crate) fn cell_text(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(value) => value.trim().to_string(),
        Data::Int(value) => value.to_string(),
        Data::Float(value) => {
            if value.fract() == 0.0 && value.abs() < 1e15 {
                format!("{}", *value as i64)
            } else {
                value.to_string()
            }
        }
        Data::Bool(value) => value.to_string(),
        other => other.to_string().trim().to_string(),
    }
}

/// Converts a Gregorian date into the Jalali format used by the database.
pub(crate) fn gregorian_to_jalali_date(year: u32, month: u32, day: u32) -> Result<String, String> {
    let jalali = jalali_calendar::JalaliDate::from_gregorian(year as i32, month, day)
        .map_err(|_| "تاریخ میلادی فایل اکسل معتبر یا قابل تبدیل نیست.".to_string())?;
    let value = format!("{:04}/{:02}/{:02}", jalali.year(), jalali.month(), jalali.day());
    crate::vouchers::validate_voucher_date(&value)
}

/// Reads date values from Excel without assuming that every date is stored as
/// a preformatted Jalali text cell. Calamine exposes styled Excel dates as
/// `DateTime`; some exports instead provide ISO strings or serial numbers.
pub(crate) fn parse_bulk_voucher_date(cell: &Data) -> Result<String, String> {
    match cell {
        Data::DateTime(value) => {
            if !value.is_datetime() {
                return Err("سلول تاریخ به‌صورت مدت‌زمان ذخیره شده است، نه تاریخ.".to_string());
            }
            let (year, month, day, _, _, _, _) = value.to_ymd_hms_milli();
            gregorian_to_jalali_date(year as u32, month as u32, day as u32)
        }
        Data::DateTimeIso(value) => parse_bulk_voucher_date_text(value),
        Data::Int(value) => parse_bulk_voucher_date_number(*value as f64),
        Data::Float(value) => parse_bulk_voucher_date_number(*value),
        _ => parse_bulk_voucher_date_text(&cell_text(cell)),
    }
}

pub(crate) fn parse_bulk_voucher_date_number(value: f64) -> Result<String, String> {
    if !value.is_finite() {
        return Err("تاریخ عددی اکسل معتبر نیست.".to_string());
    }
    let rounded = value.round();
    if (value - rounded).abs() <= 1e-9 && rounded >= 10_000_000.0 && rounded <= 31_779_999.0 {
        return parse_bulk_voucher_date_text(&format!("{rounded:.0}"));
    }
    // Numeric cells without a date number format are ambiguous. Excel serials
    // in the normal modern-date range are supported using the 1900 date system.
    if (20_000.0..=2_958_465.999_999).contains(&value) {
        let excel_date = calamine::ExcelDateTime::new(
            value,
            calamine::ExcelDateTimeType::DateTime,
            false,
        );
        let (year, month, day, _, _, _, _) = excel_date.to_ymd_hms_milli();
        return gregorian_to_jalali_date(year as u32, month as u32, day as u32);
    }
    Err("تاریخ عددی اکسل باید یک تاریخ شمسی ۸ رقمی یا شماره سریال معتبر تاریخ باشد.".to_string())
}

pub(crate) fn parse_bulk_voucher_date_text(input: &str) -> Result<String, String> {
    let normalized = db::normalize_digits(input.trim());
    let date_part = normalized
        .trim()
        .split(|ch| ch == 'T' || ch == ' ')
        .next()
        .unwrap_or("");
    let date_part = date_part.replace('-', "/");

    if date_part.chars().all(|ch| ch.is_ascii_digit()) && date_part.len() == 8 {
        let year = date_part[0..4]
            .parse::<u32>()
            .map_err(|_| "سال تاریخ اکسل معتبر نیست.".to_string())?;
        let month = date_part[4..6]
            .parse::<u32>()
            .map_err(|_| "ماه تاریخ اکسل معتبر نیست.".to_string())?;
        let day = date_part[6..8]
            .parse::<u32>()
            .map_err(|_| "روز تاریخ اکسل معتبر نیست.".to_string())?;
        return if year >= 1600 {
            gregorian_to_jalali_date(year, month, day)
        } else {
            crate::vouchers::validate_voucher_date(&format!("{year:04}/{month:02}/{day:02}"))
        };
    }

    let parts: Vec<&str> = date_part.split('/').collect();
    if parts.len() == 3 {
        let year = parts[0]
            .parse::<u32>()
            .map_err(|_| "سال تاریخ اکسل معتبر نیست.".to_string())?;
        let month = parts[1]
            .parse::<u32>()
            .map_err(|_| "ماه تاریخ اکسل معتبر نیست.".to_string())?;
        let day = parts[2]
            .parse::<u32>()
            .map_err(|_| "روز تاریخ اکسل معتبر نیست.".to_string())?;
        if year >= 1600 {
            return gregorian_to_jalali_date(year, month, day);
        }
    }
    crate::vouchers::validate_voucher_date(&date_part)
}

pub(crate) fn normalize_imported_voucher_type(input: &str) -> Result<String, String> {
    let value = normalize_header(input);
    for voucher_type in crate::vouchers::VOUCHER_TYPES {
        if value == normalize_header(voucher_type) {
            return Ok(voucher_type.to_string());
        }
    }
    let aliases = [
        (&["عادی", "سندعادی"][..], crate::vouchers::VOUCHER_TYPES[0]),
        (&["افتتاحیه", "سندافتتاحیه"][..], crate::vouchers::VOUCHER_TYPES[1]),
        (&["اختتامیه", "سنداختتامیه"][..], crate::vouchers::VOUCHER_TYPES[2]),
        (
            &["بستن حسابهای موقت", "بستن حساب‌های موقت", "سند بستن حسابهای موقت"][..],
            crate::vouchers::VOUCHER_TYPES[3],
        ),
    ];
    for (names, canonical) in aliases {
        if names.iter().any(|name| normalize_header(name) == value) {
            return Ok(canonical.to_string());
        }
    }
    Err(format!("نوع سند «{}» در فایل اکسل شناخته‌شده نیست.", input.trim()))
}

pub(crate) fn is_total_row(row: &[Data]) -> bool {
    row.iter()
        .any(|cell| normalize_header(&cell_text(cell)) == "جمعکل")
}

pub(crate) fn has_import_values(row: &[Data], columns: &[Option<usize>]) -> bool {
    columns.iter().flatten().any(|index| {
        row.get(*index)
            .is_some_and(|cell| !cell_text(cell).is_empty())
    })
}

pub(crate) fn parse_text_amount(text: &str) -> Result<i64, String> {
    let normalized = db::normalize_digits(text.trim());
    let (sign, value) = normalized
        .strip_prefix('-')
        .map_or((1i64, normalized.as_str()), |value| (-1i64, value));
    let is_separator = |ch: char| matches!(ch, ',' | '٬' | ' ' | '\u{200c}' | '\u{00a0}');
    let separators: Vec<char> = value.chars().filter(|ch| is_separator(*ch)).collect();
    let digits = if separators.is_empty() {
        value.to_string()
    } else {
        if separators
            .iter()
            .any(|separator| *separator != separators[0])
        {
            return Err(format!("مبلغ «{text}» جداکننده‌های ناسازگار دارد."));
        }
        let groups: Vec<&str> = value.split(separators[0]).collect();
        if groups.len() < 2
            || groups[0].is_empty()
            || groups[0].len() > 3
            || !groups[0].chars().all(|ch| ch.is_ascii_digit())
            || groups[1..]
                .iter()
                .any(|group| group.len() != 3 || !group.chars().all(|ch| ch.is_ascii_digit()))
        {
            return Err(format!("مبلغ «{text}» گروه‌بندی هزارگان معتبری ندارد."));
        }
        groups.concat()
    };
    if digits.is_empty() {
        return Err(format!("مبلغ «{text}» عدد معتبری نیست."));
    }
    let magnitude = digits
        .parse::<i64>()
        .map_err(|_| format!("مبلغ «{text}» عدد معتبری نیست."))?;
    magnitude
        .checked_mul(sign)
        .ok_or_else(|| format!("مبلغ «{text}» خارج از محدوده است."))
}

pub(crate) fn validate_export_balance(total_debit: i64, total_credit: i64) -> Result<(), String> {
    if total_debit <= 0 || total_debit != total_credit {
        return Err(format!(
            "خروجی سند نامتوازن است؛ بدهکار {} و بستانکار {} است.",
            db::to_fa_digits(total_debit),
            db::to_fa_digits(total_credit)
        ));
    }
    Ok(())
}

pub(crate) fn write_total_amount(
    worksheet: &mut Worksheet,
    row: u32,
    column: u16,
    amount: i64,
    format: &Format,
) -> Result<(), String> {
    if amount.unsigned_abs() <= MAX_AMOUNT as u64 {
        worksheet
            .write_number_with_format(row, column, amount as f64, format)
            .map(|_| ())
            .map_err(xlsx_error)
    } else {
        worksheet
            .write_string_with_format(row, column, amount.to_string(), format)
            .map(|_| ())
            .map_err(xlsx_error)
    }
}

/// Reads a cell as a whole-Rial amount. Persian digits and thousands
/// separators are accepted because that is how users type them.
pub(crate) fn cell_amount(cell: &Data) -> Result<i64, String> {
    match cell {
        Data::Empty => Ok(0),
        Data::Int(value) => Ok(*value),
        Data::Float(value)
            if value.is_finite()
                && (value - value.round()).abs() <= 1e-9
                && value.round().abs() <= MAX_AMOUNT as f64 =>
        {
            Ok(value.round() as i64)
        }
        Data::Float(value) if value.round().abs() > MAX_AMOUNT as f64 => Err(format!(
            "مبلغ باید بین صفر و {} ریال باشد.",
            db::to_fa_digits(MAX_AMOUNT)
        )),
        Data::Float(_) => Err("مبلغ باید عدد صحیح و محدود باشد (بدون اعشار).".to_string()),
        _ => {
            let text = cell_text(cell);
            if text.is_empty() {
                return Ok(0);
            }
            parse_text_amount(&text)
        }
    }
}

pub(crate) fn cell_voucher_number(cell: &Data) -> Result<i64, String> {
    match cell {
        Data::Float(value)
            if value.is_finite()
                && (value - value.round()).abs() <= 1e-9
                && value.round().abs() <= MAX_VOUCHER_NUMBER as f64 =>
        {
            Ok(value.round() as i64)
        }
        Data::Float(value)
            if !value.is_finite() || value.round().abs() > MAX_VOUCHER_NUMBER as f64 =>
        {
            Err(format!(
                "شماره سند باید بین صفر و {} باشد.",
                db::to_fa_digits(MAX_VOUCHER_NUMBER)
            ))
        }
        Data::Float(_) => Err("شماره سند باید عدد صحیح باشد.".to_string()),
        _ => cell_amount(cell),
    }
}

