//! Chart-of-accounts import and export in the WebApp-compatible layout.

use crate::db;
use crate::excel::common::*;
use calamine::{open_workbook_auto, Data, Reader};
use rust_xlsxwriter::Workbook;
use serde::Serialize;
use std::collections::BTreeMap;

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
pub(crate) struct AccountImportColumns {
    pub(crate) group_code: Option<usize>,
    pub(crate) legacy_nature: Option<usize>,
    pub(crate) level1_code: Option<usize>,
    pub(crate) level1_name: Option<usize>,
    pub(crate) level1_nature: Option<usize>,
    pub(crate) level2_code: Option<usize>,
    pub(crate) level2_name: Option<usize>,
    pub(crate) level2_nature: Option<usize>,
    pub(crate) level3_code: Option<usize>,
    pub(crate) level3_name: Option<usize>,
    level3_nature: Option<usize>,
}

pub(crate) fn find_account_columns(
    range: &calamine::Range<Data>,
) -> Option<(usize, AccountImportColumns)> {
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

pub(crate) fn account_nature_label(value: &str) -> &str {
    match value {
        "creditor" => "بستانکار",
        "dual" => "دوگانه",
        _ => "بدهکار",
    }
}

pub(crate) fn account_nature(raw: &str, fallback: &str) -> Result<String, String> {
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
                    return Err(format!("کد سطح {level} وارد شده اما عنوان آن خالی است."));
                }
                if has_name && !has_code {
                    return Err(format!("عنوان سطح {level} وارد شده اما کد آن خالی است."));
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
                // سلسله‌مراتب حساب با parent_code صریح اعتبارسنجی می‌شود؛
                // الزام پیشوندی بین کد فرزند و والد، بخشی از مدل حسابداری نیست.
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
                accounts.extend(created);
            }
            Ok((row_created, row_skipped))
        })();
        match row_result {
            Ok((row_created, row_skipped)) => {
                created_count += row_created;
                skipped_count += row_skipped;
            }
            Err(error) => {
                error_count += 1;
                if errors.len() < MAX_IMPORT_ERRORS {
                    errors.push(format!("ردیف {}: {}", db::to_fa_digits(excel_row), error));
                }
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

pub(crate) trait EmptyFallback {
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
