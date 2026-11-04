//! Excel import/export tests.

use super::*;
use crate::excel::accounts::*;
use crate::excel::common::*;
use crate::excel::import::*;
use crate::excel::model::*;
use calamine::Data;
use std::collections::BTreeMap;
use std::path::Path;

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
            text("881122"),
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
            text("881123"),
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
        let custom_child = import.accounts.iter().find(|account| account.code == "881122").unwrap();
        assert_eq!(custom_parent.group_code, "10");
        assert_eq!(custom_child.group_code, "10");
        assert_eq!(custom_child.nature, "debtor");
        assert!(custom_child.nature_is_explicit);
        let merged_group_child = import.accounts.iter().find(|account| account.code == "881123").unwrap();
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
        assert!(parse_account_code_data_cell(&Data::Int(1_000_000_000_000_000), "کد حساب")
            .unwrap_err()
            .contains("به‌صورت متن"));
        assert!(parse_account_code_data_cell(&Data::Float(1101.5), "کد حساب").is_err());
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
        assert_eq!(columns.detail1_code, Some(2));
        assert_eq!(columns.description, Some(3));
        assert_eq!(columns.debit, Some(4));
        assert_eq!(columns.credit, Some(5));
    }

    #[test]
    fn exported_voucher_detail_code_column_is_recognized_without_mistaking_its_title_for_a_code() {
        let mut exported = calamine::Range::new((0, 0), (0, 5));
        for (col, value) in ["ردیف", "کد حساب", "کد تفصیلی ۱", "شرح", "بدهکار", "بستانکار"]
            .iter()
            .enumerate()
        {
            exported.set_value((0, col as u32), text(value));
        }
        let (_, columns) = find_columns(&exported).expect("exported voucher headers");
        assert_eq!(columns.account_code, Some(1));
        assert_eq!(columns.detail1_code, Some(2));

        let mut titled = calamine::Range::new((0, 0), (0, 5));
        for (col, value) in ["کد حساب", "عنوان تفصیلی ۱", "شرح", "بدهکار", "بستانکار", "ردیف"]
            .iter()
            .enumerate()
        {
            titled.set_value((0, col as u32), text(value));
        }
        let (_, columns) = find_columns(&titled).expect("voucher headers with detail title");
        assert_eq!(columns.detail1_code, None);
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
