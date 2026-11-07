//! Per-account statement classification: persistence, resolution, backfill
//! and closing system-account provisioning.
//!
//! The effective `fs_line` of an account is resolved in the same order as the
//! WebApp (`statement-classification.ts`):
//!
//! 1. the account's own valid `fs_line`;
//! 2. the nearest ancestor with a valid `fs_line` (`inherit`/`inherited`);
//! 3. its group's `default_section` suggestion (`auto`);
//! 4. the legacy group-type/code fallback (still `auto`);
//! 5. `UNCLASSIFIED`.
//!
//! Reports must use this resolver rather than the group type alone, so an
//! explicit line on a single account changes its statement placement.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension};

use crate::db::common::db_error;
use crate::db::fs_catalog::{
    get_fs_line, is_fs_line_code, legacy_section_for, permanence_of, section_default_fs_line,
    section_of, statement_of, FsNormalSide, FsPermanence, FsSection, FsStatement, UNCLASSIFIED,
};

/// How deep an inherited line may travel before the account is considered
/// unclassified. Matches the WebApp `MAX_INHERIT_DEPTH` guard.
pub(crate) const MAX_INHERIT_DEPTH: usize = 10;

/// Accounts required by the closing workflow. Roles are explicit metadata and
/// are never inferred from an account name.
pub(crate) const SYSTEM_ROLES: [&str; 6] = [
    "income_summary",
    "retained_earnings",
    "closing_balance",
    "legal_reserve",
    "depreciation_expense",
    "accum_depreciation",
];

#[derive(Debug, Clone)]
pub(crate) struct AccountClassificationRow {
    pub(crate) code: String,
    pub(crate) name: String,
    pub(crate) parent_code: String,
    pub(crate) nature: String,
    pub(crate) group_type: String,
    pub(crate) group_code: String,
    pub(crate) default_section: String,
    pub(crate) stored_fs_line: String,
    pub(crate) classification_source: String,
    pub(crate) contra_of: String,
    pub(crate) contra_kind: String,
    pub(crate) opposite_fs_line: String,
    pub(crate) split_by_sign: bool,
    pub(crate) system_role: String,
}

#[derive(Debug, Clone)]
pub(crate) struct EffectiveClassification {
    pub(crate) fs_line: String,
    pub(crate) source: String,
    pub(crate) section: FsSection,
    pub(crate) statement: FsStatement,
    pub(crate) normal_side: FsNormalSide,
    /// Used by the closing workflow to tell temporary accounts from
    /// permanent ones.
    #[allow(dead_code)]
    pub(crate) permanence: FsPermanence,
    pub(crate) contra_of: String,
    pub(crate) opposite_fs_line: String,
    pub(crate) split_by_sign: bool,
    /// Set on accounts the closing workflow depends on.
    #[allow(dead_code)]
    pub(crate) system_role: String,
}

/// Signed contribution of one account's movement to a statement line.
#[derive(Debug, Clone)]
pub(crate) struct Contribution {
    pub(crate) fs_line: String,
    pub(crate) amount: i64,
    /// `own`, `contra`, or `unclassified`.
    #[allow(dead_code)]
    pub(crate) role: &'static str,
    /// True when a split-by-sign rule moved the balance to its opposite line.
    #[allow(dead_code)]
    pub(crate) moved: bool,
}

/// Loads every account in code order, with its name. Reports iterate this list
/// so they render in the chart's natural order without a second query.
pub(crate) fn load_account_classifications(
    conn: &Connection,
) -> Result<Vec<AccountClassificationRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT a.code,
                    a.name,
                    COALESCE(a.parent_code, ''),
                    a.nature,
                    COALESCE(g.type, ''),
                    COALESCE(a.group_code, ''),
                    COALESCE(g.default_section, ''),
                    COALESCE(a.fs_line, ''),
                    COALESCE(a.classification_source, 'auto'),
                    COALESCE(a.contra_of, ''),
                    COALESCE(a.contra_kind, ''),
                    COALESCE(a.opposite_fs_line, ''),
                    COALESCE(a.split_by_sign, 0),
                    COALESCE(a.system_role, '')
               FROM accounts a
               LEFT JOIN account_groups g ON g.code = a.group_code
              ORDER BY a.code",
        )
        .map_err(db_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(AccountClassificationRow {
                code: row.get(0)?,
                name: row.get(1)?,
                parent_code: row.get(2)?,
                nature: row.get(3)?,
                group_type: row.get(4)?,
                group_code: row.get(5)?,
                default_section: row.get(6)?,
                stored_fs_line: row.get(7)?,
                classification_source: row.get(8)?,
                contra_of: row.get(9)?,
                contra_kind: row.get(10)?,
                opposite_fs_line: row.get(11)?,
                split_by_sign: row.get::<_, i64>(12)? != 0,
                system_role: row.get(13)?,
            })
        })
        .map_err(db_error)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)
}

pub(crate) fn load_classifications(
    conn: &Connection,
) -> Result<HashMap<String, AccountClassificationRow>, String> {
    Ok(load_account_classifications(conn)?
        .into_iter()
        .map(|row| (row.code.clone(), row))
        .collect())
}

fn normalized_source(value: &str) -> &'static str {
    match value {
        "seed" => "seed",
        "inherit" | "inherited" => "inherit",
        "auto" => "auto",
        _ => "manual",
    }
}

fn inherited_line(
    account: &AccountClassificationRow,
    by_code: &HashMap<String, AccountClassificationRow>,
) -> Option<String> {
    let mut current = account;
    let mut visited: HashSet<&str> = HashSet::from([account.code.as_str()]);
    for _ in 0..MAX_INHERIT_DEPTH {
        if current.parent_code.is_empty() {
            return None;
        }
        if !visited.insert(current.parent_code.as_str()) {
            return None;
        }
        let parent = by_code.get(&current.parent_code)?;
        if is_fs_line_code(&parent.stored_fs_line) {
            return Some(parent.stored_fs_line.clone());
        }
        current = parent;
    }
    None
}

pub(crate) fn resolve_account(
    account: &AccountClassificationRow,
    by_code: &HashMap<String, AccountClassificationRow>,
) -> EffectiveClassification {
    let (fs_line, source) = if is_fs_line_code(&account.stored_fs_line) {
        (
            account.stored_fs_line.clone(),
            normalized_source(&account.classification_source).to_string(),
        )
    } else if let Some(inherited) = inherited_line(account, by_code) {
        (inherited, "inherit".to_string())
    } else if let Some(section) = FsSection::from_key(&account.default_section) {
        (
            section_default_fs_line(section.key(), &account.nature)
                .unwrap_or(UNCLASSIFIED)
                .to_string(),
            "auto".to_string(),
        )
    } else {
        let legacy = legacy_section_for(&account.group_type, &account.group_code, &account.code);
        (
            section_default_fs_line(legacy.key(), &account.nature)
                .unwrap_or(UNCLASSIFIED)
                .to_string(),
            "auto".to_string(),
        )
    };

    let mut split_by_sign = account.split_by_sign;
    let mut opposite_fs_line = account.opposite_fs_line.clone();
    // Legacy dual groups share one signed line: a dual-nature account with a
    // debit balance belongs to the expense side even when its group has no
    // explicit statement section. Explicit settings always win.
    if account.nature == "dual"
        && fs_line == "IS.OOI"
        && !split_by_sign
        && opposite_fs_line.is_empty()
    {
        split_by_sign = true;
        opposite_fs_line = "IS.OOE".to_string();
    }

    let line = get_fs_line(&fs_line);
    EffectiveClassification {
        section: section_of(&fs_line),
        statement: statement_of(&fs_line).unwrap_or(FsStatement::BalanceSheet),
        normal_side: line
            .map(|line| line.normal_side)
            .unwrap_or(FsNormalSide::Debit),
        permanence: permanence_of(&fs_line).unwrap_or(FsPermanence::Permanent),
        contra_of: account.contra_of.clone(),
        opposite_fs_line,
        split_by_sign,
        system_role: account.system_role.clone(),
        fs_line,
        source,
    }
}

/// Resolves one account by code. Used by the closing workflow to inspect the
/// role accounts without loading the whole chart twice.
#[allow(dead_code)]
pub(crate) fn resolve_code(
    code: &str,
    by_code: &HashMap<String, AccountClassificationRow>,
) -> Option<EffectiveClassification> {
    by_code
        .get(code)
        .map(|account| resolve_account(account, by_code))
}

/// The contribution rules shared by both statements. The raw amount is
/// `debit − credit`; the returned amount is already signed for its target
/// line.
pub(crate) fn contribution(
    account: &AccountClassificationRow,
    effective: &EffectiveClassification,
    raw: i64,
    by_code: &HashMap<String, AccountClassificationRow>,
) -> Contribution {
    let own_line = effective.fs_line.as_str();

    if !effective.contra_of.is_empty() && effective.contra_of != account.code {
        if let Some(target) = by_code.get(&effective.contra_of) {
            let target_effective = resolve_account(target, by_code);
            if is_fs_line_code(&target_effective.fs_line) {
                return Contribution {
                    amount: crate::db::fs_catalog::sign_of(&target_effective.fs_line) * raw,
                    fs_line: target_effective.fs_line,
                    role: "contra",
                    moved: false,
                };
            }
        }
        if is_fs_line_code(own_line) {
            return Contribution {
                amount: crate::db::fs_catalog::sign_of(own_line) * raw,
                fs_line: own_line.to_string(),
                role: "contra",
                moved: false,
            };
        }
        return Contribution {
            fs_line: UNCLASSIFIED.to_string(),
            amount: raw,
            role: "contra",
            moved: false,
        };
    }

    if !is_fs_line_code(own_line) {
        return Contribution {
            fs_line: UNCLASSIFIED.to_string(),
            amount: raw,
            role: "unclassified",
            moved: false,
        };
    }

    let signed_own = crate::db::fs_catalog::sign_of(own_line) * raw;
    if effective.split_by_sign && signed_own < 0 && is_fs_line_code(&effective.opposite_fs_line) {
        return Contribution {
            amount: crate::db::fs_catalog::sign_of(&effective.opposite_fs_line) * raw,
            fs_line: effective.opposite_fs_line.clone(),
            role: "own",
            moved: true,
        };
    }

    Contribution {
        fs_line: own_line.to_string(),
        amount: signed_own,
        role: "own",
        moved: false,
    }
}

/// Validation shared by the create/update commands. An empty value clears the
/// explicit line and lets the resolver fall back to inheritance/group.
pub(crate) fn validate_fs_line(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value == UNCLASSIFIED {
        return Ok(String::new());
    }
    if !is_fs_line_code(value) {
        return Err(format!("سرفصل صورت‌های مالی نامعتبر است: «{value}»."));
    }
    Ok(value.to_string())
}

pub(crate) fn validate_system_role(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(String::new());
    }
    if !SYSTEM_ROLES.contains(&value) {
        return Err(format!("نقش سیستمی نامعتبر است: «{value}»."));
    }
    Ok(value.to_string())
}

pub(crate) const CONTRA_KINDS: [&str; 6] = [
    "accumulated_depreciation",
    "allowance_receivable",
    "sales_return",
    "purchase_return",
    "overhead_applied",
    "other",
];

pub(crate) fn validate_contra_kind(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || CONTRA_KINDS.contains(&value) {
        return Ok(value.to_string());
    }
    Err(format!("نوع حساب کاهنده نامعتبر است: «{value}»."))
}

/// Stores the classification suggested by each group section on an existing
/// book. Accounts without an explicit group section stay `NULL` so the runtime
/// resolver can keep following their ancestors and group type.
pub(crate) fn backfill_classifications(conn: &Connection) -> Result<(), String> {
    let rows = load_classifications(conn)?;
    for account in rows.values() {
        if is_fs_line_code(&account.stored_fs_line) {
            continue;
        }
        let Some(section) = FsSection::from_key(&account.default_section) else {
            continue;
        };
        let Some(line) = section_default_fs_line(section.key(), &account.nature) else {
            continue;
        };
        conn.execute(
            "UPDATE accounts
                SET fs_line = ?1, classification_source = 'auto'
              WHERE code = ?2 AND (fs_line IS NULL OR fs_line = '')",
            params![line, account.code],
        )
        .map_err(db_error)?;
    }
    Ok(())
}

/// Explicit starter-chart lines. Every other starter account inherits from its
/// parent or falls back to its group section at report time.
const STARTER_LINES: [(&str, &str); 12] = [
    ("1101", "BS.CA.CASH"),
    ("1102", "BS.CA.CASH"),
    ("1103", "BS.CA.CASH"),
    ("1201", "BS.CA.REC"),
    ("1301", "BS.CA.INV"),
    ("2101", "BS.CL.PAY"),
    ("2201", "BS.CL.TAX"),
    ("3101", "BS.EQ.CAP"),
    ("4101", "IS.REV"),
    ("5101", "IS.COGS"),
    ("6101", "IS.ADM"),
    ("710102", "IS.OOE"),
];

const STARTER_GROUPS: [(&str, &str); 11] = [
    ("1", "current_asset"),
    ("2", "non_current_asset"),
    ("3", "non_current_asset"),
    ("4", "current_liability"),
    ("5", "non_current_liability"),
    ("6", "equity"),
    ("7", "revenue"),
    ("8", "admin_expense"),
    ("9", "cogs"),
    ("10", "other_operating"),
    ("11", "other_non_operating"),
];

/// Seeds the starter chart's group sections and explicit lines. Called after
/// `seed_default_accounts` on a fresh database (the migration runs before the
/// chart exists).
pub(crate) fn seed_starter_classifications(conn: &Connection) -> Result<(), String> {
    for (group_code, section) in STARTER_GROUPS {
        conn.execute(
            "UPDATE account_groups
                SET default_section = ?1
              WHERE code = ?2 AND (default_section IS NULL OR default_section = '')",
            params![section, group_code],
        )
        .map_err(db_error)?;
    }
    for (code, line) in STARTER_LINES {
        conn.execute(
            "UPDATE accounts
                SET fs_line = ?1, classification_source = 'seed'
              WHERE code = ?2 AND (fs_line IS NULL OR fs_line = '')",
            params![line, code],
        )
        .map_err(db_error)?;
    }
    // Group 10 is dual; its expense child must not inherit the credit-normal
    // other-operating-income line. Store the explicit opposite line.
    conn.execute(
        "UPDATE accounts
            SET opposite_fs_line = 'IS.OOE', split_by_sign = 1
          WHERE code = '7101' AND (opposite_fs_line IS NULL OR opposite_fs_line = '')",
        [],
    )
    .map_err(db_error)?;
    Ok(())
}

/// Provisions the closing roles once a chart exists. It never reuses an
/// occupied code and never infers a role from an account name.
pub(crate) fn ensure_closing_system_accounts(conn: &Connection) -> Result<(), String> {
    let account_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
        .map_err(db_error)?;
    if account_count == 0 {
        return Ok(());
    }

    let definitions = [
        ("income_summary", "9801", "خلاصه سود و زیان", "6", "BS.EQ.RE"),
        (
            "retained_earnings",
            "9802",
            "سود و زیان انباشته",
            "6",
            "BS.EQ.RE",
        ),
        ("closing_balance", "9803", "کنترل اختتامیه", "9", "MEMO"),
    ];

    for (role, preferred_code, name, group_code, fs_line) in definitions {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM accounts WHERE system_role = ?1)",
                params![role],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if exists {
            continue;
        }
        let group_exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM account_groups WHERE code = ?1)",
                params![group_code],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if !group_exists {
            continue;
        }
        let mut candidate = preferred_code.to_string();
        let mut suffix = 0;
        loop {
            let occupied: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM accounts WHERE code = ?1)",
                    params![candidate],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            if !occupied {
                break;
            }
            suffix += 1;
            candidate = format!("{preferred_code}{suffix}");
        }
        let code = candidate;
        conn.execute(
            "INSERT INTO accounts
                (code, name, level, group_code, nature, parent_code, parent_name,
                 is_postable, is_active, fs_line, classification_source, system_role)
             VALUES (?1, ?2, 1, ?3, 'dual', NULL, '', 1, 1, ?4, 'seed', ?5)",
            params![code, name, group_code, fs_line, role],
        )
        .map_err(db_error)?;
    }
    Ok(())
}

/// Closing-role lookup used by the closing workflow.
#[allow(dead_code)]
pub(crate) fn role_account(
    conn: &Connection,
    role: &str,
) -> Result<Option<(String, String)>, String> {
    conn.query_row(
        "SELECT code, name FROM accounts WHERE system_role = ?1",
        params![role],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(db_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        code: &str,
        parent: &str,
        nature: &str,
        section: &str,
        fs: &str,
    ) -> AccountClassificationRow {
        AccountClassificationRow {
            code: code.to_string(),
            name: code.to_string(),
            parent_code: parent.to_string(),
            nature: nature.to_string(),
            group_type: "دارایی".to_string(),
            group_code: "1".to_string(),
            default_section: section.to_string(),
            stored_fs_line: fs.to_string(),
            classification_source: "auto".to_string(),
            contra_of: String::new(),
            contra_kind: String::new(),
            opposite_fs_line: String::new(),
            split_by_sign: false,
            system_role: String::new(),
        }
    }

    #[test]
    fn explicit_line_wins_over_inheritance_and_group() {
        let account = row("110101", "1101", "debtor", "current_asset", "BS.CA.PREP");
        let map = HashMap::from([(account.code.clone(), account.clone())]);
        let effective = resolve_account(&account, &map);
        assert_eq!(effective.fs_line, "BS.CA.PREP");
        assert_eq!(effective.source, "auto");
    }

    #[test]
    fn nearest_valid_ancestor_is_inherited() {
        let parent = row("1101", "", "debtor", "", "BS.CA.CASH");
        let child = row("110101", "1101", "debtor", "", "");
        let map = HashMap::from([
            (parent.code.clone(), parent),
            (child.code.clone(), child.clone()),
        ]);
        let effective = resolve_account(&child, &map);
        assert_eq!(effective.fs_line, "BS.CA.CASH");
        assert_eq!(effective.source, "inherit");
    }

    #[test]
    fn group_section_suggests_a_line_when_nothing_is_stored() {
        let account = row("9101", "", "creditor", "revenue", "");
        let mut map = HashMap::new();
        map.insert(account.code.clone(), account.clone());
        let effective = resolve_account(&account, &map);
        assert_eq!(effective.fs_line, "IS.REV");
        assert_eq!(effective.source, "auto");
    }

    #[test]
    fn contra_nature_keeps_negative_contribution() {
        // A creditor-nature account on a debit-normal line is a contra asset:
        // its credit balance (raw = -100) reduces the reported asset.
        let account = row("110201", "1102", "creditor", "", "BS.CA.CASH");
        let mut map = HashMap::new();
        map.insert(account.code.clone(), account.clone());
        let effective = resolve_account(&account, &map);
        assert!(effective.normal_side == FsNormalSide::Debit);
        let credit_balance = contribution(&account, &effective, -100, &map);
        assert_eq!(credit_balance.fs_line, "BS.CA.CASH");
        assert_eq!(credit_balance.amount, -100);
        // The same account with a debit movement increases the asset.
        assert_eq!(contribution(&account, &effective, 100, &map).amount, 100);
    }

    #[test]
    fn split_by_sign_moves_negative_balance_to_opposite_line() {
        let mut account = row("7101", "", "dual", "", "IS.OOI");
        account.split_by_sign = true;
        account.opposite_fs_line = "IS.OOE".to_string();
        let mut map = HashMap::new();
        map.insert(account.code.clone(), account.clone());
        let effective = resolve_account(&account, &map);
        let contribution = contribution(&account, &effective, 100, &map);
        assert_eq!(contribution.fs_line, "IS.OOE");
        assert_eq!(contribution.amount, 100);
        assert!(contribution.moved);
    }
}
