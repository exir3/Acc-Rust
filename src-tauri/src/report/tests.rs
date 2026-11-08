//! Report print-document tests.

use super::*;
use crate::report::layout::portable_base_style;
use crate::vouchers::{BalanceSheetReport, IntegrityReport, TrialBalanceReport, Voucher};

// The file is itself `tests.rs`; the inner module keeps the usual `mod tests`
// layout so every test names the same way as elsewhere in the crate.
#[allow(clippy::module_inception)]
mod tests {
    use super::*;

    #[test]
    fn balance_sheet_print_rejects_equity_total_overflow() {
        let report = BalanceSheetReport {
            as_of: "1405/01/01".to_string(),
            assets: Vec::new(),
            assets_total: 0,
            unclassified_debit: Vec::new(),
            unclassified_debit_total: 0,
            liabilities: Vec::new(),
            liabilities_total: 0,
            unclassified_credit: Vec::new(),
            unclassified_credit_total: 0,
            equity: Vec::new(),
            equity_total: i64::MAX,
            profit: 1,
            total_liabilities_equity: 0,
            balanced: false,
            integrity: Default::default(),
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
            is_temporary: true,
            voucher_type: "<b>نوع</b>".to_string(),
            description: "<img src=x onerror=alert(1)>".to_string(),
            lines: vec![crate::vouchers::VoucherLine {
                id: 1,
                row_no: 1,
                account_code: "<svg/onload=alert(1)>".to_string(),
                account_name: "<script>alert(2)</script>".to_string(),
                detail1_code: None,
                detail1_name: None,
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

        let doc = voucher_print_doc(&voucher, &meta, &IntegrityReport::default());
        assert!(!doc.html.contains("<script>"));
        assert!(!doc.html.contains("<svg/onload"));
        assert!(!doc.html.contains("<img src=x"));
        assert!(doc.html.contains("&lt;script&gt;"));
        assert!(doc.html.contains("۱۴۰۵/۰۱/۰۱&lt;script&gt;"));
    }

    #[test]
    fn print_header_escapes_company_and_period_exactly_once() {
        let meta = ReportMeta {
            company: "شرکت \"<نام>\" & مشارکت".to_string(),
            fiscal_year: "1405".to_string(),
            today: "1405/01/01\"<x>".to_string(),
            show_print_date_time: true,
        };
        let report = crate::vouchers::JournalReport {
            from: "1405/01/01\"<x>".to_string(),
            to: "1405/01/02\"<x>".to_string(),
            search: String::new(),
            rows: Vec::new(),
            row_count: 0,
            total_debit: 0,
            total_credit: 0,
            voucher_count: 0,
        };

        let doc = journal_print_doc(&report, &meta, &IntegrityReport::default());
        assert!(doc
            .html
            .contains("شرکت &quot;&lt;نام&gt;&quot; &amp; مشارکت"));
        assert!(doc.html.contains("از ۱۴۰۵/۰۱/۰۱&quot;&lt;x&gt;"));
        assert!(doc.html.contains("تا ۱۴۰۵/۰۱/۰۲&quot;&lt;x&gt;"));
        assert!(doc
            .html
            .contains("تاریخ تهیه: <b>۱۴۰۵/۰۱/۰۱&quot;&lt;x&gt;</b>"));
        assert!(!doc.html.contains("&amp;quot;"));
        assert!(!doc.html.contains("&amp;lt;"));
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
                parent_code: String::new(),
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

    fn test_meta() -> ReportMeta {
        ReportMeta {
            company: "شرکت آزمایشی".to_string(),
            fiscal_year: "1405".to_string(),
            today: "1405/12/29".to_string(),
            show_print_date_time: true,
        }
    }

    #[test]
    fn supplementary_statement_print_documents_include_required_sections() {
        use crate::vouchers::{
            AmountRow, AppropriationReport, AppropriationRow, AppropriationTotals, CashFlowLine,
            CashFlowReport, CashFlowSection, CashFlowUnresolvedVoucher, CogsReport, CogsRow,
            CogsTotals, EquityChangesReport, EquityChangesRow, EquityChangesTotals,
        };

        let equity = EquityChangesReport {
            statement: "equity_changes".to_string(),
            to_date: "1405/12/29".to_string(),
            rows: vec![
                EquityChangesRow {
                    fs_line: "BS.EQ.CAP".to_string(),
                    title: "سرمایه".to_string(),
                    kind: "line".to_string(),
                    opening: 100,
                    change: 10,
                    closing: 110,
                    accounts: Vec::new(),
                },
                EquityChangesRow {
                    fs_line: "BS.EQ.PROFIT".to_string(),
                    title: "سود (زیان) خالص دوره".to_string(),
                    kind: "profit".to_string(),
                    opening: 0,
                    change: 10,
                    closing: 10,
                    accounts: Vec::new(),
                },
                EquityChangesRow {
                    fs_line: "BS.EQ.TOTAL".to_string(),
                    title: "جمع حقوق مالکانه".to_string(),
                    kind: "total".to_string(),
                    opening: 100,
                    change: 20,
                    closing: 120,
                    accounts: Vec::new(),
                },
            ],
            totals: EquityChangesTotals {
                opening: 100,
                change: 20,
                closing: 120,
                net_income: 10,
            },
            integrity: Default::default(),
        };
        let equity_doc = equity_changes_print_doc(&equity, &test_meta());
        assert_eq!(equity_doc.file_name, "صورت-تغییرات-حقوق-مالکانه.html");
        assert!(equity_doc.html.contains("صورت تغییرات حقوق مالکانه"));
        assert!(equity_doc.html.contains("۱۲۰"));

        let cogs = CogsReport {
            statement: "cogs".to_string(),
            from_date: "1405/01/01".to_string(),
            to_date: "1405/12/29".to_string(),
            rows: vec![
                CogsRow {
                    kind: "cost".to_string(),
                    label: "بهای تمام‌شدهٔ ثبت‌شده".to_string(),
                    code: Some("5101".to_string()),
                    amount: 150,
                    note: None,
                },
                CogsRow {
                    kind: "inventory".to_string(),
                    label: "تغییر موجودی".to_string(),
                    code: None,
                    amount: -50,
                    note: Some("توضیح مکمل".to_string()),
                },
                CogsRow {
                    kind: "total".to_string(),
                    label: "بهای تمام‌شدهٔ کالای فروش‌رفته".to_string(),
                    code: None,
                    amount: 150,
                    note: None,
                },
            ],
            totals: CogsTotals {
                costs: 150,
                opening_inventory: 200,
                closing_inventory: 150,
                cogs: 150,
            },
            detailed: true,
            limitation: "محدودیت آزمایشی".to_string(),
            integrity: Default::default(),
        };
        let cogs_doc = cogs_print_doc(&cogs, &test_meta());
        assert_eq!(cogs_doc.file_name, "صورت-بهای-تمام-شده-کالای-فروش-رفته.html");
        assert!(cogs_doc.html.contains("محدودیت آزمایشی"));
        assert!(cogs_doc.html.contains("توضیح مکمل"));

        let appropriation = AppropriationReport {
            statement: "profit_appropriation".to_string(),
            to_date: "1405/12/29".to_string(),
            rows: vec![
                AppropriationRow {
                    kind: "net".to_string(),
                    fs_line: None,
                    label: "سود (زیان) خالص دوره".to_string(),
                    amount: 100,
                    accounts: Vec::new(),
                },
                AppropriationRow {
                    kind: "appropriation".to_string(),
                    fs_line: Some("BS.EQ.LEG".to_string()),
                    label: "اندوخته قانونی".to_string(),
                    amount: 10,
                    accounts: vec![AmountRow {
                        account_code: "3103".to_string(),
                        account_name: "اندوخته قانونی".to_string(),
                        amount: 10,
                        fs_line: "BS.EQ.LEG".to_string(),
                    }],
                },
                AppropriationRow {
                    kind: "retained".to_string(),
                    fs_line: None,
                    label: "ماندهٔ محاسباتی".to_string(),
                    amount: 90,
                    accounts: Vec::new(),
                },
            ],
            totals: AppropriationTotals {
                net: 100,
                appropriated: 10,
                retained: 90,
            },
            integrity: Default::default(),
        };
        let appropriation_doc = appropriation_print_doc(&appropriation, &test_meta());
        assert_eq!(appropriation_doc.file_name, "صورت-تخصیص-سود.html");
        assert!(appropriation_doc.html.contains("حساب ۳۱۰۳"));

        let cash_flow = CashFlowReport {
            statement: "cash_flow".to_string(),
            from_date: "1405/01/01".to_string(),
            to_date: "1405/12/29".to_string(),
            sections: vec![
                CashFlowSection {
                    key: "operating".to_string(),
                    title: "فعالیت‌های عملیاتی".to_string(),
                    lines: vec![CashFlowLine {
                        label: "سود (زیان) خالص دوره".to_string(),
                        amount: 100,
                        note: None,
                    }],
                    total: 100,
                },
                CashFlowSection {
                    key: "investing".to_string(),
                    title: "فعالیت‌های سرمایه‌گذاری".to_string(),
                    lines: Vec::new(),
                    total: 0,
                },
                CashFlowSection {
                    key: "financing".to_string(),
                    title: "فعالیت‌های تأمین مالی".to_string(),
                    lines: Vec::new(),
                    total: 0,
                },
            ],
            opening_cash: 0,
            closing_cash: 100,
            net_change: 100,
            variance: 0,
            unresolved_vouchers: vec![CashFlowUnresolvedVoucher {
                voucher_id: 1,
                voucher_number: 7,
                cash_movement: 50,
                reason: "طرف حساب نامشخص <script>".to_string(),
            }],
            classification_complete: false,
            integrity: Default::default(),
        };
        let cash_flow_doc = cash_flow_print_doc(&cash_flow, &test_meta());
        assert_eq!(cash_flow_doc.file_name, "صورت-جریان-های-نقدی.html");
        assert!(cash_flow_doc.html.contains("شماره سند"));
        assert!(cash_flow_doc.html.contains("۷"));
        assert!(cash_flow_doc.html.contains("صورت جریان نقد کامل نیست"));
        assert!(!cash_flow_doc.html.contains("<script>"));
        assert!(cash_flow_doc.html.contains("&lt;script&gt;"));
    }
}
