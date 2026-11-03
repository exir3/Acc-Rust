//! Excel import and export for one voucher.
//!
//! The export writes a real `.xlsx` workbook with the same layout the
//! importer understands: a header row naming the columns
//! «ردیف / کد حساب / تفصیلی ۲ / شرح / بدهکار / بستانکار» and one row per
//! voucher line. Amounts are whole Rials, exactly like the database.

use crate::db;
use crate::vouchers::{VoucherInput, VoucherLineInput, MAX_VOUCHER_NUMBER};
use calamine::{open_workbook_auto, Data, Reader};
use rust_xlsxwriter::{Color, Format, FormatAlign, FormatBorder, Workbook, Worksheet, XlsxError};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

const MAX_IMPORT_LINES: usize = 1000;
const MAX_LINE_DESCRIPTION: usize = 500;
const HEADER_SCAN_ROWS: usize = 30;
const MAX_BULK_IMPORT_ROWS: usize = 300_000;
const MAX_BULK_IMPORT_COLUMNS: usize = 40;
const MAX_VOUCHER_IMPORT_ROWS: usize = 20_000;
const MAX_ACCOUNT_IMPORT_ROWS: usize = 10_000;
const MAX_SHEET_COLUMNS: usize = 50;
const MAX_EXCEL_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_AMOUNT: i64 = 999_999_999_999_999;

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
    Ok(())
}

fn ensure_sheet_dimensions(
    range: &calamine::Range<Data>,
    max_rows: usize,
    max_columns: usize,
) -> Result<(), String> {
    if range.height() as usize > max_rows || range.width() as usize > max_columns {
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
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkVoucherPreview {
    pub file_name: String,
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
    general_code: Option<usize>,
    general_name: Option<usize>,
    subsidiary_code: Option<usize>,
    subsidiary_name: Option<usize>,
    description: Option<usize>,
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
    if matches!(cell, Data::Int(_) | Data::Float(_)) {
        return Err(format!(
            "{label} باید در اکسل به‌صورت متن ذخیره شود تا صفرهای ابتدایی کد حفظ شوند."
        ));
    }
    parse_account_code_cell(&cell_text(cell), label)
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
            .write_string_with_format(row, column, &amount.to_string(), format)
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
pub fn export_voucher(input: &VoucherInput) -> Result<Option<String>, String> {
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

    write_sheet(worksheet, input)?;
    workbook.save(&path).map_err(xlsx_error)?;
    Ok(Some(path.to_string_lossy().to_string()))
}

fn write_sheet(worksheet: &mut Worksheet, input: &VoucherInput) -> Result<(), String> {
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
    for (col, label) in ["ردیف", "کد حساب", "تفصیلی ۲", "شرح", "بدهکار", "بستانکار"]
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
        worksheet
            .write_string_with_format(row, 2, "", &code_format)
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
            voucher_number: find(&["سند", "شماره سند", "شماره‌سند"]),
            voucher_date: find(&["تاریخ", "تاریخ سند"]),
            general_code: find(&["کد کل", "کدکل"]),
            general_name: find(&["عنوان کل", "نام کل"]),
            subsidiary_code: find(&["کد معین", "کدمعین"]),
            subsidiary_name: find(&["عنوان معین", "نام معین"]),
            description: find(&["شرح", "توضیحات"]),
            debit: find(&["بدهکار"]),
            credit: find(&["بستانکار", "ستانکار"]),
        };
        if columns.voucher_number.is_some()
            && columns.voucher_date.is_some()
            && columns.general_code.is_some()
            && columns.debit.is_some()
            && columns.credit.is_some()
        {
            return Some((row_index, columns));
        }
    }
    None
}

fn imported_group_code(general_code: &str) -> &'static str {
    match general_code.chars().next() {
        Some('1') if general_code.starts_with("15") || general_code.starts_with("16") => "2",
        Some('1') if general_code.starts_with("18") => "3",
        Some('1') => "1",
        Some('2') if general_code.starts_with("23") || general_code.starts_with("24") => "5",
        Some('2') => "4",
        Some('3') => "6",
        Some('4') => "7",
        Some('5') if general_code.starts_with("525") => "10",
        Some('5') if general_code.starts_with("535") => "11",
        Some('5') if general_code.starts_with("50") => "9",
        Some('5') => "8",
        _ => "11",
    }
}

fn imported_nature(group_code: &str) -> &'static str {
    match group_code {
        "4" | "5" | "6" | "7" => "creditor",
        "10" | "11" => "dual",
        _ => "debtor",
    }
}

fn add_imported_account(
    accounts: &mut BTreeMap<String, ImportedAccount>,
    code: String,
    name: String,
    parent_code: String,
    group_code: &str,
    level: i64,
) -> Result<(), String> {
    if code.is_empty() {
        return Err("کد حساب در فایل اکسل خالی است.".to_string());
    }
    let account = ImportedAccount {
        code: code.clone(),
        name: if name.is_empty() {
            format!("حساب {code}")
        } else {
            name
        },
        parent_code,
        group_code: group_code.to_string(),
        level,
        nature: imported_nature(group_code).to_string(),
    };
    if let Some(existing) = accounts.get(&code) {
        if existing.parent_code != account.parent_code
            || existing.level != account.level
            || existing.group_code != account.group_code
        {
            return Err(format!(
                "کد حساب {} در فایل با ساختار (حساب بالاتر، سطح یا گروه) متفاوت آمده است.",
                db::to_fa_digits(code)
            ));
        }
    } else {
        accounts.insert(code, account);
    }
    Ok(())
}

/// Selects and parses a multi-voucher workbook using the WebApp column layout.
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
        open_workbook_auto(&path).map_err(|error| format!("فایل اکسل باز نشد: {error}"))?;
    let range = match workbook.worksheet_range_at(0) {
        Some(Ok(range)) => range,
        Some(Err(error)) => return Err(format!("خواندن کاربرگ اکسل ناموفق بود: {error}")),
        None => return Err("فایل اکسل هیچ کاربرگی ندارد.".to_string()),
    };
    ensure_sheet_dimensions(&range, MAX_BULK_IMPORT_ROWS, MAX_BULK_IMPORT_COLUMNS)?;

    let Some((header_row, columns)) = find_bulk_columns(&range) else {
        return Err("عنوان ستون‌های فایل پیدا نشد. ستون‌های «سند»، «تاریخ»، «کد کل»، «بدهکار» و «بستانکار» الزامی هستند.".to_string());
    };
    let read = |row: &[Data], index: Option<usize>| -> Data {
        index
            .and_then(|index| row.get(index))
            .cloned()
            .unwrap_or(Data::Empty)
    };

    let mut vouchers = BTreeMap::<i64, VoucherInput>::new();
    let mut accounts = BTreeMap::<String, ImportedAccount>::new();
    for (offset, row) in range.rows().skip(header_row + 1).enumerate() {
        let excel_row = header_row + offset + 2;
        let raw_number = cell_text(&read(row, columns.voucher_number));
        if raw_number.is_empty() {
            if is_total_row(row) {
                continue;
            }
            let value_columns = [
                columns.voucher_date,
                columns.general_code,
                columns.general_name,
                columns.subsidiary_code,
                columns.subsidiary_name,
                columns.description,
                columns.debit,
                columns.credit,
            ];
            if !has_import_values(row, &value_columns) {
                continue;
            }
            return Err(format!(
                "شماره سند ردیف {} اکسل الزامی است.",
                db::to_fa_digits(excel_row)
            ));
        }
        let number = cell_amount(&read(row, columns.voucher_number)).map_err(|error| {
            format!(
                "شماره سند ردیف {} اکسل: {error}",
                db::to_fa_digits(excel_row)
            )
        })?;
        if number <= 0 || number > MAX_VOUCHER_NUMBER {
            return Err(format!(
                "شماره سند ردیف {} اکسل باید بین ۱ و {} باشد.",
                db::to_fa_digits(excel_row),
                db::to_fa_digits(MAX_VOUCHER_NUMBER)
            ));
        }

        let raw_date = db::normalize_digits(&cell_text(&read(row, columns.voucher_date)))
            .trim()
            .replace('-', "/");
        let date = crate::vouchers::validate_voucher_date(&raw_date)
            .map_err(|error| format!("تاریخ ردیف {} اکسل: {error}", db::to_fa_digits(excel_row)))?;
        let general_code = parse_account_code_data_cell(
            &read(row, columns.general_code),
            &format!("کد کل ردیف {} اکسل", db::to_fa_digits(excel_row)),
        )?;
        let general_name = cell_text(&read(row, columns.general_name));
        let subsidiary_cell = read(row, columns.subsidiary_code);
        let raw_subsidiary = cell_text(&subsidiary_cell);
        let subsidiary_code = if raw_subsidiary.is_empty() {
            String::new()
        } else {
            parse_account_code_data_cell(
                &subsidiary_cell,
                &format!("کد معین ردیف {} اکسل", db::to_fa_digits(excel_row)),
            )?
        };
        let subsidiary_name = cell_text(&read(row, columns.subsidiary_name));
        let group_code = imported_group_code(&general_code);
        add_imported_account(
            &mut accounts,
            general_code.clone(),
            general_name,
            String::new(),
            group_code,
            1,
        )?;

        let account_code = if subsidiary_code.is_empty() {
            general_code.clone()
        } else {
            add_imported_account(
                &mut accounts,
                subsidiary_code.clone(),
                subsidiary_name,
                general_code.clone(),
                group_code,
                2,
            )?;
            subsidiary_code
        };
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
        voucher.lines.push(VoucherLineInput {
            account_code,
            description,
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
        vouchers: vouchers.into_values().collect(),
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
    for (offset, row) in range.rows().skip(header_row + 1).enumerate() {
        let excel_row = header_row + offset + 2;
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

    for (offset, row) in range.rows().skip(header_row + 1).enumerate() {
        let excel_row = header_row + offset + 2;
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
            errors.push(format!("ردیف {}: {}", db::to_fa_digits(excel_row), error));
            if errors.len() >= 200 {
                errors
                    .push("ادامه خطاها برای جلوگیری از مصرف بیش از حد حافظه ثبت نشد.".to_string());
                break;
            }
        }
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
        for (col, value) in ["ردیف", "کد حساب", "تفصیلی ۲", "شرح", "بده کار", "بستان کار"]
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
