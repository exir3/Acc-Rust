//! Reading vouchers and bulk-import workbooks.

use crate::db;
use crate::excel::accounts::account_nature;
use crate::excel::common::*;
use crate::excel::model::{BulkImportColumns, BulkVoucherImport, ImportedAccount};
use crate::vouchers::{VoucherInput, VoucherLineInput, MAX_VOUCHER_NUMBER};
use calamine::{open_workbook_auto, Data, Reader};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/* ── Import ─────────────────────────────────────────────────── */

#[derive(Default)]
pub(crate) struct ImportColumns {
    pub(crate) account_code: Option<usize>,
    pub(crate) detail1_code: Option<usize>,
    pub(crate) description: Option<usize>,
    pub(crate) debit: Option<usize>,
    pub(crate) credit: Option<usize>,
}

pub(crate) fn find_columns(range: &calamine::Range<Data>) -> Option<(usize, ImportColumns)> {
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
                .position(|header| header == "کد حساب" || header == "کدحساب" || header == "کد")
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
            detail1_code: find(&[
                "کد تفصیلی ۱",
                "کد تفصیلی 1",
                "کد تفصیلی",
                "کد تفضیلی ۱",
                "کد تفضیلی 1",
                "کد تفضیلی",
            ])
            .or_else(|| {
                // سازگاری با فایل‌های قدیمی فقط وقتی عنوان ستون دقیقاً
                // «تفصیلی ۱» است؛ عنوان/نام تفصیلی را به‌اشتباه کد فرض نکن.
                headers.iter().position(|header| {
                    matches!(
                        header.as_str(),
                        "تفصیلی ۱"
                            | "تفصیلی 1"
                            | "تفضیلی ۱"
                            | "تفضیلی 1"
                            | "تفصیلی1"
                            | "تفضیلی1"
                    )
                })
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

pub(crate) fn find_bulk_columns(
    range: &calamine::Range<Data>,
) -> Option<(usize, BulkImportColumns)> {
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

pub(crate) fn imported_group_code(general_code: &str) -> Result<&'static str, String> {
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

pub(crate) fn imported_nature(group_code: &str, level: i64) -> &'static str {
    match group_code {
        "4" | "5" | "6" | "7" => "creditor",
        // The built-in group 10/11 parent accounts are dual, but their child
        // accounts may be income or expense. Do not guess their nature.
        "10" | "11" if level == 1 => "dual",
        "10" | "11" => "unknown",
        _ => "debtor",
    }
}

pub(crate) fn imported_account_nature(
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

pub(crate) fn add_imported_account(
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

pub(crate) fn parse_bulk_vouchers(path: &Path) -> Result<BulkVoucherImport, String> {
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
            Err(error) if error == "در فایل اکسل هیچ ردیف سندی پیدا نشد." =>
            {
                // Skip an empty template tab, but never skip a malformed data
                // tab and silently import a different worksheet instead.
                empty_matching_sheet = Some(format!("کاربرگ «{sheet_name}» خالی است."));
            }
            Err(error) => return Err(format!("کاربرگ «{sheet_name}»: {error}")),
        }
    }

    if let Some(error) = empty_matching_sheet {
        return Err(format!(
            "{error} در سایر کاربرگ‌ها نیز داده قابل ورود پیدا نشد."
        ));
    }
    Err("در هیچ‌یک از کاربرگ‌های فایل، ستون‌های لازم برای ورود دسته‌جمعی اسناد پیدا نشد. ستون‌های شماره سند، تاریخ، کد و عنوان کل، کد و عنوان معین، بدهکار و بستانکار الزامی هستند. در صورت وجود ستون کد تفصیلی ۱، عنوان تفصیلی ۱ نیز باید وجود داشته باشد.".to_string())
}

pub(crate) fn parse_bulk_voucher_sheet(
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
            let parsed =
                cell_voucher_number(&read(row, columns.voucher_number)).map_err(|error| {
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
        if detail1_code.is_empty() && !detail1_name.trim().is_empty() {
            return Err(format!(
                "عنوان تفصیلی ۱ در ردیف {} اکسل وارد شده اما کد آن خالی است.",
                db::to_fa_digits(excel_row)
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
            if parsed.len() > 50 {
                return Err("کد گروه در فایل اکسل نمی‌تواند بیشتر از ۵۰ رقم باشد.".to_string());
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

        let detail1_code = if !detail1_code.is_empty() {
            let (detail1_nature, detail1_nature_is_explicit) = imported_account_nature(
                &cell_text(&read(row, columns.detail1_nature)),
                &group_code,
                3,
            )?;
            add_imported_account(
                &mut accounts,
                detail1_code.clone(),
                detail1_name,
                subsidiary_code.clone(),
                &group_code,
                3,
                detail1_nature,
                detail1_nature_is_explicit,
            )?;
            Some(detail1_code)
        } else {
            None
        };
        // The voucher row now stores its posting base (معین) separately
        // from the optional level-3 detail account.
        let account_code = subsidiary_code;
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
            detail1_code,
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

        let detail1_cell = read(row, columns.detail1_code);
        let detail1_text = cell_text(&detail1_cell);
        let detail1_code = if detail1_text.is_empty() {
            None
        } else {
            Some(parse_account_code_data_cell(
                &detail1_cell,
                &format!("کد تفصیلی ۱ ردیف {} اکسل", db::to_fa_digits(excel_row)),
            )?)
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
            detail1_code,
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
