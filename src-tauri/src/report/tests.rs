//! Report print-document tests.

use super::*;
use crate::report::layout::portable_base_style;
use crate::vouchers::{BalanceSheetReport, TrialBalanceReport, Voucher, VoucherLine};

mod tests {
    use super::*;

    #[test]
    fn balance_sheet_print_rejects_equity_total_overflow() {
        let report = BalanceSheetReport {
            as_of: "1405/01/01".to_string(),
            assets: Vec::new(),
            assets_total: 0,
            liabilities: Vec::new(),
            liabilities_total: 0,
            equity: Vec::new(),
            equity_total: i64::MAX,
            profit: 1,
            total_liabilities_equity: 0,
            balanced: false,
        };
        let meta = ReportMeta {
            company: String::new(),
            fiscal_year: String::new(),
            today: String::new(),
            show_print_date_time: true,
        };

        assert!(balance_sheet_print_doc(&report, &meta)
            .unwrap_err()
            .contains("محدوده عددی"));
    }

    #[test]
    fn print_documents_escape_dates_codes_and_descriptions() {
        let voucher = Voucher {
            id: 1,
            number: 1,
            date: "1405/01/01<script>alert(1)</script>".to_string(),
            updated_at: "1405-01-01 00:00:00.000".to_string(),
            voucher_type: "<b>نوع</b>".to_string(),
            description: "<img src=x onerror=alert(1)>".to_string(),
            lines: vec![crate::vouchers::VoucherLine {
                id: 1,
                row_no: 1,
                account_code: "<svg/onload=alert(1)>".to_string(),
                account_name: "<script>alert(2)</script>".to_string(),
                description: "\"quoted\" & <tag>".to_string(),
                debit: 100,
                credit: 0,
            }],
            total_debit: 100,
            total_credit: 0,
        };
        let meta = ReportMeta {
            company: "شرکت <script>".to_string(),
            fiscal_year: "1405".to_string(),
            today: "1405/01/01".to_string(),
            show_print_date_time: true,
        };

        let doc = voucher_print_doc(&voucher, &meta);
        assert!(!doc.html.contains("<script>"));
        assert!(!doc.html.contains("<svg/onload"));
        assert!(!doc.html.contains("<img src=x"));
        assert!(doc.html.contains("&lt;script&gt;"));
        assert!(doc.html.contains("۱۴۰۵/۰۱/۰۱&lt;script&gt;"));
    }

    #[test]
    fn trial_balance_print_hides_optional_identity_and_date_columns() {
        let report = TrialBalanceReport {
            from: "1405/01/01".to_string(),
            to: "1405/01/31".to_string(),
            rows: vec![crate::vouchers::TrialBalanceRow {
                account_code: "1101".to_string(),
                account_name: "صندوق".to_string(),
                group_name: "دارایی‌های جاری".to_string(),
                level: 1,
                opening_debit: 100,
                opening_credit: 0,
                period_debit: 25,
                period_credit: 5,
                closing_debit: 120,
                closing_credit: 0,
            }],
            opening_debit: 100,
            opening_credit: 0,
            period_debit: 25,
            period_credit: 5,
            closing_debit: 120,
            closing_credit: 0,
        };
        let meta = ReportMeta {
            company: "شرکت آزمایشی".to_string(),
            fiscal_year: "1405".to_string(),
            today: "1405/02/01".to_string(),
            show_print_date_time: false,
        };

        let minimal = trial_balance_print_doc(&report, &meta, "شش ستونی", false, false, false)
            .expect("minimal trial balance print doc");
        assert!(!minimal.html.contains("کد حساب"));
        assert!(!minimal.html.contains("عنوان حساب"));
        assert!(!minimal.html.contains("تاریخ تهیه"));
        assert!(minimal.html.contains("گروه"));
        assert!(minimal.html.contains("جمع کل"));

        let full = trial_balance_print_doc(&report, &meta, "شش ستونی", false, true, true)
            .expect("full trial balance print doc");
        assert!(full.html.contains("کد حساب"));
        assert!(full.html.contains("عنوان حساب"));
    }

    #[test]
    fn portable_print_style_embeds_regular_and_bold_fonts() {
        let style = portable_base_style();
        assert_eq!(style.matches("data:font/ttf;base64,").count(), 2);
        assert!(style.contains("font-weight: 400"));
        assert!(style.contains("font-weight: 700"));
    }
}
