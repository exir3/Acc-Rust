//! Account-group and chart-of-accounts persistence.

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::db::common::{db_error, normalize_code, to_fa_digits};
use crate::db::model::{
    Account, AccountGroup, AccountGroupList, AccountList, BulkAccountDeleteFailure,
    BulkAccountDeleteResult, Database, NewAccount,
};
use crate::db::validation::{
    validate_account_code, validate_group_code, validate_level, validate_name, validate_nature,
    validate_section, validate_type,
};

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
        account.level,
        &account.parent_code,
        &account.group_code,
        false,
    )?;

    // Level 1/2 accounts remain valid posting bases even when children exist.
    // The optional level-3 detail is stored in a separate voucher column.
    let postable = account.level <= 2;
    conn.execute(
        "INSERT INTO accounts
            (code, name, level, group_code, nature, parent_code, parent_name, is_postable, is_active)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            account.code,
            account.name,
            account.level,
            account.group_code,
            account.nature,
            parent_code,
            parent_name,
            i64::from(postable),
            i64::from(account.is_active)
        ],
    )
    .map_err(db_error)?;
    let new_id = conn.last_insert_rowid();

    fetch_account(conn, new_id)
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
            let existing_code = tx
                .query_row(
                    "SELECT code FROM account_groups WHERE id = ?1",
                    params![id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(db_error)?;
            let existing_code = existing_code.ok_or_else(|| "گروه حساب یافت نشد.".to_string())?;
            if existing_code != code {
                return Err("تغییر کد گروه حساب مجاز نیست.".to_string());
            }

            tx.execute(
                "UPDATE account_groups
                    SET name = ?1, type = ?2, default_section = ?3,
                        updated_at = datetime('now', 'localtime')
                  WHERE id = ?4",
                params![name, group_type, default_section, id],
            )
            .map_err(db_error)?;
            let group = fetch_group(&tx, id)?;
            tx.commit().map_err(db_error)?;
            Ok(group)
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

            tx.execute("DELETE FROM account_groups WHERE id = ?1", params![id])
                .map_err(db_error)?;
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
    pub fn create_accounts_atomic(&self, accounts: Vec<NewAccount>) -> Result<Vec<Account>, String> {
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
            });
        }

        self.with_conn(move |conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            let mut created = Vec::with_capacity(normalized.len());
            for account in &normalized {
                created.push(insert_account(&tx, account)?);
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
        is_postable: bool,
    ) -> Result<Account, String> {
        self.update_account_inner(id, name, group_code, nature, parent_code, is_postable, None)
    }

    pub fn update_account_with_active(
        &self,
        id: i64,
        name: &str,
        group_code: &str,
        nature: &str,
        parent_code: &str,
        is_postable: bool,
        is_active: bool,
    ) -> Result<Account, String> {
        self.update_account_inner(
            id,
            name,
            group_code,
            nature,
            parent_code,
            is_postable,
            Some(is_active),
        )
    }

    fn update_account_inner(
        &self,
        id: i64,
        name: &str,
        group_code: &str,
        nature: &str,
        parent_code: &str,
        is_postable: bool,
        is_active: Option<bool>,
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
                    "SELECT code, level, group_code, COALESCE(parent_code, '') FROM accounts WHERE id = ?1",
                    params![id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )
                .optional()
                .map_err(db_error)?
                .ok_or_else(|| "حساب یافت نشد.".to_string())?;

            let (code, level, old_group, old_parent_code) = existing;

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

            let children: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM accounts WHERE parent_code = ?1",
                    params![code],
                    |row| row.get(0),
                )
                .map_err(db_error)?;

            if (old_group != group_code || old_parent_code != parent_code)
                && account_subtree_has_lines(&tx, &code)?
            {
                return Err(
                    "این حساب یا یکی از زیرحساب‌های آن در اسناد ثبت‌شده استفاده شده است؛ برای جلوگیری از تغییر طبقه‌بندی سوابق، تغییر حساب بالاتر یا گروه مجاز نیست."
                        .to_string(),
                );
            }

            // Keeping an existing parent is allowed even when it has been
            // deactivated, so historical accounts stay editable. Choosing a
            // different inactive parent is rejected, matching the WebApp.
            let (parent_code, parent_name) = resolve_parent(
                &tx,
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

            let postable = level <= 2 && is_postable;
            // A level-1/2 account is the posting base for both direct rows and
            // rows carrying detail1_code. Do not let the user disable it while
            // any voucher still references that base, even when it has children.
            if !postable && level <= 2 && account_has_lines(&tx, &code)? {
                return Err(
                    "این حساب در اسناد استفاده شده است و باید قابل ثبت بماند.".to_string(),
                );
            }

            tx.execute(
                "UPDATE accounts
                    SET name = ?1, group_code = ?2, nature = ?3,
                        parent_code = ?4, parent_name = ?5, is_postable = ?6,
                        is_active = COALESCE(?10, is_active),
                        postable_before_children = CASE
                            WHEN ?8 = 1 THEN COALESCE(postable_before_children, ?9)
                            ELSE NULL
                        END,
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
                    i64::from(children > 0),
                    i64::from(is_postable),
                    is_active.map(i64::from)
                ],
            )
            .map_err(db_error)?;

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

            if old_parent_code != new_parent_code {
                // Parent postability is independent from child existence in v5.
                // Re-parenting a child therefore does not toggle the parent flag.
            }

            let updated = fetch_account(&tx, id)?;
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
            let updated = conn
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
            fetch_account(conn, id)
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
            delete_account_tx(&tx, id)?;
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
                    Ok(()) => deleted_count += 1,
                    Err(error) => {
                        let (code, name) = label.unwrap_or_else(|| {
                            ("—".to_string(), format!("شناسه {}", to_fa_digits(id)))
                        });
                        blocked.push(BulkAccountDeleteFailure {
                            code,
                            name,
                            error,
                        });
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

fn list_groups(conn: &Connection) -> Result<Vec<AccountGroup>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT g.id, g.code, g.name, g.type, g.default_section, g.created_at, g.updated_at,
                    (SELECT COUNT(*) FROM accounts a WHERE a.group_code = g.code)
               FROM account_groups g
              ORDER BY CAST(g.code AS INTEGER), g.code",
        )
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
                    a.is_active, a.created_at, a.updated_at
               FROM accounts a
               JOIN account_groups g ON g.code = a.group_code
               LEFT JOIN accounts p ON p.code = a.parent_code
              ORDER BY a.code",
        )
        .map_err(db_error)?;
    let rows = stmt.query_map([], row_to_account).map_err(db_error)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)
}

fn fetch_group(conn: &Connection, id: i64) -> Result<AccountGroup, String> {
    conn.query_row(
        "SELECT g.id, g.code, g.name, g.type, g.default_section, g.created_at, g.updated_at,
                (SELECT COUNT(*) FROM accounts a WHERE a.group_code = g.code)
           FROM account_groups g
          WHERE g.id = ?1",
        params![id],
        row_to_group,
    )
    .optional()
    .map_err(db_error)?
    .ok_or_else(|| "گروه حساب یافت نشد.".to_string())
}

fn fetch_account(conn: &Connection, id: i64) -> Result<Account, String> {
    conn.query_row(
        "SELECT a.id, a.code, a.name, a.level, a.group_code, g.name, g.type,
                a.nature, COALESCE(a.parent_code, ''), COALESCE(p.name, ''),
                CASE WHEN a.level IN (1, 2) THEN a.is_postable ELSE 0 END,
                a.is_active, a.created_at, a.updated_at
           FROM accounts a
           JOIN account_groups g ON g.code = a.group_code
           LEFT JOIN accounts p ON p.code = a.parent_code
          WHERE a.id = ?1",
        params![id],
        row_to_account,
    )
    .optional()
    .map_err(db_error)?
    .ok_or_else(|| "حساب یافت نشد.".to_string())
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
    })
}

fn row_to_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    let postable: i64 = row.get(10)?;
    let active: i64 = row.get(11)?;
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
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
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
    level: i64,
    parent_code: &str,
    group_code: &str,
    allow_inactive_parent: bool,
) -> Result<(Option<String>, String), String> {
    if level == 1 {
        return Ok((None, String::new()));
    }
    if parent_code.is_empty() {
        return Err("برای این سطح، انتخاب حساب بالاتر الزامی است.".to_string());
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
        .ok_or_else(|| "حساب بالاتر انتخاب‌شده وجود ندارد.".to_string())?;

    if parent.1 != level - 1 {
        return Err("سطح حساب بالاتر باید یک سطح کمتر از حساب جاری باشد.".to_string());
    }
    if parent.2 != group_code {
        return Err("حساب بالاتر باید به همان گروه حساب تعلق داشته باشد.".to_string());
    }
    if parent.3 == 0 && !allow_inactive_parent {
        return Err("حساب بالاتر غیرفعال است؛ ابتدا آن را فعال کنید.".to_string());
    }
    Ok((Some(parent_code.to_string()), parent.0))
}

