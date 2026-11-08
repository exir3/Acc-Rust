//! Account-group and chart-of-accounts persistence.

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::db::audit;
use crate::db::classification::{self, AccountClassificationRow, EffectiveClassification};
use crate::db::common::{db_error, normalize_code, to_fa_digits, today_jalali};
use crate::db::fs_catalog::{get_fs_line, is_fs_line_code, FsNormalSide};
use crate::db::model::{
    Account, AccountGroup, AccountGroupClassificationImpact, AccountGroupImpactAccount,
    AccountGroupList, AccountList, BulkAccountDeleteFailure, BulkAccountDeleteResult, Database,
    NewAccount,
};
use crate::db::validation::{
    validate_account_code, validate_group_code, validate_level, validate_name, validate_nature,
    validate_section, validate_type,
};

/// Classification fields accepted by the account edit command. An empty
/// `fs_line` clears the explicit line so resolution can fall back to
/// inheritance or the group section.
#[derive(Debug, Clone, Default)]
pub struct ClassificationUpdate {
    pub(crate) fs_line: String,
    pub(crate) contra_of: String,
    pub(crate) contra_kind: String,
    pub(crate) opposite_fs_line: String,
    pub(crate) split_by_sign: bool,
    pub(crate) system_role: String,
}

fn insert_account(conn: &Connection, account: &NewAccount) -> Result<Account, String> {
    let group_exists = conn
        .query_row(
            "SELECT 1 FROM account_groups WHERE code = ?1",
            params![account.group_code],
            |_| Ok(()),
        )
        .optional()
        .map_err(db_error)?
        .is_some();
    if !group_exists {
        return Err("گروه حساب انتخاب‌شده وجود ندارد.".to_string());
    }

    let duplicate = conn
        .query_row(
            "SELECT 1 FROM accounts WHERE code = ?1",
            params![account.code],
            |_| Ok(()),
        )
        .optional()
        .map_err(db_error)?;
    if duplicate.is_some() {
        return Err("این کد حساب قبلاً ثبت شده است.".to_string());
    }

    let (parent_code, parent_name) = resolve_parent(
        conn,
        &account.code,
        account.level,
        &account.parent_code,
        &account.group_code,
        false,
    )?;

    // Same posting policy as WebApp (migration v14 / POST /api/accounts):
    // level 1 (کل) and level 2 (معین) are always the voucher base;
    // level 3 (تفصیلی) is only detail1_code and is never postable.
    // The is_postable flag on NewAccount is ignored so callers cannot drift.
    let postable = account.level <= 2;
    if !account.system_role.is_empty() {
        let role_taken: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM accounts WHERE system_role = ?1)",
                params![account.system_role],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if role_taken {
            return Err("این نقش سیستمی قبلاً به حساب دیگری اختصاص یافته است.".to_string());
        }
    }
    conn.execute(
        "INSERT INTO accounts
            (code, name, level, group_code, nature, parent_code, parent_name,
             is_postable, is_active, fs_line, contra_of, contra_kind,
             opposite_fs_line, split_by_sign, system_role, classification_source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            account.code,
            account.name,
            account.level,
            account.group_code,
            account.nature,
            parent_code,
            parent_name,
            i64::from(postable),
            i64::from(account.is_active),
            optional_text(&account.fs_line),
            optional_text(&account.contra_of),
            optional_text(&account.contra_kind),
            optional_text(&account.opposite_fs_line),
            i64::from(account.split_by_sign),
            optional_text(&account.system_role),
            if account.fs_line.trim().is_empty() {
                "auto"
            } else {
                "manual"
            },
        ],
    )
    .map_err(db_error)?;
    let new_id = conn.last_insert_rowid();
    classification::snapshot_classifications(
        conn,
        std::slice::from_ref(&account.code),
        classification::HISTORY_BASELINE,
        "نسخه اولیه طبقه‌بندی حساب",
    )?;

    let mut created = fetch_account(conn, new_id)?;
    enrich_accounts(conn, std::slice::from_mut(&mut created))?;
    Ok(created)
}

fn optional_text(value: &str) -> Option<&str> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value.trim())
    }
}

/// All codes in the branch rooted at `code`, including the root itself.
fn subtree_codes(conn: &Connection, code: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "WITH RECURSIVE subtree(code) AS (
                 SELECT ?1
                 UNION
                 SELECT a.code
                   FROM accounts a
                   JOIN subtree s ON a.parent_code = s.code
             )
             SELECT code FROM subtree ORDER BY code",
        )
        .map_err(db_error)?;
    let rows = stmt
        .query_map(params![code], |row| row.get(0))
        .map_err(db_error)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)
}

fn group_account_codes(conn: &Connection, group_code: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT code FROM accounts WHERE group_code = ?1 ORDER BY code")
        .map_err(db_error)?;
    let rows = stmt
        .query_map(params![group_code], |row| row.get(0))
        .map_err(db_error)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)
}

impl Database {
    pub fn list_account_groups(&self) -> Result<AccountGroupList, String> {
        let groups = self.with_conn(|conn| list_groups(conn))?;
        Ok(AccountGroupList {
            groups,
            database_path: self.path_string(),
        })
    }

    pub fn create_account_group(
        &self,
        code: &str,
        name: &str,
        group_type: &str,
        default_section: &str,
    ) -> Result<AccountGroup, String> {
        let code = validate_group_code(code)?;
        let name = validate_name(name, "نام گروه")?;
        let group_type = validate_type(group_type)?;
        let default_section = validate_section(default_section)?;

        self.with_conn(|conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            let duplicate = tx
                .query_row(
                    "SELECT id FROM account_groups WHERE code = ?1",
                    params![code],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(db_error)?;
            if duplicate.is_some() {
                return Err("این کد گروه قبلاً ثبت شده است.".to_string());
            }

            tx.execute(
                "INSERT INTO account_groups (code, name, type, default_section)
                 VALUES (?1, ?2, ?3, ?4)",
                params![code, name, group_type, default_section],
            )
            .map_err(db_error)?;
            let group = fetch_group(&tx, tx.last_insert_rowid())?;
            audit::log(
                &tx,
                "create",
                "account_group",
                Some(group.id),
                None,
                Some(&serde_json::to_string(&group).map_err(|error| error.to_string())?),
                "",
            )?;
            tx.commit().map_err(db_error)?;
            Ok(group)
        })
    }

    pub fn update_account_group(
        &self,
        id: i64,
        code: &str,
        name: &str,
        group_type: &str,
        default_section: &str,
    ) -> Result<AccountGroup, String> {
        self.update_account_group_confirmed(id, code, name, group_type, default_section, false)
    }

    /// Updates a group. When the group already has posted voucher lines, a
    /// change to `type` or `default_section` would silently reclassify
    /// historical reports, so it is rejected unless the caller confirms.
    #[allow(clippy::too_many_arguments)]
    pub fn update_account_group_confirmed(
        &self,
        id: i64,
        code: &str,
        name: &str,
        group_type: &str,
        default_section: &str,
        confirm_reclassification: bool,
    ) -> Result<AccountGroup, String> {
        if id <= 0 {
            return Err("شناسه گروه حساب نامعتبر است.".to_string());
        }
        let code = validate_group_code(code)?;
        let name = validate_name(name, "نام گروه")?;
        let group_type = validate_type(group_type)?;
        let default_section = validate_section(default_section)?;

        self.with_conn(|conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            let existing = tx
                .query_row(
                    "SELECT code, type, default_section FROM account_groups WHERE id = ?1",
                    params![id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()
                .map_err(db_error)?;
            let (existing_code, existing_type, existing_section) =
                existing.ok_or_else(|| "گروه حساب یافت نشد.".to_string())?;
            if existing_code != code {
                return Err("تغییر کد گروه حساب مجاز نیست.".to_string());
            }

            let classification_changed =
                existing_type != group_type || existing_section != default_section;
            if classification_changed && !confirm_reclassification {
                let posted_line_count: i64 = tx
                    .query_row(
                        "SELECT COUNT(*)
                           FROM voucher_lines l
                           JOIN accounts a
                             ON a.code = COALESCE(NULLIF(l.detail1_code, ''), l.account_code)
                          WHERE a.group_code = ?1",
                        params![code],
                        |row| row.get(0),
                    )
                    .map_err(db_error)?;
                if posted_line_count > 0 {
                    return Err(format!(
                        "این گروه {} ردیف سند ثبت‌شده دارد. تغییر نوع یا بخش پیش‌فرض از امروز در گزارش‌ها اعمال می‌شود و گزارش‌های تاریخ‌های گذشته با طبقه‌بندی قبلی خود باقی می‌مانند؛ برای ادامه، تغییر را تأیید کنید.",
                        to_fa_digits(posted_line_count)
                    ));
                }
            }

            let before_group = fetch_group(&tx, id)?;
            tx.execute(
                "UPDATE account_groups
                    SET name = ?1, type = ?2, default_section = ?3,
                        updated_at = datetime('now', 'localtime')
                  WHERE id = ?4",
                params![name, group_type, default_section, id],
            )
            .map_err(db_error)?;
            let group = fetch_group(&tx, id)?;
            if classification_changed {
                let codes = group_account_codes(&tx, &code)?;
                classification::snapshot_classifications(
                    &tx,
                    &codes,
                    &today_jalali(),
                    "تغییر طبقه‌بندی گروه حساب",
                )?;
            }
            audit::log(
                &tx,
                "update",
                "account_group",
                Some(id),
                Some(&serde_json::to_string(&before_group).map_err(|error| error.to_string())?),
                Some(&serde_json::to_string(&group).map_err(|error| error.to_string())?),
                "",
            )?;
            tx.commit().map_err(db_error)?;
            Ok(group)
        })
    }

    /// Previews which accounts would change statement line if the group's type
    /// or default section were saved. It performs no writes, so the UI can show
    /// the affected chart before the operator confirms a reclassification.
    pub fn preview_account_group_classification_change(
        &self,
        id: i64,
        group_type: &str,
        default_section: &str,
    ) -> Result<AccountGroupClassificationImpact, String> {
        if id <= 0 {
            return Err("شناسه گروه حساب نامعتبر است.".to_string());
        }
        let group_type = validate_type(group_type)?;
        let default_section = validate_section(default_section)?;
        self.with_conn(|conn| {
            let group_code: String = conn
                .query_row(
                    "SELECT code FROM account_groups WHERE id = ?1",
                    params![id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(db_error)?
                .ok_or_else(|| "گروه حساب یافت نشد.".to_string())?;

            let posted_line_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*)
                       FROM voucher_lines l
                       JOIN accounts a
                         ON a.code = COALESCE(NULLIF(l.detail1_code, ''), l.account_code)
                      WHERE a.group_code = ?1",
                    params![group_code],
                    |row| row.get(0),
                )
                .map_err(db_error)?;

            let current = classification::load_classifications(conn)?;
            let mut projected = current.clone();
            for account in projected.values_mut() {
                if account.group_code == group_code {
                    account.group_type = group_type.clone();
                    account.default_section = default_section.clone();
                }
            }

            let mut affected = Vec::new();
            let mut explicit_count = 0i64;
            let mut inherited_count = 0i64;
            let mut account_count = 0i64;
            for (code, account) in &current {
                if account.group_code != group_code {
                    continue;
                }
                account_count += 1;
                let before = classification::resolve_account(account, &current);
                let after = classification::resolve_account(
                    projected
                        .get(code)
                        .expect("projected chart contains every current account"),
                    &projected,
                );
                if before.fs_line == after.fs_line {
                    continue;
                }
                let explicit = is_fs_line_code(&account.stored_fs_line);
                if explicit {
                    explicit_count += 1;
                } else {
                    inherited_count += 1;
                }
                affected.push(AccountGroupImpactAccount {
                    code: account.code.clone(),
                    name: account.name.clone(),
                    current_fs_line_title: get_fs_line(&before.fs_line)
                        .map(|line| line.title.to_string())
                        .unwrap_or_else(|| "طبقه‌بندی‌نشده".to_string()),
                    current_fs_line: before.fs_line,
                    current_source: before.source,
                    new_fs_line_title: get_fs_line(&after.fs_line)
                        .map(|line| line.title.to_string())
                        .unwrap_or_else(|| "طبقه‌بندی‌نشده".to_string()),
                    new_fs_line: after.fs_line,
                    explicit_line: explicit,
                });
            }
            affected.sort_by(|left, right| left.code.cmp(&right.code));
            Ok(AccountGroupClassificationImpact {
                group_id: id,
                group_code,
                posted_line_count,
                account_count,
                changed_count: affected.len() as i64,
                explicit_count,
                inherited_count,
                accounts: affected,
            })
        })
    }

    pub fn delete_account_group(&self, id: i64) -> Result<(), String> {
        if id <= 0 {
            return Err("شناسه گروه حساب نامعتبر است.".to_string());
        }

        self.with_conn(|conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;

            let code = tx
                .query_row(
                    "SELECT code FROM account_groups WHERE id = ?1",
                    params![id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(db_error)?;
            let code = code.ok_or_else(|| "گروه حساب با این شناسه یافت نشد.".to_string())?;

            let total: i64 = tx
                .query_row("SELECT COUNT(*) FROM account_groups", [], |row| row.get(0))
                .map_err(db_error)?;
            if total <= 1 {
                return Err(
                    "حداقل یک گروه حساب باید وجود داشته باشد؛ حذف آخرین گروه مجاز نیست."
                        .to_string(),
                );
            }

            let accounts: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM accounts WHERE group_code = ?1",
                    params![code],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            if accounts > 0 {
                return Err(format!(
                    "این گروه دارای {} حساب است. ابتدا حساب‌ها را حذف یا جابجا کنید.",
                    to_fa_digits(accounts)
                ));
            }

            let before_group = fetch_group(&tx, id)?;
            tx.execute("DELETE FROM account_groups WHERE id = ?1", params![id])
                .map_err(db_error)?;
            audit::log(
                &tx,
                "delete",
                "account_group",
                Some(id),
                Some(&serde_json::to_string(&before_group).map_err(|error| error.to_string())?),
                None,
                "",
            )?;
            tx.commit().map_err(db_error)?;
            Ok(())
        })
    }

    /* ── Chart of accounts ────────────────────────────────────── */

    pub fn list_accounts(&self) -> Result<AccountList, String> {
        let accounts = self.with_conn(list_accounts)?;
        Ok(AccountList {
            accounts,
            database_path: self.path_string(),
        })
    }

    /// Kept flat because the Tauri command layer passes one field per
    /// JavaScript argument; grouping them would change the public invoke API.
    #[allow(clippy::too_many_arguments)]
    pub fn create_account(
        &self,
        code: &str,
        name: &str,
        level: i64,
        group_code: &str,
        nature: &str,
        parent_code: &str,
        is_postable: bool,
    ) -> Result<Account, String> {
        self.create_account_active(
            code,
            name,
            level,
            group_code,
            nature,
            parent_code,
            is_postable,
            true,
        )
    }

    /// Creates one account and lets the caller choose its active flag.
    /// The seven-argument `create_account` above remains the default entry
    /// point used by older callers and tests.
    #[allow(clippy::too_many_arguments)]
    pub fn create_account_active(
        &self,
        code: &str,
        name: &str,
        level: i64,
        group_code: &str,
        nature: &str,
        parent_code: &str,
        is_postable: bool,
        is_active: bool,
    ) -> Result<Account, String> {
        let mut created = self.create_accounts_atomic(vec![NewAccount::new(
            code,
            name,
            level,
            group_code,
            nature,
            parent_code,
            is_postable,
        )
        .with_active(is_active)])?;
        created
            .pop()
            .ok_or_else(|| "ساخت حساب انجام نشد.".to_string())
    }

    /// Creates a group of accounts in one transaction. If any account in the
    /// batch fails validation or insertion, none of them is saved. This keeps
    /// an imported hierarchy level from being left behind half-created.
    pub fn create_accounts_atomic(
        &self,
        accounts: Vec<NewAccount>,
    ) -> Result<Vec<Account>, String> {
        if accounts.is_empty() {
            return Ok(Vec::new());
        }
        let mut normalized = Vec::with_capacity(accounts.len());
        for account in accounts {
            normalized.push(NewAccount {
                code: validate_account_code(&account.code)?,
                name: validate_name(&account.name, "نام حساب")?,
                level: validate_level(account.level)?,
                group_code: normalize_code(&account.group_code),
                nature: validate_nature(&account.nature)?,
                parent_code: normalize_code(&account.parent_code),
                is_postable: account.is_postable,
                is_active: account.is_active,
                fs_line: classification::validate_fs_line(&account.fs_line)?,
                contra_of: normalize_code(&account.contra_of),
                contra_kind: classification::validate_contra_kind(&account.contra_kind)?,
                opposite_fs_line: classification::validate_fs_line(&account.opposite_fs_line)?,
                split_by_sign: account.split_by_sign,
                system_role: classification::validate_system_role(&account.system_role)?,
            });
        }

        self.with_conn(move |conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            let mut created = Vec::with_capacity(normalized.len());
            for account in &normalized {
                let created_account = insert_account(&tx, account)?;
                audit::log(
                    &tx,
                    "create",
                    "account",
                    Some(created_account.id),
                    None,
                    Some(
                        &serde_json::to_string(&created_account)
                            .map_err(|error| error.to_string())?,
                    ),
                    "",
                )?;
                created.push(created_account);
            }
            // Contra references are checked after every account in the batch
            // exists, so a batch may create a target and its contra together.
            for account in &normalized {
                if account.contra_of.is_empty() {
                    if !account.contra_kind.is_empty() {
                        return Err("«نوع کاهنده» بدون تعیین «حساب اصلی» معنا ندارد.".to_string());
                    }
                    continue;
                }
                if account.contra_kind.is_empty() {
                    return Err("برای حساب کاهنده انتخاب نوع کاهنده الزامی است.".to_string());
                }
                classification::validate_contra_target(&tx, &account.code, &account.contra_of)?;
            }
            tx.commit().map_err(db_error)?;
            Ok(created)
        })
    }

    pub fn update_account(
        &self,
        id: i64,
        name: &str,
        group_code: &str,
        nature: &str,
        parent_code: &str,
    ) -> Result<Account, String> {
        self.update_account_inner(id, name, group_code, nature, parent_code, None, None)
    }

    pub fn update_account_with_active(
        &self,
        id: i64,
        name: &str,
        group_code: &str,
        nature: &str,
        parent_code: &str,
        is_active: bool,
    ) -> Result<Account, String> {
        self.update_account_inner(
            id,
            name,
            group_code,
            nature,
            parent_code,
            Some(is_active),
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_account_with_classification(
        &self,
        id: i64,
        name: &str,
        group_code: &str,
        nature: &str,
        parent_code: &str,
        is_active: Option<bool>,
        classification: ClassificationUpdate,
    ) -> Result<Account, String> {
        self.update_account_inner(
            id,
            name,
            group_code,
            nature,
            parent_code,
            is_active,
            Some(classification),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn update_account_inner(
        &self,
        id: i64,
        name: &str,
        group_code: &str,
        nature: &str,
        parent_code: &str,
        is_active: Option<bool>,
        classification: Option<ClassificationUpdate>,
    ) -> Result<Account, String> {
        if id <= 0 {
            return Err("شناسه حساب نامعتبر است.".to_string());
        }
        let name = validate_name(name, "نام حساب")?;
        let nature = validate_nature(nature)?;
        let group_code = normalize_code(group_code);
        let parent_code = normalize_code(parent_code);

        self.with_conn(|conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            let existing = tx
                .query_row(
                    "SELECT code, level, group_code, nature, COALESCE(parent_code, '')
                       FROM accounts WHERE id = ?1",
                    params![id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(db_error)?
                .ok_or_else(|| "حساب یافت نشد.".to_string())?;

            let (code, level, old_group, old_nature, old_parent_code) = existing;

            let group_exists = tx
                .query_row(
                    "SELECT 1 FROM account_groups WHERE code = ?1",
                    params![group_code],
                    |_| Ok(()),
                )
                .optional()
                .map_err(db_error)?
                .is_some();
            if !group_exists {
                return Err("گروه حساب انتخاب‌شده وجود ندارد.".to_string());
            }

            // Historical lines keep their meaning only if the branch they were
            // posted through stays in the same group, nature and hierarchy.
            // Renaming and activating/deactivating remain allowed.
            let hierarchy_changed =
                old_group != group_code || old_nature != nature || old_parent_code != parent_code;
            if hierarchy_changed && account_subtree_has_lines(&tx, &code)? {
                return Err(
                    "این حساب یا یکی از زیرحساب‌های آن در اسناد ثبت‌شده استفاده شده است؛ تغییر گروه، ماهیت یا حساب بالاتر مجاز نیست."
                        .to_string(),
                );
            }

            // Keeping an existing parent is allowed even when it has been
            // deactivated, so historical accounts stay editable. Choosing a
            // different inactive parent is rejected, matching the WebApp.
            let (parent_code, parent_name) = resolve_parent(
                &tx,
                &code,
                level,
                &parent_code,
                &group_code,
                old_parent_code == parent_code,
            )?;
            let new_parent_code = parent_code.as_deref().unwrap_or("");

            if !new_parent_code.is_empty() {
                let creates_cycle: bool = tx
                    .query_row(
                        "WITH RECURSIVE ancestors(code, depth) AS (
                            SELECT parent_code, 1 FROM accounts WHERE code = ?1
                            UNION ALL
                            SELECT a.parent_code, ancestors.depth + 1
                              FROM accounts a
                              JOIN ancestors ON a.code = ancestors.code
                             WHERE a.parent_code IS NOT NULL AND ancestors.depth < 50
                        )
                        SELECT EXISTS(SELECT 1 FROM ancestors WHERE code = ?2)",
                        params![new_parent_code, code],
                        |row| row.get(0),
                    )
                    .map_err(db_error)?;
                if creates_cycle {
                    return Err(
                        "والد انتخابی زیرشاخه همین حساب است؛ این عملیات چرخه‌ای در درخت حساب‌ها ایجاد می‌کند."
                            .to_string(),
                    );
                }
            }

            let postable = level <= 2;

            let before_account = fetch_account(&tx, id)?;
            tx.execute(
                "UPDATE accounts
                    SET name = ?1, group_code = ?2, nature = ?3,
                        parent_code = ?4, parent_name = ?5, is_postable = ?6,
                        is_active = COALESCE(?8, is_active),
                        updated_at = datetime('now', 'localtime')
                  WHERE id = ?7",
                params![
                    name,
                    group_code,
                    nature,
                    parent_code,
                    parent_name,
                    i64::from(postable),
                    id,
                    is_active.map(i64::from)
                ],
            )
            .map_err(db_error)?;

            if let Some(classification) = classification {
                let fs_line = classification::validate_fs_line(&classification.fs_line)?;
                let contra_of = normalize_code(&classification.contra_of);
                let contra_kind = classification::validate_contra_kind(&classification.contra_kind)?;
                let opposite_fs_line =
                    classification::validate_fs_line(&classification.opposite_fs_line)?;
                let system_role =
                    classification::validate_system_role(&classification.system_role)?;
                if !contra_of.is_empty() && contra_of == code {
                    return Err("حساب کاهنده نمی‌تواند به خودش اشاره کند.".to_string());
                }
                if !contra_of.is_empty() && contra_kind.is_empty() {
                    return Err(
                        "برای حساب کاهنده انتخاب نوع کاهنده الزامی است.".to_string()
                    );
                }
                if contra_of.is_empty() && !contra_kind.is_empty() {
                    return Err(
                        "«نوع کاهنده» بدون تعیین «حساب اصلی» معنا ندارد.".to_string()
                    );
                }
                // The unique system-role index rejects a second account for
                // the same role.
                if !system_role.is_empty() {
                    let occupied: Option<i64> = tx
                        .query_row(
                            "SELECT id FROM accounts WHERE system_role = ?1 AND id <> ?2",
                            params![system_role, id],
                            |row| row.get(0),
                        )
                        .optional()
                        .map_err(db_error)?;
                    if occupied.is_some() {
                        return Err("این نقش سیستمی قبلاً به حساب دیگری اختصاص یافته است.".to_string());
                    }
                }
                tx.execute(
                    "UPDATE accounts
                        SET fs_line = ?1, contra_of = ?2, contra_kind = ?3,
                            opposite_fs_line = ?4, split_by_sign = ?5, system_role = ?6,
                            classification_source = ?7,
                            updated_at = datetime('now', 'localtime')
                      WHERE id = ?8",
                    params![
                        optional_text(&fs_line),
                        optional_text(&contra_of),
                        optional_text(&contra_kind),
                        optional_text(&opposite_fs_line),
                        i64::from(classification.split_by_sign),
                        optional_text(&system_role),
                        if fs_line.is_empty() { "auto" } else { "manual" },
                        id
                    ],
                )
                .map_err(db_error)?;
            }

            // parent_name is denormalized for fast hierarchy rendering. Keep
            // direct children in sync with the parent's new display name.
            tx.execute(
                "UPDATE accounts
                    SET parent_name = ?1, updated_at = datetime('now', 'localtime')
                  WHERE parent_code = ?2",
                params![name, code],
            )
            .map_err(db_error)?;
            // Match WebApp: changing a parent account's group also changes
            // every descendant in the same hierarchy, in this transaction.
            if old_group != group_code {
                tx.execute(
                    "WITH RECURSIVE descendants(code) AS (
                        SELECT code FROM accounts WHERE parent_code = ?1
                        UNION
                        SELECT child.code
                          FROM accounts child
                          JOIN descendants parent ON child.parent_code = parent.code
                    )
                    UPDATE accounts
                       SET group_code = ?2, updated_at = datetime('now', 'localtime')
                     WHERE code IN (SELECT code FROM descendants)",
                    params![code, group_code],
                )
                .map_err(db_error)?;
            }

            // Re-read the persisted contra reference: a group or parent change
            // can invalidate an existing target even when the classification
            // fields were not part of this update.
            let current_contra: String = tx
                .query_row(
                    "SELECT COALESCE(contra_of, '') FROM accounts WHERE id = ?1",
                    params![id],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            if !current_contra.is_empty() {
                classification::validate_contra_target(&tx, &code, &current_contra)?;
            }

            let updated = fetch_account(&tx, id)?;
            let classification_changed = before_account.fs_line != updated.fs_line
                || before_account.contra_of != updated.contra_of
                || before_account.contra_kind != updated.contra_kind
                || before_account.opposite_fs_line != updated.opposite_fs_line
                || before_account.split_by_sign != updated.split_by_sign
                || before_account.classification_source != updated.classification_source;
            let branch_changed =
                old_group != group_code || old_nature != nature || old_parent_code != new_parent_code;
            if classification_changed || branch_changed {
                let codes = if classification_changed
                    || old_group != group_code
                    || old_parent_code != new_parent_code
                {
                    subtree_codes(&tx, &code)?
                } else {
                    vec![code.clone()]
                };
                classification::snapshot_classifications(
                    &tx,
                    &codes,
                    &today_jalali(),
                    "تغییر طبقه‌بندی حساب",
                )?;
            }
            audit::log(
                &tx,
                "update",
                "account",
                Some(id),
                Some(
                    &serde_json::to_string(&before_account)
                        .map_err(|error| error.to_string())?,
                ),
                Some(&serde_json::to_string(&updated).map_err(|error| error.to_string())?),
                "",
            )?;
            tx.commit().map_err(db_error)?;
            Ok(updated)
        })
    }

    /// Toggles whether a definition may be chosen for new voucher lines.
    /// Historical vouchers keep their lines and stay editable.
    pub fn set_account_active(&self, id: i64, is_active: bool) -> Result<Account, String> {
        if id <= 0 {
            return Err("شناسه حساب نامعتبر است.".to_string());
        }
        self.with_conn(|conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            let before = fetch_account(&tx, id)?;
            let updated = tx
                .execute(
                    "UPDATE accounts
                        SET is_active = ?1, updated_at = datetime('now', 'localtime')
                      WHERE id = ?2",
                    params![i64::from(is_active), id],
                )
                .map_err(db_error)?;
            if updated == 0 {
                return Err("حساب یافت نشد.".to_string());
            }
            let account = fetch_account(&tx, id)?;
            audit::log(
                &tx,
                "status",
                "account",
                Some(id),
                Some(&serde_json::to_string(&before).map_err(|error| error.to_string())?),
                Some(&serde_json::to_string(&account).map_err(|error| error.to_string())?),
                if is_active {
                    "حساب فعال شد."
                } else {
                    "حساب غیرفعال شد."
                },
            )?;
            tx.commit().map_err(db_error)?;
            Ok(account)
        })
    }

    pub fn delete_account(&self, id: i64) -> Result<(), String> {
        if id <= 0 {
            return Err("شناسه حساب نامعتبر است.".to_string());
        }

        self.with_conn(|conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            let before = fetch_account(&tx, id)?;
            delete_account_tx(&tx, id)?;
            audit::log(
                &tx,
                "delete",
                "account",
                Some(id),
                Some(&serde_json::to_string(&before).map_err(|error| error.to_string())?),
                None,
                "",
            )?;
            tx.commit().map_err(db_error)?;
            Ok(())
        })
    }

    /// Deletes many accounts in one transaction. Accounts that have children
    /// or voucher lines are reported instead of aborting the whole operation,
    /// while an unexpected database error rolls everything back.
    pub fn delete_accounts_bulk(&self, ids: Vec<i64>) -> Result<BulkAccountDeleteResult, String> {
        if ids.is_empty() {
            return Err("حسابی برای حذف انتخاب نشده است.".to_string());
        }
        self.with_conn(move |conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            let mut deleted_count = 0;
            let mut blocked = Vec::new();
            for id in ids {
                let label = tx
                    .query_row(
                        "SELECT code, name FROM accounts WHERE id = ?1",
                        params![id],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                    )
                    .optional()
                    .map_err(db_error)?;
                match delete_account_tx(&tx, id) {
                    Ok(()) => {
                        if let Some((code, name)) = &label {
                            audit::log(
                                &tx,
                                "delete",
                                "account",
                                Some(id),
                                Some(
                                    &serde_json::json!({ "code": code, "name": name }).to_string(),
                                ),
                                None,
                                "",
                            )?;
                        }
                        deleted_count += 1;
                    }
                    Err(error) => {
                        let (code, name) = label.unwrap_or_else(|| {
                            ("—".to_string(), format!("شناسه {}", to_fa_digits(id)))
                        });
                        blocked.push(BulkAccountDeleteFailure { code, name, error });
                    }
                }
            }
            tx.commit().map_err(db_error)?;
            Ok(BulkAccountDeleteResult {
                deleted_count,
                blocked,
            })
        })
    }
}

/// Group rows with account and posted-line counts.
///
/// The counts are aggregated once over the whole chart. Correlated `COUNT(*)`
/// subqueries made `list_account_groups` scan every `voucher_lines` row once
/// per group, which took seconds on a real book; this keeps the same result in
/// one scan and is also reused when a single group is fetched after a save.
const GROUP_WITH_COUNTS: &str = "
    WITH account_counts AS (
        SELECT group_code, COUNT(*) AS account_count
          FROM accounts
         GROUP BY group_code
    ),
    line_counts AS (
        SELECT a.group_code, COUNT(*) AS line_count
          FROM voucher_lines l
          JOIN accounts a
            ON a.code = COALESCE(NULLIF(l.detail1_code, ''), l.account_code)
         GROUP BY a.group_code
    )
    SELECT g.id, g.code, g.name, g.type, g.default_section,
           g.created_at, g.updated_at,
           COALESCE(ac.account_count, 0),
           COALESCE(lc.line_count, 0)
      FROM account_groups g
      LEFT JOIN account_counts ac ON ac.group_code = g.code
      LEFT JOIN line_counts lc ON lc.group_code = g.code
";

fn list_groups(conn: &Connection) -> Result<Vec<AccountGroup>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "{GROUP_WITH_COUNTS} ORDER BY CAST(g.code AS INTEGER), g.code"
        ))
        .map_err(db_error)?;
    let rows = stmt.query_map([], row_to_group).map_err(db_error)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)
}

fn list_accounts(conn: &mut Connection) -> Result<Vec<Account>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.code, a.name, a.level, a.group_code, g.name, g.type,
                    a.nature, COALESCE(a.parent_code, ''), COALESCE(p.name, ''),
                    CASE WHEN a.level IN (1, 2) THEN a.is_postable ELSE 0 END,
                    a.is_active,
                    COALESCE(a.fs_line, ''), COALESCE(a.contra_of, ''),
                    COALESCE(a.contra_kind, ''), COALESCE(a.opposite_fs_line, ''),
                    COALESCE(a.split_by_sign, 0), COALESCE(a.system_role, ''),
                    COALESCE(a.classification_source, 'auto'),
                    a.created_at, a.updated_at
               FROM accounts a
               JOIN account_groups g ON g.code = a.group_code
               LEFT JOIN accounts p ON p.code = a.parent_code
              ORDER BY a.code",
        )
        .map_err(db_error)?;
    let rows = stmt.query_map([], row_to_account).map_err(db_error)?;
    let mut accounts = rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db_error)?;
    enrich_accounts(conn, &mut accounts)?;
    Ok(accounts)
}

/// Fills the effective line, its section/statement and contra flag from the
/// shared resolver. The stored columns stay untouched.
fn enrich_accounts(conn: &Connection, accounts: &mut [Account]) -> Result<(), String> {
    if accounts.is_empty() {
        return Ok(());
    }
    let map = classification::load_classifications(conn)?;
    for account in accounts.iter_mut() {
        if let Some(row) = map.get(&account.code) {
            apply_classification(account, row, &map);
        }
    }
    Ok(())
}

fn apply_classification(
    account: &mut Account,
    row: &AccountClassificationRow,
    map: &std::collections::HashMap<String, AccountClassificationRow>,
) {
    let effective = classification::resolve_account(row, map);
    account.effective_fs_line = effective.fs_line.clone();
    account.effective_section = effective.section.key().to_string();
    account.effective_statement = statement_key(effective.statement).to_string();
    account.effective_fs_line_title = get_fs_line(&effective.fs_line)
        .map(|line| line.title.to_string())
        .unwrap_or_else(|| "طبقه‌بندی‌نشده".to_string());
    account.is_contra = is_contra(row, &effective);
    account.classification_source = effective.source.clone();
    account.fs_line = row.stored_fs_line.clone();
    account.contra_of = row.contra_of.clone();
    account.contra_kind = row.contra_kind.clone();
    account.opposite_fs_line = row.opposite_fs_line.clone();
    account.split_by_sign = row.split_by_sign;
    account.system_role = row.system_role.clone();
}

fn is_contra(row: &AccountClassificationRow, effective: &EffectiveClassification) -> bool {
    if row.nature == "dual" {
        return false;
    }
    let nature_side = if row.nature == "debtor" {
        FsNormalSide::Debit
    } else {
        FsNormalSide::Credit
    };
    nature_side != effective.normal_side
}

fn statement_key(statement: crate::db::fs_catalog::FsStatement) -> &'static str {
    use crate::db::fs_catalog::FsStatement;
    match statement {
        FsStatement::BalanceSheet => "balance_sheet",
        FsStatement::IncomeStatement => "income_statement",
        FsStatement::Memorandum => "memorandum",
    }
}

fn fetch_group(conn: &Connection, id: i64) -> Result<AccountGroup, String> {
    conn.query_row(
        &format!("{GROUP_WITH_COUNTS} WHERE g.id = ?1"),
        params![id],
        row_to_group,
    )
    .optional()
    .map_err(db_error)?
    .ok_or_else(|| "گروه حساب یافت نشد.".to_string())
}

fn fetch_account(conn: &Connection, id: i64) -> Result<Account, String> {
    let mut account = conn
        .query_row(
            "SELECT a.id, a.code, a.name, a.level, a.group_code, g.name, g.type,
                    a.nature, COALESCE(a.parent_code, ''), COALESCE(p.name, ''),
                    CASE WHEN a.level IN (1, 2) THEN a.is_postable ELSE 0 END,
                    a.is_active,
                    COALESCE(a.fs_line, ''), COALESCE(a.contra_of, ''),
                    COALESCE(a.contra_kind, ''), COALESCE(a.opposite_fs_line, ''),
                    COALESCE(a.split_by_sign, 0), COALESCE(a.system_role, ''),
                    COALESCE(a.classification_source, 'auto'),
                    a.created_at, a.updated_at
               FROM accounts a
               JOIN account_groups g ON g.code = a.group_code
               LEFT JOIN accounts p ON p.code = a.parent_code
              WHERE a.id = ?1",
            params![id],
            row_to_account,
        )
        .optional()
        .map_err(db_error)?
        .ok_or_else(|| "حساب یافت نشد.".to_string())?;
    enrich_accounts(conn, std::slice::from_mut(&mut account))?;
    Ok(account)
}

fn row_to_group(row: &rusqlite::Row<'_>) -> rusqlite::Result<AccountGroup> {
    Ok(AccountGroup {
        id: row.get(0)?,
        code: row.get(1)?,
        name: row.get(2)?,
        group_type: row.get(3)?,
        default_section: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        account_count: row.get(7)?,
        posted_line_count: row.get(8)?,
    })
}

fn row_to_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    let postable: i64 = row.get(10)?;
    let active: i64 = row.get(11)?;
    let split_by_sign: i64 = row.get(16)?;
    Ok(Account {
        id: row.get(0)?,
        code: row.get(1)?,
        name: row.get(2)?,
        level: row.get(3)?,
        group_code: row.get(4)?,
        group_name: row.get(5)?,
        group_type: row.get(6)?,
        nature: row.get(7)?,
        parent_code: row.get(8)?,
        parent_name: row.get(9)?,
        is_postable: postable != 0,
        is_active: active != 0,
        fs_line: row.get(12)?,
        effective_fs_line: String::new(),
        effective_fs_line_title: String::new(),
        effective_section: String::new(),
        effective_statement: String::new(),
        classification_source: row.get(18)?,
        system_role: row.get(17)?,
        contra_of: row.get(13)?,
        contra_kind: row.get(14)?,
        opposite_fs_line: row.get(15)?,
        split_by_sign: split_by_sign != 0,
        is_contra: false,
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
    })
}

fn account_has_lines(conn: &Connection, code: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM voucher_lines WHERE account_code = ?1 OR detail1_code = ?1)",
        params![code],
        |row| row.get(0),
    )
    .map_err(db_error)
}

/// Deletes one account inside an existing transaction. Parent postability
/// is independent from whether child accounts remain (schema v5).
fn delete_account_tx(conn: &Connection, id: i64) -> Result<(), String> {
    if id <= 0 {
        return Err("شناسه حساب نامعتبر است.".to_string());
    }
    let code = conn
        .query_row(
            "SELECT code FROM accounts WHERE id = ?1",
            params![id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(db_error)?
        .ok_or_else(|| "حساب با این شناسه یافت نشد.".to_string())?;

    let children: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM accounts WHERE parent_code = ?1",
            params![code],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    if children > 0 {
        return Err("این حساب دارای زیرحساب است؛ ابتدا زیرحساب‌ها را حذف کنید.".to_string());
    }

    if account_has_lines(conn, &code)? {
        return Err("این حساب در اسناد حسابداری استفاده شده و قابل حذف نیست.".to_string());
    }

    conn.execute("DELETE FROM accounts WHERE id = ?1", params![id])
        .map_err(db_error)?;

    // Parent postability does not depend on whether children remain.
    Ok(())
}

fn account_subtree_has_lines(conn: &Connection, code: &str) -> Result<bool, String> {
    conn.query_row(
        "WITH RECURSIVE account_tree(code) AS (
            SELECT code FROM accounts WHERE code = ?1
            UNION
            SELECT child.code
              FROM accounts child
              JOIN account_tree parent ON child.parent_code = parent.code
        )
        SELECT EXISTS (
            SELECT 1
              FROM voucher_lines line
              JOIN account_tree tree ON tree.code = COALESCE(NULLIF(line.detail1_code, ''), line.account_code)
        )",
        params![code],
        |row| row.get(0),
    )
    .map_err(db_error)
}

fn resolve_parent(
    conn: &Connection,
    account_code: &str,
    level: i64,
    parent_code: &str,
    group_code: &str,
    allow_inactive_parent: bool,
) -> Result<(Option<String>, String), String> {
    // Wording matches WebApp validateParentForWrite.
    if level == 1 {
        if !parent_code.is_empty() {
            return Err("حساب سطح اول نباید حساب والد داشته باشد.".to_string());
        }
        return Ok((None, String::new()));
    }
    if parent_code.is_empty() {
        return Err("برای حساب‌های غیر سطح اول، حساب والد الزامی است.".to_string());
    }
    if parent_code == account_code {
        return Err("حساب نمی‌تواند والد خودش باشد.".to_string());
    }

    let parent = conn
        .query_row(
            "SELECT name, level, group_code, is_active FROM accounts WHERE code = ?1",
            params![parent_code],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(db_error)?
        .ok_or_else(|| "حساب والد معتبر نیست.".to_string())?;

    if parent.1 != level - 1 {
        return Err("سطح حساب والد باید دقیقاً یک سطح بالاتر باشد.".to_string());
    }
    if parent.2 != group_code {
        return Err("گروه حساب والد باید با گروه حساب جاری یکسان باشد.".to_string());
    }
    if parent.3 == 0 && !allow_inactive_parent {
        return Err("حساب والد غیرفعال است.".to_string());
    }
    Ok((Some(parent_code.to_string()), parent.0))
}
