//! Writing a voucher to a single-sheet `.xlsx` workbook.

use crate::db;
use crate::excel::common::*;
use crate::vouchers::VoucherInput;
use rust_xlsxwriter::{Color, Format, FormatAlign, FormatBorder, Workbook, Worksheet};

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

pub(crate) fn write_sheet(worksheet: &mut Worksheet, input: &VoucherInput, accounts: &[db::Account]) -> Result<(), String> {
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
    for (col, label) in ["ردیف", "کد حساب", "کد تفصیلی ۱", "شرح", "بدهکار", "بستانکار"]
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
        // کد تفصیلی مستقل از حساب پایه صادر می‌شود تا همان فایل دوباره
        // بدون از دست رفتن تفصیلی قابل ورود باشد.
        let detail1_code = line.detail1_code.as_deref().unwrap_or("");
        if !detail1_code.is_empty() {
            let detail = accounts
                .iter()
                .find(|account| account.code == detail1_code && account.level == 3)
                .ok_or_else(|| format!("حساب تفصیلی {} در فهرست حساب‌ها پیدا نشد.", db::to_fa_digits(detail1_code)))?;
            if detail.parent_code != line.account_code {
                return Err(format!(
                    "تفصیلی {} فرزند مستقیم حساب پایه {} نیست.",
                    db::to_fa_digits(detail1_code),
                    db::to_fa_digits(&line.account_code)
                ));
            }
        }
        worksheet
            .write_string_with_format(row, 2, detail1_code, &code_format)
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

