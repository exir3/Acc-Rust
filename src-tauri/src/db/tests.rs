//! Database integration tests.

use super::*;
use crate::db::backup::{
    prune_backup_files, prune_restore_safety_files, prune_snapshot_files,
    validate_backup_connection,
};
use crate::db::common::{
    BULK_DELETE_SNAPSHOT_KEY, CURRENT_SCHEMA_VERSION, LEGACY_DB_FILE_NAME, LEGACY_IDENTIFIER,
};
use crate::db::connection::migrate_legacy_database;
use crate::vouchers::{
    DEFAULT_JOURNAL_PAGE_SIZE, DEFAULT_VOUCHER_PAGE_SIZE, MAX_AMOUNT, MAX_DESCRIPTION, MAX_LINES,
    MAX_LINE_DESCRIPTION, MAX_PAGE_SIZE, MAX_VOUCHER_NUMBER,
};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(test)]
// The file is itself `tests.rs`; the inner module keeps the usual `mod tests`
// layout so every test names the same way as elsewhere in the crate.
#[allow(clippy::module_inception)]
mod tests {
    use super::*;

    fn test_database() -> Database {
        memory_database()
    }

    #[test]
    fn account_hierarchy_uses_explicit_parent_not_code_prefix() {
        let db = test_database();

        db.create_account("9901", "کل با کد سفارشی", 1, "1", "debtor", "", true)
            .unwrap();
        let child = db
            .create_account("881122", "معین با کد مستقل", 2, "1", "debtor", "9901", true)
            .unwrap();

        assert_eq!(child.parent_code, "9901");
        assert_eq!(child.level, 2);
    }

    fn insert_test_line(
        conn: &Connection,
        voucher_id: i64,
        row_no: i64,
        base_code: &str,
        detail1_code: Option<&str>,
        debit: i64,
        credit: i64,
    ) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO voucher_lines
                (voucher_id, row_no, account_id, account_code, detail1_code, debit, credit)
             SELECT ?1, ?2, a.id, a.code, ?3, ?4, ?5
               FROM accounts a WHERE a.code = ?6",
            params![voucher_id, row_no, detail1_code, debit, credit, base_code],
        )
    }

    #[test]
    fn database_recovers_from_poisoned_connection_and_path_mutexes() {
        let db = test_database();

        let path_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = db.path.lock().unwrap();
            panic!("simulate a panic while holding the path mutex");
        }));
        assert!(path_panic.is_err());
        assert_eq!(db.path_string(), ":memory:");

        let connection_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = db.conn.lock().unwrap();
            panic!("simulate a panic while holding the connection mutex");
        }));
        assert!(connection_panic.is_err());
        assert_eq!(db.list_account_groups().unwrap().groups.len(), 11);

        // Reattaching after a poisoned lock must also remain non-panicking.
        db.attach(
            Connection::open_in_memory().unwrap(),
            PathBuf::from(":memory:reopened"),
        );
        assert_eq!(db.path_string(), ":memory:reopened");
    }

    #[test]
    fn backup_refuses_to_publish_a_snapshot_with_invalid_accounting_data() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hesabdari-invalid-backup-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let active_path = dir.join(DB_FILE_NAME);
        let backup_path = dir.join("backup.sqlite3");
        let db = file_database(&active_path);
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO vouchers (number, date, type) VALUES (1, '1405/13/01', ?1)",
                params![crate::vouchers::VOUCHER_TYPES[0]],
            )
            .map_err(db_error)?;
            Ok(())
        })
        .unwrap();

        let error = db.backup_to(&backup_path).unwrap_err();
        assert!(error.contains("تاریخ"));
        assert!(!backup_path.exists());
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sqlite_rejects_invalid_voucher_numbers_and_lines() {
        let db = test_database();
        db.with_conn(|conn| {
            assert!(conn
                .execute(
                    "INSERT INTO vouchers (number, date, type) VALUES (0, '1405/01/01', ?1)",
                    params![crate::vouchers::VOUCHER_TYPES[0]],
                )
                .is_err());
            conn.execute(
                "INSERT INTO vouchers (number, date, type) VALUES (1, '1405/01/01', ?1)",
                params![crate::vouchers::VOUCHER_TYPES[0]],
            )
            .map_err(db_error)?;
            let voucher_id = conn.last_insert_rowid();

            assert!(conn
                .execute(
                    "INSERT INTO voucher_lines (voucher_id, row_no, account_id, account_code, detail1_code, debit, credit)
                     SELECT ?1, 1, id, code, NULL, 0, 0 FROM accounts WHERE code = '110101'",
                    params![voucher_id],
                )
                .is_err());
            insert_test_line(conn, voucher_id, 1, "110101", None, 100, 0)
            .map_err(db_error)?;
            assert!(insert_test_line(conn, voucher_id, 1, "410101", None, 0, 100).is_err());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn seeds_default_groups_and_accounts_once() {
        let db = test_database();
        let list = db.list_account_groups().unwrap();
        assert_eq!(list.groups.len(), 11);
        assert_eq!(list.groups[0].code, "1");

        let accounts = db.list_accounts().unwrap();
        // 34 starter accounts plus the three closing roles provisioned with
        // the chart.
        assert_eq!(accounts.accounts.len(), 37);
        assert_eq!(accounts.accounts[0].code, "1101");
        assert_eq!(
            accounts
                .accounts
                .iter()
                .filter(|account| !account.system_role.is_empty())
                .count(),
            3
        );
        assert!(accounts.accounts[0].is_postable);
        assert!(accounts
            .accounts
            .iter()
            .any(|account| account.code == "510101" && account.is_postable));
        assert!(accounts
            .accounts
            .iter()
            .any(|account| account.code == "710101" && account.is_postable));
        assert!(accounts
            .accounts
            .iter()
            .any(|account| account.code == "810102" && account.is_postable));
    }

    #[test]
    fn deleting_the_starter_chart_is_not_undone_by_later_seed_runs() {
        let db = test_database();
        db.with_conn(|conn| {
            conn.execute("DELETE FROM accounts", []).map_err(db_error)?;
            conn.execute("DELETE FROM account_groups", [])
                .map_err(db_error)?;
            seed_default_groups(conn).map_err(db_error)?;
            seed_default_accounts(conn).map_err(db_error)?;
            let groups: i64 = conn
                .query_row("SELECT COUNT(*) FROM account_groups", [], |row| row.get(0))
                .map_err(db_error)?;
            let accounts: i64 = conn
                .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
                .map_err(db_error)?;
            assert_eq!(groups, 0, "deleted groups must stay deleted");
            assert_eq!(accounts, 0, "deleted accounts must stay deleted");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn migration_folds_arabic_letters_in_existing_names() {
        let db = test_database();
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE account_groups SET name = 'گروه عربي' WHERE code = '1'",
                [],
            )
            .map_err(db_error)?;
            conn.execute(
                "UPDATE accounts
                    SET name = 'حساب عربي', parent_name = 'والد عربي'
                  WHERE code = '1101'",
                [],
            )
            .map_err(db_error)?;
            conn.execute(
                "UPDATE app_meta SET value = '13' WHERE key = 'schema_version'",
                [],
            )
            .map_err(db_error)?;

            apply_schema_migrations(conn)?;

            let group: String = conn
                .query_row(
                    "SELECT name FROM account_groups WHERE code = '1'",
                    [],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            let (name, parent_name): (String, String) = conn
                .query_row(
                    "SELECT name, parent_name FROM accounts WHERE code = '1101'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(db_error)?;
            assert_eq!(group, "گروه عربی");
            assert_eq!(name, "حساب عربی");
            assert_eq!(parent_name, "والد عربی");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn upgrading_an_existing_database_marks_the_starter_chart_as_seeded() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        apply_base_schema(&conn).unwrap();
        // Run the base migration first so the schema has every column an
        // upgraded database has, then pretend the file came from the previous
        // release and upgrade it again.
        apply_schema_migrations(&mut conn).unwrap();
        conn.execute(
            "UPDATE app_meta SET value = '13' WHERE key = 'schema_version'",
            [],
        )
        .unwrap();

        apply_schema_migrations(&mut conn).unwrap();

        let flags: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM app_meta
                  WHERE key IN ('starter_groups_seeded', 'starter_accounts_seeded')
                    AND value = '1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(flags, 2);
    }

    #[test]
    fn pre_migration_backup_snapshots_old_files_and_keeps_three() {
        let dir = test_temp_dir("migration-backup");
        let path = dir.join(DB_FILE_NAME);
        {
            let conn = Connection::open(&path).unwrap();
            apply_base_schema(&conn).unwrap();
        }

        let conn = Connection::open(&path).unwrap();
        let snapshot = backup_before_migration(&conn, &path)
            .unwrap()
            .expect("a version 4 database needs a pre-migration snapshot");
        assert!(snapshot.exists());
        {
            let copy = Connection::open(&snapshot).unwrap();
            let version: String = copy
                .query_row(
                    "SELECT value FROM app_meta WHERE key = 'schema_version'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(version, "4");
        }

        conn.execute(
            "UPDATE app_meta SET value = ?1 WHERE key = 'schema_version'",
            params![CURRENT_SCHEMA_VERSION.to_string()],
        )
        .unwrap();
        assert!(backup_before_migration(&conn, &path).unwrap().is_none());

        for index in 0..4 {
            std::fs::write(
                dir.join(format!(
                    "before-migrate-v4-{index}-{}.sqlite3",
                    1_000 + index
                )),
                b"old snapshot",
            )
            .unwrap();
        }
        prune_migration_backups(&dir).unwrap();
        let remaining = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("before-migrate-v")
            })
            .count();
        assert_eq!(remaining, 3);
        assert!(snapshot.exists(), "the newest snapshot must be kept");

        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn starter_chart_exposes_effective_classification() {
        let db = test_database();
        let accounts = db.list_accounts().unwrap().accounts;
        let find = |code: &str| {
            accounts
                .iter()
                .find(|account| account.code == code)
                .unwrap_or_else(|| panic!("account {code} is missing"))
        };

        // An explicit starter line is stored and used as-is.
        let cash = find("1101");
        assert_eq!(cash.fs_line, "BS.CA.CASH");
        assert_eq!(cash.classification_source, "seed");
        assert_eq!(cash.effective_fs_line, "BS.CA.CASH");
        assert_eq!(cash.effective_statement, "balance_sheet");

        // A child without a stored line follows its nearest ancestor.
        let cash_child = find("110101");
        assert_eq!(cash_child.fs_line, "");
        assert_eq!(cash_child.effective_fs_line, "BS.CA.CASH");
        assert_eq!(cash_child.effective_section, "current_asset");
        assert_eq!(cash_child.classification_source, "inherit");

        // An account under a sectioned group with no explicit line anywhere in
        // its branch falls back to the group's default section.
        let non_operating = find("810102");
        assert_eq!(non_operating.fs_line, "");
        assert_eq!(non_operating.effective_fs_line, "IS.NOI");
        assert_eq!(non_operating.effective_statement, "income_statement");
        assert_eq!(non_operating.classification_source, "auto");

        // Group 10 is dual-nature; its expense child carries the explicit
        // expense line rather than inheriting the income line.
        assert_eq!(find("710102").fs_line, "IS.OOE");

        // The closing workflow's role accounts are provisioned once.
        assert_eq!(find("9801").system_role, "income_summary");
        assert_eq!(find("9802").system_role, "retained_earnings");
        assert_eq!(find("9803").system_role, "closing_balance");
    }

    #[test]
    fn system_role_is_unique_across_accounts() {
        let db = test_database();
        let duplicate = NewAccount::new("9901", "خلاصه دوم", 1, "6", "dual", "", true)
            .with_fs_line("BS.EQ.RE")
            .with_classification("", "", "", false, "income_summary");
        let error = db.create_accounts_atomic(vec![duplicate]).unwrap_err();
        assert!(
            error.contains("نقش سیستمی"),
            "unexpected role error: {error}"
        );

        // A second, unclaimed role is accepted and provisioned without
        // touching the existing three.
        let ok = db
            .create_accounts_atomic(vec![NewAccount::new(
                "9902",
                "ذخیره قانونی",
                1,
                "6",
                "creditor",
                "",
                true,
            )
            .with_fs_line("BS.EQ.RES")
            .with_classification("", "", "", false, "legal_reserve")])
            .unwrap();
        assert_eq!(ok[0].system_role, "legal_reserve");
        assert_eq!(
            db.list_accounts()
                .unwrap()
                .accounts
                .iter()
                .filter(|account| account.system_role == "income_summary")
                .count(),
            1
        );
    }

    #[test]
    fn contra_accounts_require_an_existing_same_line_non_contra_target() {
        let db = test_database();
        let target = NewAccount::new("1109", "نقد اصلی", 1, "1", "debtor", "", true)
            .with_fs_line("BS.CA.CASH");
        let valid = NewAccount::new("110901", "نقد کاهنده", 2, "1", "creditor", "1109", true)
            .with_fs_line("BS.CA.CASH")
            .with_classification("1109", "other", "", false, "");
        let created = db
            .create_accounts_atomic(vec![target.clone(), valid])
            .unwrap();
        assert_eq!(created.len(), 2);
        assert_eq!(created[1].contra_of, "1109");

        let missing = NewAccount::new("110902", "کاهنده بی‌مقصد", 2, "1", "creditor", "1109", true)
            .with_fs_line("BS.CA.CASH")
            .with_classification("999999", "other", "", false, "");
        let error = db.create_accounts_atomic(vec![missing]).unwrap_err();
        assert!(error.contains("یافت نشد"), "{error}");

        let different_line = NewAccount::new(
            "110903",
            "کاهنده سرفصل دیگر",
            2,
            "1",
            "creditor",
            "1109",
            true,
        )
        .with_fs_line("BS.CA.REC")
        .with_classification("1109", "other", "", false, "");
        assert!(db
            .create_accounts_atomic(vec![different_line])
            .unwrap_err()
            .contains("همان سرفصل"));

        let self_reference =
            NewAccount::new("110904", "کاهنده خود", 2, "1", "creditor", "1109", true)
                .with_fs_line("BS.CA.CASH")
                .with_classification("110904", "other", "", false, "");
        assert!(db
            .create_accounts_atomic(vec![self_reference])
            .unwrap_err()
            .contains("خودش"));

        let without_kind =
            NewAccount::new("110905", "کاهنده بی‌نوع", 2, "1", "creditor", "1109", true)
                .with_fs_line("BS.CA.CASH")
                .with_classification("1109", "", "", false, "");
        assert!(db
            .create_accounts_atomic(vec![without_kind])
            .unwrap_err()
            .contains("نوع کاهنده"));

        let kind_without_target =
            NewAccount::new("110906", "نوع بی‌حساب", 2, "1", "creditor", "1109", true)
                .with_fs_line("BS.CA.CASH")
                .with_classification("", "other", "", false, "");
        assert!(db
            .create_accounts_atomic(vec![kind_without_target])
            .unwrap_err()
            .contains("حساب اصلی"));

        // A target that is itself a contra account is rejected.
        let nested = NewAccount::new("110907", "کاهنده دوم", 2, "1", "creditor", "1109", true)
            .with_fs_line("BS.CA.CASH")
            .with_classification("110901", "other", "", false, "");
        assert!(db
            .create_accounts_atomic(vec![nested])
            .unwrap_err()
            .contains("غیرکاهنده"));

        // Contra account with same nature as normal side (debtor for BS.CA.CASH) is rejected.
        let same_nature = NewAccount::new(
            "110908",
            "کاهنده با ماهیت عادی",
            2,
            "1",
            "debtor",
            "1109",
            true,
        )
        .with_fs_line("BS.CA.CASH")
        .with_classification("1109", "other", "", false, "");
        let error = db.create_accounts_atomic(vec![same_nature]).unwrap_err();
        assert!(error.contains("ماهیتی مخالف سمت عادی"), "{error}");

        // Contra account with dual nature is rejected.
        let dual_nature = NewAccount::new(
            "110909",
            "کاهنده با ماهیت دوگانه",
            2,
            "1",
            "dual",
            "1109",
            true,
        )
        .with_fs_line("BS.CA.CASH")
        .with_classification("1109", "other", "", false, "");
        let error = db.create_accounts_atomic(vec![dual_nature]).unwrap_err();
        assert!(error.contains("ماهیت دوگانه"), "{error}");

        // Contra account with incompatible contra_kind is rejected.
        let mismatched_kind = NewAccount::new(
            "110910",
            "نوع کاهنده ناسازگار",
            2,
            "1",
            "creditor",
            "1109",
            true,
        )
        .with_fs_line("BS.CA.CASH")
        .with_classification("1109", "sales_return", "", false, "");
        let error = db
            .create_accounts_atomic(vec![mismatched_kind])
            .unwrap_err();
        assert!(error.contains("سازگار نیست"), "{error}");
    }

    #[test]
    fn group_classification_preview_lists_affected_accounts_before_save() {
        let db = test_database();
        db.create_account("1901", "حساب آزمایشی گروه", 1, "1", "debtor", "", true)
            .unwrap();
        let group = db
            .list_account_groups()
            .unwrap()
            .groups
            .into_iter()
            .find(|group| group.code == "1")
            .unwrap();

        let preview = db
            .preview_account_group_classification_change(
                group.id,
                &group.group_type,
                "non_current_asset",
            )
            .unwrap();

        assert_eq!(preview.group_code, "1");
        assert_eq!(preview.changed_count, 1);
        assert_eq!(preview.inherited_count, 1);
        assert_eq!(preview.explicit_count, 0);
        let account = preview
            .accounts
            .iter()
            .find(|account| account.code == "1901")
            .expect("affected account is listed");
        assert_eq!(account.current_fs_line, "BS.CA.REC");
        assert_eq!(account.new_fs_line, "BS.NCA.OTH");
        assert!(!account.explicit_line);
    }

    #[test]
    fn updating_a_contra_account_to_another_line_is_rejected() {
        let db = test_database();
        db.create_accounts_atomic(vec![
            NewAccount::new("1109", "نقد اصلی", 1, "1", "debtor", "", true)
                .with_fs_line("BS.CA.CASH"),
            NewAccount::new("110901", "نقد کاهنده", 2, "1", "creditor", "1109", true)
                .with_fs_line("BS.CA.CASH")
                .with_classification("1109", "other", "", false, ""),
        ])
        .unwrap();
        let contra = db
            .list_accounts()
            .unwrap()
            .accounts
            .into_iter()
            .find(|account| account.code == "110901")
            .unwrap();

        let error = db
            .update_account_with_classification(
                contra.id,
                "نقد کاهنده",
                "1",
                "creditor",
                "1109",
                None,
                ClassificationUpdate {
                    fs_line: "BS.CA.REC".to_string(),
                    contra_of: "1109".to_string(),
                    contra_kind: "other".to_string(),
                    opposite_fs_line: String::new(),
                    split_by_sign: false,
                    system_role: String::new(),
                },
            )
            .unwrap_err();
        assert!(error.contains("همان سرفصل"));

        let error = db
            .update_account_with_classification(
                contra.id,
                "نقد کاهنده",
                "1",
                "creditor",
                "1109",
                None,
                ClassificationUpdate {
                    fs_line: "BS.CA.CASH".to_string(),
                    contra_of: String::new(),
                    contra_kind: "other".to_string(),
                    opposite_fs_line: String::new(),
                    split_by_sign: false,
                    system_role: String::new(),
                },
            )
            .unwrap_err();
        assert!(error.contains("حساب اصلی"), "{error}");
    }

    #[test]
    fn migration_backfills_classification_for_an_existing_chart() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        apply_base_schema(&conn).unwrap();
        seed_default_groups(&mut conn).unwrap();
        seed_default_accounts(&mut conn).unwrap();
        // Simulate a v10 book whose groups already carry a default section.
        conn.execute(
            "UPDATE account_groups SET default_section = 'current_asset' WHERE code = '1'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE account_groups SET default_section = 'cogs' WHERE code = '9'",
            [],
        )
        .unwrap();

        apply_schema_migrations(&mut conn).unwrap();

        let stored: String = conn
            .query_row(
                "SELECT COALESCE(fs_line, '') FROM accounts WHERE code = '110201'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, "BS.CA.REC");
        let source: String = conn
            .query_row(
                "SELECT COALESCE(classification_source, '') FROM accounts WHERE code = '110201'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(source, "auto");

        // A group with no default section is left without a stored line so the
        // runtime resolver can keep following the legacy group/code rule.
        let other_operating: String = conn
            .query_row(
                "SELECT COALESCE(fs_line, '') FROM accounts WHERE code = '710101'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(other_operating, "");
        let version: String = conn
            .query_row(
                "SELECT value FROM app_meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, CURRENT_SCHEMA_VERSION.to_string());
    }

    #[test]
    fn fs_catalog_exposes_every_statement_line_and_role() {
        let catalog = crate::db::fs_catalog::fs_catalog();
        assert!(catalog.lines.len() >= 30);
        assert!(catalog
            .lines
            .iter()
            .any(|line| line.code == "BS.CA.CASH" && line.statement == "balance_sheet"));
        assert!(catalog
            .lines
            .iter()
            .any(|line| line.code == "IS.REV" && line.statement == "income_statement"));
        assert!(catalog.lines.iter().any(|line| line.code == "MEMO"));
        assert!(catalog
            .system_roles
            .iter()
            .any(|role| role.value == "income_summary"));
        assert!(catalog
            .contra_kinds
            .iter()
            .any(|kind| kind.value == "accumulated_depreciation"));
    }

    #[test]
    fn level_two_parent_remains_postable_when_it_has_a_detail_child() {
        let db = test_database();
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE accounts SET is_postable = 1 WHERE code = '110201'",
                [],
            )
            .map_err(db_error)?;
            Ok(())
        })
        .unwrap();

        let parent = db
            .list_accounts()
            .unwrap()
            .accounts
            .into_iter()
            .find(|account| account.code == "110201")
            .unwrap();
        let stored_flag = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT is_postable FROM accounts WHERE code = '110201'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(db_error)
            })
            .unwrap();

        assert!(parent.is_postable);
        assert_eq!(stored_flag, 1);
    }

    #[test]
    fn database_backup_restores_data_and_keeps_a_safety_copy() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hesabdari-backup-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let active_path = dir.join(DB_FILE_NAME);
        let backup_path = dir.join("backup.sqlite3");
        let db = file_database(&active_path);
        let group = db
            .create_account_group("90", "گروه پشتیبان", "دارایی", "")
            .unwrap();

        db.backup_to(&backup_path).unwrap();
        db.delete_account_group(group.id).unwrap();
        assert!(!db
            .list_account_groups()
            .unwrap()
            .groups
            .iter()
            .any(|item| item.code == "90"));

        let safety_path = db.restore_from(&backup_path).unwrap();
        assert!(safety_path.exists());
        assert!(db
            .list_account_groups()
            .unwrap()
            .groups
            .iter()
            .any(|item| item.code == "90"));
        drop(db);

        let invalid_path = dir.join("invalid.sqlite3");
        std::fs::write(&invalid_path, "not a database").unwrap();
        let db = file_database(&active_path);
        assert!(db.restore_from(&invalid_path).is_err());
        let old_schema_path = dir.join("old-schema.sqlite3");
        let old_schema = Connection::open(&old_schema_path).unwrap();
        apply_base_schema(&old_schema).unwrap();
        old_schema
            .execute(
                "UPDATE app_meta SET value = '2' WHERE key = 'schema_version'",
                [],
            )
            .unwrap();
        drop(old_schema);
        assert!(db
            .restore_from(&old_schema_path)
            .unwrap_err()
            .contains("سازگار نیست"));
        assert!(db
            .list_account_groups()
            .unwrap()
            .groups
            .iter()
            .any(|item| item.code == "90"));
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restores_a_v4_backup_and_migrates_legacy_detail_rows() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hesabdari-v4-restore-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let active_path = dir.join(DB_FILE_NAME);
        let backup_path = dir.join("v4-backup.sqlite3");

        // Build a genuine v4-shaped backup: the old schema has account_code
        // only, and an old voucher can point directly at a level-3 detail.
        let mut old = Connection::open(&backup_path).unwrap();
        old.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        apply_base_schema(&old).unwrap();
        seed_default_groups(&mut old).unwrap();
        seed_default_accounts(&mut old).unwrap();
        seed_default_settings(&old).unwrap();
        old.execute(
            "INSERT INTO vouchers (number, date, type) VALUES (1, '1405/01/01', 'سند حسابداری عادی')",
            [],
        ).unwrap();
        let voucher_id = old.last_insert_rowid();
        old.execute(
            "INSERT INTO voucher_lines (voucher_id, row_no, account_code, debit, credit)
             VALUES (?1, 1, '110201001', 750, 0), (?1, 2, '410101', 0, 750)",
            params![voucher_id],
        )
        .unwrap();
        drop(old);

        let db = file_database(&active_path);
        let safety_path = db.restore_from(&backup_path).unwrap();
        assert!(safety_path.exists());
        let restored = db.get_voucher(1).unwrap().unwrap();
        assert_eq!(restored.lines[0].account_code, "110201");
        assert_eq!(restored.lines[0].detail1_code.as_deref(), Some("110201001"));
        assert_eq!(restored.lines[0].debit, 750);
        assert_eq!(restored.lines[1].account_code, "410101");
        assert_eq!(restored.lines[1].detail1_code, None);
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bulk_delete_backup_preserves_and_restores_legacy_unbalanced_vouchers() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hesabdari-bulk-delete-legacy-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let active_path = dir.join(DB_FILE_NAME);
        let db = file_database(&active_path);

        // Simulate an older/externally-created document that is readable but
        // has only one posting line. It must not block deletion of voucher 2.
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO vouchers (number, date, type) VALUES (1, '1405/01/01', ?1)",
                params![crate::vouchers::VOUCHER_TYPES[0]],
            )
            .map_err(db_error)?;
            let voucher_id = conn.last_insert_rowid();
            insert_test_line(conn, voucher_id, 1, "110101", None, 100, 0).map_err(db_error)?;
            Ok(())
        })
        .unwrap();
        db.save_voucher(crate::vouchers::VoucherInput {
            id: None,
            number: Some(2),
            date: "1405/01/02".to_string(),
            voucher_type: crate::vouchers::VOUCHER_TYPES[0].to_string(),
            description: "سند سالم".to_string(),
            fiscal_year_id: None,
            lines: vec![
                crate::vouchers::VoucherLineInput {
                    id: None,
                    account_code: "110101".to_string(),
                    detail1_code: None,
                    description: String::new(),
                    debit: 200,
                    credit: 0,
                },
                crate::vouchers::VoucherLineInput {
                    id: None,
                    account_code: "410101".to_string(),
                    detail1_code: None,
                    description: String::new(),
                    debit: 0,
                    credit: 200,
                },
            ],
        })
        .unwrap();

        let input = crate::vouchers::BulkVoucherDeleteInput {
            scope: "numberRange".to_string(),
            from_date: String::new(),
            to_date: String::new(),
            from_number: Some(2),
            to_number: Some(2),
        };
        let preview = db.preview_bulk_voucher_delete(&input).unwrap();
        let result = db
            .apply_bulk_voucher_delete(&input, &preview.plan_token, true)
            .unwrap();
        assert_eq!(result.deleted_count, 1);
        let backup_path = PathBuf::from(result.pre_delete_backup_file.unwrap());
        assert!(backup_path.exists());
        assert_eq!(db.list_vouchers("", "", "").unwrap().total_count, 1);

        // The pre-delete copy is restorable, and the internal marker is
        // removed from the live database once restoration succeeds.
        db.restore_from(&backup_path).unwrap();
        assert_eq!(db.list_vouchers("", "", "").unwrap().total_count, 2);
        let marker_count: i64 = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM app_meta WHERE key = ?1",
                    params![BULK_DELETE_SNAPSHOT_KEY],
                    |row| row.get(0),
                )
                .map_err(db_error)
            })
            .unwrap();
        assert_eq!(marker_count, 0);

        // A copied snapshot without its signed sidecar manifest is not trusted:
        // the marker inside SQLite is metadata only and must not unlock the
        // relaxed restore path.
        let marker_only_copy = dir.join("marker-only.sqlite3");
        std::fs::copy(&backup_path, &marker_only_copy).unwrap();
        let error = db.restore_from(&marker_only_copy).unwrap_err();
        assert!(
            error.contains("تراز") || error.contains("ساختار ردیف‌ها"),
            "unexpected error: {error}"
        );
        assert_eq!(db.list_vouchers("", "", "").unwrap().total_count, 2);

        // Ordinary user-selected backups remain strict and still report the
        // pre-existing voucher defect rather than silently accepting it.
        let regular_backup = dir.join("regular-backup.sqlite3");
        assert!(db.backup_to(&regular_backup).is_err());
        assert!(!regular_backup.exists());

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backup_retention_keeps_newest_files_and_prunes_old_restore_copies() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hesabdari-backup-retention-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        for index in 1..=35 {
            std::fs::write(
                dir.join(format!("hesabdari-backup-{index}.sqlite3")),
                b"backup",
            )
            .unwrap();
        }
        for index in 1..=7 {
            std::fs::write(
                dir.join(format!(
                    "hesabdari.sqlite3.before-restore-{}-{index}",
                    std::process::id()
                )),
                b"safety",
            )
            .unwrap();
        }
        std::fs::write(dir.join("notes.txt"), b"keep").unwrap();

        assert_eq!(prune_backup_files(&dir).unwrap(), 5);
        let backups = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.starts_with("hesabdari-backup-") && name.ends_with(".sqlite3")
            })
            .count();
        assert_eq!(backups, 30);
        assert!(dir.join("hesabdari-backup-35.sqlite3").exists());
        assert!(!dir.join("hesabdari-backup-1.sqlite3").exists());

        assert_eq!(
            prune_restore_safety_files(&dir.join("hesabdari.sqlite3")).unwrap(),
            2
        );
        let safety_copies = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains(".before-restore-")
            })
            .count();
        assert_eq!(safety_copies, 5);
        assert!(dir.join("notes.txt").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_retention_prunes_old_pre_delete_and_migration_pairs() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hesabdari-snapshot-retention-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let base = SystemTime::now() - Duration::from_secs(120);

        let mut names = Vec::new();
        for index in 1..=12 {
            let pre_delete = format!("pre_delete_20260101_000000_{index:02}.sqlite3");
            let migration = format!("level4-migration-{index:02}.sqlite3");
            for name in [&pre_delete, &migration] {
                let path = dir.join(name);
                std::fs::write(&path, b"snapshot").unwrap();
                let modified = base + Duration::from_secs(index);
                std::fs::File::options()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_modified(modified)
                    .unwrap();
                std::fs::write(
                    dir.join(format!("{name}.manifest.json")),
                    b"{\"kind\":\"test\"}",
                )
                .unwrap();
            }
            names.push((pre_delete, migration));
        }
        std::fs::write(dir.join("hesabdari-backup-1.sqlite3"), b"regular").unwrap();
        std::fs::write(dir.join("snapshot-integrity.key"), [7u8; 32]).unwrap();
        std::fs::write(dir.join("notes.txt"), b"keep").unwrap();

        assert_eq!(prune_snapshot_files(&dir).unwrap(), 4);
        for (pre_delete, migration) in names {
            let pre_path = dir.join(&pre_delete);
            let migration_path = dir.join(&migration);
            let pre_manifest = dir.join(format!("{pre_delete}.manifest.json"));
            let migration_manifest = dir.join(format!("{migration}.manifest.json"));
            if pre_delete.contains("_01.") || pre_delete.contains("_02.") {
                assert!(!pre_path.exists());
                assert!(!pre_manifest.exists());
            } else {
                assert!(pre_path.exists());
                assert!(pre_manifest.exists());
            }
            if migration.contains("-01.") || migration.contains("-02.") {
                assert!(!migration_path.exists());
                assert!(!migration_manifest.exists());
            } else {
                assert!(migration_path.exists());
                assert!(migration_manifest.exists());
            }
        }
        assert!(dir.join("hesabdari-backup-1.sqlite3").exists());
        assert!(dir.join("snapshot-integrity.key").exists());
        assert!(dir.join("notes.txt").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backup_validation_rejects_inconsistent_rows() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        apply_base_schema(&conn).unwrap();
        apply_schema_migrations(&mut conn).unwrap();
        seed_default_groups(&mut conn).unwrap();
        seed_default_accounts(&mut conn).unwrap();
        seed_default_settings(&conn).unwrap();
        validate_backup_connection(&conn).unwrap();

        conn.execute(
            "INSERT INTO vouchers (number, date, type)
             VALUES (1, '1405/13/01', 'سند حسابداری عادی')",
            [],
        )
        .unwrap();
        assert!(validate_backup_connection(&conn)
            .unwrap_err()
            .contains("تاریخ"));
        conn.execute("DELETE FROM vouchers", []).unwrap();

        conn.execute(
            "INSERT INTO vouchers (number, date, type)
             VALUES (1, '1405/01/01', 'سند حسابداری عادی')",
            [],
        )
        .unwrap();
        let voucher_id = conn.last_insert_rowid();
        insert_test_line(&conn, voucher_id, 1, "110101", None, 1000, 0).unwrap();
        assert!(validate_backup_connection(&conn)
            .unwrap_err()
            .contains("تراز"));
        conn.execute("DELETE FROM voucher_lines", []).unwrap();

        conn.execute("DROP TRIGGER voucher_lines_account_pair_insert", [])
            .unwrap();
        conn.execute(
            "INSERT INTO voucher_lines
                (voucher_id, row_no, account_id, account_code, detail1_code, debit, credit)
             VALUES (?1, 1, 999999, '999999', NULL, 1000, 0)",
            params![voucher_id],
        )
        .unwrap();
        assert!(validate_backup_connection(&conn)
            .unwrap_err()
            .contains("کلید خارجی"));
    }

    #[test]
    fn backup_validation_rejects_unexpected_triggers() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        apply_base_schema(&conn).unwrap();
        apply_schema_migrations(&mut conn).unwrap();
        seed_default_groups(&mut conn).unwrap();
        seed_default_accounts(&mut conn).unwrap();
        seed_default_settings(&conn).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER unexpected_voucher_trigger AFTER INSERT ON vouchers
             BEGIN UPDATE settings SET value = 'changed' WHERE key = 'company_name'; END;",
        )
        .unwrap();

        assert!(validate_backup_connection(&conn)
            .unwrap_err()
            .contains("trigger ناشناخته"));
    }

    #[test]
    fn legacy_migration_prefers_the_most_recent_database() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("hesabdari-legacy-{stamp}"));
        let data_dir = root.join("app-data");
        let previous_data_dir = root.join("previous-app-data");
        let old_dir = root.join(LEGACY_IDENTIFIER);
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::create_dir_all(&previous_data_dir).unwrap();
        std::fs::create_dir_all(&old_dir).unwrap();
        let destination = data_dir.join(DB_FILE_NAME);

        let write = |path: &Path, value: &str| {
            let conn = Connection::open(path).unwrap();
            conn.execute_batch("CREATE TABLE sample (value TEXT NOT NULL);")
                .unwrap();
            conn.execute("INSERT INTO sample (value) VALUES (?1)", params![value])
                .unwrap();
        };
        write(&old_dir.join(LEGACY_DB_FILE_NAME), "قدیمی");
        std::thread::sleep(Duration::from_millis(30));
        write(&data_dir.join(LEGACY_DB_FILE_NAME), "جدید");
        std::thread::sleep(Duration::from_millis(30));
        write(&previous_data_dir.join(DB_FILE_NAME), "دیتابیس فعلی");

        migrate_legacy_database(&data_dir, &destination, Some(&previous_data_dir), None).unwrap();
        let copied = Connection::open(&destination).unwrap();
        let value: String = copied
            .query_row("SELECT value FROM sample", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "دیتابیس فعلی");

        drop(copied);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn legacy_migration_preserves_database_from_executable_directory() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("hesabdari-executable-db-{stamp}"));
        let data_dir = root.join("app-data");
        let executable_dir = root.join("release");
        let executable_database = executable_dir.join("database").join(DB_FILE_NAME);
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::create_dir_all(executable_database.parent().unwrap()).unwrap();

        let source = Connection::open(&executable_database).unwrap();
        source
            .execute_batch("CREATE TABLE sample (value TEXT NOT NULL);")
            .unwrap();
        source
            .execute("INSERT INTO sample (value) VALUES ('اطلاعات قبلی')", [])
            .unwrap();
        drop(source);

        let destination = data_dir.join(DB_FILE_NAME);
        migrate_legacy_database(&data_dir, &destination, None, Some(&executable_dir)).unwrap();
        let copied = Connection::open(&destination).unwrap();
        let value: String = copied
            .query_row("SELECT value FROM sample", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "اطلاعات قبلی");
        assert!(executable_database.exists());

        drop(copied);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn creates_updates_and_deletes_group() {
        let db = test_database();
        let created = db
            .create_account_group("۱۲", "  گروه   آزمایشی ", "دارایی", "current_asset")
            .unwrap();
        assert_eq!(created.code, "12");
        assert_eq!(created.name, "گروه آزمایشی");
        assert_eq!(created.default_section, "current_asset");

        let updated = db
            .update_account_group(created.id, "12", "گروه اصلاح‌شده", "هزینه", "")
            .unwrap();
        assert_eq!(updated.name, "گروه اصلاح‌شده");
        assert_eq!(updated.group_type, "هزینه");

        db.delete_account_group(created.id).unwrap();
        assert_eq!(db.list_account_groups().unwrap().groups.len(), 11);
    }

    #[test]
    fn group_counts_cover_base_and_detail_voucher_lines() {
        let db = test_database();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO vouchers (number, date, type) VALUES (1, '1405/01/01', 'عادی')",
                [],
            )
            .map_err(db_error)?;
            let voucher_id = conn.last_insert_rowid();
            // A line on the posting base and a line on one of its details both
            // belong to group "1"; the credit line belongs to group "7".
            insert_test_line(conn, voucher_id, 1, "110101", None, 300, 0).map_err(db_error)?;
            insert_test_line(conn, voucher_id, 2, "110201", Some("110201001"), 700, 0)
                .map_err(db_error)?;
            insert_test_line(conn, voucher_id, 3, "410101", None, 0, 1000).map_err(db_error)?;
            Ok(())
        })
        .unwrap();

        let accounts = db.list_accounts().unwrap().accounts;
        let expected_group_one_accounts = accounts
            .iter()
            .filter(|account| account.group_code == "1")
            .count() as i64;

        let groups = db.list_account_groups().unwrap().groups;
        let group_one = groups.iter().find(|group| group.code == "1").unwrap();
        assert_eq!(group_one.account_count, expected_group_one_accounts);
        assert_eq!(group_one.posted_line_count, 2);

        let group_seven = groups.iter().find(|group| group.code == "7").unwrap();
        assert_eq!(group_seven.posted_line_count, 1);
        assert!(groups
            .iter()
            .any(|group| group.account_count == 0 && group.posted_line_count == 0));
    }

    #[test]
    fn created_and_updated_group_reports_its_counts() {
        let db = test_database();
        let created = db
            .create_account_group("90", "گروه شمارش", "دارایی", "")
            .unwrap();
        assert_eq!(created.account_count, 0);
        assert_eq!(created.posted_line_count, 0);

        db.create_account("9001", "حساب شمارش", 1, "90", "debtor", "", true)
            .unwrap();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO vouchers (number, date, type) VALUES (1, '1405/01/01', 'عادی')",
                [],
            )
            .map_err(db_error)?;
            let voucher_id = conn.last_insert_rowid();
            insert_test_line(conn, voucher_id, 1, "9001", None, 100, 0).map_err(db_error)?;
            Ok(())
        })
        .unwrap();

        // Saving returns the group from `fetch_group`, so its counts must match
        // what the list reports.
        let updated = db
            .update_account_group(created.id, "90", "گروه شمارش", "دارایی", "")
            .unwrap();
        assert_eq!(updated.account_count, 1);
        assert_eq!(updated.posted_line_count, 1);

        let listed = db
            .list_account_groups()
            .unwrap()
            .groups
            .into_iter()
            .find(|group| group.id == created.id)
            .unwrap();
        assert_eq!(listed.account_count, updated.account_count);
        assert_eq!(listed.posted_line_count, updated.posted_line_count);
    }

    #[test]
    fn creates_and_updates_account() {
        let db = test_database();
        let created = db
            .create_account(
                "110101003",
                "حساب آزمایشی",
                3,
                "1",
                "debtor",
                "110101",
                true,
            )
            .unwrap();
        assert_eq!(created.group_name, "دارایی‌های جاری");
        assert_eq!(created.parent_name, "صندوق مرکزی");
        // Level-3 is a detail reference, never a posting base on its own.
        assert!(!created.is_postable);
        assert!(created.is_active);

        // A definition can be paused without touching its vouchers, and the
        // list reports the new state.
        assert!(!db.set_account_active(created.id, false).unwrap().is_active);
        let paused = db
            .list_accounts()
            .unwrap()
            .accounts
            .into_iter()
            .find(|account| account.id == created.id)
            .unwrap();
        assert!(!paused.is_active);
        assert!(db.set_account_active(created.id, true).unwrap().is_active);

        let updated = db
            .update_account(created.id, "حساب آزمایشی ۲", "1", "debtor", "110101")
            .unwrap();
        assert_eq!(updated.name, "حساب آزمایشی ۲");
        assert!(!updated.is_postable);
        assert!(updated.is_active);

        let updated = db
            .update_account_with_active(
                created.id,
                "حساب آزمایشی ۳",
                "1",
                "debtor",
                "110101",
                false,
            )
            .unwrap();
        assert_eq!(updated.name, "حساب آزمایشی ۳");
        assert!(!updated.is_active);

        db.delete_account(created.id).unwrap();
        let restored_parent = db
            .list_accounts()
            .unwrap()
            .accounts
            .into_iter()
            .find(|account| account.code == "110101")
            .unwrap();
        assert!(restored_parent.is_postable);
        assert_eq!(db.list_accounts().unwrap().accounts.len(), 37);
    }

    #[test]
    fn rejects_inactive_parent_for_new_accounts_but_keeps_existing_ones_editable() {
        let db = test_database();
        let parent = db
            .create_account("995", "کل موقت", 1, "1", "debtor", "", false)
            .unwrap();
        let child = db
            .create_account("99501", "معین موقت", 2, "1", "debtor", "995", true)
            .unwrap();

        db.set_account_active(parent.id, false).unwrap();

        let error = db
            .create_account("99502", "معین دوم", 2, "1", "debtor", "995", true)
            .unwrap_err();
        assert!(error.contains("غیرفعال"));

        // Editing an account that keeps its (now inactive) parent is allowed.
        let updated = db
            .update_account(child.id, "معین اصلاح‌شده", "1", "debtor", "995")
            .unwrap();
        assert_eq!(updated.name, "معین اصلاح‌شده");
    }

    #[test]
    fn bulk_delete_keeps_successful_deletions_and_reports_blocked_accounts() {
        let db = test_database();
        let parent = db
            .create_account("993", "کل آزمایشی", 1, "1", "debtor", "", false)
            .unwrap();
        let child = db
            .create_account("99301", "معین آزمایشی", 2, "1", "debtor", "993", true)
            .unwrap();

        let result = db.delete_accounts_bulk(vec![parent.id, child.id]).unwrap();
        assert_eq!(result.deleted_count, 1);
        assert_eq!(result.blocked.len(), 1);
        assert_eq!(result.blocked[0].code, "993");
        assert!(result.blocked[0].error.contains("زیرحساب"));

        let accounts = db.list_accounts().unwrap().accounts;
        assert!(accounts.iter().any(|account| account.code == "993"));
        assert!(!accounts.iter().any(|account| account.code == "99301"));
    }

    #[test]
    fn account_batch_is_atomic_when_a_later_level_fails() {
        let db = test_database();
        let before = db.list_accounts().unwrap().accounts.len();

        let error = db
            .create_accounts_atomic(vec![
                NewAccount::new("110199", "معین موقت", 2, "1", "debtor", "1101", true),
                NewAccount::new("110199", "معین تکراری", 2, "1", "debtor", "1101", true),
            ])
            .unwrap_err();

        assert!(error.contains("قبلاً"));
        assert_eq!(db.list_accounts().unwrap().accounts.len(), before);
    }

    #[test]
    fn changing_account_parent_does_not_toggle_parent_postability() {
        let db = test_database();
        db.create_account("1102020", "والد قدیمی", 2, "1", "debtor", "1102", true)
            .unwrap();
        let child = db
            .create_account("11020201", "زیرحساب", 3, "1", "debtor", "1102020", true)
            .unwrap();

        let before = db.list_accounts().unwrap().accounts;
        assert!(
            before
                .iter()
                .find(|account| account.code == "1102020")
                .unwrap()
                .is_postable
        );
        assert!(
            before
                .iter()
                .find(|account| account.code == "110202")
                .unwrap()
                .is_postable
        );

        db.update_account(
            child.id,
            &child.name,
            &child.group_code,
            &child.nature,
            "110202",
        )
        .unwrap();

        let after = db.list_accounts().unwrap().accounts;
        assert!(
            after
                .iter()
                .find(|account| account.code == "1102020")
                .unwrap()
                .is_postable
        );
        assert!(
            after
                .iter()
                .find(|account| account.code == "110202")
                .unwrap()
                .is_postable
        );
    }

    #[test]
    fn changing_parent_group_updates_all_descendants() {
        let db = test_database();
        let all_accounts = db.list_accounts().unwrap().accounts;
        let root = all_accounts
            .iter()
            .find(|account| {
                account.level == 1
                    && account.group_code == "1"
                    && all_accounts
                        .iter()
                        .any(|child| child.parent_code == account.code)
            })
            .unwrap()
            .clone();
        let target_group = db
            .create_account_group("90", "گروه مقصد آزمون", &root.group_type, "")
            .unwrap();

        db.update_account(root.id, &root.name, &target_group.code, &root.nature, "")
            .unwrap();

        let accounts = db.list_accounts().unwrap().accounts;
        let mut branch = vec![root.code.clone()];
        let mut cursor = 0;
        while cursor < branch.len() {
            let parent = branch[cursor].clone();
            for child in accounts.iter().filter(|a| a.parent_code == parent) {
                branch.push(child.code.clone());
            }
            cursor += 1;
        }
        assert!(branch.len() > 1);
        for code in branch {
            let account = accounts.iter().find(|a| a.code == code).unwrap();
            assert_eq!(account.group_code, target_group.code);
        }
    }

    #[test]
    fn account_codes_support_webapp_maximum_length_of_fifty_digits() {
        let db = test_database();
        let code = "9".repeat(50);
        let created = db
            .create_account(&code, "حساب با کد پنجاه رقمی", 1, "1", "debtor", "", false)
            .unwrap();
        assert_eq!(created.code, code);

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hesabdari-long-account-code-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let backup_path = dir.join("backup.sqlite3");
        db.backup_to(&backup_path).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn kol_and_moein_are_always_postable_like_webapp() {
        let db = test_database();
        let created = db
            .create_account("995", "کل موقت", 1, "1", "debtor", "", false)
            .unwrap();
        assert!(created.is_postable);

        // The caller flag is ignored: WebApp forces is_postable for levels 1 and 2.
        let subsidiary = db
            .create_account(
                "99501",
                "معین همیشه قابل ثبت",
                2,
                "1",
                "debtor",
                "995",
                false,
            )
            .unwrap();
        assert!(subsidiary.is_postable);

        let detail = db
            .create_account("99501001", "تفصیلی", 3, "1", "debtor", "99501", true)
            .unwrap();
        assert!(!detail.is_postable);
    }

    #[test]
    fn renaming_parent_refreshes_denormalized_child_parent_names() {
        let db = test_database();
        let accounts = db.list_accounts().unwrap().accounts;
        let parent = accounts
            .iter()
            .find(|account| account.code == "120101")
            .unwrap()
            .clone();
        let child = accounts
            .iter()
            .find(|account| account.code == "120101001")
            .unwrap();
        assert_eq!(child.parent_name, "بدهکاران تجاری");

        db.update_account(
            parent.id,
            "بدهکاران اصلاح‌شده",
            &parent.group_code,
            &parent.nature,
            &parent.parent_code,
        )
        .unwrap();

        let refreshed = db
            .list_accounts()
            .unwrap()
            .accounts
            .into_iter()
            .find(|account| account.code == "120101001")
            .unwrap();
        assert_eq!(refreshed.parent_name, "بدهکاران اصلاح‌شده");
    }

    #[test]
    fn rejects_invalid_account_code_and_parent() {
        let db = test_database();
        assert!(db
            .create_account("abc", "بد", 2, "1", "debtor", "120101", true)
            .is_err());
        assert!(db
            .create_account("120103", "بدون والد", 2, "1", "debtor", "", true)
            .is_err());
        assert!(db
            .create_account("120104", "والد اشتباه", 3, "1", "debtor", "1101", true)
            .is_err());
        // WebApp validates the explicit parent relationship; it does not
        // require child codes to repeat the parent's digits.
        let differently_prefixed = db
            .create_account(
                "990101",
                "کد مستقل از پیشوند والد",
                2,
                "1",
                "debtor",
                "1201",
                true,
            )
            .unwrap();
        assert_eq!(differently_prefixed.parent_code, "1201");
    }

    #[test]
    fn migrates_schema_v2_to_current_without_losing_account_rows() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        apply_base_schema(&conn).unwrap();
        seed_default_groups(&mut conn).unwrap();
        seed_default_accounts(&mut conn).unwrap();
        conn.execute(
            "UPDATE app_meta SET value = '2' WHERE key = 'schema_version'",
            [],
        )
        .unwrap();

        apply_schema_migrations(&mut conn).unwrap();
        let version: String = conn
            .query_row(
                "SELECT value FROM app_meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let legacy_column_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('accounts') WHERE name = 'postable_before_children'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let account_code: String = conn
            .query_row(
                "SELECT code FROM accounts WHERE code = '110101'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, CURRENT_SCHEMA_VERSION.to_string());
        // The unused legacy column is no longer part of the schema or migrations.
        assert_eq!(legacy_column_count, 0);
        assert_eq!(account_code, "110101");
    }

    #[test]
    fn group_change_is_rejected_for_a_posted_subtree() {
        let db = test_database();
        let group = db
            .list_account_groups()
            .unwrap()
            .groups
            .into_iter()
            .find(|group| group.code == "1")
            .unwrap();
        let root = db
            .list_accounts()
            .unwrap()
            .accounts
            .into_iter()
            .find(|account| account.code == "1101")
            .unwrap();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO vouchers (number, date, type) VALUES (1, '1405/01/01', 'سند حسابداری عادی')",
                [],
            )
            .map_err(db_error)?;
            let voucher_id = conn.last_insert_rowid();
            insert_test_line(conn, voucher_id, 1, "110101", None, 1000, 0)
            .map_err(db_error)?;
            Ok(())
        })
        .unwrap();

        let error = db
            .update_account_group(group.id, &group.code, &group.name, "بدهی", "")
            .unwrap_err();
        assert!(error.contains("ردیف سند ثبت‌شده"));

        // An explicit confirmation still lets the operator reclassify.
        let updated_group = db
            .update_account_group_confirmed(group.id, &group.code, &group.name, "بدهی", "", true)
            .unwrap();
        assert_eq!(updated_group.group_type, "بدهی");

        // Moving a branch that has posted lines would reclassify historical
        // reports, so the account definition itself must stay put.
        let error = db
            .update_account(root.id, &root.name, "4", &root.nature, "")
            .unwrap_err();
        assert!(error.contains("اسناد ثبت‌شده"));

        let accounts = db.list_accounts().unwrap().accounts;
        assert_eq!(
            accounts
                .iter()
                .find(|a| a.code == "1101")
                .unwrap()
                .group_code,
            "1"
        );
        assert_eq!(
            accounts
                .iter()
                .find(|a| a.code == "110101")
                .unwrap()
                .group_code,
            "1"
        );
    }

    #[test]
    fn migrates_legacy_database_including_wal_data() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hesabdari-migrate-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join(LEGACY_DB_FILE_NAME);
        let destination = dir.join(DB_FILE_NAME);

        {
            let conn = Connection::open(&source).unwrap();
            conn.execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA wal_autocheckpoint = 0;
                 CREATE TABLE sample (value TEXT NOT NULL);
                 INSERT INTO sample (value) VALUES ('داده قدیمی');",
            )
            .unwrap();

            // The source connection stays open with its data in the WAL, which
            // is exactly the situation a plain file copy would get wrong.
            migrate_legacy_database(&dir, &destination, None, None).unwrap();
        }

        let copied = Connection::open(&destination).unwrap();
        let value: String = copied
            .query_row("SELECT value FROM sample", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "داده قدیمی");

        drop(copied);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_to_delete_group_or_account_in_use() {
        let db = test_database();
        let group = db.list_account_groups().unwrap().groups[0].clone();
        assert!(db.delete_account_group(group.id).is_err());

        let account = db
            .list_accounts()
            .unwrap()
            .accounts
            .into_iter()
            .find(|a| a.code == "110101")
            .unwrap();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO vouchers (number, date, type) VALUES (1, '1405/01/01', 'عادی')",
                [],
            )
            .map_err(db_error)?;
            let id = conn.last_insert_rowid();
            insert_test_line(conn, id, 1, &account.code, None, 1000, 0).map_err(db_error)?;
            Ok(())
        })
        .unwrap();
        assert!(db.delete_account(account.id).is_err());
    }
    #[test]
    fn account_with_vouchers_can_gain_detail_children_without_losing_postability() {
        let db = test_database();
        let used = db
            .list_accounts()
            .unwrap()
            .accounts
            .into_iter()
            .find(|account| account.code == "110101")
            .unwrap();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO vouchers (number, date, type) VALUES (1, '1405/01/01', 'عادی')",
                [],
            )
            .map_err(db_error)?;
            let id = conn.last_insert_rowid();
            insert_test_line(conn, id, 1, "110101", None, 1000, 0).map_err(db_error)?;
            Ok(())
        })
        .unwrap();

        db.create_account(
            "110101001",
            "تفصیلی جدید",
            3,
            "1",
            "debtor",
            "110101",
            false,
        )
        .unwrap();
        let parent = db
            .list_accounts()
            .unwrap()
            .accounts
            .into_iter()
            .find(|account| account.code == "110101")
            .unwrap();
        assert!(parent.is_postable);

        // Editing name/active state of a posting base with history is allowed;
        // is_postable is derived from level and cannot be turned off.
        let updated = db
            .update_account(used.id, &used.name, "1", "debtor", "1101")
            .unwrap();
        assert!(updated.is_postable);
    }

    #[test]
    fn schema_v5_migrates_level3_lines_to_base_and_detail1_without_changing_totals() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        apply_base_schema(&conn).unwrap();
        seed_default_groups(&mut conn).unwrap();
        seed_default_accounts(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO vouchers (number, date, type) VALUES (1, '1405/01/01', 'سند حسابداری عادی')",
            [],
        ).unwrap();
        let voucher_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO voucher_lines (voucher_id, row_no, account_code, debit, credit)
             VALUES (?1, 1, '110201001', 750, 0)",
            params![voucher_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO voucher_lines (voucher_id, row_no, account_code, debit, credit)
             VALUES (?1, 2, '410101', 0, 750)",
            params![voucher_id],
        )
        .unwrap();

        apply_schema_migrations(&mut conn).unwrap();
        let rows: Vec<(String, Option<String>, i64, i64)> = {
            let mut stmt = conn.prepare(
                "SELECT account_code, detail1_code, debit, credit FROM voucher_lines WHERE voucher_id=?1 ORDER BY row_no"
            ).unwrap();
            stmt.query_map(params![voucher_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
        };
        assert_eq!(
            rows[0],
            ("110201".to_string(), Some("110201001".to_string()), 750, 0)
        );
        assert_eq!(rows[1], ("410101".to_string(), None, 0, 750));
        let fk_issues: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(fk_issues, 0);
    }

    #[test]
    fn schema_v9_preserves_level4_data_as_pending_without_losing_totals() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        apply_base_schema(&conn).unwrap();
        seed_default_groups(&mut conn).unwrap();
        seed_default_accounts(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO accounts
                (code, name, level, group_code, nature, parent_code, parent_name, is_postable)
             VALUES ('110201001001', 'تفصیلی سطح چهار قدیمی', 4, '1', 'debtor', '110201001', 'حساب تفصیلی', 1)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO vouchers (number, date, type) VALUES (1, '1405/01/01', 'سند حسابداری عادی')",
            [],
        ).unwrap();
        let voucher_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO voucher_lines (voucher_id, row_no, account_code, debit, credit)
             VALUES (?1, 1, '110201001001', 1000, 0)",
            params![voucher_id],
        )
        .unwrap();

        apply_schema_migrations(&mut conn).unwrap();
        let version: String = conn
            .query_row(
                "SELECT value FROM app_meta WHERE key='schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let level4_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM accounts WHERE level=4", [], |row| {
                row.get(0)
            })
            .unwrap();
        let (line_count, account_id, pending, account_code): (i64, Option<i64>, i64, String) = conn
            .query_row(
                "SELECT COUNT(*), MAX(account_id), MAX(legacy_level4_pending), MAX(account_code)
                   FROM voucher_lines WHERE voucher_id=?1",
                params![voucher_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(version, CURRENT_SCHEMA_VERSION.to_string());
        assert_eq!(level4_count, 1);
        assert_eq!(line_count, 1);
        assert_eq!(account_id, None);
        assert_eq!(pending, 1);
        assert_eq!(account_code, "110201001001");
    }

    #[test]
    fn reopening_does_not_recreate_a_deleted_fiscal_year() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        apply_base_schema(&conn).unwrap();
        seed_default_groups(&mut conn).unwrap();
        seed_default_accounts(&mut conn).unwrap();
        seed_default_settings(&conn).unwrap();

        // The upgrade turns the textual default year into a real period.
        apply_schema_migrations(&mut conn).unwrap();
        let years: i64 = conn
            .query_row("SELECT COUNT(*) FROM fiscal_years", [], |row| row.get(0))
            .unwrap();
        assert_eq!(years, 1);

        // Deleting an empty year is an explicit operator action; the next
        // launch must not silently recreate it from the display setting.
        conn.execute("DELETE FROM fiscal_years", []).unwrap();
        apply_schema_migrations(&mut conn).unwrap();
        let years: i64 = conn
            .query_row("SELECT COUNT(*) FROM fiscal_years", [], |row| row.get(0))
            .unwrap();
        assert_eq!(years, 0);
    }

    #[test]
    fn explicit_level4_migration_converts_convertible_lines_and_removes_account() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hesabdari-level4-migration-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let db = file_database(&dir.join("hesabdari.sqlite3"));

        let voucher_id = db
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO accounts
                        (code, name, level, group_code, nature, parent_code, parent_name, is_postable)
                     VALUES ('110201001001', 'تفصیلی سطح چهار قدیمی', 4, '1', 'debtor', '110201001', 'حساب تفصیلی', 1)",
                    [],
                )
                .map_err(db_error)?;
                conn.execute(
                    "INSERT INTO vouchers (number, date, type) VALUES (1, '1405/01/01', 'سند حسابداری عادی')",
                    [],
                )
                .map_err(db_error)?;
                let voucher_id = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO voucher_lines
                        (voucher_id, row_no, account_id, account_code, legacy_level4_pending, debit, credit)
                     VALUES (?1, 1, NULL, '110201001001', 1, 1000, 0)",
                    params![voucher_id],
                )
                .map_err(db_error)?;
                conn.execute(
                    "INSERT INTO voucher_lines
                        (voucher_id, row_no, account_id, account_code, legacy_level4_pending, debit, credit)
                     SELECT ?1, 2, id, code, 0, 0, 1000 FROM accounts WHERE code = '110101'",
                    params![voucher_id],
                )
                .map_err(db_error)?;
                Ok(voucher_id)
            })
            .unwrap();

        let analysis = db.analyze_legacy_level4().unwrap();
        assert_eq!(analysis.account_count, 1);
        assert_eq!(analysis.line_count, 1);
        assert_eq!(analysis.convertible_line_count, 1);
        assert_eq!(analysis.ambiguous_line_count, 0);

        let backup = dir.join("backup.sqlite3");
        let result = db.apply_legacy_level4_migration(&backup).unwrap();
        assert_eq!(result.converted_lines, 1);
        assert_eq!(result.pending_lines, 0);
        assert_eq!(result.removed_accounts, 1);
        assert!(backup.is_file());

        db.with_conn(|conn| {
            let (account_code, detail1, pending, account_id): (
                String,
                Option<String>,
                i64,
                Option<i64>,
            ) = conn
                .query_row(
                    "SELECT account_code, detail1_code, legacy_level4_pending, account_id
                       FROM voucher_lines WHERE voucher_id=?1",
                    params![voucher_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .map_err(db_error)?;
            assert_eq!(account_code, "110201");
            assert_eq!(detail1.as_deref(), Some("110201001"));
            assert_eq!(pending, 0);
            assert!(account_id.is_some());
            let level4: i64 = conn
                .query_row("SELECT COUNT(*) FROM accounts WHERE level=4", [], |row| {
                    row.get(0)
                })
                .map_err(db_error)?;
            assert_eq!(level4, 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn manual_level4_resolution_maps_an_ambiguous_account_to_a_chosen_detail() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hesabdari-level4-resolution-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let db = file_database(&dir.join("hesabdari.sqlite3"));

        let voucher_id = db
            .with_conn(|conn| {
                // The parent is a level-3 account, but the group mismatch makes
                // the automatic conversion refuse this row.
                conn.execute(
                    "INSERT INTO accounts
                        (code, name, level, group_code, nature, parent_code, parent_name, is_postable)
                     VALUES ('110201001001', 'سطح چهار مبهم', 4, '2', 'debtor', '110201001', 'حساب تفصیلی', 1)",
                    [],
                )
                .map_err(db_error)?;
                conn.execute(
                    "INSERT INTO vouchers (number, date, type) VALUES (1, '1405/01/01', 'سند حسابداری عادی')",
                    [],
                )
                .map_err(db_error)?;
                let voucher_id = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO voucher_lines
                        (voucher_id, row_no, account_id, account_code, legacy_level4_pending, debit, credit)
                     VALUES (?1, 1, NULL, '110201001001', 1, 1000, 0)",
                    params![voucher_id],
                )
                .map_err(db_error)?;
                conn.execute(
                    "INSERT INTO voucher_lines
                        (voucher_id, row_no, account_id, account_code, legacy_level4_pending, debit, credit)
                     SELECT ?1, 2, id, code, 0, 0, 1000 FROM accounts WHERE code = '110101'",
                    params![voucher_id],
                )
                .map_err(db_error)?;
                Ok(voucher_id)
            })
            .unwrap();

        let analysis = db.analyze_legacy_level4().unwrap();
        assert_eq!(analysis.convertible_line_count, 0);
        assert_eq!(analysis.ambiguous_line_count, 1);

        let preview = db
            .preview_legacy_level4_resolution("110201001001", "110201001")
            .unwrap();
        assert_eq!(preview.line_count, 1);
        assert_eq!(preview.voucher_count, 1);
        assert_eq!(preview.total_debit, 1000);
        assert_eq!(preview.total_credit, 0);
        assert_eq!(preview.balance, 1000);
        assert_eq!(preview.target_account_code, "110201");

        let backup = dir.join("resolution-backup.sqlite3");
        let result = db
            .resolve_legacy_level4_account("110201001001", "110201001", &backup)
            .unwrap();
        assert_eq!(result.converted_lines, 1);
        assert_eq!(result.pending_lines, 0);
        assert_eq!(result.removed_accounts, 1);
        assert!(backup.is_file());

        db.with_conn(|conn| {
            let (account_code, detail1, pending): (String, Option<String>, i64) = conn
                .query_row(
                    "SELECT account_code, detail1_code, legacy_level4_pending
                       FROM voucher_lines WHERE voucher_id=?1 AND row_no=1",
                    params![voucher_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .map_err(db_error)?;
            assert_eq!(account_code, "110201");
            assert_eq!(detail1.as_deref(), Some("110201001"));
            assert_eq!(pending, 0);
            let totals: (i64, i64) = conn
                .query_row(
                    "SELECT COALESCE(SUM(debit),0), COALESCE(SUM(credit),0)
                       FROM voucher_lines WHERE voucher_id=?1",
                    params![voucher_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(db_error)?;
            assert_eq!(totals, (1000, 1000));
            let level4: i64 = conn
                .query_row("SELECT COUNT(*) FROM accounts WHERE level=4", [], |row| {
                    row.get(0)
                })
                .map_err(db_error)?;
            assert_eq!(level4, 0);
            Ok(())
        })
        .unwrap();

        // The mandatory pre-migration backup intentionally contains the
        // ambiguous level-4 hierarchy; restoring it must accept that snapshot
        // and bring the pending row back unchanged.
        db.restore_from(&backup).unwrap();
        db.with_conn(|conn| {
            let level4: i64 = conn
                .query_row("SELECT COUNT(*) FROM accounts WHERE level=4", [], |row| {
                    row.get(0)
                })
                .map_err(db_error)?;
            assert_eq!(level4, 1);
            let (account_code, pending): (String, i64) = conn
                .query_row(
                    "SELECT account_code, legacy_level4_pending
                       FROM voucher_lines WHERE voucher_id=?1 AND row_no=1",
                    params![voucher_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(db_error)?;
            assert_eq!(account_code, "110201001001");
            assert_eq!(pending, 1);
            Ok(())
        })
        .unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_detail_account_used_in_vouchers_cannot_be_reparented() {
        let db = test_database();
        let detail = db
            .list_accounts()
            .unwrap()
            .accounts
            .into_iter()
            .find(|account| account.code == "110201001")
            .unwrap();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO vouchers (number, date, type) VALUES (1, '1405/01/01', 'عادی')",
                [],
            )
            .map_err(db_error)?;
            let voucher_id = conn.last_insert_rowid();
            insert_test_line(conn, voucher_id, 1, "110201", Some("110201001"), 1000, 0)
                .map_err(db_error)?;
            Ok(())
        })
        .unwrap();

        let error = db
            .update_account(
                detail.id,
                &detail.name,
                &detail.group_code,
                &detail.nature,
                "110202",
            )
            .unwrap_err();
        assert!(error.contains("زیرحساب"));
    }

    #[test]
    fn a_posting_base_with_historical_lines_cannot_be_reparented() {
        let db = test_database();
        let base = db
            .list_accounts()
            .unwrap()
            .accounts
            .into_iter()
            .find(|account| account.code == "110201")
            .unwrap();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO vouchers (number, date, type) VALUES (1, '1405/01/01', 'عادی')",
                [],
            )
            .map_err(db_error)?;
            let voucher_id = conn.last_insert_rowid();
            insert_test_line(conn, voucher_id, 1, "110201", None, 1000, 0).map_err(db_error)?;
            insert_test_line(conn, voucher_id, 2, "410101", None, 0, 1000).map_err(db_error)?;
            Ok(())
        })
        .unwrap();

        let error = db
            .update_account(base.id, &base.name, &base.group_code, &base.nature, "1103")
            .unwrap_err();
        assert!(error.contains("اسناد ثبت‌شده"));
    }

    #[test]
    fn settings_expose_the_same_limits_as_voucher_validation() {
        let db = test_database();
        let settings = db.get_settings().unwrap();
        assert_eq!(settings.limits.max_voucher_lines, MAX_LINES);
        assert_eq!(settings.limits.max_amount, MAX_AMOUNT);
        assert_eq!(settings.limits.max_voucher_number, MAX_VOUCHER_NUMBER);
        assert_eq!(settings.limits.max_description, MAX_DESCRIPTION);
        assert_eq!(settings.limits.max_line_description, MAX_LINE_DESCRIPTION);
        assert_eq!(settings.limits.voucher_page_size, DEFAULT_VOUCHER_PAGE_SIZE);
        assert_eq!(settings.limits.journal_page_size, DEFAULT_JOURNAL_PAGE_SIZE);
        assert_eq!(settings.limits.max_page_size, MAX_PAGE_SIZE);

        let saved = db.save_settings("شرکت آزمایشی", "۱۴۰۵").unwrap();
        assert_eq!(saved.limits.max_voucher_lines, MAX_LINES);
        assert_eq!(saved.limits.max_amount, MAX_AMOUNT);
    }

    #[test]
    fn portable_backup_is_encrypted_and_rejects_wrong_password_and_tampering() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hesabdari-portable-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let active_path = dir.join(DB_FILE_NAME);
        let portable_path = dir.join("backup.hacc");
        let db = file_database(&active_path);
        let group = db
            .create_account_group("91", "گروه پشتیبان رمزنگاری‌شده", "دارایی", "")
            .unwrap();

        db.backup_encrypted_to(&portable_path, "correct horse battery")
            .unwrap();
        assert!(is_encrypted_backup(&portable_path).unwrap());
        let raw = std::fs::read(&portable_path).unwrap();
        assert_eq!(&raw[..8], b"HESABDBK");
        assert!(
            !raw.windows(6).any(|window| window == b"SQLite"),
            "the portable backup must not contain the SQLite header in clear text"
        );

        let error = db
            .restore_encrypted_from(&portable_path, "wrong password")
            .unwrap_err();
        assert!(error.contains("رمز عبور"), "unexpected error: {error}");
        assert!(db
            .list_account_groups()
            .unwrap()
            .groups
            .iter()
            .any(|item| item.code == "91"));

        let tampered_path = dir.join("tampered.hacc");
        let mut tampered = raw.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0x01;
        std::fs::write(&tampered_path, tampered).unwrap();
        let error = db
            .restore_encrypted_from(&tampered_path, "correct horse battery")
            .unwrap_err();
        assert!(
            error.contains("رمز عبور")
                || error.contains("آسیب")
                || error.contains("دستکاری")
                || error.contains("صحت"),
            "unexpected error: {error}"
        );

        db.delete_account_group(group.id).unwrap();
        let safety_path = db
            .restore_encrypted_from(&portable_path, "correct horse battery")
            .unwrap();
        assert!(safety_path.exists());
        assert!(db
            .list_account_groups()
            .unwrap()
            .groups
            .iter()
            .any(|item| item.code == "91"));

        let leftover_plaintext: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| {
                name.contains("portable-restore-tmp") || name.contains("portable-backup-tmp")
            })
            .collect();
        assert!(
            leftover_plaintext.is_empty(),
            "temporary plaintext files were not cleaned up: {leftover_plaintext:?}"
        );

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod closing_workflow_tests {
    use super::*;
    use crate::vouchers::{VoucherInput, VoucherLineInput, VOUCHER_TYPES};

    fn line(account_code: &str, debit: i64, credit: i64) -> VoucherLineInput {
        VoucherLineInput {
            id: None,
            account_code: account_code.to_string(),
            detail1_code: None,
            description: String::new(),
            debit,
            credit,
        }
    }

    fn save_cash_sale(db: &Database, year_id: i64, date: &str, amount: i64) {
        let saved = db
            .save_voucher(VoucherInput {
                id: None,
                number: None,
                date: date.to_string(),
                voucher_type: String::new(),
                description: "فروش نقدی آزمایشی".to_string(),
                fiscal_year_id: Some(year_id),
                lines: vec![line("110101", amount, 0), line("410101", 0, amount)],
            })
            .unwrap();
        db.set_voucher_permanent(saved.id, Some(&saved.updated_at))
            .unwrap();
    }

    fn year_status(db: &Database, year_id: i64) -> String {
        db.list_fiscal_years()
            .unwrap()
            .years
            .into_iter()
            .find(|year| year.id == year_id)
            .unwrap()
            .status
    }

    fn voucher_count(db: &Database, year_id: i64, voucher_type: &str) -> i64 {
        db.with_conn(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM vouchers WHERE fiscal_year_id = ?1 AND type = ?2",
                params![year_id, voucher_type],
                |row| row.get(0),
            )
            .map_err(db_error)
        })
        .unwrap()
    }

    #[test]
    fn closing_preview_executes_and_reverses_atomically() {
        let db = memory_database();
        let year = db
            .create_fiscal_year("سال ۱۴۰۵", "1405/01/01", "1405/12/29", "open")
            .unwrap();
        save_cash_sale(&db, year.id, "1405/01/10", 1_000_000);

        let preview = db.get_closing_preview(year.id, None).unwrap();
        assert!(
            preview.can_execute,
            "unexpected blockers: {:?}",
            preview
                .blockers
                .iter()
                .map(|blocker| &blocker.message)
                .collect::<Vec<_>>()
        );
        assert_eq!(preview.net_income, 1_000_000);
        assert!(preview.roles.income_summary.is_some());
        assert!(preview.roles.retained_earnings.is_some());
        assert!(preview.roles.closing_balance.is_some());
        assert_eq!(preview.temp_lines.len(), 4);
        assert_eq!(preview.closing_lines.len(), 4);

        let result = db
            .execute_closing(year.id, None, None, None, None, Some("بستن آزمایشی"))
            .unwrap();
        assert_eq!(result.net_income, 1_000_000);
        assert!(result.temp_voucher.is_some());
        assert!(result.closing_voucher.is_some());
        assert_eq!(year_status(&db, year.id), "closed");
        assert_eq!(voucher_count(&db, year.id, VOUCHER_TYPES[3]), 1);
        assert_eq!(voucher_count(&db, year.id, VOUCHER_TYPES[2]), 1);

        let after = db.get_closing_preview(year.id, None).unwrap();
        assert!(!after.can_execute);
        assert!(after
            .blockers
            .iter()
            .any(|blocker| blocker.code == "already_closed"));
        assert!(after.run.is_some());

        let reversed = db
            .reverse_closing(year.id, Some("برگشت آزمایشی"), false)
            .unwrap();
        assert_eq!(reversed.deleted_vouchers, 2);
        assert_eq!(year_status(&db, year.id), "open");
        assert_eq!(voucher_count(&db, year.id, VOUCHER_TYPES[3]), 0);
        assert_eq!(voucher_count(&db, year.id, VOUCHER_TYPES[2]), 0);

        let reopened = db.get_closing_preview(year.id, None).unwrap();
        assert!(reopened.can_execute, "{:?}", reopened.blockers);
        assert!(reopened.run.is_none());
        assert!(reopened.latest_run.is_some());
    }

    #[test]
    fn reversing_closing_requires_force_when_the_next_year_has_an_opening_voucher() {
        let db = memory_database();
        let source = db
            .create_fiscal_year("سال ۱۴۰۵", "1405/01/01", "1405/12/29", "open")
            .unwrap();
        let target = db
            .create_fiscal_year("سال ۱۴۰۶", "1406/01/01", "1406/12/29", "open")
            .unwrap();
        save_cash_sale(&db, source.id, "1405/01/10", 1_000_000);
        db.execute_closing(source.id, None, None, None, None, None)
            .unwrap();
        db.execute_carry_forward(source.id, target.id, false)
            .unwrap();

        let error = db
            .reverse_closing(source.id, Some("اصلاح"), false)
            .unwrap_err();
        assert!(error.contains("افتتاحیه"), "{error}");

        // The explicit confirmation is the only way to reopen the source year
        // while the next year's opening voucher exists.
        db.reverse_closing(source.id, Some("اصلاح"), true).unwrap();
        assert_eq!(year_status(&db, source.id), "open");
    }

    #[test]
    fn carry_forward_needs_an_executed_close_and_replaces_opening_only_with_force() {
        let db = memory_database();
        let source = db
            .create_fiscal_year("سال ۱۴۰۵", "1405/01/01", "1405/12/29", "open")
            .unwrap();
        let target = db
            .create_fiscal_year("سال ۱۴۰۶", "1406/01/01", "1406/12/29", "open")
            .unwrap();
        save_cash_sale(&db, source.id, "1405/01/10", 1_000_000);

        let early = db.get_carry_forward_preview(source.id, target.id).unwrap();
        assert!(!early.errors.is_empty());

        db.execute_closing(source.id, None, None, None, None, None)
            .unwrap();

        let plan = db.get_carry_forward_preview(source.id, target.id).unwrap();
        assert!(plan.errors.is_empty(), "{:?}", plan.errors);
        assert_eq!(plan.source, "closing");
        assert!(plan.has_closing_voucher);
        assert_eq!(plan.totals.debit, 1_000_000);
        assert_eq!(plan.totals.credit, 1_000_000);
        assert!(plan
            .lines
            .iter()
            .any(|line| line.account_code == "110101" && line.debit == 1_000_000));
        assert!(plan
            .lines
            .iter()
            .any(|line| line.account_code == "9802" && line.credit == 1_000_000));

        let result = db
            .execute_carry_forward(source.id, target.id, false)
            .unwrap();
        assert_eq!(result.line_count, 2);
        assert_eq!(voucher_count(&db, target.id, VOUCHER_TYPES[1]), 1);

        let second = db
            .execute_carry_forward(source.id, target.id, false)
            .unwrap_err();
        assert!(second.contains("تأیید صریح"), "{second}");

        let again = db.get_carry_forward_preview(source.id, target.id).unwrap();
        assert!(again.requires_overwrite);
        assert!(again.existing_opening_voucher.is_some());

        db.execute_carry_forward(source.id, target.id, true)
            .unwrap();
        assert_eq!(voucher_count(&db, target.id, VOUCHER_TYPES[1]), 1);
    }

    #[test]
    fn finalization_requires_the_close_and_reopening_requires_its_reversal() {
        let db = memory_database();
        let year = db
            .create_fiscal_year("سال ۱۴۰۵", "1405/01/01", "1405/12/29", "open")
            .unwrap();

        let not_closed = db
            .set_fiscal_year_status(year.id, "final", None, false)
            .unwrap_err();
        assert!(not_closed.contains("بستن سال مالی"), "{not_closed}");

        save_cash_sale(&db, year.id, "1405/01/10", 1_000_000);
        db.execute_closing(year.id, None, None, None, None, None)
            .unwrap();
        db.set_fiscal_year_status(year.id, "final", None, false)
            .unwrap();
        assert_eq!(year_status(&db, year.id), "final");

        let reopen = db
            .set_fiscal_year_status(year.id, "open", Some("تلاش"), true)
            .unwrap_err();
        assert!(reopen.contains("برگشت بستن"), "{reopen}");

        let reverse_final = db.reverse_closing(year.id, Some("تلاش"), false).unwrap_err();
        assert!(reverse_final.contains("بازگشایی"), "{reverse_final}");

        db.set_fiscal_year_status(year.id, "closed", Some("اصلاح وضعیت"), true)
            .unwrap();
        db.reverse_closing(year.id, Some("برگشت آزمایشی"), false)
            .unwrap();
        assert_eq!(year_status(&db, year.id), "open");

        let final_again = db
            .set_fiscal_year_status(year.id, "final", None, false)
            .unwrap_err();
        assert!(final_again.contains("بستن سال مالی"), "{final_again}");
    }
}
