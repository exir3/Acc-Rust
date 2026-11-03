//! Excel import and export for one voucher.
//!
//! The export writes a real `.xlsx` workbook with the same layout the
//! importer understands: a header row naming the columns
//! «ردیف / کد حساب / تفصیلی ۱ / شرح / بدهکار / بستانکار» and one row per
//! voucher line. Amounts are whole Rials, exactly like the database.

use crate::db;
use crate::vouchers::{VoucherInput, VoucherLineInput, MAX_VOUCHER_NUMBER};
use calamine::{open_workbook_auto, Data, Reader};
use rust_xlsxwriter::{Color, Format, FormatAlign, FormatBorder, Workbook, Worksheet, XlsxError};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

const MAX_IMPORT_LINES: usize = 1000;
const MAX_LINE_DESCRIPTION: usize = 500;
const HEADER_SCAN_ROWS: usize = 30;
const MAX_BULK_IMPORT_ROWS: usize = 300_000;
const MAX_BULK_IMPORT_COLUMNS: usize = 40;
const MAX_VOUCHER_IMPORT_ROWS: usize = 20_000;
const MAX_ACCOUNT_IMPORT_ROWS: usize = 10_000;
const MAX_NUMERIC_ACCOUNT_CODE: i64 = 999_999_999_999_999;
const MAX_SHEET_COLUMNS: usize = 50;
const MAX_EXCEL_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_XLSX_ENTRY_BYTES: u64 = 128 * 1024 * 1024;
const MAX_XLSX_UNCOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_AMOUNT: i64 = 999_999_999_999_999;
const MAX_IMPORT_ERRORS: usize = 200;

/// Rejects an oversized workbook before calamine allocates its used range.
fn ensure_excel_file_size(path: &Path) -> Result<(), String> {
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
fn ensure_xlsx_archive_budget(path: &Path) -> Result<(), String> {
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

fn ensure_sheet_dimensions(
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

#[derive(Debug, Clone)]
pub struct BulkVoucherImport {
    pub file_name: String,
    pub sheet_name: String,
    pub vouchers: Vec<VoucherInput>,
    pub accounts: Vec<ImportedAccount>,
}

#[derive(Debug, Clone)]
pub struct ImportedAccount {
    pub code: String,
    pub name: String,
    pub parent_code: String,
    pub group_code: String,
    pub level: i64,
    pub nature: String,
    pub nature_is_explicit: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkVoucherPreview {
    pub file_name: String,
    pub sheet_name: String,
    pub voucher_count: usize,
    pub line_count: usize,
    pub account_count: usize,
    pub new_account_count: usize,
    pub number_conflict_count: usize,
}

#[derive(Default)]
struct BulkImportColumns {
    voucher_number: Option<usize>,
    voucher_date: Option<usize>,
    group_code: Option<usize>,
    general_code: Option<usize>,
    general_name: Option<usize>,
    general_nature: Option<usize>,
    subsidiary_code: Option<usize>,
    subsidiary_name: Option<usize>,
    subsidiary_nature: Option<usize>,
    detail1_code: Option<usize>,
    detail1_name: Option<usize>,
    detail1_nature: Option<usize>,
    voucher_type: Option<usize>,
    voucher_description: Option<usize>,
    line_description: Option<usize>,
    debit: Option<usize>,
    credit: Option<usize>,
}

/* ── Shared helpers ─────────────────────────────────────────── */

fn xlsx_error(error: XlsxError) -> String {
    format!("ساخت فایل اکسل ناموفق بود: {error}")
}

fn normalize_header(value: &str) -> String {
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
fn code_text(value: &str) -> String {
    db::normalize_digits(value).trim().to_string()
}

fn parse_code(value: &str, label: &str) -> Result<String, String> {
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

fn parse_account_code_cell(value: &str, label: &str) -> Result<String, String> {
    let code = parse_code(value, label)?;
    if code.chars().count() > 20 {
        return Err(format!("{label} نمی‌تواند بیشتر از ۲۰ رقم باشد."));
    }
    Ok(code)
}

fn parse_account_code_data_cell(cell: &Data, label: &str) -> Result<String, String> {
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

fn cell_text(cell: &Data) -> String {
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
fn gregorian_to_jalali_date(year: u32, month: u32, day: u32) -> Result<String, String> {
    let jalali = jalali_calendar::JalaliDate::from_gregorian(year as i32, month, day)
        .map_err(|_| "تاریخ میلادی فایل اکسل معتبر یا قابل تبدیل نیست.".to_string())?;
    let value = format!("{:04}/{:02}/{:02}", jalali.year(), jalali.month(), jalali.day());
    crate::vouchers::validate_voucher_date(&value)
}

/// Reads date values from Excel without assuming that every date is stored as
/// a preformatted Jalali text cell. Calamine exposes styled Excel dates as
/// `DateTime`; some exports instead provide ISO strings or serial numbers.
fn parse_bulk_voucher_date(cell: &Data) -> Result<String, String> {
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

fn parse_bulk_voucher_date_number(value: f64) -> Result<String, String> {
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

fn parse_bulk_voucher_date_text(input: &str) -> Result<String, String> {
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

fn normalize_imported_voucher_type(input: &str) -> Result<String, String> {
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

fn is_total_row(row: &[Data]) -> bool {
    row.iter()
        .any(|cell| normalize_header(&cell_text(cell)) == "جمعکل")
}

fn has_import_values(row: &[Data], columns: &[Option<usize>]) -> bool {
    columns.iter().flatten().any(|index| {
        row.get(*index)
            .is_some_and(|cell| !cell_text(cell).is_empty())
    })
}

fn parse_text_amount(text: &str) -> Result<i64, String> {
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

fn validate_export_balance(total_debit: i64, total_credit: i64) -> Result<(), String> {
    if total_debit <= 0 || total_debit != total_credit {
        return Err(format!(
            "خروجی سند نامتوازن است؛ بدهکار {} و بستانکار {} است.",
            db::to_fa_digits(total_debit),
            db::to_fa_digits(total_credit)
        ));
    }
    Ok(())
}

fn write_total_amount(
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
fn cell_amount(cell: &Data) -> Result<i64, String> {
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

/* ── Export ─────────────────────────────────────────────────── */

/// Asks for a path and writes the voucher as an `.xlsx` file.
/// `Ok(None)` means the user closed the save dialog.
pub fn export_voucher(input: &VoucherInput, accounts: &[db::Account]) -> Result<Option<String>, String> {
    if input.lines.len() > MAX_IMPORT_LINES {
        return Err(format!(
            "تعداد ردیف‌های خروجی نمی‌تواند بیشتر از {} باشد.",
            db::to_fa_digits(MAX_IMPORT_LINES)
        ));
    }
    let mut checked_debit = 0i64;
    let mut checked_credit = 0i64;
    for (index, line) in input.lines.iter().enumerate() {
        let row_no = db::to_fa_digits(index + 1);
        if line.debit < 0 || line.credit < 0 || line.debit > MAX_AMOUNT || line.credit > MAX_AMOUNT
        {
            return Err(format!("مبلغ ردیف {row_no} خارج از محدوده مجاز است."));
        }
        if line.debit > 0 && line.credit > 0 {
            return Err(format!(
                "در ردیف {row_no} نمی‌توان هم‌زمان بدهکار و بستانکار داشت."
            ));
        }
        if line.debit == 0 && line.credit == 0 {
            return Err(format!("ردیف {row_no} مبلغ بدهکار یا بستانکار ندارد."));
        }
        if line.description.chars().count() > MAX_LINE_DESCRIPTION {
            return Err(format!(
                "شرح ردیف {row_no} بیشتر از {} کاراکتر است.",
                db::to_fa_digits(MAX_LINE_DESCRIPTION)
            ));
        }
        checked_debit = checked_debit
            .checked_add(line.debit)
            .ok_or_else(|| "جمع بدهکار خروجی اکسل از محدوده عددی مجاز بیشتر است.".to_string())?;
        checked_credit = checked_credit
            .checked_add(line.credit)
            .ok_or_else(|| "جمع بستانکار خروجی اکسل از محدوده عددی مجاز بیشتر است.".to_string())?;
    }
    validate_export_balance(checked_debit, checked_credit)?;
    let number = input.number.unwrap_or(0);
    let file_label = if number > 0 {
        number.to_string()
    } else {
        "جدید".to_string()
    };
    let default_name = format!("سند-{file_label}.xlsx");

    let Some(mut path) = rfd::FileDialog::new()
        .set_title("خروجی اکسل سند حسابداری")
        .set_file_name(&default_name)
        .add_filter("Excel", &["xlsx"])
        .save_file()
    else {
        return Ok(None);
    };
    if !path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("xlsx"))
    {
        path.set_extension("xlsx");
    }

    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("سند حسابداری").map_err(xlsx_error)?;
    worksheet.set_right_to_left(true);

    write_sheet(worksheet, input, accounts)?;
    workbook.save(&path).map_err(xlsx_error)?;
    Ok(Some(path.to_string_lossy().to_string()))
}

fn write_sheet(worksheet: &mut Worksheet, input: &VoucherInput, accounts: &[db::Account]) -> Result<(), String> {
    let title = if input.number.unwrap_or(0) > 0 {
        format!(
            "سند حسابداری شماره {}",
            db::to_fa_digits(input.number.unwrap_or(0))
        )
    } else {
        "سند حسابداری جدید".to_string()
    };

    let title_format = Format::new()
        .set_bold()
        .set_font_name("Tahoma")
        .set_font_size(14)
        .set_align(FormatAlign::Center)
        .set_background_color(Color::RGB(0xDCE9F7))
        .set_border(FormatBorder::Thin);
    let label_format = Format::new()
        .set_bold()
        .set_font_name("Tahoma")
        .set_font_size(11)
        .set_background_color(Color::RGB(0xF1F5F9))
        .set_border(FormatBorder::Thin);
    let value_format = Format::new()
        .set_font_name("Tahoma")
        .set_font_size(11)
        .set_border(FormatBorder::Thin);
    let header_format = Format::new()
        .set_bold()
        .set_font_name("Tahoma")
        .set_font_size(11)
        .set_align(FormatAlign::Center)
        .set_background_color(Color::RGB(0xE4EDF5))
        .set_border(FormatBorder::Thin);
    let code_format = Format::new()
        .set_font_name("Tahoma")
        .set_font_size(11)
        .set_align(FormatAlign::Center)
        .set_border(FormatBorder::Thin);
    let description_format = Format::new()
        .set_font_name("Tahoma")
        .set_font_size(11)
        .set_align(FormatAlign::Right)
        .set_border(FormatBorder::Thin);
    let amount_format = Format::new()
        .set_font_name("Tahoma")
        .set_font_size(11)
        .set_num_format("#,##0")
        .set_border(FormatBorder::Thin);
    let total_label_format = Format::new()
        .set_bold()
        .set_font_name("Tahoma")
        .set_font_size(11)
        .set_align(FormatAlign::Center)
        .set_background_color(Color::RGB(0xFDF3D6))
        .set_border(FormatBorder::Thin);
    let total_amount_format = Format::new()
        .set_bold()
        .set_font_name("Tahoma")
        .set_font_size(11)
        .set_num_format("#,##0")
        .set_background_color(Color::RGB(0xFDF3D6))
        .set_border(FormatBorder::Thin);

    worksheet.set_column_width(0, 7).map_err(xlsx_error)?;
    worksheet.set_column_width(1, 15).map_err(xlsx_error)?;
    worksheet.set_column_width(2, 15).map_err(xlsx_error)?;
    worksheet.set_column_width(3, 44).map_err(xlsx_error)?;
    worksheet.set_column_width(4, 18).map_err(xlsx_error)?;
    worksheet.set_column_width(5, 18).map_err(xlsx_error)?;

    worksheet
        .merge_range(0, 0, 0, 5, &title, &title_format)
        .map_err(xlsx_error)?;
    worksheet.set_row_height(0, 26).map_err(xlsx_error)?;

    worksheet
        .write_string_with_format(1, 0, "تاریخ سند :", &label_format)
        .map_err(xlsx_error)?;
    worksheet
        .write_string_with_format(1, 1, db::to_fa_digits(&input.date), &value_format)
        .map_err(xlsx_error)?;
    worksheet
        .write_string_with_format(1, 2, "نوع سند :", &label_format)
        .map_err(xlsx_error)?;
    worksheet
        .merge_range(1, 3, 1, 5, &input.voucher_type, &value_format)
        .map_err(xlsx_error)?;

    worksheet
        .write_string_with_format(2, 0, "شرح سند :", &label_format)
        .map_err(xlsx_error)?;
    worksheet
        .merge_range(2, 1, 2, 5, &input.description, &value_format)
        .map_err(xlsx_error)?;

    let header_row = 4u32;
    for (col, label) in ["ردیف", "کد حساب", "تفصیلی ۱", "شرح", "بدهکار", "بستانکار"]
        .iter()
        .enumerate()
    {
        worksheet
            .write_string_with_format(header_row, col as u16, *label, &header_format)
            .map_err(xlsx_error)?;
    }

    let mut total_debit = 0i64;
    let mut total_credit = 0i64;
    for (index, line) in input.lines.iter().enumerate() {
        let row = header_row + 1 + index as u32;
        worksheet
            .write_number_with_format(row, 0, (index + 1) as f64, &code_format)
            .map_err(xlsx_error)?;
        worksheet
            .write_string_with_format(row, 1, &line.account_code, &code_format)
            .map_err(xlsx_error)?;
        let detail1_name = accounts
            .iter()
            .find(|account| account.code == line.account_code && account.level == 3)
            .map(|account| account.name.as_str())
            .unwrap_or("");
        worksheet
            .write_string_with_format(row, 2, detail1_name, &code_format)
            .map_err(xlsx_error)?;
        worksheet
            .write_string_with_format(row, 3, &line.description, &description_format)
            .map_err(xlsx_error)?;
        if line.debit != 0 {
            worksheet
                .write_number_with_format(row, 4, line.debit as f64, &amount_format)
                .map_err(xlsx_error)?;
        }
        if line.credit != 0 {
            worksheet
                .write_number_with_format(row, 5, line.credit as f64, &amount_format)
                .map_err(xlsx_error)?;
        }
        total_debit = total_debit
            .checked_add(line.debit)
            .ok_or_else(|| "جمع بدهکار خروجی اکسل از محدوده عددی مجاز بیشتر است.".to_string())?;
        total_credit = total_credit
            .checked_add(line.credit)
            .ok_or_else(|| "جمع بستانکار خروجی اکسل از محدوده عددی مجاز بیشتر است.".to_string())?;
    }

    let total_row = header_row + 1 + input.lines.len() as u32;
    worksheet
        .merge_range(total_row, 0, total_row, 3, "جمع کل", &total_label_format)
        .map_err(xlsx_error)?;
    write_total_amount(worksheet, total_row, 4, total_debit, &total_amount_format)?;
    write_total_amount(worksheet, total_row, 5, total_credit, &total_amount_format)?;

    worksheet
        .set_freeze_panes(header_row + 1, 0)
        .map_err(xlsx_error)?;
    Ok(())
}

/* ── Import ─────────────────────────────────────────────────── */

#[derive(Default)]
struct ImportColumns {
    account_code: Option<usize>,
    description: Option<usize>,
    debit: Option<usize>,
    credit: Option<usize>,
}

fn find_columns(range: &calamine::Range<Data>) -> Option<(usize, ImportColumns)> {
    for (row_index, row) in range.rows().enumerate().take(HEADER_SCAN_ROWS) {
        let headers: Vec<String> = row
            .iter()
            .map(|cell| normalize_header(&cell_text(cell)))
            .collect();
        let find = |needles: &[&str]| -> Option<usize> {
            // Prefer an exact alias. Otherwise a header such as «کد سند» could
            // win over «کد حساب» only because it appears in an earlier column.
            for needle in needles {
                let needle = normalize_header(needle);
                if let Some(index) = headers.iter().position(|header| header == &needle) {
                    return Some(index);
                }
            }
            let matches: Vec<usize> = headers
                .iter()
                .enumerate()
                .filter(|(_, header)| {
                    needles
                        .iter()
                        .any(|needle| header.contains(&normalize_header(needle)))
                })
                .map(|(index, _)| index)
                .collect();
            if matches.len() == 1 {
                Some(matches[0])
            } else {
                None
            }
        };

        let columns = ImportColumns {
            account_code: headers
                .iter()
                .position(|header| {
                    header == "کد حساب" || header == "کدحساب" || header == "کد"
                })
                .or_else(|| {
                    let matches: Vec<usize> = headers
                        .iter()
                        .enumerate()
                        .filter(|(_, header)| header.contains("حساب") && header.contains("کد"))
                        .map(|(index, _)| index)
                        .collect();
                    if matches.len() == 1 {
                        Some(matches[0])
                    } else {
                        None
                    }
                }),
            description: find(&["شرح"]),
            debit: find(&["بدهکار", "بدهی"]),
            credit: find(&["بستانکار", "ستانکار"]),
        };

        if columns.account_code.is_some() && columns.debit.is_some() && columns.credit.is_some() {
            return Some((row_index, columns));
        }
    }
    None
}

fn find_bulk_columns(range: &calamine::Range<Data>) -> Option<(usize, BulkImportColumns)> {
    for (row_index, row) in range.rows().enumerate().take(HEADER_SCAN_ROWS) {
        let headers: Vec<String> = row
            .iter()
            .map(|cell| normalize_header(&cell_text(cell)))
            .collect();
        let find = |aliases: &[&str]| {
            aliases.iter().find_map(|alias| {
                let alias = normalize_header(alias);
                headers.iter().position(|header| header == &alias)
            })
        };
        let columns = BulkImportColumns {
            voucher_number: find(&["شماره سند", "شماره‌سند", "سند"]),
            voucher_date: find(&["تاریخ سند", "تاریخ"]),
            voucher_type: find(&["نوع سند"]),
            voucher_description: find(&["شرح سند", "توضیحات سند"]),
            group_code: find(&["کد گروه", "کدگروه"]),
            general_code: find(&["کد کل", "کدکل", "کد حساب کل", "کد حسابِ کل"]),
            general_name: find(&["عنوان کل", "نام کل", "عنوان حساب کل", "نام حساب کل"]),
            general_nature: find(&["ماهیت کل", "ماهیت حساب کل"]),
            subsidiary_code: find(&["کد معین", "کدمعین", "کد حساب معین", "کد حسابِ معین"]),
            subsidiary_name: find(&["عنوان معین", "نام معین", "عنوان حساب معین", "نام حساب معین"]),
            subsidiary_nature: find(&["ماهیت معین", "ماهیت حساب معین"]),
            detail1_code: find(&[
                "کد تفصیلی ۱",
                "کد تفصیلی 1",
                "کد تفصیلی",
                "کد حساب تفصیلی ۱",
                "کد حساب تفصیلی 1",
                "کد حساب تفصیلی",
                "کد حساب تفضیل",
                "کد تفضیل ۱",
                "کد تفضیل 1",
                "کد تفضیل",
                "کد تفضيل",
                "کد تفضیلی ۱",
                "کد تفضیلی 1",
                "کد تفضیلی",
                "کدتفصیلی۱",
                "کدتفصیلی1",
            ]),
            detail1_name: find(&[
                "عنوان تفصیلی ۱",
                "عنوان تفصیلی 1",
                "نام تفصیلی ۱",
                "نام تفصیلی 1",
                "عنوان تفصیلی",
                "نام تفصیلی",
                "عنوان حساب تفصیلی ۱",
                "عنوان حساب تفصیلی 1",
                "نام حساب تفصیلی",
                "عنوان حساب تفضیل",
                "نام حساب تفضیل",
                "عنوان تفضیل ۱",
                "عنوان تفضیل 1",
                "عنوان تفضیل",
                "عنوان تفضيل",
                "نام تفضیل ۱",
                "نام تفضیل 1",
                "نام تفضیل",
                "عنوان تفضیلی ۱",
                "عنوان تفضیلی 1",
                "نام تفضیلی ۱",
                "نام تفضیلی 1",
            ]),
            detail1_nature: find(&[
                "ماهیت تفصیلی ۱",
                "ماهیت تفصیلی 1",
                "ماهیت تفصیلی",
                "ماهیت حساب تفصیلی",
                "ماهیت تفضیل",
                "ماهیت تفضيل",
                "ماهیت تفضیلی ۱",
                "ماهیت تفضیلی 1",
                "ماهیت تفضیلی",
            ]),
            line_description: find(&["شرح ردیف", "توضیحات ردیف", "شرح", "توضیحات"]),
            debit: find(&["بدهکار"]),
            credit: find(&["بستانکار", "ستانکار"]),
        };
        if columns.voucher_number.is_some()
            && columns.voucher_date.is_some()
            && columns.general_code.is_some()
            && columns.general_name.is_some()
            && columns.subsidiary_code.is_some()
            && columns.subsidiary_name.is_some()
            && (columns.detail1_code.is_none() || columns.detail1_name.is_some())
            && columns.debit.is_some()
            && columns.credit.is_some()
        {
            return Some((row_index, columns));
        }
    }
    None
}

fn imported_group_code(general_code: &str) -> Result<&'static str, String> {
    // This mapping follows the application's built-in chart of accounts.
    // The previous mapping treated only 50xx as cost of sales and did not
    // recognize 6xxx/7xxx/8xxx at all, despite those prefixes existing in the
    // seeded chart (for example 5101, 6101, 7101 and 8101).
    let group = match general_code.chars().next() {
        Some('1') if general_code.starts_with("15") || general_code.starts_with("16") => "2",
        Some('1') if general_code.starts_with("18") => "3",
        Some('1') => "1",
        Some('2') if general_code.starts_with("23") || general_code.starts_with("24") => "5",
        Some('2') => "4",
        Some('3') => "6",
        Some('4') => "7",
        Some('5') if general_code.starts_with("525") => "10",
        Some('5') if general_code.starts_with("535") => "11",
        Some('5') if general_code.starts_with("50") || general_code.starts_with("51") => "9",
        Some('5') => "8",
        Some('6') => "8",
        Some('7') => "10",
        Some('8') => "11",
        _ => {
            return Err(format!(
                "پیشوند حساب کل «{}» با هیچ گروه حسابداری شناخته‌شده‌ای تطبیق ندارد؛ ابتدا حساب و گروه آن را در برنامه تعریف کنید.",
                db::to_fa_digits(general_code)
            ));
        }
    };
    Ok(group)
}

fn imported_nature(group_code: &str, level: i64) -> &'static str {
    match group_code {
        "4" | "5" | "6" | "7" => "creditor",
        // The built-in group 10/11 parent accounts are dual, but their child
        // accounts may be income or expense. Do not guess their nature.
        "10" | "11" if level == 1 => "dual",
        "10" | "11" => "unknown",
        _ => "debtor",
    }
}

fn imported_account_nature(
    raw: &str,
    group_code: &str,
    level: i64,
) -> Result<(String, bool), String> {
    let fallback = imported_nature(group_code, level);
    if raw.trim().is_empty() {
        Ok((fallback.to_string(), false))
    } else {
        Ok((account_nature(raw, fallback)?, true))
    }
}

fn add_imported_account(
    accounts: &mut BTreeMap<String, ImportedAccount>,
    code: String,
    name: String,
    parent_code: String,
    group_code: &str,
    level: i64,
    nature: String,
    nature_is_explicit: bool,
) -> Result<(), String> {
    if code.is_empty() {
        return Err("کد حساب در فایل اکسل خالی است.".to_string());
    }
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.chars().count() > 255 {
        return Err(format!(
            "عنوان حساب {} بیشتر از ۲۵۵ کاراکتر است.",
            db::to_fa_digits(&code)
        ));
    }
    let account = ImportedAccount {
        code: code.clone(),
        name: name.clone(),
        parent_code,
        group_code: group_code.to_string(),
        level,
        nature,
        nature_is_explicit,
    };
    if let Some(existing) = accounts.get_mut(&code) {
        if existing.parent_code != account.parent_code
            || existing.level != account.level
            || existing.group_code != account.group_code
        {
            return Err(format!(
                "کد حساب {} در فایل با ساختار (حساب بالاتر، سطح یا گروه) متفاوت آمده است.",
                db::to_fa_digits(code)
            ));
        }
        // Excel often merges repeated account-title cells vertically. A blank
        // repeated title inherits the first value; a different title is an error.
        if !name.is_empty() && existing.name != name {
            return Err(format!(
                "کد حساب {} در فایل اکسل با دو عنوان متفاوت تکرار شده است.",
                db::to_fa_digits(code)
            ));
        }
        if account.nature_is_explicit {
            if existing.nature_is_explicit && existing.nature != account.nature {
                return Err(format!(
                    "ماهیت حساب {} در ردیف‌های اکسل یکسان نیست.",
                    db::to_fa_digits(code)
                ));
            }
            existing.nature = account.nature;
            existing.nature_is_explicit = true;
        }
    } else {
        if name.is_empty() {
            return Err(format!(
                "عنوان حساب با کد «{}» در فایل اکسل خالی است.",
                db::to_fa_digits(&code)
            ));
        }
        accounts.insert(code, account);
    }
    Ok(())
}

/// Selects and parses the first worksheet containing the bulk-voucher layout.
/// Cover sheets and other unrelated tabs are skipped instead of making the
/// import depend on the workbook's tab order.
pub fn import_vouchers() -> Result<Option<BulkVoucherImport>, String> {
    let Some(path) = rfd::FileDialog::new()
        .set_title("ورود دسته‌جمعی اسناد از اکسل")
        .add_filter("Excel", &["xlsx", "xlsm", "xls", "xlsb", "ods"])
        .pick_file()
    else {
        return Ok(None);
    };

    parse_bulk_vouchers(&path).map(Some)
}

fn parse_bulk_vouchers(path: &Path) -> Result<BulkVoucherImport, String> {
    ensure_excel_file_size(path)?;
    let mut workbook =
        open_workbook_auto(path).map_err(|error| format!("فایل اکسل باز نشد: {error}"))?;
    let sheet_names = workbook.sheet_names().to_vec();
    if sheet_names.is_empty() {
        return Err("فایل اکسل هیچ کاربرگی ندارد.".to_string());
    }

    let mut empty_matching_sheet = None;
    for sheet_name in sheet_names {
        let range = workbook
            .worksheet_range(&sheet_name)
            .map_err(|error| format!("خواندن کاربرگ «{sheet_name}» ناموفق بود: {error}"))?;
        if find_bulk_columns(&range).is_none() {
            continue;
        }
        match parse_bulk_voucher_sheet(path, &sheet_name, &range) {
            Ok(import) => return Ok(import),
            Err(error) if error == "در فایل اکسل هیچ ردیف سندی پیدا نشد." => {
                // Skip an empty template tab, but never skip a malformed data
                // tab and silently import a different worksheet instead.
                empty_matching_sheet = Some(format!("کاربرگ «{sheet_name}» خالی است."));
            }
            Err(error) => return Err(format!("کاربرگ «{sheet_name}»: {error}")),
        }
    }

    if let Some(error) = empty_matching_sheet {
        return Err(format!("{error} در سایر کاربرگ‌ها نیز داده قابل ورود پیدا نشد."));
    }
    Err("در هیچ‌یک از کاربرگ‌های فایل، ستون‌های لازم برای ورود دسته‌جمعی اسناد پیدا نشد. ستون‌های شماره سند، تاریخ، کد و عنوان کل، کد و عنوان معین، بدهکار و بستانکار الزامی هستند. در صورت وجود ستون کد تفصیلی ۱، عنوان تفصیلی ۱ نیز باید وجود داشته باشد.".to_string())
}

fn parse_bulk_voucher_sheet(
    path: &Path,
    sheet_name: &str,
    range: &calamine::Range<Data>,
) -> Result<BulkVoucherImport, String> {
    ensure_sheet_dimensions(range, MAX_BULK_IMPORT_ROWS, MAX_BULK_IMPORT_COLUMNS)?;
    let Some((header_row, columns)) = find_bulk_columns(range) else {
        return Err("عنوان ستون‌های لازم در این کاربرگ پیدا نشد.".to_string());
    };
    let read = |row: &[Data], index: Option<usize>| -> Data {
        index
            .and_then(|index| row.get(index))
            .cloned()
            .unwrap_or(Data::Empty)
    };

    let mut vouchers = BTreeMap::<i64, VoucherInput>::new();
    let mut voucher_order = Vec::<i64>::new();
    let mut closed_voucher_numbers = std::collections::HashSet::<i64>::new();
    let mut accounts = BTreeMap::<String, ImportedAccount>::new();
    let mut voucher_types = HashMap::<i64, Option<String>>::new();
    let mut voucher_descriptions = HashMap::<i64, Option<String>>::new();
    let mut last_voucher_number = None::<i64>;

    let first_excel_row = range.start().map(|(row, _)| row as usize).unwrap_or(0);
    for (offset, row) in range.rows().skip(header_row + 1).enumerate() {
        // Range rows are relative to its used area, which may not start at A1.
        let excel_row = first_excel_row + header_row + offset + 2;
        let raw_number = cell_text(&read(row, columns.voucher_number));
        let number = if raw_number.is_empty() {
            if is_total_row(row) {
                continue;
            }
            let value_columns = [
                columns.voucher_date,
                columns.general_code,
                columns.general_name,
                columns.general_nature,
                columns.subsidiary_code,
                columns.subsidiary_name,
                columns.subsidiary_nature,
                columns.detail1_code,
                columns.detail1_name,
                columns.detail1_nature,
                columns.voucher_type,
                columns.voucher_description,
                columns.group_code,
                columns.line_description,
                columns.debit,
                columns.credit,
            ];
            if !has_import_values(row, &value_columns) {
                continue;
            }
            last_voucher_number.ok_or_else(|| {
                format!(
                    "شماره سند ردیف {} اکسل خالی است و سند قبلی برای ادامه وجود ندارد.",
                    db::to_fa_digits(excel_row)
                )
            })?
        } else {
            let parsed = cell_amount(&read(row, columns.voucher_number)).map_err(|error| {
                format!(
                    "شماره سند ردیف {} اکسل: {error}",
                    db::to_fa_digits(excel_row)
                )
            })?;
            if parsed <= 0 || parsed > MAX_VOUCHER_NUMBER {
                return Err(format!(
                    "شماره سند ردیف {} اکسل باید بین ۱ و {} باشد.",
                    db::to_fa_digits(excel_row),
                    db::to_fa_digits(MAX_VOUCHER_NUMBER)
                ));
            }
            if let Some(previous) = last_voucher_number {
                if previous != parsed {
                    closed_voucher_numbers.insert(previous);
                }
            }
            if closed_voucher_numbers.contains(&parsed) {
                return Err(format!(
                    "شماره سند {} پس از شروع سند دیگری دوباره در فایل تکرار شده است؛ ردیف‌های هر سند باید پشت سر هم باشند.",
                    db::to_fa_digits(parsed)
                ));
            }
            last_voucher_number = Some(parsed);
            parsed
        };

        let date_cell = read(row, columns.voucher_date);
        let date = if cell_text(&date_cell).is_empty() {
            vouchers.get(&number).map(|voucher| voucher.date.clone()).ok_or_else(|| {
                format!(
                    "تاریخ سند {} در ردیف {} اکسل خالی است و تاریخ ردیف قبلی برای این سند وجود ندارد.",
                    db::to_fa_digits(number),
                    db::to_fa_digits(excel_row)
                )
            })?
        } else {
            parse_bulk_voucher_date(&date_cell).map_err(|error| {
                format!("تاریخ ردیف {} اکسل: {error}", db::to_fa_digits(excel_row))
            })?
        };

        let general_code = parse_account_code_data_cell(
            &read(row, columns.general_code),
            &format!("کد کل ردیف {} اکسل", db::to_fa_digits(excel_row)),
        )?;
        let general_name = cell_text(&read(row, columns.general_name));
        let subsidiary_cell = read(row, columns.subsidiary_code);
        let subsidiary_code = parse_account_code_data_cell(
            &subsidiary_cell,
            &format!("کد معین ردیف {} اکسل", db::to_fa_digits(excel_row)),
        )?;
        let subsidiary_name = cell_text(&read(row, columns.subsidiary_name));
        let detail1_cell = read(row, columns.detail1_code);
        let raw_detail1_code = cell_text(&detail1_cell);
        let detail1_code = if raw_detail1_code.is_empty() {
            String::new()
        } else {
            parse_account_code_data_cell(
                &detail1_cell,
                &format!("کد تفصیلی ۱ ردیف {} اکسل", db::to_fa_digits(excel_row)),
            )?
        };
        let detail1_name = cell_text(&read(row, columns.detail1_name));
        if !subsidiary_code.starts_with(&general_code)
            || subsidiary_code.len() <= general_code.len()
        {
            return Err(format!(
                "کد معین {} در ردیف {} اکسل باید زیرمجموعه حساب کل {} باشد.",
                db::to_fa_digits(&subsidiary_code),
                db::to_fa_digits(excel_row),
                db::to_fa_digits(&general_code)
            ));
        }
        if detail1_code.is_empty() && !detail1_name.trim().is_empty() {
            return Err(format!(
                "عنوان تفصیلی ۱ در ردیف {} اکسل وارد شده اما کد آن خالی است.",
                db::to_fa_digits(excel_row)
            ));
        }
        if !detail1_code.is_empty()
            && (!detail1_code.starts_with(&subsidiary_code)
                || detail1_code.len() <= subsidiary_code.len())
        {
            return Err(format!(
                "کد تفصیلی ۱ {} در ردیف {} اکسل باید زیرمجموعه حساب معین {} باشد.",
                db::to_fa_digits(&detail1_code),
                db::to_fa_digits(excel_row),
                db::to_fa_digits(&subsidiary_code)
            ));
        }

        let raw_group_code = cell_text(&read(row, columns.group_code));
        let group_code = if raw_group_code.trim().is_empty() {
            // A vertically merged group-code cell is populated only on the
            // first row. Reuse the group already assigned to this general
            // account before falling back to the built-in prefix mapping.
            if let Some(existing) = accounts.get(&general_code) {
                existing.group_code.clone()
            } else {
                imported_group_code(&general_code)?.to_string()
            }
        } else {
            let parsed = parse_code(&raw_group_code, "کد گروه")?;
            if parsed.len() > 4 {
                return Err("کد گروه در فایل اکسل نمی‌تواند بیشتر از ۴ رقم باشد.".to_string());
            }
            parsed
        };
        let (general_nature, general_nature_is_explicit) = imported_account_nature(
            &cell_text(&read(row, columns.general_nature)),
            &group_code,
            1,
        )?;
        let (subsidiary_nature, subsidiary_nature_is_explicit) = imported_account_nature(
            &cell_text(&read(row, columns.subsidiary_nature)),
            &group_code,
            2,
        )?;
        add_imported_account(
            &mut accounts,
            general_code.clone(),
            general_name,
            String::new(),
            &group_code,
            1,
            general_nature,
            general_nature_is_explicit,
        )?;
        add_imported_account(
            &mut accounts,
            subsidiary_code.clone(),
            subsidiary_name,
            general_code.clone(),
            &group_code,
            2,
            subsidiary_nature,
            subsidiary_nature_is_explicit,
        )?;
        if accounts.len() > MAX_ACCOUNT_IMPORT_ROWS {
            return Err(format!(
                "تعداد حساب‌های یکتای فایل بیش از حد مجاز است (حداکثر {} حساب).",
                db::to_fa_digits(MAX_ACCOUNT_IMPORT_ROWS)
            ));
        }

        let account_code = if !detail1_code.is_empty() {
            let (detail1_nature, detail1_nature_is_explicit) = imported_account_nature(
                &cell_text(&read(row, columns.detail1_nature)),
                &group_code,
                3,
            )?;
            add_imported_account(
                &mut accounts,
                detail1_code.clone(),
                detail1_name,
                subsidiary_code,
                &group_code,
                3,
                detail1_nature,
                detail1_nature_is_explicit,
            )?;
            detail1_code
        } else {
            subsidiary_code
        };
        if accounts.len() > MAX_ACCOUNT_IMPORT_ROWS {
            return Err(format!(
                "تعداد حساب‌های یکتای فایل بیش از حد مجاز است (حداکثر {} حساب).",
                db::to_fa_digits(MAX_ACCOUNT_IMPORT_ROWS)
            ));
        }

        let line_description = cell_text(&read(row, columns.line_description));
        if line_description.chars().count() > MAX_LINE_DESCRIPTION {
            return Err(format!(
                "شرح ردیف {} اکسل بیشتر از {} کاراکتر است.",
                db::to_fa_digits(excel_row),
                db::to_fa_digits(MAX_LINE_DESCRIPTION)
            ));
        }
        let debit = cell_amount(&read(row, columns.debit)).map_err(|error| {
            format!("بدهکار ردیف {} اکسل: {error}", db::to_fa_digits(excel_row))
        })?;
        let credit = cell_amount(&read(row, columns.credit)).map_err(|error| {
            format!(
                "بستانکار ردیف {} اکسل: {error}",
                db::to_fa_digits(excel_row)
            )
        })?;
        if debit < 0 || credit < 0 || debit > MAX_AMOUNT || credit > MAX_AMOUNT {
            return Err(format!(
                "مبلغ ردیف {} اکسل خارج از محدوده مجاز است.",
                db::to_fa_digits(excel_row)
            ));
        }
        if debit == 0 && credit == 0 {
            return Err(format!(
                "ردیف {} اکسل مبلغ بدهکار یا بستانکار ندارد.",
                db::to_fa_digits(excel_row)
            ));
        }
        if debit > 0 && credit > 0 {
            return Err(format!(
                "در ردیف {} اکسل هم بدهکار و هم بستانکار مقدار دارد.",
                db::to_fa_digits(excel_row)
            ));
        }

        let imported_type = columns
            .voucher_type
            .map(|index| cell_text(&read(row, Some(index))))
            .filter(|value| !value.trim().is_empty())
            .map(|value| normalize_imported_voucher_type(&value))
            .transpose()?;
        let imported_description = columns
            .voucher_description
            .map(|index| cell_text(&read(row, Some(index))))
            .filter(|value| !value.trim().is_empty());
        if let Some(description) = imported_description.as_ref() {
            if description.chars().count() > crate::vouchers::MAX_DESCRIPTION {
                return Err(format!(
                    "شرح سند {} در ردیف {} اکسل بیشتر از {} کاراکتر است.",
                    db::to_fa_digits(number),
                    db::to_fa_digits(excel_row),
                    db::to_fa_digits(crate::vouchers::MAX_DESCRIPTION)
                ));
            }
        }

        if !vouchers.contains_key(&number) {
            voucher_order.push(number);
        }
        let voucher = vouchers.entry(number).or_insert_with(|| VoucherInput {
            id: None,
            number: Some(number),
            date: date.clone(),
            voucher_type: crate::vouchers::VOUCHER_TYPES[0].to_string(),
            description: String::new(),
            lines: Vec::new(),
        });
        if voucher.date != date {
            return Err(format!(
                "تاریخ سند {} در ردیف‌های اکسل یکسان نیست.",
                db::to_fa_digits(number)
            ));
        }

        if let Some(voucher_type) = imported_type {
            let recorded_type = voucher_types.entry(number).or_default();
            if recorded_type
                .as_ref()
                .is_some_and(|existing| existing != &voucher_type)
            {
                return Err(format!(
                    "نوع سند {} در ردیف‌های اکسل یکسان نیست.",
                    db::to_fa_digits(number)
                ));
            }
            *recorded_type = Some(voucher_type.clone());
            voucher.voucher_type = voucher_type;
        }

        if let Some(description) = imported_description {
            let recorded_description = voucher_descriptions.entry(number).or_default();
            if recorded_description
                .as_ref()
                .is_some_and(|existing| existing != &description)
            {
                return Err(format!(
                    "شرح سند {} در ردیف‌های اکسل یکسان نیست.",
                    db::to_fa_digits(number)
                ));
            }
            *recorded_description = Some(description.clone());
            voucher.description = description;
        }

        voucher.lines.push(VoucherLineInput {
            id: None,
            account_code,
            description: line_description,
            debit,
            credit,
        });
        if voucher.lines.len() > MAX_IMPORT_LINES {
            return Err(format!(
                "تعداد ردیف‌های سند {} بیشتر از {} است.",
                db::to_fa_digits(number),
                db::to_fa_digits(MAX_IMPORT_LINES)
            ));
        }
    }

    if vouchers.is_empty() {
        return Err("در فایل اکسل هیچ ردیف سندی پیدا نشد.".to_string());
    }
    for voucher in vouchers.values() {
        if voucher.lines.len() < 2 {
            return Err(format!(
                "سند {} کمتر از دو ردیف دارد.",
                db::to_fa_digits(voucher.number.unwrap_or_default())
            ));
        }
        let debit = voucher
            .lines
            .iter()
            .try_fold(0i64, |sum, line| sum.checked_add(line.debit))
            .ok_or_else(|| "جمع بدهکار یک سند از محدوده عددی مجاز بیشتر است.".to_string())?;
        let credit = voucher
            .lines
            .iter()
            .try_fold(0i64, |sum, line| sum.checked_add(line.credit))
            .ok_or_else(|| "جمع بستانکار یک سند از محدوده عددی مجاز بیشتر است.".to_string())?;
        if debit != credit {
            return Err(format!(
                "سند {} تراز نیست؛ بدهکار {} و بستانکار {} است.",
                db::to_fa_digits(voucher.number.unwrap_or_default()),
                db::to_fa_digits(debit),
                db::to_fa_digits(credit)
            ));
        }
    }

    Ok(BulkVoucherImport {
        file_name: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("اکسل")
            .to_string(),
        sheet_name: sheet_name.to_string(),
        vouchers: voucher_order
            .into_iter()
            .filter_map(|number| vouchers.remove(&number))
            .collect(),
        accounts: accounts.into_values().collect(),
    })
}

/// Asks for a workbook and returns the voucher lines it contains.
/// Only the first worksheet is read.
pub fn import_voucher() -> Result<Option<Vec<VoucherLineInput>>, String> {
    let Some(path) = rfd::FileDialog::new()
        .set_title("دریافت سند از اکسل")
        .add_filter("Excel", &["xlsx", "xlsm", "xls", "xlsb", "ods"])
        .pick_file()
    else {
        return Ok(None);
    };

    ensure_excel_file_size(&path)?;
    let mut workbook =
        open_workbook_auto(&path).map_err(|error| format!("فایل اکسل باز نشد: {error}"))?;
    let range = match workbook.worksheet_range_at(0) {
        Some(Ok(range)) => range,
        Some(Err(error)) => return Err(format!("خواندن کاربرگ اکسل ناموفق بود: {error}")),
        None => return Err("فایل اکسل هیچ کاربرگی ندارد.".to_string()),
    };

    let Some((header_row, columns)) = find_columns(&range) else {
        return Err(
            "ردیف عنوان ستون‌ها پیدا نشد. فایل باید ستون‌های «کد حساب»، «شرح»، «بدهکار» و «بستانکار» داشته باشد."
                .to_string(),
        );
    };

    ensure_sheet_dimensions(&range, MAX_VOUCHER_IMPORT_ROWS, MAX_SHEET_COLUMNS)?;

    let read = |row: &[Data], index: Option<usize>| -> Data {
        index
            .and_then(|index| row.get(index))
            .cloned()
            .unwrap_or(Data::Empty)
    };

    let mut lines: Vec<VoucherLineInput> = Vec::new();
    let first_excel_row = range.start().map(|(row, _)| row as usize).unwrap_or(0);
    for (offset, row) in range.rows().skip(header_row + 1).enumerate() {
        // Range rows are relative to its used area, which may not start at A1.
        let excel_row = first_excel_row + header_row + offset + 2;
        let account_code_cell = read(row, columns.account_code);
        let raw_account_code = cell_text(&account_code_cell);
        if raw_account_code.is_empty() {
            if is_total_row(row) {
                continue;
            }
            let value_columns = [columns.description, columns.debit, columns.credit];
            if !has_import_values(row, &value_columns) {
                continue;
            }
            return Err(format!(
                "کد حساب ردیف {} اکسل الزامی است.",
                db::to_fa_digits(excel_row)
            ));
        }
        let account_code = parse_account_code_data_cell(
            &account_code_cell,
            &format!("کد حساب ردیف {} اکسل", db::to_fa_digits(excel_row)),
        )?;

        let description = cell_text(&read(row, columns.description));
        if description.chars().count() > MAX_LINE_DESCRIPTION {
            return Err(format!(
                "شرح ردیف {} اکسل بیشتر از {} کاراکتر است.",
                db::to_fa_digits(excel_row),
                db::to_fa_digits(MAX_LINE_DESCRIPTION)
            ));
        }

        let debit = cell_amount(&read(row, columns.debit)).map_err(|error| {
            format!("بدهکار ردیف {} اکسل: {error}", db::to_fa_digits(excel_row))
        })?;
        let credit = cell_amount(&read(row, columns.credit)).map_err(|error| {
            format!(
                "بستانکار ردیف {} اکسل: {error}",
                db::to_fa_digits(excel_row)
            )
        })?;
        if debit < 0 || credit < 0 {
            return Err(format!(
                "مبلغ ردیف {} اکسل نمی‌تواند منفی باشد.",
                db::to_fa_digits(excel_row)
            ));
        }
        if debit > MAX_AMOUNT || credit > MAX_AMOUNT {
            return Err(format!(
                "مبلغ ردیف {} اکسل نمی‌تواند بیشتر از {} ریال باشد.",
                db::to_fa_digits(excel_row),
                db::to_fa_digits(MAX_AMOUNT)
            ));
        }
        if debit > 0 && credit > 0 {
            return Err(format!(
                "در ردیف {} اکسل هم بدهکار و هم بستانکار مقدار دارد.",
                db::to_fa_digits(excel_row)
            ));
        }

        lines.push(VoucherLineInput {
            id: None,
            account_code,
            description,
            debit,
            credit,
        });
        if lines.len() > MAX_IMPORT_LINES {
            return Err(format!(
                "تعداد ردیف‌های اکسل نمی‌تواند بیشتر از {} باشد.",
                db::to_fa_digits(MAX_IMPORT_LINES)
            ));
        }
    }

    if lines.is_empty() {
        return Err(format!(
            "در فایل «{}» هیچ ردیف سندی با کد حساب پیدا نشد.",
            Path::new(&path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("اکسل")
        ));
    }
    Ok(Some(lines))
}

/* ── Chart of accounts import/export (WebApp-compatible layout) ── */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountImportResult {
    pub created_count: usize,
    pub skipped_count: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountExportResult {
    pub path: String,
    pub skipped_codes: Vec<String>,
}

#[derive(Default)]
struct AccountImportColumns {
    group_code: Option<usize>,
    legacy_nature: Option<usize>,
    level1_code: Option<usize>,
    level1_name: Option<usize>,
    level1_nature: Option<usize>,
    level2_code: Option<usize>,
    level2_name: Option<usize>,
    level2_nature: Option<usize>,
    level3_code: Option<usize>,
    level3_name: Option<usize>,
    level3_nature: Option<usize>,
}

fn find_account_columns(range: &calamine::Range<Data>) -> Option<(usize, AccountImportColumns)> {
    for (row_index, row) in range.rows().enumerate().take(HEADER_SCAN_ROWS) {
        let headers: Vec<String> = row
            .iter()
            .map(|cell| normalize_header(&cell_text(cell)))
            .collect();
        let find = |aliases: &[&str]| {
            aliases.iter().find_map(|alias| {
                let normalized = normalize_header(alias);
                headers.iter().position(|header| header == &normalized)
            })
        };
        let columns = AccountImportColumns {
            group_code: find(&["کد گروه", "کدگروه"]),
            legacy_nature: find(&["ماهیت"]),
            level1_code: find(&["کد کل", "کدکل"]),
            level1_name: find(&["عنوان کل", "نام کل", "عنوانکل"]),
            level1_nature: find(&["ماهیت کل", "ماهیتکل"]),
            level2_code: find(&["کد معین", "کدمعین"]),
            level2_name: find(&["عنوان معین", "نام معین", "عنوانمعین"]),
            level2_nature: find(&["ماهیت معین", "ماهیت‌معین", "ماهیت‌معین"]),
            level3_code: find(&["کد تفصیلی 1", "کد تفصیلی۱", "کد تفصیلی", "کدتفصیلی1"]),
            level3_name: find(&[
                "عنوان تفصیلی 1",
                "عنوان تفصیلی۱",
                "عنوان تفصیلی",
                "نام تفصیلی 1",
            ]),
            level3_nature: find(&["ماهیت تفصیلی 1", "ماهیت تفصیلی۱", "ماهیت تفصیلی"]),
        };
        if columns.group_code.is_some()
            && columns.level1_code.is_some()
            && columns.level1_name.is_some()
        {
            return Some((row_index, columns));
        }
    }
    None
}

fn account_nature_label(value: &str) -> &str {
    match value {
        "creditor" => "بستانکار",
        "dual" => "دوگانه",
        _ => "بدهکار",
    }
}

fn account_nature(raw: &str, fallback: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(if fallback.is_empty() {
            "debtor".to_string()
        } else {
            fallback.to_string()
        });
    }
    if value.contains("بستانکار") {
        Ok("creditor".to_string())
    } else if value.contains("بدهکار") {
        Ok("debtor".to_string())
    } else if value.contains("دوگانه") {
        Ok("dual".to_string())
    } else {
        Err(format!(
            "ماهیت «{value}» معتبر نیست؛ بدهکار، بستانکار یا دوگانه وارد کنید."
        ))
    }
}

/// Export the chart in the same 11-column hierarchy layout used by WebApp.
pub fn export_accounts(database: &db::Database) -> Result<Option<AccountExportResult>, String> {
    let accounts = database.list_accounts()?.accounts;
    let skipped_codes: Vec<String> = accounts
        .iter()
        .filter(|account| account.level > 3)
        .map(|account| account.code.clone())
        .collect();
    let by_code: BTreeMap<String, db::Account> = accounts
        .iter()
        .cloned()
        .map(|account| (account.code.clone(), account))
        .collect();

    let Some(mut path) = rfd::FileDialog::new()
        .set_title("خروجی اکسل فهرست حساب‌ها")
        .set_file_name("لیست-حساب‌ها.xlsx")
        .add_filter("Excel", &["xlsx"])
        .save_file()
    else {
        return Ok(None);
    };
    if !path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("xlsx"))
    {
        path.set_extension("xlsx");
    }

    let headers = [
        "کد گروه",
        "ماهیت",
        "کد کل",
        "عنوان کل",
        "ماهیت کل",
        "کد معین",
        "عنوان معین",
        "ماهیت معین",
        "کد تفصیلی 1",
        "عنوان تفصیلی 1",
        "ماهیت تفصیلی 1",
    ];
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("حساب‌ها").map_err(xlsx_error)?;
    worksheet.set_right_to_left(true);
    for (column, header) in headers.iter().enumerate() {
        worksheet
            .write_string(0, column as u16, *header)
            .map_err(xlsx_error)?;
    }

    // One row per terminal account (up to WebApp's three supported levels).
    // Repeated parent data is intentional: it makes every row independently importable.
    let parent_codes: std::collections::BTreeSet<&str> = accounts
        .iter()
        .filter(|account| account.level <= 3)
        .map(|account| account.parent_code.as_str())
        .filter(|code| !code.is_empty())
        .collect();
    let mut terminals: Vec<&db::Account> = accounts
        .iter()
        .filter(|account| account.level <= 3 && !parent_codes.contains(account.code.as_str()))
        .collect();
    terminals.sort_by(|a, b| a.code.cmp(&b.code));

    for (index, terminal) in terminals.iter().enumerate() {
        let mut chain: Vec<&db::Account> = Vec::new();
        let mut current = Some(*terminal);
        let mut visited = std::collections::BTreeSet::new();
        while let Some(account) = current {
            if !visited.insert(account.code.clone()) {
                return Err(format!(
                    "چرخه در ساختار حساب «{}» شناسایی شد.",
                    db::to_fa_digits(&account.code)
                ));
            }
            chain.push(account);
            current = if account.parent_code.is_empty() {
                None
            } else {
                by_code.get(&account.parent_code)
            };
        }
        chain.reverse();
        if chain.is_empty() || chain[0].level != 1 {
            continue;
        }
        let mut values = vec![String::new(); headers.len()];
        values[0] = chain[0].group_code.clone();
        values[1] = account_nature_label(&chain[0].nature).to_string();
        for account in chain {
            let offset = match account.level {
                1 => 2,
                2 => 5,
                3 => 8,
                _ => continue,
            };
            values[offset] = account.code.clone();
            values[offset + 1] = account.name.clone();
            values[offset + 2] = account_nature_label(&account.nature).to_string();
        }
        let row = index as u32 + 1;
        for (column, value) in values.iter().enumerate() {
            worksheet
                .write_string(row, column as u16, value.as_str())
                .map_err(xlsx_error)?;
        }
    }
    worksheet.set_column_width(0, 12).map_err(xlsx_error)?;
    for column in 1..headers.len() {
        worksheet
            .set_column_width(column as u16, if column % 3 == 0 { 28 } else { 16 })
            .map_err(xlsx_error)?;
    }
    worksheet.set_freeze_panes(1, 0).map_err(xlsx_error)?;
    workbook.save(&path).map_err(xlsx_error)?;
    Ok(Some(AccountExportResult {
        path: path.to_string_lossy().to_string(),
        skipped_codes,
    }))
}

/// Import accounts from the WebApp 11-column workbook. Each hierarchy level is
/// validated and saved before its children; existing matching accounts are skipped.
pub fn import_accounts(database: &db::Database) -> Result<Option<AccountImportResult>, String> {
    let Some(path) = rfd::FileDialog::new()
        .set_title("ورودی اکسل حساب‌ها")
        .add_filter("Excel", &["xlsx"])
        .pick_file()
    else {
        return Ok(None);
    };

    ensure_excel_file_size(&path)?;
    let mut workbook =
        open_workbook_auto(&path).map_err(|error| format!("فایل اکسل باز نشد: {error}"))?;
    let range = match workbook.worksheet_range_at(0) {
        Some(Ok(range)) => range,
        Some(Err(error)) => return Err(format!("خواندن کاربرگ اکسل ناموفق بود: {error}")),
        None => return Err("فایل اکسل هیچ کاربرگی ندارد.".to_string()),
    };
    ensure_sheet_dimensions(&range, MAX_ACCOUNT_IMPORT_ROWS, MAX_SHEET_COLUMNS)?;
    let Some((header_row, columns)) = find_account_columns(&range) else {
        return Err("ستون‌های فایل با قالب حساب‌های WebApp مطابقت ندارد؛ ستون‌های «کد گروه»، «کد کل» و «عنوان کل» الزامی هستند.".to_string());
    };
    let read = |row: &[Data], index: Option<usize>| -> String {
        index
            .and_then(|column| row.get(column))
            .map(cell_text)
            .unwrap_or_default()
    };
    let read_code_cell = |row: &[Data], index: Option<usize>| -> Data {
        index
            .and_then(|column| row.get(column))
            .cloned()
            .unwrap_or(Data::Empty)
    };

    let mut accounts = database.list_accounts()?.accounts;
    let groups = database.list_account_groups()?.groups;
    let group_codes: std::collections::BTreeSet<String> =
        groups.into_iter().map(|group| group.code).collect();
    let mut created_count = 0usize;
    let mut skipped_count = 0usize;
    let mut errors = Vec::new();
    let mut error_count = 0usize;

    let first_excel_row = range.start().map(|(row, _)| row as usize).unwrap_or(0);
    for (offset, row) in range.rows().skip(header_row + 1).enumerate() {
        // Range rows are relative to its used area, which may not start at A1.
        let excel_row = first_excel_row + header_row + offset + 2;
        let raw_group_code = read(row, columns.group_code);
        let legacy_nature = read(row, columns.legacy_nature);
        let account_code_cells = [
            read_code_cell(row, columns.level1_code),
            read_code_cell(row, columns.level2_code),
            read_code_cell(row, columns.level3_code),
        ];
        let cells = [
            (
                read(row, columns.level1_code),
                read(row, columns.level1_name),
                read(row, columns.level1_nature).if_empty(&legacy_nature),
            ),
            (
                read(row, columns.level2_code),
                read(row, columns.level2_name),
                read(row, columns.level2_nature),
            ),
            (
                read(row, columns.level3_code),
                read(row, columns.level3_name),
                read(row, columns.level3_nature),
            ),
        ];
        if cells
            .iter()
            .all(|cell| code_text(&cell.0).is_empty() && cell.1.trim().is_empty())
        {
            continue;
        }
        let row_result = (|| -> Result<(usize, usize), String> {
            let group_code = parse_code(&raw_group_code, "کد گروه")?;
            if !group_codes.contains(&group_code) {
                return Err(format!(
                    "کد گروه «{}» در ردیف اکسل معتبر نیست.",
                    db::to_fa_digits(&group_code)
                ));
            }
            if cells[0].1.trim().is_empty() {
                return Err("عنوان حساب کل خالی است.".to_string());
            }
            let first_empty = cells.iter().position(|cell| cell.1.trim().is_empty());
            if let Some(empty_at) = first_empty {
                if cells
                    .iter()
                    .skip(empty_at + 1)
                    .any(|cell| !cell.1.trim().is_empty())
                {
                    return Err("سطوح حساب باید به‌ترتیب و بدون فاصله تکمیل شوند.".to_string());
                }
            }
            for (level_index, (_, raw_name, _)) in cells.iter().enumerate() {
                let level = level_index + 1;
                let has_code = !cell_text(&account_code_cells[level_index]).is_empty();
                let has_name = !raw_name.trim().is_empty();
                if has_code && !has_name {
                    return Err(format!(
                        "کد سطح {level} وارد شده اما عنوان آن خالی است."
                    ));
                }
                if has_name && !has_code {
                    return Err(format!(
                        "عنوان سطح {level} وارد شده اما کد آن خالی است."
                    ));
                }
            }
            let mut parent_code = String::new();
            let mut parent_nature = "debtor".to_string();
            let mut row_created = 0usize;
            let mut row_skipped = 0usize;
            let mut pending: Vec<db::NewAccount> = Vec::new();
            for (level_index, (_, raw_name, raw_nature)) in cells.iter().enumerate() {
                if raw_name.trim().is_empty() {
                    break;
                }
                let level = level_index as i64 + 1;
                let code = parse_account_code_data_cell(
                    &account_code_cells[level_index],
                    &format!("کد سطح {level}"),
                )?;
                let name = raw_name.split_whitespace().collect::<Vec<_>>().join(" ");
                if name.chars().count() > 255 {
                    return Err(format!("نام حساب «{}» بیش از ۲۵۵ کاراکتر است.", name));
                }
                if level > 1 && !code.starts_with(&parent_code) {
                    return Err(format!(
                        "کد «{}» با کد حساب بالاتر «{}» آغاز نمی‌شود.",
                        db::to_fa_digits(&code),
                        db::to_fa_digits(&parent_code)
                    ));
                }
                let nature = account_nature(raw_nature, &parent_nature)?;
                if let Some(existing) = accounts.iter().find(|account| account.code == code) {
                    if existing.level != level
                        || existing.group_code != group_code
                        || existing.parent_code != parent_code
                        || existing.name != name
                        || existing.nature != nature
                    {
                        return Err(format!(
                            "کد «{}» قبلاً با مشخصات متفاوت ثبت شده است.",
                            db::to_fa_digits(&code)
                        ));
                    }
                    skipped_count += 1;
                    row_skipped += 1;
                    parent_code = code;
                    parent_nature = nature;
                    continue;
                }
                pending.push(db::NewAccount::new(
                    &code,
                    &name,
                    level,
                    &group_code,
                    &nature,
                    &parent_code,
                    level > 1,
                ));
                parent_code = code;
                parent_nature = nature;
            }
            if !pending.is_empty() {
                let created = database.create_accounts_atomic(pending)?;
                row_created += created.len();
                created_count += created.len();
                accounts.extend(created);
            }
            Ok((row_created, row_skipped))
        })();
        if let Err(error) = row_result {
            error_count += 1;
            if errors.len() < MAX_IMPORT_ERRORS {
                errors.push(format!("ردیف {}: {}", db::to_fa_digits(excel_row), error));
            }
        }
    }

    if error_count > errors.len() {
        errors.push(format!(
            "{} خطای دیگر ثبت نشد؛ مجموع خطاها {} مورد است.",
            db::to_fa_digits(error_count - errors.len()),
            db::to_fa_digits(error_count)
        ));
    }

    Ok(Some(AccountImportResult {
        created_count,
        skipped_count,
        errors,
    }))
}

trait EmptyFallback {
    fn if_empty(self, fallback: &str) -> String;
}
impl EmptyFallback for String {
    fn if_empty(self, fallback: &str) -> String {
        if self.trim().is_empty() {
            fallback.to_string()
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Data {
        Data::String(value.to_string())
    }

    #[test]
    fn account_workbook_headers_match_webapp_template() {
        let mut range = calamine::Range::new((0, 0), (0, 10));
        for (column, value) in [
            "کد گروه",
            "ماهیت",
            "کد کل",
            "عنوان کل",
            "ماهیت کل",
            "کد معین",
            "عنوان معین",
            "ماهیت معین",
            "کد تفصیلی 1",
            "عنوان تفصیلی 1",
            "ماهیت تفصیلی 1",
        ]
        .iter()
        .enumerate()
        {
            range.set_value((0, column as u32), text(value));
        }
        let (row, columns) = find_account_columns(&range).expect("WebApp account header row");
        assert_eq!(row, 0);
        assert_eq!(columns.group_code, Some(0));
        assert_eq!(columns.level1_code, Some(2));
        assert_eq!(columns.level2_code, Some(5));
        assert_eq!(columns.level3_code, Some(8));
    }

    #[test]
    fn bulk_voucher_headers_include_optional_metadata_and_detail1_columns() {
        let mut range = calamine::Range::new((0, 0), (0, 16));
        for (column, value) in [
            "شماره سند",
            "تاریخ سند",
            "نوع سند",
            "شرح سند",
            "کد گروه",
            "کد کل",
            "عنوان کل",
            "ماهیت کل",
            "کد معین",
            "عنوان معین",
            "ماهیت معین",
            "کد تفصیلی ۱",
            "عنوان تفصیلی ۱",
            "ماهیت تفصیلی ۱",
            "شرح ردیف",
            "بدهکار",
            "بستانکار",
        ]
        .iter()
        .enumerate()
        {
            range.set_value((0, column as u32), text(value));
        }
        let (row, columns) = find_bulk_columns(&range).expect("bulk voucher headers");
        assert_eq!(row, 0);
        assert_eq!(columns.voucher_type, Some(2));
        assert_eq!(columns.voucher_description, Some(3));
        assert_eq!(columns.group_code, Some(4));
        assert_eq!(columns.general_code, Some(5));
        assert_eq!(columns.general_nature, Some(7));
        assert_eq!(columns.subsidiary_code, Some(8));
        assert_eq!(columns.subsidiary_nature, Some(10));
        assert_eq!(columns.detail1_code, Some(11));
        assert_eq!(columns.detail1_name, Some(12));
        assert_eq!(columns.detail1_nature, Some(13));
        assert_eq!(columns.line_description, Some(14));
        assert_eq!(columns.debit, Some(15));
        assert_eq!(columns.credit, Some(16));
    }

    #[test]
    fn bulk_import_recognizes_tafzil_column_spellings() {
        let mut range = calamine::Range::new((0, 0), (0, 9));
        for (column, value) in [
            "شماره سند",
            "تاریخ",
            "کد کل",
            "عنوان کل",
            "کد معین",
            "عنوان معین",
            "کد تفضیل",
            "عنوان تفضیل",
            "بدهکار",
            "بستانکار",
        ]
        .iter()
        .enumerate()
        {
            range.set_value((0, column as u32), text(value));
        }
        let (_, columns) = find_bulk_columns(&range).expect("bulk import headers");
        assert_eq!(columns.detail1_code, Some(6));
        assert_eq!(columns.detail1_name, Some(7));
    }

    #[test]
    fn bulk_import_requires_a_subsidiary_code_column() {
        let mut range = calamine::Range::new((0, 0), (0, 6));
        for (column, value) in [
            "شماره سند",
            "تاریخ سند",
            "کد کل",
            "عنوان کل",
            "عنوان معین",
            "بدهکار",
            "بستانکار",
        ]
        .iter()
        .enumerate()
        {
            range.set_value((0, column as u32), text(value));
        }
        assert!(find_bulk_columns(&range).is_none());
    }

    #[test]
    fn bulk_import_requires_account_title_columns() {
        let mut range = calamine::Range::new((0, 0), (0, 5));
        for (column, value) in ["شماره سند", "تاریخ سند", "کد کل", "کد معین", "بدهکار", "بستانکار"]
            .iter()
            .enumerate()
        {
            range.set_value((0, column as u32), text(value));
        }
        assert!(find_bulk_columns(&range).is_none());
    }

    #[test]
    fn repeated_account_codes_allow_merged_blank_titles_but_reject_conflicts() {
        let mut accounts = BTreeMap::new();
        add_imported_account(
            &mut accounts,
            "1101".to_string(),
            "حساب آزمایشی".to_string(),
            String::new(),
            "1",
            1,
            "debtor".to_string(),
            false,
        )
        .unwrap();
        add_imported_account(
            &mut accounts,
            "1101".to_string(),
            String::new(),
            String::new(),
            "1",
            1,
            "debtor".to_string(),
            false,
        )
        .unwrap();
        assert!(add_imported_account(
            &mut accounts,
            "1101".to_string(),
            "عنوان متفاوت".to_string(),
            String::new(),
            "1",
            1,
            "debtor".to_string(),
            false,
        )
        .is_err());
    }

    #[test]
    fn bulk_import_accepts_an_explicit_group_code_for_custom_account_codes() {
        let headers = [
            "شماره سند",
            "تاریخ سند",
            "کد گروه",
            "کد کل",
            "عنوان کل",
            "کد معین",
            "عنوان معین",
            "ماهیت معین",
            "بدهکار",
            "بستانکار",
        ];
        let mut range = calamine::Range::new((0, 0), (3, 9));
        for (column, value) in headers.iter().enumerate() {
            range.set_value((0, column as u32), text(value));
        }
        let first_row = [
            text("1"),
            text("1405/01/01"),
            text("10"),
            text("9901"),
            text("سایر درآمدها و هزینه‌های سفارشی"),
            text("990101"),
            text("هزینه عملیاتی سفارشی"),
            text("بدهکار"),
            Data::Int(100),
            Data::Empty,
        ];
        let merged_group_row = [
            text("1"),
            text("1405/01/01"),
            Data::Empty,
            text("9901"),
            Data::Empty,
            text("990102"),
            text("هزینه عملیاتی دوم"),
            text("بدهکار"),
            Data::Int(50),
            Data::Empty,
        ];
        let second_row = [
            text("1"),
            text("1405/01/01"),
            text("7"),
            text("4101"),
            text("فروش و درآمدها"),
            text("410101"),
            text("فروش محصولات"),
            text("بستانکار"),
            Data::Empty,
            Data::Int(150),
        ];
        for (column, value) in first_row.into_iter().enumerate() {
            range.set_value((1, column as u32), value);
        }
        for (column, value) in merged_group_row.into_iter().enumerate() {
            range.set_value((2, column as u32), value);
        }
        for (column, value) in second_row.into_iter().enumerate() {
            range.set_value((3, column as u32), value);
        }

        let import = parse_bulk_voucher_sheet(Path::new("custom.xlsx"), "اسناد", &range).unwrap();
        let custom_parent = import.accounts.iter().find(|account| account.code == "9901").unwrap();
        let custom_child = import.accounts.iter().find(|account| account.code == "990101").unwrap();
        assert_eq!(custom_parent.group_code, "10");
        assert_eq!(custom_child.group_code, "10");
        assert_eq!(custom_child.nature, "debtor");
        assert!(custom_child.nature_is_explicit);
        let merged_group_child = import.accounts.iter().find(|account| account.code == "990102").unwrap();
        assert_eq!(merged_group_child.group_code, "10");
        assert_eq!(import.vouchers.len(), 1);
        assert_eq!(import.vouchers[0].lines.len(), 3);
        assert!(imported_group_code("9901").is_err());
    }

    #[test]
    fn bulk_import_accepts_safe_numeric_account_codes_but_rejects_unsafe_numbers() {
        assert_eq!(
            parse_account_code_data_cell(&Data::Int(1101), "کد حساب").unwrap(),
            "1101"
        );
        assert_eq!(
            parse_account_code_data_cell(&Data::Float(1102.0), "کد حساب").unwrap(),
            "1102"
        );
        assert!(parse_account_code_data_cell(&Data::Float(1102.5), "کد حساب").is_err());
        assert_eq!(
            parse_account_code_data_cell(&text("001101"), "کد حساب").unwrap(),
            "001101"
        );
        assert!(parse_account_code_data_cell(
            &Data::Int(1_000_000_000_000_000),
            "کد حساب"
        )
        .is_err());
    }

    #[test]
    fn bulk_import_group_mapping_matches_the_seeded_chart() {
        for (code, group) in [
            ("1101", "1"),
            ("1501", "2"),
            ("1801", "3"),
            ("2101", "4"),
            ("2301", "5"),
            ("3101", "6"),
            ("4101", "7"),
            ("5101", "9"),
            ("6101", "8"),
            ("7101", "10"),
            ("8101", "11"),
            ("5251", "10"),
            ("5351", "11"),
        ] {
            assert_eq!(imported_group_code(code).unwrap(), group, "{code}");
        }
        assert_eq!(imported_nature("10", 1), "dual");
        assert_eq!(imported_nature("11", 1), "dual");
        assert_eq!(imported_nature("10", 2), "unknown");
        assert_eq!(
            imported_account_nature("بستانکار", "10", 2).unwrap(),
            ("creditor".to_string(), true)
        );
        assert!(imported_account_nature("ماهیت اشتباه", "10", 2).is_err());
    }

    #[test]
    fn bulk_import_preserves_excel_voucher_order() {
        let mut range = calamine::Range::new((0, 0), (4, 7));
        for (column, value) in [
            "شماره سند",
            "تاریخ سند",
            "کد کل",
            "عنوان کل",
            "کد معین",
            "عنوان معین",
            "بدهکار",
            "بستانکار",
        ]
        .iter()
        .enumerate()
        {
            range.set_value((0, column as u32), text(value));
        }

        let rows = [
            ["2", "1405/01/01", "1101", "صندوق", "110101", "صندوق اصلی", "100", ""],
            ["", "", "4101", "درآمد", "410101", "درآمد فروش", "", "100"],
            ["1", "1405/01/02", "1101", "صندوق", "110101", "صندوق اصلی", "200", ""],
            ["", "", "4101", "درآمد", "410101", "درآمد فروش", "", "200"],
        ];
        for (row_index, row) in rows.iter().enumerate() {
            for (column, value) in row.iter().enumerate() {
                range.set_value(((row_index + 1) as u32, column as u32), text(value));
            }
        }

        let import = parse_bulk_voucher_sheet(Path::new("sample.xlsx"), "اسناد", &range).unwrap();
        assert_eq!(
            import.vouchers.iter().map(|voucher| voucher.number.unwrap()).collect::<Vec<_>>(),
            vec![2, 1]
        );
    }

    #[test]
    fn bulk_import_rejects_non_contiguous_voucher_number_blocks() {
        let mut range = calamine::Range::new((0, 0), (3, 7));
        for (column, value) in [
            "شماره سند",
            "تاریخ سند",
            "کد کل",
            "عنوان کل",
            "کد معین",
            "عنوان معین",
            "بدهکار",
            "بستانکار",
        ]
        .iter()
        .enumerate()
        {
            range.set_value((0, column as u32), text(value));
        }
        let rows = [
            ["2", "1405/01/01", "1101", "صندوق", "110101", "صندوق اصلی", "100", ""],
            ["1", "1405/01/02", "4101", "درآمد", "410101", "درآمد فروش", "", "100"],
            ["2", "1405/01/01", "4101", "درآمد", "410101", "درآمد فروش", "", "100"],
        ];
        for (row_index, row) in rows.iter().enumerate() {
            for (column, value) in row.iter().enumerate() {
                range.set_value(((row_index + 1) as u32, column as u32), text(value));
            }
        }
        let error = parse_bulk_voucher_sheet(Path::new("sample.xlsx"), "اسناد", &range).unwrap_err();
        assert!(error.contains("پشت سر هم"));
    }

    #[test]
    fn bulk_import_dates_support_jalali_strings_excel_serials_and_gregorian_iso() {
        assert_eq!(parse_bulk_voucher_date(&text("۱۴۰۵/۰۱/۰۵")).unwrap(), "1405/01/05");
        assert_eq!(parse_bulk_voucher_date(&text("14050105")).unwrap(), "1405/01/05");
        assert_eq!(parse_bulk_voucher_date(&text("2024-03-20")).unwrap(), "1403/01/01");
        assert_eq!(parse_bulk_voucher_date(&text("1405/01/05 08:30:00")).unwrap(), "1405/01/05");
        assert_eq!(parse_bulk_voucher_date(&Data::Int(45371)).unwrap(), "1403/01/01");
        assert_eq!(
            parse_bulk_voucher_date(&Data::DateTime(calamine::ExcelDateTime::new(
                45371.0,
                calamine::ExcelDateTimeType::DateTime,
                false,
            )))
            .unwrap(),
            "1403/01/01"
        );
        assert_eq!(
            parse_bulk_voucher_date(&Data::DateTimeIso("2024-03-20T00:00:00".to_string())).unwrap(),
            "1403/01/01"
        );
        assert!(parse_bulk_voucher_date(&text("۱۴۰۴/۱۲/۳۰")).is_err());
    }

    #[test]
    fn bulk_import_normalizes_voucher_type_aliases() {
        assert_eq!(normalize_imported_voucher_type("افتتاحیه").unwrap(), crate::vouchers::VOUCHER_TYPES[1]);
        assert_eq!(normalize_imported_voucher_type("سند حسابداری اختتامیه").unwrap(), crate::vouchers::VOUCHER_TYPES[2]);
        assert_eq!(normalize_imported_voucher_type("بستن حساب‌های موقت").unwrap(), crate::vouchers::VOUCHER_TYPES[3]);
        assert!(normalize_imported_voucher_type("نوع نامعتبر").is_err());
    }

    #[test]
    fn account_nature_accepts_webapp_persian_labels_and_parent_fallback() {
        assert_eq!(account_nature("بستانکار", "debtor").unwrap(), "creditor");
        assert_eq!(account_nature("بدهکار", "creditor").unwrap(), "debtor");
        assert_eq!(account_nature("دوگانه", "debtor").unwrap(), "dual");
        assert_eq!(account_nature("", "creditor").unwrap(), "creditor");
        assert!(account_nature("نامعتبر", "debtor").is_err());
    }

    #[test]
    fn amounts_accept_persian_digits_and_separators() {
        assert_eq!(cell_amount(&text("۱٬۲۳۴٬۵۶۷")).unwrap(), 1_234_567);
        assert_eq!(cell_amount(&text("2,500,000")).unwrap(), 2_500_000);
        assert!(cell_amount(&text("12,34")).is_err());
        assert!(cell_amount(&text("1٬234,567")).is_err());
        assert_eq!(cell_amount(&text("")).unwrap(), 0);
        assert_eq!(cell_amount(&Data::Float(4200.0)).unwrap(), 4200);
        assert_eq!(
            cell_amount(&Data::Float(2_500_000.0000000005)).unwrap(),
            2_500_000
        );
        assert!(cell_amount(&Data::Float(1.005)).is_err());
        assert!(cell_amount(&Data::Float(4.5)).is_err());
        assert_eq!(
            cell_amount(&Data::Float(999_999_999_999_999.0)).unwrap(),
            MAX_AMOUNT
        );
        assert!(cell_amount(&Data::Float(1_000_000_000_000_000.0)).is_err());
    }

    #[test]
    fn account_codes_must_be_text_to_preserve_leading_zeroes() {
        assert_eq!(
            parse_account_code_data_cell(&text("001101"), "کد حساب").unwrap(),
            "001101"
        );
        assert!(parse_account_code_data_cell(&Data::Int(1101), "کد حساب")
            .unwrap_err()
            .contains("به‌صورت متن"));
        assert!(parse_account_code_data_cell(&Data::Float(1101.0), "کد حساب").is_err());
    }

    #[test]
    fn export_requires_a_balanced_nonzero_voucher() {
        assert!(validate_export_balance(100, 90).is_err());
        assert!(validate_export_balance(0, 0).is_err());
        assert!(validate_export_balance(100, 100).is_ok());
    }

    #[test]
    fn import_rows_with_values_are_not_mistaken_for_blanks_or_totals() {
        let mut row = vec![Data::Empty; 4];
        assert!(!has_import_values(&row, &[Some(1), Some(2), Some(3)]));
        assert!(!is_total_row(&row));

        row[2] = text("100");
        assert!(has_import_values(&row, &[Some(1), Some(2), Some(3)]));
        assert!(!is_total_row(&row));

        row[0] = text("جمع کل");
        assert!(is_total_row(&row));
    }

    #[test]
    fn headers_are_matched_after_normalization() {
        let mut range = calamine::Range::new((0, 0), (0, 5));
        for (col, value) in ["ردیف", "کد حساب", "تفصیلی ۱", "شرح", "بده کار", "بستان کار"]
            .iter()
            .enumerate()
        {
            range.set_value((0, col as u32), text(value));
        }
        let (row, columns) = find_columns(&range).expect("header row");
        assert_eq!(row, 0);
        assert_eq!(columns.account_code, Some(1));
        assert_eq!(columns.description, Some(3));
        assert_eq!(columns.debit, Some(4));
        assert_eq!(columns.credit, Some(5));
    }

    #[test]
    fn account_code_is_not_taken_from_a_document_code_column() {
        let mut range = calamine::Range::new((0, 0), (0, 5));
        for (col, value) in ["کد سند", "کد حساب", "شرح", "بدهکار", "بستانکار", "ردیف"]
            .iter()
            .enumerate()
        {
            range.set_value((0, col as u32), text(value));
        }
        let (_, columns) = find_columns(&range).expect("header row");
        assert_eq!(columns.account_code, Some(1));
    }

    #[test]
    fn exact_account_code_header_wins_over_a_similar_one() {
        let mut range = calamine::Range::new((0, 0), (0, 4));
        for (col, value) in ["کد حساب", "کد حساب تفصیلی", "بدهکار", "بستانکار", "شرح"]
            .iter()
            .enumerate()
        {
            range.set_value((0, col as u32), text(value));
        }
        let (_, columns) = find_columns(&range).expect("header row");
        assert_eq!(columns.account_code, Some(0));
    }

    #[test]
    fn ambiguous_account_columns_are_rejected() {
        let mut range = calamine::Range::new((0, 0), (0, 4));
        for (col, value) in [
            "کد حساب تفصیلی",
            "شماره حساب کد",
            "بدهکار",
            "بستانکار",
            "شرح",
        ]
        .iter()
        .enumerate()
        {
            range.set_value((0, col as u32), text(value));
        }
        assert!(find_columns(&range).is_none());
    }

    #[test]
    fn parses_the_root_bulk_import_workbook() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../Book1.xlsx");
        if !path.exists() {
            eprintln!("Book1.xlsx is unavailable; skipping the sample workbook test");
            return;
        }
        let import = parse_bulk_vouchers(&path).expect("root workbook should parse");
        assert_eq!(import.vouchers.len(), 5282);
        assert_eq!(
            import
                .vouchers
                .iter()
                .map(|voucher| voucher.lines.len())
                .sum::<usize>(),
            114_699
        );
        assert_eq!(import.vouchers[0].number, Some(1));
        assert_eq!(import.vouchers[0].lines.len(), 598);
        let database = crate::db::memory_database();
        assert_eq!(database.import_vouchers(import, false).unwrap(), 5282);
        assert_eq!(database.get_voucher(1).unwrap().unwrap().lines.len(), 598);
    }
}
