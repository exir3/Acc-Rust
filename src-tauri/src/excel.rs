//! Excel import and export for one voucher.
//!
//! The export writes a real `.xlsx` workbook with the same layout the
//! importer understands: a header row naming the columns
//! «ردیف / کد حساب / تفصیلی ۲ / شرح / بدهکار / بستانکار» and one row per
//! voucher line. Amounts are whole Rials, exactly like the database.

use crate::db;
use crate::vouchers::{VoucherInput, VoucherLineInput};
use calamine::{open_workbook_auto, Data, Reader};
use rust_xlsxwriter::{
    Color, Format, FormatAlign, FormatBorder, Workbook, Worksheet, XlsxError,
};
use std::path::Path;

const MAX_IMPORT_LINES: usize = 500;
const MAX_LINE_DESCRIPTION: usize = 500;
const HEADER_SCAN_ROWS: usize = 30;
const MAX_AMOUNT: i64 = 999_999_999_999_999;

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

fn digits_only(value: &str) -> String {
    db::normalize_digits(value)
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect()
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

/// Reads a cell as a whole-Rial amount. Persian digits and thousands
/// separators are accepted because that is how users type them.
fn cell_amount(cell: &Data) -> Result<i64, String> {
    match cell {
        Data::Empty => Ok(0),
        Data::Int(value) => Ok(*value),
        Data::Float(value)
            if value.is_finite()
                && value.fract() == 0.0
                && value.abs() <= MAX_AMOUNT as f64 =>
        {
            Ok(*value as i64)
        }
        Data::Float(value) if value.fract() == 0.0 => Err(format!(
            "مبلغ باید بین صفر و {} ریال باشد.",
            db::to_fa_digits(MAX_AMOUNT)
        )),
        Data::Float(_) => Err("مبلغ باید عدد صحیح و محدود باشد (بدون اعشار).".to_string()),
        _ => {
            let text = cell_text(cell);
            let cleaned: String = db::normalize_digits(&text)
                .chars()
                .filter(|ch| !matches!(ch, ',' | '٬' | ' ' | '\u{200c}' | '\u{00a0}'))
                .collect();
            if cleaned.is_empty() || cleaned == "-" {
                return Ok(0);
            }
            cleaned
                .parse::<i64>()
                .map_err(|_| format!("مبلغ «{text}» عدد معتبری نیست."))
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
        if line.debit < 0 || line.credit < 0 || line.debit > MAX_AMOUNT || line.credit > MAX_AMOUNT {
            return Err(format!("مبلغ ردیف {row_no} خارج از محدوده مجاز است."));
        }
        if line.debit > 0 && line.credit > 0 {
            return Err(format!("در ردیف {row_no} نمی‌توان هم‌زمان بدهکار و بستانکار داشت."));
        }
        if line.description.chars().count() > MAX_LINE_DESCRIPTION {
            return Err(format!("شرح ردیف {row_no} بیشتر از {} کاراکتر است.", db::to_fa_digits(MAX_LINE_DESCRIPTION)));
        }
        checked_debit = checked_debit.checked_add(line.debit)
            .ok_or_else(|| "جمع بدهکار خروجی اکسل از محدوده عددی مجاز بیشتر است.".to_string())?;
        checked_credit = checked_credit.checked_add(line.credit)
            .ok_or_else(|| "جمع بستانکار خروجی اکسل از محدوده عددی مجاز بیشتر است.".to_string())?;
    }
    let _validated_totals = (checked_debit, checked_credit);
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
    worksheet
        .set_name("سند حسابداری")
        .map_err(xlsx_error)?;
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
        total_debit = total_debit.checked_add(line.debit)
            .ok_or_else(|| "جمع بدهکار خروجی اکسل از محدوده عددی مجاز بیشتر است.".to_string())?;
        total_credit = total_credit.checked_add(line.credit)
            .ok_or_else(|| "جمع بستانکار خروجی اکسل از محدوده عددی مجاز بیشتر است.".to_string())?;
    }

    let total_row = header_row + 1 + input.lines.len() as u32;
    worksheet
        .merge_range(total_row, 0, total_row, 3, "جمع کل", &total_label_format)
        .map_err(xlsx_error)?;
    worksheet
        .write_number_with_format(total_row, 4, total_debit as f64, &total_amount_format)
        .map_err(xlsx_error)?;
    worksheet
        .write_number_with_format(total_row, 5, total_credit as f64, &total_amount_format)
        .map_err(xlsx_error)?;

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
        let headers: Vec<String> = row.iter().map(|cell| normalize_header(&cell_text(cell))).collect();
        let find = |needles: &[&str]| {
            headers
                .iter()
                .position(|header| needles.iter().any(|needle| header.contains(needle)))
        };

        let columns = ImportColumns {
            account_code: find(&["کدحساب", "کد"]),
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

    let mut workbook = open_workbook_auto(&path)
        .map_err(|error| format!("فایل اکسل باز نشد: {error}"))?;
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

    let read = |row: &[Data], index: Option<usize>| -> Data {
        index
            .and_then(|index| row.get(index))
            .cloned()
            .unwrap_or(Data::Empty)
    };

    let mut lines: Vec<VoucherLineInput> = Vec::new();
    for (offset, row) in range.rows().skip(header_row + 1).enumerate() {
        let excel_row = header_row + offset + 2;
        let account_code = digits_only(&cell_text(&read(row, columns.account_code)));
        if account_code.is_empty() {
            // Empty spacer rows and the total row («جمع کل») have no code.
            continue;
        }

        let description = cell_text(&read(row, columns.description));
        if description.chars().count() > MAX_LINE_DESCRIPTION {
            return Err(format!(
                "شرح ردیف {} اکسل بیشتر از {} کاراکتر است.",
                db::to_fa_digits(excel_row),
                db::to_fa_digits(MAX_LINE_DESCRIPTION)
            ));
        }

        let debit = cell_amount(&read(row, columns.debit))
            .map_err(|error| format!("بدهکار ردیف {} اکسل: {error}", db::to_fa_digits(excel_row)))?;
        let credit = cell_amount(&read(row, columns.credit)).map_err(|error| {
            format!("بستانکار ردیف {} اکسل: {error}", db::to_fa_digits(excel_row))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Data {
        Data::String(value.to_string())
    }

    #[test]
    fn amounts_accept_persian_digits_and_separators() {
        assert_eq!(cell_amount(&text("۱٬۲۳۴٬۵۶۷")).unwrap(), 1_234_567);
        assert_eq!(cell_amount(&text("2,500,000")).unwrap(), 2_500_000);
        assert_eq!(cell_amount(&text("")).unwrap(), 0);
        assert_eq!(cell_amount(&Data::Float(4200.0)).unwrap(), 4200);
        assert!(cell_amount(&Data::Float(4.5)).is_err());
        assert_eq!(cell_amount(&Data::Float(999_999_999_999_999.0)).unwrap(), MAX_AMOUNT);
        assert!(cell_amount(&Data::Float(1_000_000_000_000_000.0)).is_err());
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
}
