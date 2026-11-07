//! Canonical financial-statement line catalog.
//!
//! The catalog is a pure, database-free mirror of the WebApp
//! `lib/accounting/fs-catalog.ts`. Reports must be driven by an account's
//! effective `fs_line` (explicit, inherited from an ancestor, or suggested by
//! its group section) rather than by the group type or the first digit of the
//! account code alone. Keeping the catalog in one place means the migration,
//! the resolver and the report engine cannot drift apart.

/// Which statement a line belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FsStatement {
    BalanceSheet,
    IncomeStatement,
    Memorandum,
}

/// Statement section a line belongs to. Keys are stored in
/// `account_groups.default_section` and must match `ACCOUNT_GROUP_SECTIONS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FsSection {
    NonCurrentAsset,
    CurrentAsset,
    Equity,
    NonCurrentLiability,
    CurrentLiability,
    Revenue,
    Cogs,
    SellingExpense,
    AdminExpense,
    OtherOperating,
    FinanceCost,
    OtherNonOperating,
    IncomeTax,
    Memorandum,
    Unclassified,
}

impl FsSection {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::NonCurrentAsset => "non_current_asset",
            Self::CurrentAsset => "current_asset",
            Self::Equity => "equity",
            Self::NonCurrentLiability => "non_current_liability",
            Self::CurrentLiability => "current_liability",
            Self::Revenue => "revenue",
            Self::Cogs => "cogs",
            Self::SellingExpense => "selling_expense",
            Self::AdminExpense => "admin_expense",
            Self::OtherOperating => "other_operating",
            Self::FinanceCost => "finance_cost",
            Self::OtherNonOperating => "other_non_operating",
            Self::IncomeTax => "income_tax",
            Self::Memorandum => "memorandum",
            Self::Unclassified => "unclassified",
        }
    }

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "non_current_asset" => Self::NonCurrentAsset,
            "current_asset" => Self::CurrentAsset,
            "equity" => Self::Equity,
            "non_current_liability" => Self::NonCurrentLiability,
            "current_liability" => Self::CurrentLiability,
            "revenue" => Self::Revenue,
            "cogs" => Self::Cogs,
            "selling_expense" => Self::SellingExpense,
            "admin_expense" => Self::AdminExpense,
            "other_operating" => Self::OtherOperating,
            "finance_cost" => Self::FinanceCost,
            "other_non_operating" => Self::OtherNonOperating,
            "income_tax" => Self::IncomeTax,
            "memorandum" => Self::Memorandum,
            _ => return None,
        })
    }
}

/// Natural (increase) side of a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FsNormalSide {
    Debit,
    Credit,
}

impl FsStatement {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::BalanceSheet => "balance_sheet",
            Self::IncomeStatement => "income_statement",
            Self::Memorandum => "memorandum",
        }
    }
}

impl FsNormalSide {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Debit => "debit",
            Self::Credit => "credit",
        }
    }
}

/// Whether a line holds permanent balances, temporary (period) balances, or
/// memorandum balances that never enter the double-entry result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FsPermanence {
    Permanent,
    Temporary,
    Memorandum,
}

impl FsPermanence {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Permanent => "permanent",
            Self::Temporary => "temporary",
            Self::Memorandum => "memorandum",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct FsLine {
    pub(crate) code: &'static str,
    pub(crate) title: &'static str,
    pub(crate) statement: FsStatement,
    pub(crate) section: FsSection,
    pub(crate) normal_side: FsNormalSide,
    pub(crate) permanence: FsPermanence,
}

pub(crate) const UNCLASSIFIED: &str = "UNCLASSIFIED";

macro_rules! fs_line {
    ($code:expr, $title:expr, $statement:ident, $section:ident, $side:ident, $permanence:ident) => {
        FsLine {
            code: $code,
            title: $title,
            statement: FsStatement::$statement,
            section: FsSection::$section,
            normal_side: FsNormalSide::$side,
            permanence: FsPermanence::$permanence,
        }
    };
}

/// The full statement catalog. Order is the display order inside each
/// statement/section and is stable for the UI.
pub(crate) const FS_LINES: [FsLine; 36] = [
    // ── Non-current assets ────────────────────────────────────────
    fs_line!(
        "BS.NCA.PPE",
        "دارایی ثابت",
        BalanceSheet,
        NonCurrentAsset,
        Debit,
        Permanent
    ),
    fs_line!(
        "BS.NCA.INT",
        "دارایی نامشهود",
        BalanceSheet,
        NonCurrentAsset,
        Debit,
        Permanent
    ),
    fs_line!(
        "BS.NCA.LTI",
        "سرمایه‌گذاری بلندمدت",
        BalanceSheet,
        NonCurrentAsset,
        Debit,
        Permanent
    ),
    fs_line!(
        "BS.NCA.LTR",
        "مطالبات بلندمدت",
        BalanceSheet,
        NonCurrentAsset,
        Debit,
        Permanent
    ),
    fs_line!(
        "BS.NCA.OTH",
        "سایر دارایی‌های غیر جاری",
        BalanceSheet,
        NonCurrentAsset,
        Debit,
        Permanent
    ),
    // ── Current assets ────────────────────────────────────────────
    fs_line!(
        "BS.CA.CASH",
        "موجودی نقد",
        BalanceSheet,
        CurrentAsset,
        Debit,
        Permanent
    ),
    fs_line!(
        "BS.CA.STI",
        "سرمایه‌گذاری کوتاه‌مدت",
        BalanceSheet,
        CurrentAsset,
        Debit,
        Permanent
    ),
    fs_line!(
        "BS.CA.REC",
        "مطالبات تجاری",
        BalanceSheet,
        CurrentAsset,
        Debit,
        Permanent
    ),
    fs_line!(
        "BS.CA.INV",
        "موجودی کالا",
        BalanceSheet,
        CurrentAsset,
        Debit,
        Permanent
    ),
    fs_line!(
        "BS.CA.PREP",
        "پیش‌پرداخت‌ها",
        BalanceSheet,
        CurrentAsset,
        Debit,
        Permanent
    ),
    fs_line!(
        "BS.CA.HFS",
        "دارایی نگهداری برای فروش",
        BalanceSheet,
        CurrentAsset,
        Debit,
        Permanent
    ),
    // ── Equity ────────────────────────────────────────────────────
    fs_line!(
        "BS.EQ.CAP",
        "سرمایه",
        BalanceSheet,
        Equity,
        Credit,
        Permanent
    ),
    fs_line!(
        "BS.EQ.SHP",
        "صرف سهام",
        BalanceSheet,
        Equity,
        Credit,
        Permanent
    ),
    fs_line!(
        "BS.EQ.LEG",
        "اندوخته قانونی",
        BalanceSheet,
        Equity,
        Credit,
        Permanent
    ),
    fs_line!(
        "BS.EQ.RES",
        "سایر اندوخته‌ها",
        BalanceSheet,
        Equity,
        Credit,
        Permanent
    ),
    fs_line!(
        "BS.EQ.REV",
        "تجدید ارزیابی",
        BalanceSheet,
        Equity,
        Credit,
        Permanent
    ),
    fs_line!(
        "BS.EQ.RE",
        "سود و زیان انباشته",
        BalanceSheet,
        Equity,
        Credit,
        Permanent
    ),
    // ── Non-current liabilities ───────────────────────────────────
    fs_line!(
        "BS.NCL.FIN",
        "تسهیلات مالی بلندمدت",
        BalanceSheet,
        NonCurrentLiability,
        Credit,
        Permanent
    ),
    fs_line!(
        "BS.NCL.EOS",
        "مزایای پایان خدمت",
        BalanceSheet,
        NonCurrentLiability,
        Credit,
        Permanent
    ),
    fs_line!(
        "BS.NCL.OTH",
        "سایر بدهی‌های غیر جاری",
        BalanceSheet,
        NonCurrentLiability,
        Credit,
        Permanent
    ),
    // ── Current liabilities ───────────────────────────────────────
    fs_line!(
        "BS.CL.PAY",
        "حساب‌های پرداختنی",
        BalanceSheet,
        CurrentLiability,
        Credit,
        Permanent
    ),
    fs_line!(
        "BS.CL.ADV",
        "پیش‌دریافت‌ها",
        BalanceSheet,
        CurrentLiability,
        Credit,
        Permanent
    ),
    fs_line!(
        "BS.CL.TAX",
        "مالیات پرداختنی",
        BalanceSheet,
        CurrentLiability,
        Credit,
        Permanent
    ),
    fs_line!(
        "BS.CL.DIV",
        "سود سهام پرداختنی",
        BalanceSheet,
        CurrentLiability,
        Credit,
        Permanent
    ),
    fs_line!(
        "BS.CL.FIN",
        "تسهیلات مالی کوتاه‌مدت",
        BalanceSheet,
        CurrentLiability,
        Credit,
        Permanent
    ),
    fs_line!(
        "BS.CL.PRV",
        "ذخایر جاری",
        BalanceSheet,
        CurrentLiability,
        Credit,
        Permanent
    ),
    // ── Income statement ──────────────────────────────────────────
    fs_line!(
        "IS.REV",
        "درآمد فروش",
        IncomeStatement,
        Revenue,
        Credit,
        Temporary
    ),
    fs_line!(
        "IS.COGS",
        "بهای تمام‌شده کالای فروش‌رفته",
        IncomeStatement,
        Cogs,
        Debit,
        Temporary
    ),
    fs_line!(
        "IS.SELL",
        "هزینه‌های فروش",
        IncomeStatement,
        SellingExpense,
        Debit,
        Temporary
    ),
    fs_line!(
        "IS.ADM",
        "هزینه‌های اداری و عمومی",
        IncomeStatement,
        AdminExpense,
        Debit,
        Temporary
    ),
    fs_line!(
        "IS.OOI",
        "سایر درآمدهای عملیاتی",
        IncomeStatement,
        OtherOperating,
        Credit,
        Temporary
    ),
    fs_line!(
        "IS.OOE",
        "سایر هزینه‌های عملیاتی",
        IncomeStatement,
        OtherOperating,
        Debit,
        Temporary
    ),
    fs_line!(
        "IS.FIN",
        "هزینه‌های مالی",
        IncomeStatement,
        FinanceCost,
        Debit,
        Temporary
    ),
    fs_line!(
        "IS.NOI",
        "سایر درآمدها و هزینه‌های غیرعملیاتی",
        IncomeStatement,
        OtherNonOperating,
        Credit,
        Temporary
    ),
    fs_line!(
        "IS.TAX",
        "مالیات بر درآمد",
        IncomeStatement,
        IncomeTax,
        Debit,
        Temporary
    ),
    fs_line!(
        "MEMO",
        "حساب‌های انتظامی",
        Memorandum,
        Memorandum,
        Debit,
        Memorandum
    ),
];

pub(crate) fn get_fs_line(code: &str) -> Option<&'static FsLine> {
    FS_LINES.iter().find(|line| line.code == code)
}

pub(crate) fn is_fs_line_code(code: &str) -> bool {
    get_fs_line(code).is_some()
}

pub(crate) fn sign_of(code: &str) -> i64 {
    match get_fs_line(code) {
        Some(line) if line.normal_side == FsNormalSide::Debit => 1,
        Some(_) => -1,
        // Unclassified balances keep the raw debit−credit direction so an
        // accidental misclassification is still visible and balanced.
        None => 1,
    }
}

pub(crate) fn section_of(code: &str) -> FsSection {
    get_fs_line(code)
        .map(|line| line.section)
        .unwrap_or(FsSection::Unclassified)
}

pub(crate) fn permanence_of(code: &str) -> Option<FsPermanence> {
    get_fs_line(code).map(|line| line.permanence)
}

pub(crate) fn statement_of(code: &str) -> Option<FsStatement> {
    get_fs_line(code).map(|line| line.statement)
}

/// Suggested line for a group section. `nature` lets a dual-nature group pick
/// the income or expense counterpart of an otherwise shared section.
pub(crate) fn section_default_fs_line(section: &str, nature: &str) -> Option<&'static str> {
    Some(match section {
        "non_current_asset" => "BS.NCA.OTH",
        "current_asset" => "BS.CA.REC",
        "equity" => "BS.EQ.RES",
        "non_current_liability" => "BS.NCL.OTH",
        "current_liability" => "BS.CL.PAY",
        "revenue" => "IS.REV",
        "cogs" => "IS.COGS",
        "selling_expense" => "IS.SELL",
        "admin_expense" => "IS.ADM",
        "other_operating" => {
            if nature == "debtor" {
                "IS.OOE"
            } else {
                "IS.OOI"
            }
        }
        "finance_cost" => "IS.FIN",
        // WebApp uses one signed line for other non-operating items; a debit
        // balance shows as an expense through its negative contribution.
        "other_non_operating" => "IS.NOI",
        "income_tax" => "IS.TAX",
        "memorandum" => "MEMO",
        _ => return None,
    })
}

/// Legacy section suggestion used when a group never had a `default_section`
/// assigned. It preserves the pre-classification behaviour of existing books.
pub(crate) fn legacy_section_for(
    group_type: &str,
    group_code: &str,
    account_code: &str,
) -> FsSection {
    match group_type {
        "دارایی" => FsSection::CurrentAsset,
        "بدهی" => FsSection::CurrentLiability,
        "سرمایه" => FsSection::Equity,
        "درآمد" => FsSection::Revenue,
        "هزینه" => FsSection::AdminExpense,
        _ => match group_code {
            "10" => FsSection::OtherOperating,
            "11" => FsSection::OtherNonOperating,
            "9" => FsSection::Cogs,
            _ => match account_code.chars().next() {
                Some('1') => FsSection::CurrentAsset,
                Some('2') => FsSection::CurrentLiability,
                Some('3') => FsSection::Equity,
                Some('4') => FsSection::Revenue,
                _ => FsSection::AdminExpense,
            },
        },
    }
}

pub(crate) fn section_title(section: &str) -> &'static str {
    match section {
        "non_current_asset" => "دارایی غیرجاری",
        "current_asset" => "دارایی جاری",
        "equity" => "حقوق مالکانه",
        "non_current_liability" => "بدهی غیرجاری",
        "current_liability" => "بدهی جاری",
        "revenue" => "درآمد عملیاتی",
        "cogs" => "بهای تمام‌شده",
        "selling_expense" => "هزینه‌های فروش",
        "admin_expense" => "هزینه‌های اداری",
        "other_operating" => "سایر درآمد/هزینه عملیاتی",
        "finance_cost" => "هزینه‌های مالی",
        "other_non_operating" => "سایر درآمد/هزینه غیرعملیاتی",
        "income_tax" => "مالیات بر درآمد",
        "memorandum" => "حساب‌های انتظامی",
        _ => "بدون سرفصل",
    }
}

fn statement_title(statement: FsStatement) -> &'static str {
    match statement {
        FsStatement::BalanceSheet => "صورت وضعیت مالی",
        FsStatement::IncomeStatement => "صورت سود و زیان",
        FsStatement::Memorandum => "حساب‌های انتظامی",
    }
}

fn permanence_title(permanence: FsPermanence) -> &'static str {
    match permanence {
        FsPermanence::Permanent => "دائمی",
        FsPermanence::Temporary => "موقت",
        FsPermanence::Memorandum => "انتظامی",
    }
}

/// Serializable catalog consumed by the account forms. Labels live here so the
/// webview never has to duplicate the Rust catalog.
pub(crate) fn fs_catalog() -> crate::db::FsCatalog {
    use crate::db::classification::{CONTRA_KINDS, SYSTEM_ROLES};
    use crate::db::{FsCatalog, FsCatalogOption, FsLineInfo};

    let lines = FS_LINES
        .iter()
        .map(|line| FsLineInfo {
            code: line.code.to_string(),
            title: line.title.to_string(),
            statement: line.statement.key().to_string(),
            statement_title: statement_title(line.statement).to_string(),
            section: line.section.key().to_string(),
            section_title: section_title(line.section.key()).to_string(),
            normal_side: line.normal_side.key().to_string(),
            permanence: line.permanence.key().to_string(),
            permanence_title: permanence_title(line.permanence).to_string(),
        })
        .collect();

    let contra_kinds = CONTRA_KINDS
        .iter()
        .map(|kind| FsCatalogOption {
            value: (*kind).to_string(),
            label: match *kind {
                "accumulated_depreciation" => "استهلاک انباشته",
                "allowance_receivable" => "ذخیره مطالبات مشکوک",
                "sales_return" => "برگشت از فروش",
                "purchase_return" => "برگشت از خرید",
                "overhead_applied" => "سربار جذب‌شده",
                _ => "سایر",
            }
            .to_string(),
        })
        .collect();

    let system_roles = SYSTEM_ROLES
        .iter()
        .map(|role| FsCatalogOption {
            value: (*role).to_string(),
            label: match *role {
                "income_summary" => "خلاصه سود و زیان",
                "retained_earnings" => "سود (زیان) انباشته",
                "closing_balance" => "تراز اختتامیه",
                "legal_reserve" => "اندوخته قانونی",
                "depreciation_expense" => "هزینه استهلاک",
                "accum_depreciation" => "استهلاک انباشته (سیستمی)",
                _ => role,
            }
            .to_string(),
        })
        .collect();

    FsCatalog {
        lines,
        contra_kinds,
        system_roles,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_codes_are_unique_and_well_formed() {
        let mut seen = std::collections::HashSet::new();
        for line in FS_LINES {
            assert!(seen.insert(line.code), "duplicate fs line {}", line.code);
            assert!(
                line.code == "MEMO"
                    || line.code == "UNCLASSIFIED"
                    || line.code.starts_with("BS.")
                    || line.code.starts_with("IS."),
                "unexpected fs line {}",
                line.code
            );
        }
        assert!(get_fs_line(UNCLASSIFIED).is_none());
    }

    #[test]
    fn sign_follows_normal_side() {
        assert_eq!(sign_of("BS.CA.CASH"), 1);
        assert_eq!(sign_of("IS.REV"), -1);
        assert_eq!(sign_of("IS.COGS"), 1);
    }

    #[test]
    fn section_defaults_cover_every_group_section() {
        for section in crate::db::model::ACCOUNT_GROUP_SECTIONS {
            assert!(
                section_default_fs_line(section, "debtor").is_some() || section == "unclassified",
                "missing default for {section}"
            );
        }
    }
}
