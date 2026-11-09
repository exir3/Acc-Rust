//! Year-end closing and carry-forward.
//!
//! The workflow mirrors the WebApp closing service (`closing-service.ts`) and
//! carry-forward module (`carry-forward.ts`):
//!
//! 1. a temporary closing voucher closes every income-statement account into
//!    the income-summary account and transfers the result to retained earnings;
//! 2. a final closing voucher closes every permanent and memorandum account
//!    into the closing-balance control account, whose own net stays zero;
//! 3. `closing_runs` records the executed run atomically with both vouchers.
//!
//! A carry-forward plan reads the *pre-closing* ledger balance (opening plus
//! normal vouchers plus the temporary closing), never the closing-voucher
//! lines. The closing voucher is used only to verify that account and detail
//! balances cancel on the closed period.
//!
//! Amounts are signed `debit − credit`; a positive amount is a debit.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::db::audit;
use crate::db::classification::{self, AccountClassificationRow, EffectiveClassification};
use crate::db::common::{db_error, to_fa_digits};
use crate::db::fs_catalog::{is_fs_line_code, FsStatement, UNCLASSIFIED};
use crate::db::model::{
    CarryForwardExecutionResult, CarryForwardLine, CarryForwardPlan, ClosingBlocker, ClosingCounts,
    ClosingExecutionResult, ClosingLine, ClosingLineTotals, ClosingPreview, ClosingReversalResult,
    ClosingRoleAccount, ClosingRoles, ClosingRunRecord, ClosingTotals, ClosingVoucherRef, Database,
};
use crate::vouchers::normalize_date;
use crate::vouchers::{CLOSING_FINAL, CLOSING_TEMPORARY, TOUCHED_AT};

const STATUS_EXECUTED: &str = "executed";
const STATUS_REVERSED: &str = "reversed";
const ROLE_INCOME_SUMMARY: &str = "income_summary";
const ROLE_RETAINED_EARNINGS: &str = "retained_earnings";
const ROLE_CLOSING_BALANCE: &str = "closing_balance";
const RETAINED_LINE: &str = "BS.EQ.RE";

/* ── Year and account metadata ───────────────────────────────── */

#[derive(Debug, Clone)]
struct YearRow {
    id: i64,
    title: String,
    start_date: String,
    end_date: String,
    status: String,
}

#[derive(Debug, Clone)]
struct AccountMeta {
    code: String,
    name: String,
    level: i64,
    parent_code: String,
    is_postable: bool,
    is_active: bool,
    system_role: String,
}

fn fetch_year(conn: &Connection, id: i64) -> Result<YearRow, String> {
    conn.query_row(
        "SELECT id, title, start_date, end_date, status FROM fiscal_years WHERE id = ?1",
        params![id],
        |row| {
            Ok(YearRow {
                id: row.get(0)?,
                title: row.get(1)?,
                start_date: row.get(2)?,
                end_date: row.get(3)?,
                status: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(db_error)?
    .ok_or_else(|| "سال مالی یافت نشد.".to_string())
}

fn load_accounts(conn: &Connection) -> Result<HashMap<String, AccountMeta>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT code, name, level, COALESCE(parent_code, ''),
                    is_postable, is_active, COALESCE(system_role, '')
               FROM accounts",
        )
        .map_err(db_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(AccountMeta {
                code: row.get(0)?,
                name: row.get(1)?,
                level: row.get(2)?,
                parent_code: row.get(3)?,
                is_postable: row.get::<_, i64>(4)? != 0,
                is_active: row.get::<_, i64>(5)? != 0,
                system_role: row.get(6)?,
            })
        })
        .map_err(db_error)?;
    let mut map = HashMap::new();
    for row in rows {
        let row = row.map_err(db_error)?;
        map.insert(row.code.clone(), row);
    }
    Ok(map)
}

/* ── Ledger balances ─────────────────────────────────────────── */

/// One account/detail balance. `account_code` is always the base (level 1/2)
/// account; the optional detail is kept separate so carry-forward does not
/// merge independent balances that share an account.
#[derive(Debug, Clone, Default)]
struct DetailPair {
    account_code: String,
    detail1_code: String,
    debit: i64,
    credit: i64,
}

impl DetailPair {
    fn net(&self) -> i64 {
        self.debit - self.credit
    }
}

/// Voucher predicates for the WebApp bases. The turnover bases exclude
/// opening vouchers because opening balances are loaded separately. Every
/// predicate references `?4`, `?5` and `?6` (even the unused ones) so one
/// parameter list can be passed to all four queries.
const BASIS_OPENING: &str = "v.type = ?4 AND ?5 IS NOT NULL AND ?6 IS NOT NULL";
const BASIS_STATEMENT_TURNOVER: &str = "v.type NOT IN (?4, ?5, ?6)";
const BASIS_PRE_TURNOVER: &str = "v.type NOT IN (?4, ?5) AND ?6 IS NOT NULL";
const BASIS_CLOSING: &str = "v.type = ?5 AND ?4 IS NOT NULL AND ?6 IS NOT NULL";

fn load_raw_rows(
    conn: &Connection,
    year: &YearRow,
    as_of: &str,
    predicate: &str,
) -> Result<Vec<DetailPair>, String> {
    let sql = format!(
        "SELECT l.account_code, COALESCE(l.detail1_code, ''),
                COALESCE(SUM(l.debit), 0), COALESCE(SUM(l.credit), 0)
           FROM voucher_lines l
           JOIN vouchers v ON v.id = l.voucher_id
          WHERE v.date BETWEEN ?1 AND ?2
            AND (v.fiscal_year_id = ?3 OR v.fiscal_year_id IS NULL)
            AND {predicate}
          GROUP BY l.account_code, l.detail1_code"
    );
    let mut stmt = conn.prepare(&sql).map_err(db_error)?;
    let rows = stmt
        .query_map(
            params![
                year.start_date,
                as_of,
                year.id,
                VOUCHER_OPENING,
                CLOSING_FINAL,
                CLOSING_TEMPORARY
            ],
            |row| {
                Ok(DetailPair {
                    account_code: row.get(0)?,
                    detail1_code: row.get(1)?,
                    debit: row.get(2)?,
                    credit: row.get(3)?,
                })
            },
        )
        .map_err(db_error)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)
}

const VOUCHER_OPENING: &str = crate::vouchers::VOUCHER_TYPES[1];

fn normalize_rows(
    rows: Vec<DetailPair>,
    accounts: &HashMap<String, AccountMeta>,
) -> Result<Vec<DetailPair>, String> {
    let mut map: BTreeMap<(String, String), DetailPair> = BTreeMap::new();
    for row in rows {
        let mut base = row.account_code;
        let mut detail = row.detail1_code;
        if let Some(meta) = accounts.get(&base) {
            if meta.level >= 3 && !meta.parent_code.is_empty() {
                if detail.is_empty() {
                    detail = base.clone();
                }
                base = meta.parent_code.clone();
            }
        }
        let entry = map
            .entry((base.clone(), detail.clone()))
            .or_insert_with(|| DetailPair {
                account_code: base,
                detail1_code: detail,
                debit: 0,
                credit: 0,
            });
        entry.debit = entry
            .debit
            .checked_add(row.debit)
            .ok_or_else(|| "جمع مانده حساب از محدوده عددی مجاز بیشتر است.".to_string())?;
        entry.credit = entry
            .credit
            .checked_add(row.credit)
            .ok_or_else(|| "جمع مانده حساب از محدوده عددی مجاز بیشتر است.".to_string())?;
    }
    Ok(map.into_values().collect())
}

fn merge_details(sources: &[&[DetailPair]]) -> Result<Vec<DetailPair>, String> {
    let mut map: BTreeMap<(String, String), DetailPair> = BTreeMap::new();
    for rows in sources {
        for row in *rows {
            let entry = map
                .entry((row.account_code.clone(), row.detail1_code.clone()))
                .or_insert_with(|| DetailPair {
                    account_code: row.account_code.clone(),
                    detail1_code: row.detail1_code.clone(),
                    debit: 0,
                    credit: 0,
                });
            entry.debit = entry
                .debit
                .checked_add(row.debit)
                .ok_or_else(|| "جمع مانده حساب از محدوده عددی مجاز بیشتر است.".to_string())?;
            entry.credit = entry
                .credit
                .checked_add(row.credit)
                .ok_or_else(|| "جمع مانده حساب از محدوده عددی مجاز بیشتر است.".to_string())?;
        }
    }
    Ok(map.into_values().collect())
}

fn net_of(rows: &[DetailPair], code: &str) -> Result<i64, String> {
    let mut total = 0i64;
    for row in rows.iter().filter(|row| row.account_code == code) {
        total = total
            .checked_add(row.net())
            .ok_or_else(|| "جمع مانده حساب از محدوده عددی مجاز بیشتر است.".to_string())?;
    }
    Ok(total)
}

fn has_balance(rows: &[DetailPair], code: &str) -> bool {
    rows.iter()
        .any(|row| row.account_code == code && row.net() != 0)
}

/* ── Classification helpers ──────────────────────────────────── */

fn effective_of(
    classifications: &HashMap<String, AccountClassificationRow>,
    code: &str,
) -> Option<EffectiveClassification> {
    classifications
        .get(code)
        .map(|row| classification::resolve_account(row, classifications))
}

fn group_type_matches_fs_line(group_type: &str, fs_line: &str) -> bool {
    classification::group_type_matches_fs_line(group_type, fs_line)
}

fn classification_status(
    row: &AccountClassificationRow,
    effective: &EffectiveClassification,
) -> &'static str {
    if effective.fs_line == UNCLASSIFIED {
        return "unclassified";
    }
    if !group_type_matches_fs_line(&row.group_type, &effective.fs_line) {
        return "inconsistent";
    }
    match effective.source.as_str() {
        "auto" => "suggested",
        "inherit" => "inherited",
        _ => "explicit",
    }
}

/* ── Closing runs ────────────────────────────────────────────── */

const RUN_COLUMNS: &str = "id, fiscal_year_id, status, temp_voucher_id, closing_voucher_id,
    net_income, income_summary_code, retained_earnings_code, closing_balance_code,
    executed_at, reversed_at, reverse_reason";

fn row_to_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClosingRunRecord> {
    Ok(ClosingRunRecord {
        id: row.get(0)?,
        fiscal_year_id: row.get(1)?,
        status: row.get(2)?,
        temp_voucher_id: row.get(3)?,
        closing_voucher_id: row.get(4)?,
        net_income: row.get(5)?,
        income_summary_code: row.get(6)?,
        retained_earnings_code: row.get(7)?,
        closing_balance_code: row.get(8)?,
        executed_at: row.get(9)?,
        reversed_at: row.get(10)?,
        reverse_reason: row.get(11)?,
    })
}

fn executed_run(conn: &Connection, year_id: i64) -> Result<Option<ClosingRunRecord>, String> {
    conn.query_row(
        &format!(
            "SELECT {RUN_COLUMNS} FROM closing_runs
              WHERE fiscal_year_id = ?1 AND status = ?2
              ORDER BY id DESC LIMIT 1"
        ),
        params![year_id, STATUS_EXECUTED],
        row_to_run,
    )
    .optional()
    .map_err(db_error)
}

fn latest_run(conn: &Connection, year_id: i64) -> Result<Option<ClosingRunRecord>, String> {
    conn.query_row(
        &format!(
            "SELECT {RUN_COLUMNS} FROM closing_runs
              WHERE fiscal_year_id = ?1
              ORDER BY id DESC LIMIT 1"
        ),
        params![year_id],
        row_to_run,
    )
    .optional()
    .map_err(db_error)
}

/* ── Roles ───────────────────────────────────────────────────── */

#[derive(Debug, Clone)]
struct RoleInternal {
    code: String,
    name: String,
    effective: EffectiveClassification,
    is_postable: bool,
    is_active: bool,
}

impl RoleInternal {
    fn to_dto(&self) -> ClosingRoleAccount {
        ClosingRoleAccount {
            code: self.code.clone(),
            name: self.name.clone(),
            fs_line: is_fs_line_code(&self.effective.fs_line)
                .then(|| self.effective.fs_line.clone()),
            statement: Some(self.effective.statement.key().to_string()),
            is_postable: self.is_postable,
            is_active: self.is_active,
        }
    }

    fn fs_line(&self) -> &str {
        &self.effective.fs_line
    }
}

#[derive(Debug, Clone, Default)]
struct ResolvedRoles {
    income_summary: Option<RoleInternal>,
    retained_earnings: Option<RoleInternal>,
    closing_balance: Option<RoleInternal>,
}

impl ResolvedRoles {
    fn to_dto(&self) -> ClosingRoles {
        ClosingRoles {
            income_summary: self.income_summary.as_ref().map(RoleInternal::to_dto),
            retained_earnings: self.retained_earnings.as_ref().map(RoleInternal::to_dto),
            closing_balance: self.closing_balance.as_ref().map(RoleInternal::to_dto),
        }
    }
}

fn resolve_roles(
    accounts: &HashMap<String, AccountMeta>,
    classifications: &HashMap<String, AccountClassificationRow>,
) -> ResolvedRoles {
    let find = |role: &str| -> Option<RoleInternal> {
        let meta = accounts
            .values()
            .find(|account| account.system_role == role)?;
        let effective = effective_of(classifications, &meta.code)?;
        Some(RoleInternal {
            code: meta.code.clone(),
            name: meta.name.clone(),
            effective,
            is_postable: meta.is_postable,
            is_active: meta.is_active,
        })
    };
    ResolvedRoles {
        income_summary: find(ROLE_INCOME_SUMMARY),
        retained_earnings: find(ROLE_RETAINED_EARNINGS),
        closing_balance: find(ROLE_CLOSING_BALANCE),
    }
}

/* ── Preview snapshot ────────────────────────────────────────── */

struct Snapshot {
    year: YearRow,
    as_of: String,
    accounts: HashMap<String, AccountMeta>,
    classifications: HashMap<String, AccountClassificationRow>,
    opening_details: Vec<DetailPair>,
    statement_details: Vec<DetailPair>,
    pre_details: Vec<DetailPair>,
    closing_details: Vec<DetailPair>,
    run: Option<ClosingRunRecord>,
    latest_run: Option<ClosingRunRecord>,
    outside_count: i64,
    outside_number: Option<i64>,
    draft_count: i64,
    draft_number: Option<i64>,
    orphan_closing: bool,
}

fn load_snapshot(
    conn: &Connection,
    year_id: i64,
    as_of_date: Option<&str>,
) -> Result<Snapshot, String> {
    let year = fetch_year(conn, year_id)?;
    let as_of = match as_of_date.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => normalize_date(value, "تاریخ بستن")?,
        None => year.end_date.clone(),
    };
    let accounts = load_accounts(conn)?;
    let classifications = classification::load_classifications_as_of(conn, &as_of)?;

    let opening_rows = normalize_rows(
        load_raw_rows(conn, &year, &as_of, BASIS_OPENING)?,
        &accounts,
    )?;
    let statement_turnover = normalize_rows(
        load_raw_rows(conn, &year, &as_of, BASIS_STATEMENT_TURNOVER)?,
        &accounts,
    )?;
    let pre_turnover = normalize_rows(
        load_raw_rows(conn, &year, &as_of, BASIS_PRE_TURNOVER)?,
        &accounts,
    )?;
    let closing_rows = normalize_rows(
        load_raw_rows(conn, &year, &as_of, BASIS_CLOSING)?,
        &accounts,
    )?;

    let statement_details = merge_details(&[&opening_rows, &statement_turnover])?;
    let pre_details = merge_details(&[&opening_rows, &pre_turnover])?;

    let (outside_count, outside_number) = conn
        .query_row(
            "SELECT COUNT(*), MIN(v.number)
               FROM vouchers v
              WHERE (v.fiscal_year_id = ?1
                     OR (v.fiscal_year_id IS NULL AND v.date BETWEEN ?2 AND ?3))
                AND (v.date < ?2 OR v.date > ?4)",
            params![year_id, year.start_date, year.end_date, as_of],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?)),
        )
        .map_err(db_error)?;

    let (draft_count, draft_number) = conn
        .query_row(
            "SELECT COUNT(*), MIN(v.number)
               FROM vouchers v
              WHERE v.is_temporary = 1
                AND (v.fiscal_year_id = ?1
                     OR (v.fiscal_year_id IS NULL AND v.date BETWEEN ?2 AND ?3))",
            params![year_id, year.start_date, year.end_date],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?)),
        )
        .map_err(db_error)?;

    let orphan_closing: bool = conn
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM vouchers v
                 WHERE v.type IN (?4, ?5)
                   AND (v.fiscal_year_id = ?1
                        OR (v.fiscal_year_id IS NULL AND v.date BETWEEN ?2 AND ?3)))",
            params![
                year_id,
                year.start_date,
                year.end_date,
                CLOSING_TEMPORARY,
                CLOSING_FINAL
            ],
            |row| row.get(0),
        )
        .map_err(db_error)?;

    Ok(Snapshot {
        run: executed_run(conn, year_id)?,
        latest_run: latest_run(conn, year_id)?,
        year,
        as_of,
        accounts,
        classifications,
        opening_details: opening_rows,
        statement_details,
        pre_details,
        closing_details: closing_rows,
        outside_count,
        outside_number,
        draft_count,
        draft_number,
        orphan_closing,
    })
}

/* ── Plan ────────────────────────────────────────────────────── */

struct Plan {
    temp_lines: Vec<ClosingLine>,
    closing_lines: Vec<ClosingLine>,
    net_income: i64,
    counts: ClosingCounts,
}

fn make_line(
    account_code: &str,
    account_name: &str,
    fs_line: &str,
    role: &str,
    block: &str,
    detail1_code: &str,
    amount: i64,
) -> ClosingLine {
    ClosingLine {
        account_code: account_code.to_string(),
        detail1_code: detail1_code.to_string(),
        account_name: account_name.to_string(),
        fs_line: fs_line.to_string(),
        role: role.to_string(),
        block: block.to_string(),
        amount,
        debit: if amount > 0 { amount } else { 0 },
        credit: if amount < 0 { -amount } else { 0 },
    }
}

fn line_totals(lines: &[ClosingLine]) -> Result<ClosingLineTotals, String> {
    let mut totals = ClosingLineTotals::default();
    for line in lines {
        totals.debit = totals
            .debit
            .checked_add(line.debit)
            .ok_or_else(|| "جمع بدهکار ردیف‌های بستن از محدوده عددی مجاز بیشتر است.".to_string())?;
        totals.credit = totals.credit.checked_add(line.credit).ok_or_else(|| {
            "جمع بستانکار ردیف‌های بستن از محدوده عددی مجاز بیشتر است.".to_string()
        })?;
    }
    Ok(totals)
}

fn build_plan(snapshot: &Snapshot, roles: &ResolvedRoles) -> Result<Plan, String> {
    let mut temp_lines: Vec<ClosingLine> = Vec::new();
    let mut net_income: i64 = 0;
    let mut temporary_accounts: HashSet<String> = HashSet::new();

    for row in &snapshot.statement_details {
        let Some(account) = snapshot.accounts.get(&row.account_code) else {
            continue;
        };
        let Some(effective) = effective_of(&snapshot.classifications, &row.account_code) else {
            continue;
        };
        if effective.statement != FsStatement::IncomeStatement {
            continue;
        }
        let balance = row.net();
        if balance == 0 {
            continue;
        }
        let amount = balance
            .checked_neg()
            .ok_or_else(|| "مبلغ بستن حساب از محدوده عددی مجاز بیشتر است.".to_string())?;
        net_income = net_income
            .checked_add(amount)
            .ok_or_else(|| "خالص سود و زیان از محدوده عددی مجاز بیشتر است.".to_string())?;
        temporary_accounts.insert(account.code.clone());
        temp_lines.push(make_line(
            &account.code,
            &account.name,
            &effective.fs_line,
            "temporary",
            "close",
            &row.detail1_code,
            amount,
        ));
    }

    if let (Some(summary), Some(retained)) = (
        roles.income_summary.as_ref(),
        roles.retained_earnings.as_ref(),
    ) {
        if net_income != 0 {
            temp_lines.push(make_line(
                &summary.code,
                &summary.name,
                summary.fs_line(),
                "income_summary",
                "close",
                "",
                net_income
                    .checked_neg()
                    .ok_or_else(|| "خالص سود و زیان از محدوده عددی مجاز بیشتر است.".to_string())?,
            ));
            temp_lines.push(make_line(
                &summary.code,
                &summary.name,
                summary.fs_line(),
                "income_summary",
                "transfer",
                "",
                net_income,
            ));
            temp_lines.push(make_line(
                &retained.code,
                &retained.name,
                retained.fs_line(),
                "retained_earnings",
                "transfer",
                "",
                net_income
                    .checked_neg()
                    .ok_or_else(|| "خالص سود و زیان از محدوده عددی مجاز بیشتر است.".to_string())?,
            ));
        }
    }

    let mut permanent_details = snapshot.statement_details.clone();
    if let Some(retained) = roles.retained_earnings.as_ref() {
        if net_income != 0 {
            permanent_details.push(DetailPair {
                account_code: retained.code.clone(),
                detail1_code: String::new(),
                debit: if net_income < 0 { -net_income } else { 0 },
                credit: if net_income > 0 { net_income } else { 0 },
            });
            permanent_details = merge_details(&[&permanent_details])?;
        }
    }

    let mut closing_lines: Vec<ClosingLine> = Vec::new();
    let mut permanent_accounts: HashSet<String> = HashSet::new();
    let control_code = roles
        .closing_balance
        .as_ref()
        .map(|role| role.code.as_str());

    for row in &permanent_details {
        let Some(account) = snapshot.accounts.get(&row.account_code) else {
            continue;
        };
        if Some(account.code.as_str()) == control_code {
            continue;
        }
        let Some(effective) = effective_of(&snapshot.classifications, &row.account_code) else {
            continue;
        };
        if !matches!(
            effective.statement,
            FsStatement::BalanceSheet | FsStatement::Memorandum
        ) {
            continue;
        }
        let balance = row.net();
        if balance == 0 {
            continue;
        }
        permanent_accounts.insert(account.code.clone());
        closing_lines.push(make_line(
            &account.code,
            &account.name,
            &effective.fs_line,
            "permanent",
            "closing",
            &row.detail1_code,
            balance
                .checked_neg()
                .ok_or_else(|| "مبلغ اختتامیه از محدوده عددی مجاز بیشتر است.".to_string())?,
        ));
        if let Some(control) = roles.closing_balance.as_ref() {
            closing_lines.push(make_line(
                &control.code,
                &control.name,
                control.fs_line(),
                "closing_balance",
                "closing",
                "",
                balance,
            ));
        }
    }

    Ok(Plan {
        counts: ClosingCounts {
            temporary_accounts: temporary_accounts.len() as i64,
            permanent_accounts: permanent_accounts.len() as i64,
        },
        temp_lines,
        closing_lines,
        net_income,
    })
}

/* ── Blockers and preview ────────────────────────────────────── */

fn push_blocker(blockers: &mut Vec<ClosingBlocker>, code: &str, severity: &str, message: String) {
    blockers.push(ClosingBlocker {
        code: code.to_string(),
        severity: severity.to_string(),
        message,
        count: None,
        accounts: Vec::new(),
    });
}

fn push_list_blocker(
    blockers: &mut Vec<ClosingBlocker>,
    code: &str,
    message: String,
    accounts: &[String],
) {
    blockers.push(ClosingBlocker {
        code: code.to_string(),
        severity: "error".to_string(),
        message,
        count: Some(accounts.len() as i64),
        accounts: accounts.iter().take(20).cloned().collect(),
    });
}

fn build_blockers(
    snapshot: &Snapshot,
    roles: &ResolvedRoles,
    plan: &Plan,
) -> Result<Vec<ClosingBlocker>, String> {
    let mut blockers: Vec<ClosingBlocker> = Vec::new();

    if snapshot.outside_count > 0 {
        let number = snapshot
            .outside_number
            .map(|value| to_fa_digits(value.to_string()))
            .unwrap_or_else(|| "—".to_string());
        push_blocker(
            &mut blockers,
            "voucher_outside_period",
            "error",
            format!(
                "تاریخ سند شماره {number} خارج از بازه سال مالی «{}» است؛ بستن نباید گردش سال دیگر را شامل شود.",
                snapshot.year.title
            ),
        );
    }

    if snapshot.run.is_some() {
        push_blocker(
            &mut blockers,
            "already_closed",
            "error",
            "این دوره قبلاً بسته شده است؛ برای بستن دوباره ابتدا «برگشت بستن» را اجرا کنید."
                .to_string(),
        );
    } else if snapshot.year.status == "final" {
        push_blocker(
            &mut blockers,
            "year_final",
            "error",
            "این سال مالی نهایی شده است و بستن دوباره یا تغییر آن مجاز نیست.".to_string(),
        );
    }

    if roles.income_summary.is_none() {
        push_blocker(
            &mut blockers,
            "missing_income_summary",
            "error",
            "حساب «خلاصه سود و زیان» (نقش سیستمی income_summary) تعیین نشده است؛ در همین بخش انتخاب کنید."
                .to_string(),
        );
    }
    if roles.retained_earnings.is_none() {
        push_blocker(
            &mut blockers,
            "missing_retained_earnings",
            "error",
            "حساب «سود (زیان) انباشته» (نقش سیستمی retained_earnings) تعیین نشده است؛ در همین بخش انتخاب کنید."
                .to_string(),
        );
    }
    if roles.closing_balance.is_none() {
        push_blocker(
            &mut blockers,
            "missing_closing_balance",
            "error",
            "حساب «تراز اختتامیه» تعیین نشده است؛ یک حساب کنترلی انتظامی فعال و قابل ثبت انتخاب کنید."
                .to_string(),
        );
    }

    if let (Some(summary), Some(retained)) = (
        roles.income_summary.as_ref(),
        roles.retained_earnings.as_ref(),
    ) {
        if summary.code == retained.code {
            push_blocker(
                &mut blockers,
                "role_conflict",
                "error",
                "حساب «خلاصه سود و زیان» و «سود (زیان) انباشته» نباید یکسان باشند.".to_string(),
            );
        }
    }
    if let Some(control) = roles.closing_balance.as_ref() {
        let conflicts = [
            roles.income_summary.as_ref(),
            roles.retained_earnings.as_ref(),
        ]
        .into_iter()
        .flatten()
        .any(|role| role.code == control.code);
        if conflicts {
            push_blocker(
                &mut blockers,
                "role_conflict",
                "error",
                "حساب تراز اختتامیه باید از حساب‌های خلاصه سود و زیان و سود انباشته جدا باشد."
                    .to_string(),
            );
        }
    }

    for (role, label) in [
        (roles.income_summary.as_ref(), "خلاصه سود و زیان"),
        (roles.retained_earnings.as_ref(), "سود (زیان) انباشته"),
    ] {
        let Some(role) = role else { continue };
        if role.fs_line() != RETAINED_LINE {
            push_blocker(
                &mut blockers,
                "role_not_permanent",
                "error",
                format!(
                    "حساب نقش سیستمی «{} — {}» باید در سرفصل سود (زیان) انباشته حقوق مالکانه طبقه‌بندی شود.",
                    role.code, role.name
                ),
            );
        } else if !role.is_active || !role.is_postable {
            push_blocker(
                &mut blockers,
                "role_not_postable",
                "error",
                format!(
                    "حساب «{label}» ({} — {}) غیرفعال یا غیرقابل ثبت سند است.",
                    role.code, role.name
                ),
            );
        }
    }
    if let Some(control) = roles.closing_balance.as_ref() {
        if control.effective.statement != FsStatement::Memorandum {
            push_blocker(
                &mut blockers,
                "role_not_permanent",
                "error",
                "حساب تراز اختتامیه باید حساب کنترلی انتظامی باشد و در دارایی، بدهی، حقوق مالکانه، درآمد یا هزینه گزارش نشود."
                    .to_string(),
            );
        }
        if !control.is_active || !control.is_postable {
            push_blocker(
                &mut blockers,
                "role_not_postable",
                "error",
                "حساب تراز اختتامیه باید فعال و قابل ثبت سند باشد.".to_string(),
            );
        }
    }

    for role in [
        roles.income_summary.as_ref(),
        roles.closing_balance.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        if has_balance(&snapshot.pre_details, &role.code) {
            push_blocker(
                &mut blockers,
                "role_has_balance",
                "error",
                format!(
                    "حساب کنترلی «{} — {}» پیش از بستن مانده دارد؛ ابتدا این مانده را اصلاح یا به سود انباشته منتقل کنید.",
                    role.code, role.name
                ),
            );
        }
    }

    if snapshot.run.is_none() && snapshot.orphan_closing {
        push_blocker(
            &mut blockers,
            "existing_closing_vouchers",
            "error",
            "سند بستن/اختتامیه در این سال وجود دارد ولی رکورد بستن ثبت نشده است؛ وضعیت ناسازگار است و باید ابتدا بررسی/اصلاح شود."
                .to_string(),
        );
    }

    let mut not_postable: Vec<String> = Vec::new();
    let mut invalid_classification: Vec<String> = Vec::new();
    let mut unclassified: Vec<String> = Vec::new();
    let mut temporary_opening: Vec<String> = Vec::new();
    let mut total_debit: i64 = 0;
    let mut total_credit: i64 = 0;
    let mut memorandum_net: i64 = 0;

    let mut codes: Vec<String> = snapshot.accounts.keys().cloned().collect();
    codes.sort();
    for code in &codes {
        let Some(account) = snapshot.accounts.get(code) else {
            continue;
        };
        let effective = effective_of(&snapshot.classifications, code);
        let pre_has_balance = has_balance(&snapshot.pre_details, code);
        if pre_has_balance && (!account.is_active || !account.is_postable) {
            not_postable.push(code.clone());
        }
        let Some(effective) = effective else {
            if has_balance(&snapshot.statement_details, code) {
                unclassified.push(code.clone());
            }
            continue;
        };
        if pre_has_balance {
            if let Some(row) = snapshot.classifications.get(code) {
                let status = classification_status(row, &effective);
                if status == "inconsistent" || status == "suggested" {
                    invalid_classification.push(code.clone());
                }
            }
        }
        if effective.fs_line == UNCLASSIFIED || !is_fs_line_code(&effective.fs_line) {
            if net_of(&snapshot.statement_details, code)? != 0
                || has_balance(&snapshot.statement_details, code)
            {
                unclassified.push(code.clone());
            }
            continue;
        }
        if effective.statement == FsStatement::IncomeStatement
            && (net_of(&snapshot.opening_details, code)? != 0
                || has_balance(&snapshot.opening_details, code))
        {
            temporary_opening.push(code.clone());
        }
        if effective.statement == FsStatement::Memorandum {
            memorandum_net = memorandum_net
                .checked_add(net_of(&snapshot.statement_details, code)?)
                .ok_or_else(|| "جمع حساب‌های انتظامی از محدوده عددی مجاز بیشتر است.".to_string())?;
        }
    }

    for row in &snapshot.statement_details {
        total_debit = total_debit
            .checked_add(row.debit)
            .ok_or_else(|| "جمع بدهکار تراز آزمایشی از محدوده عددی مجاز بیشتر است.".to_string())?;
        total_credit = total_credit.checked_add(row.credit).ok_or_else(|| {
            "جمع بستانکار تراز آزمایشی از محدوده عددی مجاز بیشتر است.".to_string()
        })?;
    }

    if !not_postable.is_empty() {
        push_list_blocker(
            &mut blockers,
            "account_not_postable",
            "حساب دارای مانده غیرفعال یا غیرقابل ثبت وجود دارد؛ پیش از بستن وضعیت حساب‌ها را اصلاح کنید."
                .to_string(),
            &not_postable,
        );
    }
    if !invalid_classification.is_empty() {
        push_list_blocker(
            &mut blockers,
            "invalid_classification",
            "طبقه‌بندی حساب‌های دارای مانده ناسازگار است یا فقط از پیشنهاد گروه آمده؛ سرفصل صورت مالی را پیش از بستن تأیید و اصلاح کنید."
                .to_string(),
            &invalid_classification,
        );
    }
    if memorandum_net != 0 {
        push_blocker(
            &mut blockers,
            "memorandum_with_balance",
            "error",
            "جمع حساب‌های انتظامی و طرف حساب انتظامی متوازن نیست؛ پیش از بستن، ثبت‌های انتظامی را اصلاح کنید."
                .to_string(),
        );
    }
    if !unclassified.is_empty() {
        push_list_blocker(
            &mut blockers,
            "unclassified_with_balance",
            format!(
                "{} حساب دارای مانده سرفصل صورت مالی ندارد؛ پیش از بستن، سرفصلشان را در «بررسی طبقه‌بندی» تعیین کنید.",
                to_fa_digits(unclassified.len() as i64)
            ),
            &unclassified,
        );
    }
    if total_debit != total_credit {
        push_blocker(
            &mut blockers,
            "trial_balance",
            "error",
            format!(
                "تراز آزمایشی برقرار نیست (اختلاف {}); پیش از بستن باید اصلاح شود.",
                to_fa_digits(total_debit.abs_diff(total_credit) as i64)
            ),
        );
    }
    if !temporary_opening.is_empty() {
        push_list_blocker(
            &mut blockers,
            "temporary_opening_balance",
            format!(
                "{} حساب سودوزیانی در سند افتتاحیه مانده دارد (حساب‌های موقت باید در ابتدای دوره صفر باشند).",
                to_fa_digits(temporary_opening.len() as i64)
            ),
            &temporary_opening,
        );
    }

    if snapshot.draft_count > 0 {
        let number = snapshot
            .draft_number
            .map(|value| to_fa_digits(value.to_string()))
            .unwrap_or_else(|| "—".to_string());
        push_blocker(
            &mut blockers,
            "draft_normal_voucher",
            "warning",
            format!(
                "{} سند موقت قابل ویرایش (از جمله شماره {number}) وجود دارد و در مانده‌ها محاسبه شده است؛ با بستن دوره، تغییر همه اسناد تا برگشت بستن قفل می‌شود.",
                to_fa_digits(snapshot.draft_count)
            ),
        );
    }

    let temp_totals = line_totals(&plan.temp_lines)?;
    let closing_totals = line_totals(&plan.closing_lines)?;
    if !plan.temp_lines.is_empty() {
        if plan.temp_lines.len() < 2 {
            push_blocker(
                &mut blockers,
                "insufficient_lines",
                "error",
                "سند بستن موقت کمتر از دو ردیف می‌شود و قابل ثبت نیست؛ وضعیت داده را بررسی کنید."
                    .to_string(),
            );
        }
        if temp_totals.debit != temp_totals.credit {
            push_blocker(
                &mut blockers,
                "unbalanced_lines",
                "error",
                "ردیف‌های سند بستن موقت موازنه نیستند؛ وضعیت داده را بررسی کنید.".to_string(),
            );
        }
    }
    if !plan.closing_lines.is_empty() {
        if plan.closing_lines.len() < 2 {
            push_blocker(
                &mut blockers,
                "insufficient_lines",
                "error",
                "سند اختتامیه کمتر از دو ردیف می‌شود و قابل ثبت نیست؛ وضعیت داده را بررسی کنید."
                    .to_string(),
            );
        }
        if closing_totals.debit != closing_totals.credit {
            push_blocker(
                &mut blockers,
                "unbalanced_lines",
                "error",
                "ردیف‌های سند اختتامیه موازنه نیستند؛ وضعیت داده را بررسی کنید.".to_string(),
            );
        }
    }
    if plan.temp_lines.is_empty() && plan.closing_lines.is_empty() {
        push_blocker(
            &mut blockers,
            "no_activity",
            "error",
            "مانده‌ای برای بستن وجود ندارد (همه حساب‌ها صفرند).".to_string(),
        );
    }

    Ok(blockers)
}

fn preview_from_snapshot(snapshot: Snapshot) -> Result<ClosingPreview, String> {
    let roles = resolve_roles(&snapshot.accounts, &snapshot.classifications);
    let plan = build_plan(&snapshot, &roles)?;
    let blockers = build_blockers(&snapshot, &roles, &plan)?;
    let can_execute = !blockers.iter().any(|blocker| blocker.severity == "error");
    let status = if snapshot.run.is_some() {
        "closed"
    } else {
        "open"
    };
    let totals = ClosingTotals {
        temp: line_totals(&plan.temp_lines)?,
        closing: line_totals(&plan.closing_lines)?,
    };

    Ok(ClosingPreview {
        fiscal_year_id: snapshot.year.id,
        fiscal_year_title: snapshot.year.title.clone(),
        status: status.to_string(),
        run: snapshot.run,
        latest_run: snapshot.latest_run,
        roles: roles.to_dto(),
        blockers,
        can_execute,
        net_income: plan.net_income,
        profit: plan.net_income >= 0,
        temp_lines: plan.temp_lines,
        closing_lines: plan.closing_lines,
        totals,
        counts: plan.counts,
        as_of_date: snapshot.as_of,
    })
}

/* ── Role assignment ─────────────────────────────────────────── */

fn validate_role_account(
    accounts: &HashMap<String, AccountMeta>,
    classifications: &HashMap<String, AccountClassificationRow>,
    code: &str,
    label: &str,
    role: &str,
    control: bool,
) -> Result<(), String> {
    let account = accounts
        .get(code)
        .ok_or_else(|| format!("حساب انتخابی برای «{label}» با کد {code} یافت نشد."))?;
    if !account.system_role.is_empty() && account.system_role != role {
        return Err(format!(
            "حساب «{code} — {}» نقش سیستمی دیگری دارد و نمی‌تواند برای «{label}» انتخاب شود.",
            account.name
        ));
    }
    let effective = effective_of(classifications, code)
        .ok_or_else(|| format!("طبقه‌بندی حساب {code} یافت نشد."))?;
    if control {
        if effective.statement != FsStatement::Memorandum {
            return Err("حساب تراز اختتامیه باید سرفصل انتظامی داشته باشد.".to_string());
        }
    } else if effective.fs_line != RETAINED_LINE {
        return Err(format!(
            "حساب انتخابی برای «{label}» باید در سرفصل سود (زیان) انباشته حقوق مالکانه باشد."
        ));
    }
    if !account.is_active || !account.is_postable {
        return Err(format!(
            "حساب انتخابی برای «{label}» باید فعال و قابل ثبت سند باشد."
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn assign_roles_in_conn(
    conn: &Connection,
    year_id: i64,
    income_summary_code: Option<&str>,
    retained_earnings_code: Option<&str>,
    closing_balance_code: Option<&str>,
) -> Result<ResolvedRoles, String> {
    fetch_year(conn, year_id)?;
    if executed_run(conn, year_id)?.is_some() {
        return Err(
            "نقش‌های بستن در دوره بسته قابل تغییر نیستند؛ ابتدا برگشت بستن را اجرا کنید."
                .to_string(),
        );
    }

    let accounts = load_accounts(conn)?;
    let classifications = classification::load_classifications(conn)?;

    let summary = income_summary_code
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let retained = retained_earnings_code
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let control = closing_balance_code
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if let Some(code) = summary {
        validate_role_account(
            &accounts,
            &classifications,
            code,
            "خلاصه سود و زیان",
            ROLE_INCOME_SUMMARY,
            false,
        )?;
    }
    if let Some(code) = retained {
        validate_role_account(
            &accounts,
            &classifications,
            code,
            "سود (زیان) انباشته",
            ROLE_RETAINED_EARNINGS,
            false,
        )?;
    }
    if let Some(code) = control {
        validate_role_account(
            &accounts,
            &classifications,
            code,
            "تراز اختتامیه",
            ROLE_CLOSING_BALANCE,
            true,
        )?;
    }

    let mut selected = [summary, retained, control]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let before_len = selected.len();
    selected.sort_unstable();
    selected.dedup();
    if selected.len() != before_len {
        return Err("هر نقش بستن باید حساب مستقلی داشته باشد.".to_string());
    }

    // The caller owns the transaction, so the role swap is atomic with the
    // surrounding preview/execution work.
    if let Some(code) = summary {
        conn.execute(
            "UPDATE accounts SET system_role = NULL WHERE system_role = ?1",
            params![ROLE_INCOME_SUMMARY],
        )
        .map_err(db_error)?;
        conn.execute(
            "UPDATE accounts SET system_role = ?1 WHERE code = ?2",
            params![ROLE_INCOME_SUMMARY, code],
        )
        .map_err(db_error)?;
    }
    if let Some(code) = retained {
        conn.execute(
            "UPDATE accounts SET system_role = NULL WHERE system_role = ?1",
            params![ROLE_RETAINED_EARNINGS],
        )
        .map_err(db_error)?;
        conn.execute(
            "UPDATE accounts SET system_role = ?1 WHERE code = ?2",
            params![ROLE_RETAINED_EARNINGS, code],
        )
        .map_err(db_error)?;
    }
    if let Some(code) = control {
        conn.execute(
            "UPDATE accounts SET system_role = NULL WHERE system_role = ?1",
            params![ROLE_CLOSING_BALANCE],
        )
        .map_err(db_error)?;
        conn.execute(
            "UPDATE accounts SET system_role = ?1 WHERE code = ?2",
            params![ROLE_CLOSING_BALANCE, code],
        )
        .map_err(db_error)?;
    }

    let accounts = load_accounts(conn)?;
    let classifications = classification::load_classifications(conn)?;
    Ok(resolve_roles(&accounts, &classifications))
}

/* ── Voucher writing ─────────────────────────────────────────── */

fn next_voucher_number(conn: &Connection) -> Result<i64, String> {
    let max: i64 = conn
        .query_row("SELECT COALESCE(MAX(number), 0) FROM vouchers", [], |row| {
            row.get(0)
        })
        .map_err(db_error)?;
    max.checked_add(1)
        .ok_or_else(|| "شماره سند بعدی از محدوده مجاز خارج است.".to_string())
}

fn block_title(block: &str) -> &'static str {
    match block {
        "transfer" => "انتقال سود (زیان) دوره",
        "closing" => "بستن حساب دائمی",
        _ => "بستن حساب موقت",
    }
}

fn insert_closing_voucher(
    conn: &Connection,
    year: &YearRow,
    date: &str,
    voucher_type: &str,
    description: &str,
    lines: &[ClosingLine],
) -> Result<(ClosingVoucherRef, i64), String> {
    if lines.is_empty() {
        return Err("سند بستن بدون ردیف قابل ثبت نیست.".to_string());
    }
    let number = next_voucher_number(conn)?;
    conn.execute(
        "INSERT INTO vouchers
            (number, date, type, description, fiscal_year_id, is_temporary)
         VALUES (?1, ?2, ?3, ?4, ?5, 1)",
        params![number, date, voucher_type, description, year.id],
    )
    .map_err(db_error)?;
    let voucher_id = conn.last_insert_rowid();

    for (index, line) in lines.iter().enumerate() {
        let account_id: i64 = conn
            .query_row(
                "SELECT id FROM accounts WHERE code = ?1",
                params![line.account_code],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        let detail = if line.detail1_code.is_empty() {
            None
        } else {
            Some(line.detail1_code.as_str())
        };
        conn.execute(
            "INSERT INTO voucher_lines
                (voucher_id, row_no, account_id, account_code, detail1_code,
                 description, debit, credit)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                voucher_id,
                index as i64 + 1,
                account_id,
                line.account_code,
                detail,
                block_title(&line.block),
                line.debit,
                line.credit
            ],
        )
        .map_err(db_error)?;
    }

    conn.execute(
        &format!("UPDATE vouchers SET is_temporary = 0, updated_at = {TOUCHED_AT} WHERE id = ?1"),
        params![voucher_id],
    )
    .map_err(db_error)?;

    let line_count = lines.len() as i64;
    audit::log(
        conn,
        "create",
        "voucher",
        Some(voucher_id),
        None,
        None,
        &format!(
            "سند بستن خودکار شماره {} با {} ردیف ثبت شد.",
            to_fa_digits(number.to_string()),
            to_fa_digits(line_count)
        ),
    )?;
    Ok((
        ClosingVoucherRef {
            id: voucher_id,
            number,
        },
        line_count,
    ))
}

/* ── Public API: preview, assignment, execute, reverse ───────── */

impl Database {
    pub fn get_closing_preview(
        &self,
        year_id: i64,
        as_of_date: Option<&str>,
    ) -> Result<ClosingPreview, String> {
        if year_id <= 0 {
            return Err("شناسه سال مالی نامعتبر است.".to_string());
        }
        let as_of = as_of_date.map(str::to_string);
        self.with_conn(move |conn| {
            let snapshot = load_snapshot(conn, year_id, as_of.as_deref())?;
            preview_from_snapshot(snapshot)
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn assign_closing_roles(
        &self,
        year_id: i64,
        income_summary_code: Option<&str>,
        retained_earnings_code: Option<&str>,
        closing_balance_code: Option<&str>,
    ) -> Result<ClosingRoles, String> {
        if year_id <= 0 {
            return Err("شناسه سال مالی نامعتبر است.".to_string());
        }
        let summary = income_summary_code.map(str::to_string);
        let retained = retained_earnings_code.map(str::to_string);
        let control = closing_balance_code.map(str::to_string);
        self.with_conn(move |conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            let roles = assign_roles_in_conn(
                &tx,
                year_id,
                summary.as_deref(),
                retained.as_deref(),
                control.as_deref(),
            )?;
            tx.commit().map_err(db_error)?;
            Ok(roles.to_dto())
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn execute_closing(
        &self,
        year_id: i64,
        as_of_date: Option<&str>,
        income_summary_code: Option<&str>,
        retained_earnings_code: Option<&str>,
        closing_balance_code: Option<&str>,
        reason: Option<&str>,
    ) -> Result<ClosingExecutionResult, String> {
        if year_id <= 0 {
            return Err("شناسه سال مالی نامعتبر است.".to_string());
        }
        let as_of = as_of_date.map(str::to_string);
        let summary = income_summary_code.map(str::to_string);
        let retained = retained_earnings_code.map(str::to_string);
        let control = closing_balance_code.map(str::to_string);
        let reason = reason.unwrap_or("").trim().to_string();
        if reason.chars().count() > 500 {
            return Err("دلیل بستن نمی‌تواند بیشتر از ۵۰۰ کاراکتر باشد.".to_string());
        }

        self.with_conn(move |conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            let overrides = summary.is_some() || retained.is_some() || control.is_some();
            if overrides {
                assign_roles_in_conn(
                    &tx,
                    year_id,
                    summary.as_deref(),
                    retained.as_deref(),
                    control.as_deref(),
                )?;
            }

            let snapshot = load_snapshot(&tx, year_id, as_of.as_deref())?;
            let preview = preview_from_snapshot(snapshot)?;
            let errors: Vec<String> = preview
                .blockers
                .iter()
                .filter(|blocker| blocker.severity == "error")
                .map(|blocker| blocker.message.clone())
                .collect();
            if !errors.is_empty() {
                return Err(errors.join(" "));
            }
            let year = fetch_year(&tx, year_id)?;
            if year.status == "final" {
                return Err("سال مالی نهایی شده است؛ اجرای بستن روی آن مجاز نیست.".to_string());
            }
            let summary = preview
                .roles
                .income_summary
                .as_ref()
                .ok_or_else(|| "حساب خلاصه سود و زیان تعیین نشده است.".to_string())?;
            let retained = preview
                .roles
                .retained_earnings
                .as_ref()
                .ok_or_else(|| "حساب سود (زیان) انباشته تعیین نشده است.".to_string())?;
            let control = preview
                .roles
                .closing_balance
                .as_ref()
                .ok_or_else(|| "حساب تراز اختتامیه تعیین نشده است.".to_string())?;

            let temp = if preview.temp_lines.is_empty() {
                None
            } else {
                Some(insert_closing_voucher(
                    &tx,
                    &year,
                    &preview.as_of_date,
                    CLOSING_TEMPORARY,
                    "بستن حساب‌های موقت و انتقال سود (زیان) دوره",
                    &preview.temp_lines,
                )?)
            };
            let closing = if preview.closing_lines.is_empty() {
                None
            } else {
                Some(insert_closing_voucher(
                    &tx,
                    &year,
                    &preview.as_of_date,
                    CLOSING_FINAL,
                    "سند اختتامیه — بستن حساب‌های دائمی",
                    &preview.closing_lines,
                )?)
            };

            tx.execute(
                "INSERT INTO closing_runs
                    (fiscal_year_id, status, temp_voucher_id, closing_voucher_id,
                     net_income, income_summary_code, retained_earnings_code,
                     closing_balance_code, executed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now', 'localtime'))",
                params![
                    year_id,
                    STATUS_EXECUTED,
                    temp.as_ref().map(|value| value.0.id),
                    closing.as_ref().map(|value| value.0.id),
                    preview.net_income,
                    summary.code,
                    retained.code,
                    control.code,
                ],
            )
            .map_err(db_error)?;
            let run_id = tx.last_insert_rowid();

            tx.execute(
                "UPDATE fiscal_years
                    SET status = 'closed', updated_at = datetime('now', 'localtime')
                  WHERE id = ?1",
                params![year_id],
            )
            .map_err(db_error)?;

            let mut details = format!(
                "بستن سال مالی با خالص سود {} و {} ردیف سند موقت و {} ردیف سند اختتامیه.",
                to_fa_digits(preview.net_income.to_string()),
                to_fa_digits(preview.temp_lines.len() as i64),
                to_fa_digits(preview.closing_lines.len() as i64)
            );
            if !reason.is_empty() {
                details.push_str(&format!(" دلیل: {reason}"));
            }
            audit::log(
                &tx,
                "execute",
                "closing_run",
                Some(run_id),
                None,
                None,
                &details,
            )?;

            let result = ClosingExecutionResult {
                run_id,
                net_income: preview.net_income,
                temp_voucher: temp.as_ref().map(|value| value.0.clone()),
                closing_voucher: closing.as_ref().map(|value| value.0.clone()),
                temp_line_count: temp.as_ref().map(|value| value.1).unwrap_or(0),
                closing_line_count: closing.as_ref().map(|value| value.1).unwrap_or(0),
            };
            tx.commit().map_err(db_error)?;
            Ok(result)
        })
    }

    pub fn reverse_closing(
        &self,
        year_id: i64,
        reason: Option<&str>,
        force: bool,
    ) -> Result<ClosingReversalResult, String> {
        if year_id <= 0 {
            return Err("شناسه سال مالی نامعتبر است.".to_string());
        }
        let reason = reason.unwrap_or("").trim().to_string();
        if reason.chars().count() > 500 {
            return Err("دلیل برگشت بستن نمی‌تواند بیشتر از ۵۰۰ کاراکتر باشد.".to_string());
        }

        self.with_conn(move |conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            let run = executed_run(&tx, year_id)?.ok_or_else(|| {
                "بستن فعالی برای برگشت وجود ندارد (این دوره باز است).".to_string()
            })?;
            let year = fetch_year(&tx, year_id)?;
            if year.status == "final" {
                return Err(
                    "سال مالی نهایی شده است؛ برای برگشت بستن ابتدا با ثبت دلیل آن را بازگشایی کنید."
                        .to_string(),
                );
            }
            // Reversing the closing reopens the year for edits. If the next
            // year already received an opening voucher, that voucher would
            // silently keep balances from the now-obsolete closing. Require an
            // explicit confirmation in that case.
            if let Some(next) = next_year(&tx, &year)? {
                if let Some(opening) = target_opening_voucher(&tx, &next)? {
                    if !force {
                        return Err(format!(
                            "برای سال مالی بعد («{}») سند افتتاحیه شماره {} صادر شده است؛ برگشت بستن، آن افتتاحیه را کهنه می‌کند. ابتدا سند افتتاحیه را حذف کنید یا برگشت را با تأیید صریح انجام دهید.",
                            next.title,
                            to_fa_digits(opening.number.to_string())
                        ));
                    }
                }
            }
            let before = serde_json::to_string(&run).map_err(|error| error.to_string())?;
            let ids: Vec<i64> = [run.temp_voucher_id, run.closing_voucher_id]
                .into_iter()
                .flatten()
                .collect();

            tx.execute_batch(
                "CREATE TABLE IF NOT EXISTS _closing_reversal_allowed (
                     voucher_id INTEGER PRIMARY KEY
                 );",
            )
            .map_err(db_error)?;
            for id in &ids {
                tx.execute(
                    "INSERT OR IGNORE INTO _closing_reversal_allowed (voucher_id) VALUES (?1)",
                    params![id],
                )
                .map_err(db_error)?;
            }
            for id in &ids {
                tx.execute(
                    "DELETE FROM voucher_lines WHERE voucher_id = ?1",
                    params![id],
                )
                .map_err(db_error)?;
                tx.execute("DELETE FROM vouchers WHERE id = ?1", params![id])
                    .map_err(db_error)?;
            }
            tx.execute("DELETE FROM _closing_reversal_allowed", [])
                .map_err(db_error)?;

            tx.execute(
                "UPDATE closing_runs
                    SET status = ?1, reversed_at = datetime('now', 'localtime'), reverse_reason = ?2
                  WHERE id = ?3",
                params![STATUS_REVERSED, reason, run.id],
            )
            .map_err(db_error)?;
            tx.execute(
                "UPDATE fiscal_years
                    SET status = 'open', updated_at = datetime('now', 'localtime')
                  WHERE id = ?1",
                params![year_id],
            )
            .map_err(db_error)?;

            let details = if reason.is_empty() {
                format!("برگشت بستن و حذف {} سند.", to_fa_digits(ids.len() as i64))
            } else {
                format!(
                    "برگشت بستن و حذف {} سند. دلیل: {reason}",
                    to_fa_digits(ids.len() as i64)
                )
            };
            audit::log(
                &tx,
                "reverse",
                "closing_run",
                Some(run.id),
                Some(&before),
                None,
                &details,
            )?;
            tx.commit().map_err(db_error)?;
            Ok(ClosingReversalResult {
                run_id: run.id,
                deleted_vouchers: ids.len() as i64,
            })
        })
    }
}

/* ── Carry-forward ───────────────────────────────────────────── */

fn target_opening_voucher(
    conn: &Connection,
    year: &YearRow,
) -> Result<Option<ClosingVoucherRef>, String> {
    conn.query_row(
        "SELECT id, number FROM vouchers
          WHERE type = ?1
            AND (fiscal_year_id = ?2
                 OR (fiscal_year_id IS NULL AND date BETWEEN ?3 AND ?4))
          ORDER BY number LIMIT 1",
        params![VOUCHER_OPENING, year.id, year.start_date, year.end_date],
        |row| {
            Ok(ClosingVoucherRef {
                id: row.get(0)?,
                number: row.get(1)?,
            })
        },
    )
    .optional()
    .map_err(db_error)
}

fn next_year(conn: &Connection, year: &YearRow) -> Result<Option<YearRow>, String> {
    conn.query_row(
        "SELECT id, title, start_date, end_date, status
           FROM fiscal_years
          WHERE id <> ?1 AND start_date > ?2
          ORDER BY start_date, id LIMIT 1",
        params![year.id, year.start_date],
        |row| {
            Ok(YearRow {
                id: row.get(0)?,
                title: row.get(1)?,
                start_date: row.get(2)?,
                end_date: row.get(3)?,
                status: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(db_error)
}

fn build_carry_forward_plan_conn(
    conn: &Connection,
    source_id: i64,
    target_id: i64,
) -> Result<CarryForwardPlan, String> {
    if source_id <= 0 || target_id <= 0 {
        return Err("شناسه سال مالی مبدأ یا مقصد نامعتبر است.".to_string());
    }
    if source_id == target_id {
        return Err("سال مالی مبدأ و مقصد نباید یکسان باشند.".to_string());
    }
    let source = fetch_year(conn, source_id)?;
    let target = fetch_year(conn, target_id)?;
    if target.start_date <= source.end_date {
        return Err(
            "انتقال مانده فقط به سال مالی بعدی و پس از پایان سال مبدأ مجاز است.".to_string(),
        );
    }
    if let Some(next) = next_year(conn, &source)? {
        if next.id != target.id {
            return Err(format!(
                "انتقال مانده فقط به نزدیک‌ترین سال مالی بعدی («{}») مجاز است.",
                next.title
            ));
        }
    } else {
        return Err("سال مالی بعدی برای انتقال مانده وجود ندارد.".to_string());
    }

    let snapshot = load_snapshot(conn, source_id, None)?;
    let target_accounts = load_accounts(conn)?;
    let target_classifications =
        classification::load_classifications_as_of(conn, &target.start_date)?;

    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    if snapshot.run.is_none() {
        errors.push(
            "انتقال مانده فقط پس از اجرای موفق بستن سال مالی مبدأ مجاز است؛ ابتدا از بخش بستن، حساب‌های موقت و دائمی را ببندید."
                .to_string(),
        );
    }
    if target.status != "open" {
        errors.push(
            "سال مالی مقصد باز نیست؛ تغییر سند افتتاحیه آن تا بازگشایی مجاز نیست.".to_string(),
        );
    }
    if executed_run(conn, target_id)?.is_some() {
        errors.push("سال مالی مقصد بسته شده است و افتتاحیه آن قابل تغییر نیست.".to_string());
    }

    let has_closing_voucher = !snapshot.closing_details.is_empty();
    if !has_closing_voucher {
        warnings.push(
            "سند اختتامیه سال مالی مبدأ ثبت نشده است؛ این پیش‌نویس قابل ثبت نیست و ابتدا باید بستن سال مالی اجرا شود."
                .to_string(),
        );
    }

    if let Some(run) = snapshot.run.as_ref() {
        for (id, expected) in [
            (run.temp_voucher_id, CLOSING_TEMPORARY),
            (run.closing_voucher_id, CLOSING_FINAL),
        ] {
            if let Some(id) = id {
                let matches: bool = conn
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM vouchers WHERE id = ?1 AND type = ?2)",
                        params![id, expected],
                        |row| row.get(0),
                    )
                    .map_err(db_error)?;
                if !matches {
                    errors.push(
                        "وضعیت ناسازگار: سند ثبت‌شده در سابقه بستن یافت نشد یا نوع آن تغییر کرده است."
                            .to_string(),
                    );
                }
            }
        }
    }

    let mut closing_net: HashMap<String, i64> = HashMap::new();
    let mut closing_detail_net: HashMap<(String, String), i64> = HashMap::new();
    for row in &snapshot.closing_details {
        let net = row.net();
        *closing_net.entry(row.account_code.clone()).or_insert(0) += net;
        *closing_detail_net
            .entry((row.account_code.clone(), row.detail1_code.clone()))
            .or_insert(0) += net;
    }

    let mut lines: Vec<CarryForwardLine> = Vec::new();
    let mut unclassified: HashSet<String> = HashSet::new();
    let mut temporary: HashSet<String> = HashSet::new();
    let mut pre_net_by_account: HashMap<String, i64> = HashMap::new();
    let mut memorandum_net: i64 = 0;
    let mut income_residual: i64 = 0;

    for row in &snapshot.pre_details {
        let net = row.net();
        *pre_net_by_account
            .entry(row.account_code.clone())
            .or_insert(0) += net;
        if net == 0 {
            continue;
        }
        let Some(account) = snapshot.accounts.get(&row.account_code) else {
            continue;
        };
        if account.system_role == ROLE_CLOSING_BALANCE {
            errors.push(
                "حساب تراز اختتامیه در مانده پیش از بستن مانده دارد؛ انتقال حساب کنترلی مجاز نیست."
                    .to_string(),
            );
            continue;
        }
        if account.system_role == ROLE_INCOME_SUMMARY {
            errors.push(
                "حساب خلاصه سود و زیان هنوز مانده دارد؛ ابتدا آن را به سود (زیان) انباشته منتقل کنید."
                    .to_string(),
            );
            continue;
        }
        let effective = effective_of(&snapshot.classifications, &row.account_code);
        let Some(effective) = effective else {
            unclassified.insert(row.account_code.clone());
            continue;
        };
        if effective.statement == FsStatement::IncomeStatement {
            income_residual = income_residual
                .checked_add(net)
                .ok_or_else(|| "جمع مانده سود و زیان از محدوده عددی مجاز بیشتر است.".to_string())?;
            temporary.insert(row.account_code.clone());
            continue;
        }
        if effective.statement == FsStatement::Memorandum {
            memorandum_net = memorandum_net
                .checked_add(net)
                .ok_or_else(|| "جمع حساب‌های انتظامی از محدوده عددی مجاز بیشتر است.".to_string())?;
        }
        if effective.fs_line == UNCLASSIFIED || !is_fs_line_code(&effective.fs_line) {
            unclassified.insert(row.account_code.clone());
        }
        lines.push(CarryForwardLine {
            account_code: row.account_code.clone(),
            account_name: account.name.clone(),
            detail1_code: row.detail1_code.clone(),
            debit: if net > 0 { net } else { 0 },
            credit: if net < 0 { -net } else { 0 },
        });
    }

    if !temporary.is_empty() {
        errors.push(format!(
            "{} حساب موقت هنوز مانده دارد؛ انتقال مانده سودوزیانی یا بستن ضمنی آن در افتتاحیه مجاز نیست (خالص {}).",
            to_fa_digits(temporary.len() as i64),
            to_fa_digits(income_residual)
        ));
    }
    if memorandum_net != 0 {
        errors.push(
            "جمع مانده حساب‌های انتظامی و طرف حساب انتظامی متوازن نیست؛ ابتدا ثبت‌های انتظامی را اصلاح کنید."
                .to_string(),
        );
    }
    if !unclassified.is_empty() {
        errors.push(format!(
            "حساب‌های بدون سرفصل صورت مالی با مانده وجود دارند ({} حساب)؛ نوع دائمی یا موقت آن‌ها باید پیش از انتقال مشخص شود.",
            to_fa_digits(unclassified.len() as i64)
        ));
    }

    if has_closing_voucher {
        let mut mismatched: Vec<String> = Vec::new();
        let mut codes: Vec<String> = snapshot.accounts.keys().cloned().collect();
        codes.sort();
        for code in codes {
            let Some(account) = snapshot.accounts.get(&code) else {
                continue;
            };
            let Some(effective) = effective_of(&snapshot.classifications, &code) else {
                continue;
            };
            if !matches!(
                effective.statement,
                FsStatement::BalanceSheet | FsStatement::Memorandum
            ) || account.system_role == ROLE_CLOSING_BALANCE
            {
                continue;
            }
            let pre = pre_net_by_account.get(&code).copied().unwrap_or(0);
            let close = closing_net.get(&code).copied().unwrap_or(0);
            if pre
                .checked_add(close)
                .ok_or_else(|| "جمع مانده بستن از محدوده عددی مجاز بیشتر است.".to_string())?
                != 0
            {
                mismatched.push(code);
            }
        }
        if !mismatched.is_empty() {
            errors.push(format!(
                "سند اختتامیه با مانده دفتر هم‌خوان نیست ({} حساب)؛ مانده منتقل‌شده از دفتر محاسبه شده است.",
                to_fa_digits(mismatched.len() as i64)
            ));
        }

        let mut detail_keys: HashSet<(String, String)> = HashSet::new();
        for row in &snapshot.pre_details {
            detail_keys.insert((row.account_code.clone(), row.detail1_code.clone()));
        }
        for key in closing_detail_net.keys() {
            detail_keys.insert(key.clone());
        }
        let mut detail_mismatch = false;
        for (code, detail) in detail_keys {
            let Some(effective) = effective_of(&snapshot.classifications, &code) else {
                continue;
            };
            let account = snapshot.accounts.get(&code);
            if account.is_some_and(|account| account.system_role == ROLE_CLOSING_BALANCE)
                || !matches!(
                    effective.statement,
                    FsStatement::BalanceSheet | FsStatement::Memorandum
                )
            {
                continue;
            }
            let pre = snapshot
                .pre_details
                .iter()
                .find(|row| row.account_code == code && row.detail1_code == detail)
                .map(DetailPair::net)
                .unwrap_or(0);
            let close = closing_detail_net
                .get(&(code, detail))
                .copied()
                .unwrap_or(0);
            if pre
                .checked_add(close)
                .ok_or_else(|| "جمع مانده بستن از محدوده عددی مجاز بیشتر است.".to_string())?
                != 0
            {
                detail_mismatch = true;
                break;
            }
        }
        if detail_mismatch {
            errors.push(
                "سند اختتامیه با مانده حساب و تفصیلی دفتر هم‌خوان نیست؛ پیش از انتقال، بستن را برگشت و دوباره اجرا کنید."
                    .to_string(),
            );
        }
    }

    if snapshot.run.is_some()
        && !has_closing_voucher
        && snapshot
            .run
            .as_ref()
            .and_then(|run| run.closing_voucher_id)
            .is_some()
    {
        errors.push(
            "وضعیت ناسازگار: ثبت بستن در سال مبدأ انجام شده ولی سند اختتامیه وجود ندارد؛ ابتدا وضعیت را اصلاح کنید."
                .to_string(),
        );
    }

    // Target chart checks: every transferred line must land on an active,
    // postable account with the same permanent classification and detail.
    let mut missing_codes: Vec<String> = Vec::new();
    for line in &lines {
        let Some(target_account) = target_accounts.get(&line.account_code) else {
            missing_codes.push(line.account_code.clone());
            continue;
        };
        let Some(target_effective) = effective_of(&target_classifications, &line.account_code)
        else {
            errors.push(format!(
                "طبقه‌بندی حساب {} در سال مالی مقصد یافت نشد.",
                line.account_code
            ));
            continue;
        };
        let source_effective = effective_of(&snapshot.classifications, &line.account_code);
        if !target_account.is_active
            || !target_account.is_postable
            || !matches!(
                target_effective.statement,
                FsStatement::BalanceSheet | FsStatement::Memorandum
            )
            || source_effective
                .as_ref()
                .is_some_and(|source| source.fs_line != target_effective.fs_line)
            || target_account.system_role == ROLE_INCOME_SUMMARY
            || target_account.system_role == ROLE_CLOSING_BALANCE
        {
            errors.push(format!(
                "حساب {} در مقصد باید فعال، قابل ثبت و دارای همان طبقه‌بندی دائمی مبدأ باشد؛ ابتدا کدینگ مقصد را اصلاح کنید.",
                line.account_code
            ));
            continue;
        }
        if !line.detail1_code.is_empty() {
            let Some(detail_account) = target_accounts.get(&line.detail1_code) else {
                errors.push(format!(
                    "تفصیلی {} در سال مالی مقصد یافت نشد؛ ابتدا کدینگ مقصد را کامل کنید.",
                    line.detail1_code
                ));
                continue;
            };
            if detail_account.parent_code != line.account_code {
                errors.push(format!(
                    "تفصیلی {} در مقصد به حساب {} متصل نیست؛ ابتدا کدینگ مقصد را اصلاح کنید.",
                    line.detail1_code, line.account_code
                ));
                continue;
            }
            let source_detail = effective_of(&snapshot.classifications, &line.detail1_code);
            let target_detail = effective_of(&target_classifications, &line.detail1_code);
            if source_detail
                .as_ref()
                .zip(target_detail.as_ref())
                .is_some_and(|(source, target)| source.fs_line != target.fs_line)
            {
                errors.push(format!(
                    "طبقه‌بندی تفصیلی {} در مقصد با مبدأ یکسان نیست؛ ابتدا کدینگ مقصد را اصلاح کنید.",
                    line.detail1_code
                ));
            }
        }
    }
    if !missing_codes.is_empty() {
        missing_codes.sort();
        missing_codes.dedup();
        errors.push(format!(
            "{} حساب از مانده سال مالی مبدأ در سال مالی مقصد یافت نشد (کدهای نمونه: {}). ابتدا این حساب‌ها را در مقصد تعریف کنید.",
            to_fa_digits(missing_codes.len() as i64),
            missing_codes
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join("، ")
        ));
    }

    let mut totals = ClosingLineTotals::default();
    for line in &lines {
        totals.debit = totals
            .debit
            .checked_add(line.debit)
            .ok_or_else(|| "جمع بدهکار سند افتتاحیه از محدوده عددی مجاز بیشتر است.".to_string())?;
        totals.credit = totals.credit.checked_add(line.credit).ok_or_else(|| {
            "جمع بستانکار سند افتتاحیه از محدوده عددی مجاز بیشتر است.".to_string()
        })?;
    }
    if lines.is_empty() {
        errors.push(
            "مانده‌ای برای انتقال وجود ندارد؛ همه حساب‌های دائمی سال مالی مبدأ صفرند.".to_string(),
        );
    } else if totals.debit != totals.credit {
        errors.push(format!(
            "ردیف‌های سند افتتاحیه موازنه نیستند (اختلاف {}).",
            to_fa_digits(totals.debit.abs_diff(totals.credit) as i64)
        ));
    }

    let existing_opening = target_opening_voucher(conn, &target)?;
    if existing_opening.is_some() {
        warnings.push(
            "سال مالی مقصد از قبل سند افتتاحیه دارد؛ ثبت مجدد فقط با تأیید بازنویسی انجام می‌شود."
                .to_string(),
        );
    }

    let mut account_codes: Vec<String> =
        lines.iter().map(|line| line.account_code.clone()).collect();
    account_codes.sort();
    account_codes.dedup();

    Ok(CarryForwardPlan {
        source_fiscal_year_id: source.id,
        source_fiscal_year_title: source.title,
        target_fiscal_year_id: target.id,
        target_fiscal_year_title: target.title,
        source: if has_closing_voucher {
            "closing".to_string()
        } else {
            "draft".to_string()
        },
        lines,
        totals,
        account_count: account_codes.len() as i64,
        has_closing_voucher,
        existing_opening_voucher: existing_opening.clone(),
        requires_overwrite: existing_opening.is_some(),
        warnings,
        errors,
    })
}

/* ── Public API: carry-forward ───────────────────────────────── */

impl Database {
    pub fn get_carry_forward_preview(
        &self,
        source_year_id: i64,
        target_year_id: i64,
    ) -> Result<CarryForwardPlan, String> {
        self.with_conn(move |conn| {
            build_carry_forward_plan_conn(conn, source_year_id, target_year_id)
        })
    }

    pub fn execute_carry_forward(
        &self,
        source_year_id: i64,
        target_year_id: i64,
        force: bool,
    ) -> Result<CarryForwardExecutionResult, String> {
        self.with_conn(move |conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            let plan = build_carry_forward_plan_conn(&tx, source_year_id, target_year_id)?;
            if !plan.errors.is_empty() {
                return Err(plan.errors.join(" "));
            }
            let target = fetch_year(&tx, target_year_id)?;
            if target.status != "open" {
                return Err("سال مالی مقصد باز نیست و افتتاحیه آن قابل تغییر نیست.".to_string());
            }
            if executed_run(&tx, target_year_id)?.is_some() {
                return Err("سال مالی مقصد بسته شده است و افتتاحیه آن قابل تغییر نیست.".to_string());
            }

            let existing = target_opening_voucher(&tx, &target)?;
            if existing.is_some() && !force {
                return Err(format!(
                    "سال مالی مقصد از قبل سند افتتاحیه دارد (شماره {}). برای بازنویسی آن، تأیید صریح لازم است.",
                    to_fa_digits(
                        existing
                            .as_ref()
                            .map(|voucher| voucher.number)
                            .unwrap_or(0)
                            .to_string()
                    )
                ));
            }
            if let Some(existing) = existing.as_ref() {
                tx.execute_batch(
                    "CREATE TABLE IF NOT EXISTS _closing_reversal_allowed (
                         voucher_id INTEGER PRIMARY KEY
                     );",
                )
                .map_err(db_error)?;
                tx.execute(
                    "INSERT OR IGNORE INTO _closing_reversal_allowed (voucher_id) VALUES (?1)",
                    params![existing.id],
                )
                .map_err(db_error)?;
                tx.execute(
                    "DELETE FROM voucher_lines WHERE voucher_id = ?1",
                    params![existing.id],
                )
                .map_err(db_error)?;
                tx.execute("DELETE FROM vouchers WHERE id = ?1", params![existing.id])
                    .map_err(db_error)?;
                tx.execute("DELETE FROM _closing_reversal_allowed", [])
                    .map_err(db_error)?;
            }

            let number = next_voucher_number(&tx)?;
            tx.execute(
                "INSERT INTO vouchers
                    (number, date, type, description, fiscal_year_id, is_temporary)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1)",
                params![
                    number,
                    target.start_date,
                    VOUCHER_OPENING,
                    format!(
                        "سند افتتاحیه خودکار — انتقال مانده از سال مالی {}",
                        plan.source_fiscal_year_title
                    ),
                    target_year_id
                ],
            )
            .map_err(db_error)?;
            let voucher_id = tx.last_insert_rowid();

            for (index, line) in plan.lines.iter().enumerate() {
                let account_id: i64 = tx
                    .query_row(
                        "SELECT id FROM accounts WHERE code = ?1",
                        params![line.account_code],
                        |row| row.get(0),
                    )
                    .map_err(db_error)?;
                let detail = if line.detail1_code.is_empty() {
                    None
                } else {
                    Some(line.detail1_code.as_str())
                };
                tx.execute(
                    "INSERT INTO voucher_lines
                        (voucher_id, row_no, account_id, account_code, detail1_code,
                         description, debit, credit)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        voucher_id,
                        index as i64 + 1,
                        account_id,
                        line.account_code,
                        detail,
                        "مانده انتقالی از سال قبل",
                        line.debit,
                        line.credit
                    ],
                )
                .map_err(db_error)?;
            }

            tx.execute(
                &format!(
                    "UPDATE vouchers SET is_temporary = 0, updated_at = {TOUCHED_AT} WHERE id = ?1"
                ),
                params![voucher_id],
            )
            .map_err(db_error)?;

            audit::log(
                &tx,
                "create",
                "voucher",
                Some(voucher_id),
                None,
                None,
                &format!(
                    "انتقال مانده از سال مالی «{}» به «{}» با {} ردیف.",
                    plan.source_fiscal_year_title,
                    plan.target_fiscal_year_title,
                    to_fa_digits(plan.lines.len() as i64)
                ),
            )?;

            let result = CarryForwardExecutionResult {
                voucher_id,
                voucher_number: number,
                line_count: plan.lines.len() as i64,
                account_count: plan.account_count,
            };
            tx.commit().map_err(db_error)?;
            Ok(result)
        })
    }
}
