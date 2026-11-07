//! Supplementary financial statements (Phase 7):
//!   1. Statement of Changes in Equity (صورت تغییرات حقوق مالکانه)
//!   2. Statement of Cost of Goods Sold (صورت بهای تمام‌شده کالای فروش‌رفته)
//!   3. Statement of Profit Appropriation (صورت تخصیص سود)
//!   4. Statement of Cash Flows - Indirect Method (صورت جریان‌های نقدی)
//!
//! The four statements follow the WebApp reference engine, with two deliberate
//! corrections:
//!
//! - the opening snapshot is the **latest** opening voucher up to the report
//!   boundary, not every opening voucher ever recorded. Summing all openings
//!   double-counts carried-forward balances once more than one fiscal year is
//!   on the books.
//! - opening and closing balances are classified through the same effective
//!   `fs_line` / contra / split-by-sign resolver as the balance sheet, so a
//!   contra inventory account or an inherited line is never read as a raw code.
//!
//! All amounts stay `i64` and every arithmetic step is checked; the UI only
//! displays the result.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension};

use crate::db::classification::{
    self, resolve_account, AccountClassificationRow, EffectiveClassification,
};
use crate::db::fs_catalog::{get_fs_line, FsStatement, UNCLASSIFIED};
use crate::db::{self, Database};
use crate::vouchers::accounting::{account_activities, activities_from_totals, AccountActivity};
use crate::vouchers::model::{
    AmountRow, AppropriationReport, AppropriationRow, AppropriationTotals, CashFlowLine,
    CashFlowReport, CashFlowSection, CashFlowUnresolvedVoucher, CogsReport, CogsRow, CogsTotals,
    EquityChangesReport, EquityChangesRow, EquityChangesTotals, IntegrityIssue, ReportScope,
    CLOSING_FINAL, CLOSING_TEMPORARY, INTEGRITY_ERROR, INTEGRITY_WARNING, VOUCHER_TYPES,
};
use crate::vouchers::reports::financial_statements::build_income_statement;
use crate::vouchers::reports::integrity::run_integrity_checks;
use crate::vouchers::validation::{
    check_range, checked_add, checked_sub, checked_sum, normalize_date, normalize_filter_date,
};

const EQUITY_FS_LINES: [&str; 6] = [
    "BS.EQ.CAP",
    "BS.EQ.SHP",
    "BS.EQ.LEG",
    "BS.EQ.RES",
    "BS.EQ.REV",
    "BS.EQ.RE",
];

const APPROPRIATION_FS_LINES: [&str; 3] = ["BS.EQ.LEG", "BS.EQ.RES", "BS.CL.DIV"];

const CASH_FS_LINE: &str = "BS.CA.CASH";

const COGS_LIMITATION: &str =
    "این نسخهٔ برنامه ماژول تولید/انبار ندارد؛ صورت بهای تمام‌شده فقط ثبت‌های سرفصل IS.COGS را تجزیه می‌کند و تغییر موجودی صرفاً اطلاعات مکمل است. برای تفکیک مواد مستقیم، دستمزد، سربار و کالای در جریان ساخت باید ماژول تولید و گردش موجودی اضافه شود.";

/// Contra kinds whose movement is a non-cash expense add-back in the indirect
/// cash-flow statement. The Rust taxonomy has no `accum_amort`; amortisation is
/// expected to use `accumulated_depreciation` or `other` with a classified line.
const NON_CASH_CONTRA_KINDS: [&str; 2] = ["accumulated_depreciation", "allowance_receivable"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum CashBucket {
    Operating,
    Investing,
    Financing,
}

impl CashBucket {
    fn key(self) -> &'static str {
        match self {
            Self::Operating => "operating",
            Self::Investing => "investing",
            Self::Financing => "financing",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Operating => "فعالیت‌های عملیاتی",
            Self::Investing => "فعالیت‌های سرمایه‌گذاری",
            Self::Financing => "فعالیت‌های تأمین مالی",
        }
    }
}

/// Statement line placement for the non-cash counterpart of a cash voucher and
/// for the operating balance-delta adjustment. Lines that are not listed fall
/// back to operating, exactly like the WebApp engine.
const CASH_FLOW_LINES: [(&str, CashBucket, &str); 25] = [
    (
        "BS.NCA.PPE",
        CashBucket::Investing,
        "خرید/فروش دارایی‌های ثابت مشهود",
    ),
    (
        "BS.NCA.INT",
        CashBucket::Investing,
        "خرید/فروش دارایی‌های نامشهود",
    ),
    (
        "BS.NCA.LTI",
        CashBucket::Investing,
        "سرمایه‌گذاری‌های بلندمدت",
    ),
    ("BS.NCA.LTR", CashBucket::Investing, "دریافتنی‌های بلندمدت"),
    (
        "BS.NCA.OTH",
        CashBucket::Investing,
        "سایر دارایی‌های غیرجاری",
    ),
    (
        "BS.CA.STI",
        CashBucket::Investing,
        "سرمایه‌گذاری‌های کوتاه‌مدت",
    ),
    (
        "BS.CA.REC",
        CashBucket::Operating,
        "تغییر دریافتنی‌های تجاری",
    ),
    (
        "BS.CA.INV",
        CashBucket::Operating,
        "تغییر موجودی مواد و کالا",
    ),
    ("BS.CA.PREP", CashBucket::Operating, "تغییر پیش‌پرداخت‌ها"),
    (
        "BS.CA.HFS",
        CashBucket::Operating,
        "تغییر دارایی‌های نگه‌داری‌شده برای فروش",
    ),
    ("BS.CL.PAY", CashBucket::Operating, "تغییر حساب‌های پرداختنی"),
    ("BS.CL.ADV", CashBucket::Operating, "تغییر پیش‌دریافت‌ها"),
    ("BS.CL.TAX", CashBucket::Operating, "تغییر مالیات پرداختنی"),
    ("BS.CL.PRV", CashBucket::Operating, "تغییر ذخایر جاری"),
    (
        "BS.NCL.EOS",
        CashBucket::Operating,
        "تغییر ذخیرهٔ مزایای پایان خدمت",
    ),
    (
        "BS.NCL.OTH",
        CashBucket::Operating,
        "تغییر سایر بدهی‌های بلندمدت",
    ),
    (
        "BS.NCL.FIN",
        CashBucket::Financing,
        "تغییر تسهیلات مالی بلندمدت",
    ),
    ("BS.CL.FIN", CashBucket::Financing, "تسهیلات مالی کوتاه‌مدت"),
    ("BS.CL.DIV", CashBucket::Financing, "سود سهام پرداختنی"),
    ("BS.EQ.CAP", CashBucket::Financing, "تغییر سرمایه"),
    ("BS.EQ.SHP", CashBucket::Financing, "تغییر صرف سهام"),
    ("BS.EQ.LEG", CashBucket::Financing, "تغییر اندوختهٔ قانونی"),
    ("BS.EQ.RES", CashBucket::Financing, "تغییر سایر اندوخته‌ها"),
    ("BS.EQ.REV", CashBucket::Financing, "تغییر تجدید ارزیابی"),
    (
        "BS.EQ.RE",
        CashBucket::Financing,
        "تغییر سود (زیان) انباشته",
    ),
];

fn bucket_for(line: &str) -> Option<CashBucket> {
    CASH_FLOW_LINES
        .iter()
        .find(|(code, _, _)| *code == line)
        .map(|(_, bucket, _)| *bucket)
}

fn cash_flow_label(line: &str) -> String {
    CASH_FLOW_LINES
        .iter()
        .find(|(code, _, _)| *code == line)
        .map(|(_, _, label)| (*label).to_string())
        .or_else(|| get_fs_line(line).map(|line| line.title.to_string()))
        .unwrap_or_else(|| line.to_string())
}

/* ── Shared opening / period helpers ─────────────────────────── */

#[derive(Debug, Clone)]
struct OpeningVoucher {
    id: i64,
    date: String,
}

/// Latest opening voucher at or before `boundary`. A later opening replaces
/// earlier ones instead of adding to them, which is the whole point of a
/// carry-forward snapshot.
fn latest_opening(
    conn: &Connection,
    boundary: &str,
    scope: ReportScope,
) -> Result<Option<OpeningVoucher>, String> {
    let scope_sql = scope.sql("v");
    let mut stmt = conn
        .prepare(&format!(
            "SELECT v.id, v.date
               FROM vouchers v
              WHERE v.type = ?1
                AND (?2 = '' OR v.date <= ?2)
                AND {scope_sql}
              ORDER BY v.date DESC, v.id DESC
              LIMIT 1"
        ))
        .map_err(db::db_error)?;
    stmt.query_row(params![VOUCHER_TYPES[1], boundary], |row| {
        Ok(OpeningVoucher {
            id: row.get(0)?,
            date: row.get(1)?,
        })
    })
    .optional()
    .map_err(db::db_error)
}

fn pairs_for_voucher(
    conn: &Connection,
    voucher_id: i64,
) -> Result<HashMap<String, (i64, i64)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT COALESCE(NULLIF(l.detail1_code, ''), l.account_code),
                    COALESCE(SUM(l.debit), 0),
                    COALESCE(SUM(l.credit), 0)
               FROM voucher_lines l
              WHERE l.voucher_id = ?1
              GROUP BY 1",
        )
        .map_err(db::db_error)?;
    let rows = stmt
        .query_map(params![voucher_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(db::db_error)?;
    let mut pairs = HashMap::new();
    for row in rows {
        let (code, debit, credit) = row.map_err(db::db_error)?;
        pairs.insert(code, (debit, credit));
    }
    Ok(pairs)
}

/// Normal/temporary-closing movements between two dates. Opening and final
/// closing vouchers are excluded because the opening snapshot is loaded
/// separately and a closing voucher must not turn a period result into zero.
fn statement_pairs(
    conn: &Connection,
    from: &str,
    to: &str,
    inclusive_to: bool,
    scope: ReportScope,
) -> Result<HashMap<String, (i64, i64)>, String> {
    let scope_sql = scope.sql("v");
    let date_predicate = if inclusive_to {
        "(?2 = '' OR v.date <= ?2)"
    } else {
        "(?2 = '' OR v.date < ?2)"
    };
    let sql = format!(
        "SELECT COALESCE(NULLIF(l.detail1_code, ''), l.account_code),
                COALESCE(SUM(l.debit), 0),
                COALESCE(SUM(l.credit), 0)
           FROM voucher_lines l
           JOIN vouchers v ON v.id = l.voucher_id
          WHERE (?1 = '' OR v.date >= ?1)
            AND {date_predicate}
            AND v.type <> ?3
            AND v.type <> ?4
            AND v.type <> ?5
            AND {scope_sql}
          GROUP BY 1"
    );
    let mut stmt = conn.prepare(&sql).map_err(db::db_error)?;
    let rows = stmt
        .query_map(
            params![from, to, VOUCHER_TYPES[1], CLOSING_TEMPORARY, CLOSING_FINAL],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .map_err(db::db_error)?;
    let mut pairs = HashMap::new();
    for row in rows {
        let (code, debit, credit) = row.map_err(db::db_error)?;
        pairs.insert(code, (debit, credit));
    }
    Ok(pairs)
}

fn opening_activities(
    conn: &Connection,
    opening: Option<&OpeningVoucher>,
) -> Result<Vec<AccountActivity>, String> {
    match opening {
        Some(opening) => {
            let pairs = pairs_for_voucher(conn, opening.id)?;
            activities_from_totals(conn, &pairs)
        }
        None => Ok(Vec::new()),
    }
}

fn line_total(activities: &[AccountActivity], line: &str) -> Result<i64, String> {
    checked_sum(
        activities
            .iter()
            .filter(|activity| activity.contribution_line == line)
            .map(|activity| activity.contribution_amount),
        "جمع سرفصل صورت مالی",
    )
}

/// Per-account closing amounts for a statement line, combining the opening
/// snapshot and the period movement.
fn combined_rows(sources: &[&[AccountActivity]], line: &str) -> Result<Vec<AmountRow>, String> {
    let mut amounts: HashMap<String, AmountRow> = HashMap::new();
    for source in sources {
        for activity in *source {
            if activity.contribution_line != line || activity.contribution_amount == 0 {
                continue;
            }
            let entry = amounts
                .entry(activity.account_code.clone())
                .or_insert_with(|| AmountRow {
                    account_code: activity.account_code.clone(),
                    account_name: activity.account_name.clone(),
                    amount: 0,
                    fs_line: line.to_string(),
                });
            entry.amount = checked_add(
                entry.amount,
                activity.contribution_amount,
                "جمع حساب در سرفصل",
            )?;
        }
    }
    let mut rows: Vec<AmountRow> = amounts
        .into_values()
        .filter(|row| row.amount != 0)
        .collect();
    rows.sort_by(|left, right| left.account_code.cmp(&right.account_code));
    Ok(rows)
}

/// Period profit from the latest opening date. Opening vouchers are invalid
/// places for temporary accounts, and closing vouchers move the result to
/// equity; neither may become period profit.
fn net_income_between(
    conn: &Connection,
    from: &str,
    to: &str,
    scope: ReportScope,
) -> Result<i64, String> {
    let activities = account_activities(
        conn,
        from,
        to,
        false,
        &[CLOSING_TEMPORARY, CLOSING_FINAL, VOUCHER_TYPES[1]],
        scope,
    )?;
    Ok(build_income_statement(from.to_string(), to.to_string(), activities)?.net_profit)
}

impl Database {
    /* ── Statement of Changes in Equity ─────────────────────────── */

    pub fn get_equity_changes(&self, as_of: &str) -> Result<EquityChangesReport, String> {
        self.get_equity_changes_scoped(as_of, ReportScope::Working)
    }

    pub fn get_equity_changes_scoped(
        &self,
        as_of: &str,
        scope: ReportScope,
    ) -> Result<EquityChangesReport, String> {
        let as_of = normalize_date(as_of, "تاریخ گزارش")?;

        self.with_conn(|conn| {
            let opening = latest_opening(conn, &as_of, scope)?;
            let opening_date = opening
                .as_ref()
                .map(|voucher| voucher.date.clone())
                .unwrap_or_default();
            let opening_activities = opening_activities(conn, opening.as_ref())?;
            let period_pairs = statement_pairs(conn, &opening_date, &as_of, true, scope)?;
            let period_activities = activities_from_totals(conn, &period_pairs)?;
            let net_income = net_income_between(conn, &opening_date, &as_of, scope)?;

            let mut rows = Vec::new();
            let mut opening_total = 0i64;
            let mut closing_total = 0i64;

            for &line in &EQUITY_FS_LINES {
                let opening_amount = line_total(&opening_activities, line)?;
                let movement = line_total(&period_activities, line)?;
                let closing_amount =
                    checked_add(opening_amount, movement, "مانده پایانی حقوق مالکانه")?;
                if opening_amount == 0 && closing_amount == 0 {
                    continue;
                }
                opening_total =
                    checked_add(opening_total, opening_amount, "جمع ابتدایی حقوق مالکانه")?;
                closing_total =
                    checked_add(closing_total, closing_amount, "جمع پایانی حقوق مالکانه")?;
                let change = checked_sub(closing_amount, opening_amount, "تغییر حقوق مالکانه")?;
                let title = get_fs_line(line)
                    .map(|line| line.title)
                    .unwrap_or(line)
                    .to_string();
                let accounts = combined_rows(&[&opening_activities, &period_activities], line)?;
                rows.push(EquityChangesRow {
                    fs_line: line.to_string(),
                    title,
                    kind: "line".to_string(),
                    opening: opening_amount,
                    change,
                    closing: closing_amount,
                    accounts,
                });
            }

            rows.push(EquityChangesRow {
                fs_line: "BS.EQ.PROFIT".to_string(),
                title: "سود (زیان) خالص دوره".to_string(),
                kind: "profit".to_string(),
                opening: 0,
                change: net_income,
                closing: net_income,
                accounts: Vec::new(),
            });
            closing_total = checked_add(closing_total, net_income, "جمع پایانی با سود دوره")?;

            let total_change = checked_sub(closing_total, opening_total, "تغییر کل حقوق مالکانه")?;
            rows.push(EquityChangesRow {
                fs_line: "BS.EQ.TOTAL".to_string(),
                title: "جمع حقوق مالکانه".to_string(),
                kind: "total".to_string(),
                opening: opening_total,
                change: total_change,
                closing: closing_total,
                accounts: Vec::new(),
            });

            let totals = EquityChangesTotals {
                opening: opening_total,
                change: total_change,
                closing: closing_total,
                net_income,
            };

            let integrity = run_integrity_checks(conn, "", &as_of, true, scope, None)?;

            Ok(EquityChangesReport {
                statement: "equity_changes".to_string(),
                to_date: as_of,
                rows,
                totals,
                integrity,
            })
        })
    }

    /* ── Statement of Cost of Goods Sold ────────────────────────── */

    pub fn get_cogs(&self, from: &str, to: &str) -> Result<CogsReport, String> {
        self.get_cogs_scoped(from, to, ReportScope::Working)
    }

    pub fn get_cogs_scoped(
        &self,
        from: &str,
        to: &str,
        scope: ReportScope,
    ) -> Result<CogsReport, String> {
        let from = normalize_filter_date(from, "تاریخ شروع")?;
        let to = normalize_filter_date(to, "تاریخ پایان")?;
        check_range(&from, &to)?;

        self.with_conn(|conn| {
            let opening = latest_opening(conn, &to, scope)?;
            let opening_date = opening
                .as_ref()
                .map(|voucher| voucher.date.clone())
                .unwrap_or_default();
            // A report that starts before the current fiscal year cannot use
            // prior-year turnover: those amounts are already inside the opening
            // snapshot. Start at the opening date instead.
            let effective_from =
                if from.is_empty() || (!opening_date.is_empty() && from < opening_date) {
                    opening_date.clone()
                } else {
                    from.clone()
                };

            let opening_activities = opening_activities(conn, opening.as_ref())?;
            let before_activities = if !opening_date.is_empty() && effective_from > opening_date {
                let pairs = statement_pairs(conn, &opening_date, &effective_from, false, scope)?;
                activities_from_totals(conn, &pairs)?
            } else {
                Vec::new()
            };
            let period_pairs = statement_pairs(conn, &effective_from, &to, true, scope)?;
            let period_activities = activities_from_totals(conn, &period_pairs)?;

            let opening_inventory = checked_add(
                line_total(&opening_activities, "BS.CA.INV")?,
                line_total(&before_activities, "BS.CA.INV")?,
                "مانده ابتدایی موجودی کالا",
            )?;
            let closing_inventory = checked_add(
                opening_inventory,
                line_total(&period_activities, "BS.CA.INV")?,
                "مانده پایانی موجودی کالا",
            )?;
            let inventory_change =
                checked_sub(closing_inventory, opening_inventory, "تغییر موجودی کالا")?;

            let mut cogs_items: Vec<AmountRow> = Vec::new();
            for activity in &period_activities {
                if activity.contribution_line == "IS.COGS" && activity.contribution_amount != 0 {
                    cogs_items.push(AmountRow {
                        account_code: activity.account_code.clone(),
                        account_name: activity.account_name.clone(),
                        amount: activity.contribution_amount,
                        fs_line: activity.contribution_line.clone(),
                    });
                }
            }
            let costs = checked_sum(
                cogs_items.iter().map(|item| item.amount),
                "بهای تمام‌شده کالای فروش‌رفته",
            )?;
            let detailed = cogs_items.len() > 1;

            let mut rows = Vec::new();
            if detailed {
                for item in &cogs_items {
                    rows.push(CogsRow {
                        kind: "cost".to_string(),
                        label: format!("{} — {}", item.account_code, item.account_name),
                        code: Some(item.account_code.clone()),
                        amount: item.amount,
                        note: None,
                    });
                }
            } else {
                rows.push(CogsRow {
                    kind: "cost".to_string(),
                    label: "بهای تمام‌شدهٔ ثبت‌شدهٔ کالای فروش‌رفته".to_string(),
                    code: None,
                    amount: costs,
                    note: None,
                });
            }

            rows.push(CogsRow {
                kind: "inventory".to_string(),
                label: "تغییر موجودی مواد و کالا (اطلاعات مکمل)".to_string(),
                code: None,
                amount: inventory_change,
                note: Some(
                    "افزایش/کاهش موجودی در بازه؛ در جمع هزینهٔ فروش دوباره اعمال نمی‌شود".to_string(),
                ),
            });

            rows.push(CogsRow {
                kind: "total".to_string(),
                label: "بهای تمام‌شدهٔ کالای فروش‌رفته".to_string(),
                code: None,
                amount: costs,
                note: None,
            });

            let totals = CogsTotals {
                costs,
                opening_inventory,
                closing_inventory,
                cogs: costs,
            };

            let integrity = run_integrity_checks(conn, &effective_from, &to, false, scope, None)?;

            Ok(CogsReport {
                statement: "cogs".to_string(),
                from_date: effective_from,
                to_date: to,
                rows,
                totals,
                detailed,
                limitation: COGS_LIMITATION.to_string(),
                integrity,
            })
        })
    }

    /* ── Statement of Profit Appropriation ──────────────────────── */

    pub fn get_profit_appropriation(&self, as_of: &str) -> Result<AppropriationReport, String> {
        self.get_profit_appropriation_scoped(as_of, ReportScope::Working)
    }

    pub fn get_profit_appropriation_scoped(
        &self,
        as_of: &str,
        scope: ReportScope,
    ) -> Result<AppropriationReport, String> {
        let as_of = normalize_date(as_of, "تاریخ گزارش")?;

        self.with_conn(|conn| {
            let opening = latest_opening(conn, &as_of, scope)?;
            let opening_date = opening
                .as_ref()
                .map(|voucher| voucher.date.clone())
                .unwrap_or_default();
            let net_income = net_income_between(conn, &opening_date, &as_of, scope)?;

            let account_rows = classification::load_account_classifications(conn)?;
            let by_code: HashMap<String, AccountClassificationRow> = account_rows
                .iter()
                .map(|row| (row.code.clone(), row.clone()))
                .collect();
            let effective: HashMap<String, EffectiveClassification> = account_rows
                .iter()
                .map(|row| (row.code.clone(), resolve_account(row, &by_code)))
                .collect();
            let line_of = |code: &str| -> String {
                effective
                    .get(code)
                    .map(|classification| classification.fs_line.clone())
                    .unwrap_or_else(|| UNCLASSIFIED.to_string())
            };
            let is_destination = |code: &str| {
                APPROPRIATION_FS_LINES.contains(&line_of(code).as_str())
            };
            let is_retained = |code: &str| {
                line_of(code) == "BS.EQ.RE"
                    && by_code
                        .get(code)
                        .is_some_and(|account| account.system_role != "income_summary")
            };

            #[derive(Debug, Clone)]
            struct Posting {
                code: String,
                debit: i64,
                credit: i64,
            }

            let scope_sql = scope.sql("v");
            let sql = format!(
                "SELECT v.id, v.number,
                        COALESCE(NULLIF(l.detail1_code, ''), l.account_code) AS code,
                        COALESCE(SUM(l.debit), 0),
                        COALESCE(SUM(l.credit), 0)
                   FROM voucher_lines l
                   JOIN vouchers v ON v.id = l.voucher_id
                  WHERE v.type = ?1
                    AND (?2 = '' OR v.date <= ?2)
                    AND {scope_sql}
                  GROUP BY v.id, 3
                  ORDER BY v.date, v.number, v.id"
            );
            let mut stmt = conn.prepare(&sql).map_err(db::db_error)?;
            let postings = stmt
                .query_map(params![VOUCHER_TYPES[0], as_of], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                })
                .map_err(db::db_error)?;

            let mut order: Vec<i64> = Vec::new();
            let mut vouchers: HashMap<i64, (i64, Vec<Posting>)> = HashMap::new();
            for row in postings {
                let (id, number, code, debit, credit) = row.map_err(db::db_error)?;
                let entry = vouchers.entry(id).or_insert_with(|| {
                    order.push(id);
                    (number, Vec::new())
                });
                entry.1.push(Posting {
                    code,
                    debit,
                    credit,
                });
            }

            let mut approved: HashMap<String, i64> = HashMap::new();
            let mut excluded: Vec<i64> = Vec::new();
            let mut recognized = 0i64;

            for id in order {
                let Some((number, entries)) = vouchers.remove(&id) else {
                    continue;
                };
                let active: Vec<&Posting> = entries
                    .iter()
                    .filter(|entry| entry.debit != entry.credit)
                    .collect();
                let destinations: Vec<&&Posting> = active
                    .iter()
                    .filter(|entry| is_destination(&entry.code))
                    .collect();
                if destinations.is_empty() {
                    continue;
                }
                let retained: Vec<&&Posting> = active
                    .iter()
                    .filter(|entry| is_retained(&entry.code))
                    .collect();

                let mut retained_movement = 0i64;
                for entry in &retained {
                    retained_movement = checked_add(
                        retained_movement,
                        checked_sub(entry.debit, entry.credit, "گردش سود انباشته")?,
                        "گردش سود انباشته",
                    )?;
                }
                let direction = retained_movement.signum();
                let mut voucher_total = 0i64;
                for entry in &active {
                    voucher_total = checked_add(
                        voucher_total,
                        checked_sub(entry.debit, entry.credit, "جمع سند تخصیص سود")?,
                        "جمع سند تخصیص سود",
                    )?;
                }

                let verified = direction != 0
                    && retained.len() + destinations.len() == active.len()
                    && voucher_total == 0
                    && retained.iter().all(|entry| {
                        checked_sub(entry.debit, entry.credit, "گردش سود انباشته")
                            .map(|value| value.signum() == direction)
                            .unwrap_or(false)
                    })
                    && destinations.iter().all(|entry| {
                        checked_sub(entry.credit, entry.debit, "گردش مقصد تخصیص")
                            .map(|value| value.signum() == direction)
                            .unwrap_or(false)
                    });

                if !verified {
                    excluded.push(number);
                    continue;
                }

                recognized += 1;
                for entry in destinations {
                    let amount = checked_sub(entry.credit, entry.debit, "مبلغ تخصیص سود")?;
                    let current = approved.entry(entry.code.clone()).or_insert(0);
                    *current = checked_add(*current, amount, "مبلغ تخصیص سود")?;
                }
            }

            let mut items_by_line: HashMap<&str, Vec<AmountRow>> = HashMap::new();
            for (code, amount) in &approved {
                if *amount == 0 {
                    continue;
                }
                let line = line_of(code);
                let Some(fs_line) = APPROPRIATION_FS_LINES
                    .iter()
                    .find(|candidate| **candidate == line)
                else {
                    continue;
                };
                let account_name = by_code
                    .get(code)
                    .map(|account| account.name.clone())
                    .unwrap_or_else(|| code.clone());
                items_by_line
                    .entry(fs_line)
                    .or_default()
                    .push(AmountRow {
                        account_code: code.clone(),
                        account_name,
                        amount: *amount,
                        fs_line: fs_line.to_string(),
                    });
            }

            let mut rows = Vec::new();
            rows.push(AppropriationRow {
                kind: "net".to_string(),
                fs_line: None,
                label: "سود (زیان) خالص دوره".to_string(),
                amount: net_income,
                accounts: Vec::new(),
            });

            let mut total_appropriated = 0i64;
            for &line in &APPROPRIATION_FS_LINES {
                let Some(items) = items_by_line.get_mut(line) else {
                    continue;
                };
                items.sort_by(|left, right| left.account_code.cmp(&right.account_code));
                let amount = checked_sum(
                    items.iter().map(|item| item.amount),
                    "جمع تخصیص سود",
                )?;
                total_appropriated =
                    checked_add(total_appropriated, amount, "جمع تخصیص سود")?;
                let label = get_fs_line(line)
                    .map(|line| line.title)
                    .unwrap_or(line)
                    .to_string();
                rows.push(AppropriationRow {
                    kind: "appropriation".to_string(),
                    fs_line: Some(line.to_string()),
                    label,
                    amount,
                    accounts: items.clone(),
                });
            }

            if rows.len() > 1 {
                rows.push(AppropriationRow {
                    kind: "total".to_string(),
                    fs_line: None,
                    label: "جمع تخصیص‌ها".to_string(),
                    amount: total_appropriated,
                    accounts: Vec::new(),
                });
            }

            let retained = checked_sub(net_income, total_appropriated, "مانده پس از تخصیص")?;
            rows.push(AppropriationRow {
                kind: "retained".to_string(),
                fs_line: None,
                label: "ماندهٔ محاسباتی سود (زیان) پس از تخصیص‌های شناسایی‌شده".to_string(),
                amount: retained,
                accounts: Vec::new(),
            });

            let totals = AppropriationTotals {
                net: net_income,
                appropriated: total_appropriated,
                retained,
            };

            let mut integrity = run_integrity_checks(conn, "", &as_of, true, scope, None)?;
            if !excluded.is_empty() {
                let sample = excluded
                    .iter()
                    .take(5)
                    .map(|number| db::to_fa_digits(*number))
                    .collect::<Vec<_>>()
                    .join("، ");
                integrity.issues.push(IntegrityIssue {
                    code: "appropriation_movements".to_string(),
                    severity: INTEGRITY_WARNING.to_string(),
                    message: format!(
                        "گردش مقصد تخصیص در {} سند (از جمله {}) انتقال مستقل از سود انباشته نیست؛ پرداخت سود سهام، آورده و ثبت‌های مرکب در مبلغ تخصیص وارد نشده‌اند و باید جداگانه بررسی شوند.",
                        db::to_fa_digits(excluded.len() as i64),
                        sample
                    ),
                    count: Some(excluded.len() as i64),
                    amount: None,
                    accounts: Vec::new(),
                });
                integrity.counts.warning += 1;
            }
            if recognized > 0 {
                integrity.issues.push(IntegrityIssue {
                    code: "appropriation_basis".to_string(),
                    severity: INTEGRITY_WARNING.to_string(),
                    message: "تخصیص‌ها انتقال‌های ثبت‌شده از سود انباشته به اندوخته یا سود سهام هستند؛ تعلق آن‌ها به سود همین سال نیازمند مصوبه است. ماندهٔ گزارش محاسباتی است و ثبت انتقال سود را اثبات نمی‌کند."
                        .to_string(),
                    count: Some(recognized),
                    amount: None,
                    accounts: Vec::new(),
                });
                integrity.counts.warning += 1;
            }

            Ok(AppropriationReport {
                statement: "profit_appropriation".to_string(),
                to_date: as_of,
                rows,
                totals,
                integrity,
            })
        })
    }

    /* ── Statement of Cash Flows (Indirect Method) ──────────────── */

    pub fn get_cash_flow(&self, as_of: &str) -> Result<CashFlowReport, String> {
        self.get_cash_flow_scoped(as_of, ReportScope::Working)
    }

    pub fn get_cash_flow_scoped(
        &self,
        as_of: &str,
        scope: ReportScope,
    ) -> Result<CashFlowReport, String> {
        let as_of = normalize_date(as_of, "تاریخ گزارش")?;

        self.with_conn(|conn| {
            let opening = latest_opening(conn, &as_of, scope)?;
            let opening_date = opening
                .as_ref()
                .map(|voucher| voucher.date.clone())
                .unwrap_or_default();
            let opening_activities = opening_activities(conn, opening.as_ref())?;
            let period_pairs = statement_pairs(conn, &opening_date, &as_of, true, scope)?;
            let period_activities = activities_from_totals(conn, &period_pairs)?;

            let opening_cash = line_total(&opening_activities, CASH_FS_LINE)?;
            let closing_cash = checked_add(
                opening_cash,
                line_total(&period_activities, CASH_FS_LINE)?,
                "مانده پایانی نقد",
            )?;
            let net_change = checked_sub(closing_cash, opening_cash, "تغییر خالص نقد")?;
            let net_income = net_income_between(conn, &opening_date, &as_of, scope)?;

            let account_rows = classification::load_account_classifications(conn)?;
            let by_code: HashMap<String, AccountClassificationRow> = account_rows
                .iter()
                .map(|row| (row.code.clone(), row.clone()))
                .collect();
            let effective: HashMap<String, EffectiveClassification> = account_rows
                .iter()
                .map(|row| (row.code.clone(), resolve_account(row, &by_code)))
                .collect();

            #[derive(Debug, Clone)]
            struct CashFlowEntry {
                label: String,
                amount: i64,
                note: Option<String>,
            }

            fn add_entry(
                map: &mut HashMap<String, CashFlowEntry>,
                key: String,
                label: String,
                amount: i64,
                note: Option<String>,
            ) -> Result<(), String> {
                if amount == 0 {
                    return Ok(());
                }
                let entry = map
                    .entry(key)
                    .or_insert_with(|| CashFlowEntry {
                        label,
                        amount: 0,
                        note,
                    });
                entry.amount = checked_add(entry.amount, amount, "جمع جریان نقد")?;
                Ok(())
            }

            let mut operating: HashMap<String, CashFlowEntry> = HashMap::new();
            let mut investing: HashMap<String, CashFlowEntry> = HashMap::new();
            let mut financing: HashMap<String, CashFlowEntry> = HashMap::new();

            // 1) Operating balance-delta adjustments. A balance movement alone
            // does not prove cash flow, so only operating lines enter here;
            // investing/financing amounts come from the voucher-level analysis.
            for (code, (debit, credit)) in &period_pairs {
                if debit == credit {
                    continue;
                }
                let Some(classification) = effective.get(code) else {
                    continue;
                };
                let line = classification.fs_line.as_str();
                if line == CASH_FS_LINE {
                    continue;
                }
                let unclassified = line == UNCLASSIFIED;
                if !unclassified && classification.statement != FsStatement::BalanceSheet {
                    continue;
                }
                let amount = checked_sub(*credit, *debit, "تعدیل تغییر مانده حساب")?;
                if amount == 0 {
                    continue;
                }
                let account = by_code.get(code);
                let is_non_cash_contra = account.is_some_and(|account| {
                    NON_CASH_CONTRA_KINDS.contains(&account.contra_kind.as_str())
                });
                // Investing/financing movements are not operating adjustments;
                // their cash amounts come only from the voucher-level analysis
                // below. Non-cash contra movements stay here as add-backs.
                if !unclassified
                    && !is_non_cash_contra
                    && bucket_for(line) != Some(CashBucket::Operating)
                {
                    continue;
                }
                let (key, label) = if is_non_cash_contra {
                    (
                        format!("contra:{code}"),
                        format!(
                            "افزودن هزینهٔ غیرنقدی: {}",
                            account.map(|account| account.name.as_str()).unwrap_or(code)
                        ),
                    )
                } else if unclassified {
                    (
                        format!("unc:{code}"),
                        format!(
                            "طبقه‌بندی‌نشده: {}",
                            account.map(|account| account.name.as_str()).unwrap_or(code)
                        ),
                    )
                } else {
                    (line.to_string(), cash_flow_label(line))
                };
                add_entry(&mut operating, key, label, amount, None)?;
            }

            // 2) Voucher-level cash counterpart analysis. A voucher with cash
            // and counterparts from more than one activity (or an unreliable
            // line) is reported instead of guessed.
            let scope_sql = scope.sql("v");
            let sql = format!(
                "SELECT v.id, v.number,
                        COALESCE(NULLIF(l.detail1_code, ''), l.account_code) AS code,
                        COALESCE(SUM(l.debit), 0),
                        COALESCE(SUM(l.credit), 0)
                   FROM voucher_lines l
                   JOIN vouchers v ON v.id = l.voucher_id
                  WHERE (?1 = '' OR v.date >= ?1)
                    AND (?2 = '' OR v.date <= ?2)
                    AND v.type <> ?3
                    AND v.type <> ?4
                    AND v.type <> ?5
                    AND {scope_sql}
                  GROUP BY v.id, 3
                  ORDER BY v.date, v.number, v.id"
            );
            let mut stmt = conn.prepare(&sql).map_err(db::db_error)?;
            let postings = stmt
                .query_map(
                    params![
                        opening_date,
                        as_of,
                        VOUCHER_TYPES[1],
                        CLOSING_TEMPORARY,
                        CLOSING_FINAL
                    ],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, i64>(4)?,
                        ))
                    },
                )
                .map_err(db::db_error)?;

            #[derive(Debug, Clone)]
            struct CashPosting {
                code: String,
                debit: i64,
                credit: i64,
            }

            let mut order: Vec<i64> = Vec::new();
            let mut vouchers: HashMap<i64, (i64, Vec<CashPosting>)> = HashMap::new();
            for row in postings {
                let (id, number, code, debit, credit) = row.map_err(db::db_error)?;
                let entry = vouchers.entry(id).or_insert_with(|| {
                    order.push(id);
                    (number, Vec::new())
                });
                entry.1.push(CashPosting {
                    code,
                    debit,
                    credit,
                });
            }

            let mut unresolved_vouchers: Vec<CashFlowUnresolvedVoucher> = Vec::new();
            let mut operating_cash = 0i64;

            for id in order {
                let Some((number, entries)) = vouchers.remove(&id) else {
                    continue;
                };
                let is_cash = |code: &str| {
                    effective
                        .get(code)
                        .is_some_and(|classification| classification.fs_line == CASH_FS_LINE)
                };
                let cash_entries: Vec<&CashPosting> =
                    entries.iter().filter(|entry| is_cash(&entry.code)).collect();
                if cash_entries.is_empty() {
                    continue;
                }
                let mut cash_movement = 0i64;
                for entry in &cash_entries {
                    cash_movement = checked_add(
                        cash_movement,
                        checked_sub(entry.debit, entry.credit, "گردش نقد سند")?,
                        "گردش نقد سند",
                    )?;
                }
                let counterparts: Vec<&CashPosting> = entries
                    .iter()
                    .filter(|entry| !is_cash(&entry.code) && entry.debit != entry.credit)
                    .collect();
                let activity_of = |code: &str| -> Option<CashBucket> {
                    let classification = effective.get(code)?;
                    if classification.statement == FsStatement::IncomeStatement {
                        return Some(CashBucket::Operating);
                    }
                    if classification.statement != FsStatement::BalanceSheet {
                        return None;
                    }
                    bucket_for(&classification.fs_line)
                };
                let activities: HashSet<Option<CashBucket>> =
                    counterparts.iter().map(|entry| activity_of(&entry.code)).collect();

                let mut voucher_total = 0i64;
                for entry in &entries {
                    voucher_total = checked_add(
                        voucher_total,
                        checked_sub(entry.debit, entry.credit, "جمع سند دارای گردش نقد")?,
                        "جمع سند دارای گردش نقد",
                    )?;
                }

                let reason = if activities.contains(&None) {
                    Some("طرفِ حسابِ نقد سرفصلِ قابل اتکای جریان نقد ندارد.")
                } else if activities.len() > 1 {
                    Some("سند شامل چند فعالیت است؛ تخصیص مبلغ نقد به هر فعالیت باید مشخص شود.")
                } else if voucher_total != 0 {
                    Some("سندِ دارای گردش نقد متوازن نیست.")
                } else {
                    None
                };

                if let Some(reason) = reason {
                    unresolved_vouchers.push(CashFlowUnresolvedVoucher {
                        voucher_id: id,
                        voucher_number: number,
                        cash_movement,
                        reason: reason.to_string(),
                    });
                    continue;
                }

                let Some(activity) = activities.iter().next().and_then(|value| *value) else {
                    // Only cash accounts on both sides: an internal transfer.
                    continue;
                };

                if activity == CashBucket::Operating {
                    operating_cash =
                        checked_add(operating_cash, cash_movement, "جریان نقد عملیاتی")?;
                    continue;
                }

                let signs: HashSet<i64> = counterparts
                    .iter()
                    .map(|entry| (entry.debit - entry.credit).signum())
                    .collect();
                let target = if activity == CashBucket::Investing {
                    &mut investing
                } else {
                    &mut financing
                };
                if signs.len() > 1 {
                    let direction = if cash_movement >= 0 {
                        "دریافت"
                    } else {
                        "پرداخت"
                    };
                    let activity_title = if activity == CashBucket::Investing {
                        "سرمایه‌گذاری"
                    } else {
                        "تأمین مالی"
                    };
                    add_entry(
                        target,
                        format!("voucher:{id}"),
                        format!(
                            "خالص {direction} نقدی {activity_title} — سند {}",
                            db::to_fa_digits(number)
                        ),
                        cash_movement,
                        Some(
                            "طرف‌های غیرنقدیِ سند مرکب در مبلغ جریان نقد وارد نشده‌اند."
                                .to_string(),
                        ),
                    )?;
                } else {
                    for entry in counterparts {
                        let line = effective
                            .get(&entry.code)
                            .map(|classification| classification.fs_line.clone())
                            .unwrap_or_else(|| UNCLASSIFIED.to_string());
                        let amount = checked_sub(entry.credit, entry.debit, "جریان نقد سند")?;
                        add_entry(
                            target,
                            line.clone(),
                            cash_flow_label(&line),
                            amount,
                            None,
                        )?;
                    }
                }
            }

            let mut indirect_operating = net_income;
            for entry in operating.values() {
                indirect_operating = checked_add(
                    indirect_operating,
                    entry.amount,
                    "سود خالص تعدیل‌شده عملیاتی",
                )?;
            }
            let reclassification = checked_sub(
                operating_cash,
                indirect_operating,
                "تعدیل طبقه‌بندی جریان نقد",
            )?;
            add_entry(
                &mut operating,
                "noncash-reclassification".to_string(),
                "تعدیلات معاملات غیرنقدی و تجدید طبقه‌بندی گردش‌های دفتر".to_string(),
                reclassification,
                None,
            )?;

            fn build_section(
                bucket: CashBucket,
                first_line: Option<CashFlowLine>,
                map: HashMap<String, CashFlowEntry>,
            ) -> Result<CashFlowSection, String> {
                let mut lines = Vec::new();
                if let Some(line) = first_line {
                    lines.push(line);
                }
                let mut entries: Vec<CashFlowEntry> = map.into_values().collect();
                entries.sort_by(|left, right| left.label.cmp(&right.label));
                for entry in entries {
                    lines.push(CashFlowLine {
                        label: entry.label,
                        amount: entry.amount,
                        note: entry.note,
                    });
                }
                let total = checked_sum(
                    lines.iter().map(|line| line.amount),
                    "جمع بخش جریان نقد",
                )?;
                Ok(CashFlowSection {
                    key: bucket.key().to_string(),
                    title: bucket.title().to_string(),
                    lines,
                    total,
                })
            }

            let sections = vec![
                build_section(
                    CashBucket::Operating,
                    Some(CashFlowLine {
                        label: "سود (زیان) خالص دوره".to_string(),
                        amount: net_income,
                        note: None,
                    }),
                    operating,
                )?,
                build_section(CashBucket::Investing, None, investing)?,
                build_section(CashBucket::Financing, None, financing)?,
            ];

            let calculated_change = checked_sum(
                sections.iter().map(|section| section.total),
                "جمع کل جریان نقد",
            )?;
            let variance = checked_sub(net_change, calculated_change, "مغایرت جریان نقد")?;
            let classification_complete = unresolved_vouchers.is_empty() && variance == 0;

            let mut integrity = run_integrity_checks(conn, "", &as_of, true, scope, None)?;
            if !classification_complete {
                let message = if unresolved_vouchers.is_empty() {
                    "تغییر موجودی نقد با جریان اسناد تطبیق ندارد؛ سرفصل حساب‌های نقد و تفکیک مانده‌های خلاف‌ماهیت بررسی شود.".to_string()
                } else {
                    format!(
                        "{} سند دارای گردش نقد نیازمند طبقه‌بندی فعالیت است؛ صورت جریان نقد تا رفع موارد کامل نیست.",
                        db::to_fa_digits(unresolved_vouchers.len() as i64)
                    )
                };
                integrity.issues.push(IntegrityIssue {
                    code: "cash_flow_classification".to_string(),
                    severity: INTEGRITY_ERROR.to_string(),
                    message,
                    count: Some(unresolved_vouchers.len() as i64),
                    amount: Some(variance),
                    accounts: Vec::new(),
                });
                integrity.counts.error += 1;
                integrity.ok = false;
            }

            Ok(CashFlowReport {
                statement: "cash_flow".to_string(),
                to_date: as_of,
                sections,
                opening_cash,
                closing_cash,
                net_change,
                variance,
                unresolved_vouchers,
                classification_complete,
                integrity,
            })
        })
    }
}
