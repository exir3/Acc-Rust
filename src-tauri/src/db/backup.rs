//! Backup, restore and backup-file validation.

use rusqlite::{backup::Backup, params, Connection, OpenFlags, OptionalExtension};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::db::audit;
use crate::db::common::{
    db_error, from_hex, random_hex, sha256_file, to_fa_digits, to_hex, BULK_DELETE_SNAPSHOT_KEY,
    BULK_DELETE_SNAPSHOT_VALUE, CURRENT_SCHEMA_VERSION, LEGACY_LEVEL4_SNAPSHOT_VALUE,
};
use crate::db::model::Database;
use crate::db::schema::{
    apply_base_schema, apply_schema_migrations, ensure_permanent_voucher_guards,
    ensure_voucher_account_pair_guards,
};
use crate::vouchers::MAX_VOUCHER_NUMBER;

/// What a backup file is allowed to contain beyond a regular backup. Both
/// snapshot kinds are written by the application itself and marked in
/// `app_meta`, so restore can apply exactly the same narrow exception.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SnapshotKind {
    Regular,
    BulkDelete,
    LegacyLevel4Migration,
}

impl SnapshotKind {
    fn marker_value(self) -> Option<&'static str> {
        match self {
            Self::Regular => None,
            Self::BulkDelete => Some(BULK_DELETE_SNAPSHOT_VALUE),
            Self::LegacyLevel4Migration => Some(LEGACY_LEVEL4_SNAPSHOT_VALUE),
        }
    }

    fn from_manifest_value(value: &str) -> Option<Self> {
        match value {
            BULK_DELETE_SNAPSHOT_VALUE => Some(Self::BulkDelete),
            LEGACY_LEVEL4_SNAPSHOT_VALUE => Some(Self::LegacyLevel4Migration),
            _ => None,
        }
    }

    /// A snapshot is a reversible archive of the exact live state, so it may
    /// contain an old voucher that already violates current balancing rules.
    fn preserve_legacy_voucher_structure(self) -> bool {
        !matches!(self, Self::Regular)
    }

    /// Legacy level-4 accounts have a level-3 parent but may carry a different
    /// group code. The migration needs to back them up before mapping them.
    fn allow_legacy_level4_hierarchy(self) -> bool {
        matches!(self, Self::LegacyLevel4Migration)
    }
}

const SNAPSHOT_MANIFEST_SUFFIX: &str = ".manifest.json";
const SNAPSHOT_KEY_FILE: &str = "snapshot-integrity.key";
const SNAPSHOT_MANIFEST_VERSION: u32 = 1;

/// Sidecar proof for an internal snapshot. The marker inside SQLite is only
/// metadata; a relaxed restore path requires this manifest, whose HMAC key is
/// stored outside the backup in the user's application-data folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotManifest {
    version: u32,
    kind: String,
    file_sha256: String,
    created_at_unix: i64,
    operation_id: String,
    mac: String,
}

impl SnapshotManifest {
    fn new(kind: SnapshotKind, file_sha256: String) -> Result<Self, String> {
        let kind = kind
            .marker_value()
            .ok_or_else(|| "نوع snapshot داخلی برای ساخت manifest معتبر نیست.".to_string())?;
        let created_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or_default();
        Ok(Self {
            version: SNAPSHOT_MANIFEST_VERSION,
            kind: kind.to_string(),
            file_sha256,
            created_at_unix,
            operation_id: random_hex(16)?,
            mac: String::new(),
        })
    }

    fn mac_input(&self) -> String {
        format!(
            "hesabdari-snapshot-v1\n{}\n{}\n{}\n{}\n{}",
            self.version, self.kind, self.file_sha256, self.created_at_unix, self.operation_id
        )
    }

    fn sign(&mut self, key: &[u8]) -> Result<(), String> {
        let mut mac = Hmac::<Sha256>::new_from_slice(key)
            .map_err(|_| "کلید احراز اصالت snapshot معتبر نیست.".to_string())?;
        mac.update(self.mac_input().as_bytes());
        self.mac = to_hex(&mac.finalize().into_bytes());
        Ok(())
    }

    fn verify(&self, key: &[u8]) -> Result<(), String> {
        let expected = from_hex(&self.mac)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(key)
            .map_err(|_| "کلید احراز اصالت snapshot معتبر نیست.".to_string())?;
        mac.update(self.mac_input().as_bytes());
        mac.verify_slice(&expected)
            .map_err(|_| "امضای manifest پشتیبان داخلی معتبر نیست.".to_string())
    }

    fn snapshot_kind(&self) -> Result<SnapshotKind, String> {
        if self.version != SNAPSHOT_MANIFEST_VERSION {
            return Err("نسخه manifest پشتیبان داخلی پشتیبانی نمی‌شود.".to_string());
        }
        SnapshotKind::from_manifest_value(&self.kind)
            .ok_or_else(|| "نوع snapshot در manifest پشتیبان داخلی معتبر نیست.".to_string())
    }
}

fn snapshot_manifest_path(backup_path: &Path) -> PathBuf {
    let file_name = backup_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("backup.sqlite3");
    backup_path.with_file_name(format!("{file_name}{SNAPSHOT_MANIFEST_SUFFIX}"))
}

fn snapshot_key_path(database_path: &Path) -> Result<PathBuf, String> {
    let parent = database_path
        .parent()
        .ok_or_else(|| "مسیر پوشه داده برای کلید snapshot پیدا نشد.".to_string())?;
    Ok(parent.join(SNAPSHOT_KEY_FILE))
}

fn load_snapshot_key(database_path: &Path) -> Result<Option<Vec<u8>>, String> {
    let path = snapshot_key_path(database_path)?;
    match std::fs::read(&path) {
        Ok(bytes) if bytes.len() == 32 => Ok(Some(bytes)),
        Ok(_) => Err(format!(
            "فایل کلید احراز اصالت snapshot در «{}» معتبر نیست.",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "خواندن کلید احراز اصالت snapshot ناموفق بود: {error}"
        )),
    }
}

fn load_or_create_snapshot_key(database_path: &Path) -> Result<Vec<u8>, String> {
    if let Some(key) = load_snapshot_key(database_path)? {
        return Ok(key);
    }
    let path = snapshot_key_path(database_path)?;
    let key = from_hex(&random_hex(32)?)?;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            file.write_all(&key)
                .map_err(|error| format!("ذخیره کلید احراز اصالت snapshot ناموفق بود: {error}"))?;
            Ok(key)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            load_snapshot_key(database_path)?.ok_or_else(|| {
                "کلید احراز اصالت snapshot هم‌زمان ایجاد شد اما خوانده نشد.".to_string()
            })
        }
        Err(error) => Err(format!(
            "ساخت کلید احراز اصالت snapshot ناموفق بود: {error}"
        )),
    }
}

fn authenticated_snapshot_kind(
    database_path: &Path,
    source_path: &Path,
) -> Result<SnapshotKind, String> {
    let manifest_path = snapshot_manifest_path(source_path);
    if !manifest_path.exists() {
        // The marker inside the file is untrusted metadata. Without a signed
        // sidecar manifest, a user-selected file always takes the strict path.
        return Ok(SnapshotKind::Regular);
    }
    let key = load_snapshot_key(database_path)?.ok_or_else(|| {
        "manifest پشتیبان داخلی یافت شد اما کلید احراز اصالت آن در این دستگاه موجود نیست؛ بازیابی امن ممکن نیست.".to_string()
    })?;
    let manifest_bytes = std::fs::read(&manifest_path)
        .map_err(|error| format!("خواندن manifest پشتیبان داخلی ناموفق بود: {error}"))?;
    let manifest: SnapshotManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("manifest پشتیبان داخلی معتبر نیست: {error}"))?;
    manifest.verify(&key)?;
    let kind = manifest.snapshot_kind()?;
    let actual_hash = sha256_file(source_path)?;
    if actual_hash != manifest.file_sha256 {
        return Err(
            "فایل پشتیبان پس از ساخت manifest تغییر کرده است و قابل بازیابی نیست.".to_string(),
        );
    }
    Ok(kind)
}

impl Database {
    pub fn backup_to(&self, destination: &Path) -> Result<(), String> {
        self.backup_to_internal(destination, SnapshotKind::Regular)
    }

    /// Creates a reversible pre-delete snapshot. Unlike a user-selected backup,
    /// this must preserve legacy voucher defects so deleting another range is
    /// not blocked by an unrelated historical document. SQLite integrity,
    /// schema, account hierarchy, line-level constraints and foreign keys are
    /// still validated. A marker lets restore apply this same narrow exception.
    pub(crate) fn backup_to_for_bulk_delete(&self, destination: &Path) -> Result<(), String> {
        self.backup_to_internal(destination, SnapshotKind::BulkDelete)
    }

    /// Creates the mandatory backup taken before resolving a legacy level-4
    /// account. The source database intentionally contains a level-4 account
    /// whose hierarchy is still inconsistent, so this snapshot must accept it.
    pub(crate) fn backup_to_for_legacy_migration(&self, destination: &Path) -> Result<(), String> {
        self.backup_to_internal(destination, SnapshotKind::LegacyLevel4Migration)
    }

    fn backup_to_internal(&self, destination: &Path, kind: SnapshotKind) -> Result<(), String> {
        let current_path = PathBuf::from(self.path_string());
        if same_path(&current_path, destination) {
            return Err("مسیر پشتیبان نمی‌تواند همان مسیر پایگاه دادهٔ فعال باشد.".to_string());
        }

        self.with_conn(|source| {
            let temporary = unique_sibling_path(destination, "backup-tmp");
            let _ = std::fs::remove_file(&temporary);
            let mut target = Connection::open(&temporary).map_err(db_error)?;
            let validation = (|| -> Result<Option<PathBuf>, String> {
                {
                    let backup = Backup::new(source, &mut target).map_err(db_error)?;
                    backup
                        .run_to_completion(128, Duration::from_millis(10), None)
                        .map_err(db_error)?;
                }

                validate_backup_connection_with_options(&target, kind)?;
                if let Some(marker) = kind.marker_value() {
                    target
                        .execute(
                            "INSERT INTO app_meta(key, value) VALUES (?1, ?2)
                             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                            params![BULK_DELETE_SNAPSHOT_KEY, marker],
                        )
                        .map_err(db_error)?;
                }
                if kind == SnapshotKind::Regular {
                    return Ok(None);
                }
                let key = load_or_create_snapshot_key(&current_path)?;
                let mut manifest = SnapshotManifest::new(kind, sha256_file(&temporary)?)?;
                manifest.sign(&key)?;
                let manifest_temporary = unique_sibling_path(destination, "manifest-tmp");
                let _ = std::fs::remove_file(&manifest_temporary);
                std::fs::write(
                    &manifest_temporary,
                    serde_json::to_vec_pretty(&manifest)
                        .map_err(|error| format!("ساخت manifest پشتیبان ناموفق بود: {error}"))?,
                )
                .map_err(|error| format!("نوشتن manifest پشتیبان ناموفق بود: {error}"))?;
                Ok(Some(manifest_temporary))
            })();
            drop(target);
            let manifest_temporary = match validation {
                Ok(value) => value,
                Err(error) => {
                    let _ = std::fs::remove_file(&temporary);
                    return Err(error);
                }
            };
            replace_file(&temporary, destination)?;
            if let Some(manifest_temporary) = manifest_temporary {
                let manifest_destination = snapshot_manifest_path(destination);
                if let Err(error) = replace_file(&manifest_temporary, &manifest_destination) {
                    let _ = std::fs::remove_file(destination);
                    return Err(error);
                }
            }
            if kind != SnapshotKind::Regular {
                if let Some(directory) = destination.parent() {
                    // Retention is best-effort: a successful snapshot must not
                    // fail because Windows kept an old file locked.
                    let _ = prune_snapshot_files(directory);
                }
            }
            Ok(())
        })
    }

    pub fn restore_from(&self, source_path: &Path) -> Result<PathBuf, String> {
        self.restore_from_internal(source_path, source_path)
    }

    /// Creates a portable, password-encrypted backup. The plaintext SQLite copy
    /// is written to a short-lived temporary file in the application data
    /// folder and removed on every path, including failures.
    pub fn backup_encrypted_to(&self, destination: &Path, password: &str) -> Result<(), String> {
        super::portable::validate_portable_password(password)?;
        let current_path = PathBuf::from(self.path_string());
        let plaintext =
            PlaintextTempFile::new(unique_sibling_path(&current_path, "portable-backup-tmp"));
        self.backup_to(plaintext.path()).and_then(|()| {
            super::portable::encrypt_backup_file(
                plaintext.path(),
                destination,
                password,
                CURRENT_SCHEMA_VERSION,
            )
        })
    }

    /// Decrypts a portable backup into a short-lived file and then runs the
    /// normal restore path, which validates structure and keeps a safety copy
    /// before replacing the active database.
    pub fn restore_encrypted_from(
        &self,
        source_path: &Path,
        password: &str,
    ) -> Result<PathBuf, String> {
        let current_path = PathBuf::from(self.path_string());
        let plaintext =
            PlaintextTempFile::new(unique_sibling_path(&current_path, "portable-restore-tmp"));
        super::portable::decrypt_backup_file(source_path, plaintext.path(), password)
            .and_then(|()| self.restore_from_internal(plaintext.path(), source_path))
    }

    fn restore_from_internal(
        &self,
        source_path: &Path,
        audit_source: &Path,
    ) -> Result<PathBuf, String> {
        let current_path = PathBuf::from(self.path_string());
        if same_path(&current_path, source_path) {
            return Err("فایل انتخاب‌شده پایگاه دادهٔ فعال است، نه یک پشتیبان.".to_string());
        }

        let source = Connection::open_with_flags(source_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| format!("باز کردن فایل پشتیبان ناموفق بود: {error}"))?;
        let snapshot_kind = authenticated_snapshot_kind(&current_path, source_path)?;
        validate_backup_connection_with_options(&source, snapshot_kind)?;
        let safety_path = unique_sibling_path(&current_path, "before-restore");

        self.with_conn(|target| {
            let mut safety = Connection::open(&safety_path).map_err(db_error)?;
            {
                let backup = Backup::new(target, &mut safety).map_err(db_error)?;
                backup
                    .run_to_completion(128, Duration::from_millis(10), None)
                    .map_err(db_error)?;
            }
            drop(safety);

            let restore_result = {
                let backup = Backup::new(&source, target).map_err(db_error)?;
                backup
                    .run_to_completion(128, Duration::from_millis(10), None)
                    .map_err(db_error)
            }
            .and_then(|()| {
                // Upgrade a supported v4 backup inside the same restore safety
                // envelope. A v5 migration failure (for example, legacy level-4
                // accounts) rolls the whole restore back to the active database.
                apply_schema_migrations(target)?;
                validate_backup_connection_with_options(target, snapshot_kind)
            })
            .and_then(|()| reset_schema_triggers(target))
            .and_then(|()| {
                // The marker is internal metadata only. Always remove it from
                // the restored live database so a copied file cannot carry a
                // privileged marker forward.
                target
                    .execute(
                        "DELETE FROM app_meta WHERE key = ?1",
                        params![BULK_DELETE_SNAPSHOT_KEY],
                    )
                    .map_err(db_error)?;
                audit::log(
                    target,
                    "restore",
                    "database",
                    None,
                    None,
                    None,
                    &format!("بازیابی پایگاه داده از «{}».", audit_source.display()),
                )?;
                Ok(())
            });
            if let Err(error) = restore_result {
                let rollback_result = (|| -> Result<(), String> {
                    let safety = Connection::open(&safety_path).map_err(db_error)?;
                    {
                        let rollback = Backup::new(&safety, target).map_err(db_error)?;
                        rollback
                            .run_to_completion(128, Duration::from_millis(10), None)
                            .map_err(db_error)?;
                    }
                    Ok(())
                })();
                if let Err(rollback_error) = rollback_result {
                    let reason = format!(
                        "بازیابی پشتیبان و بازگردانی خودکار نسخهٔ ایمنی هر دو ناموفق بودند. برای جلوگیری از خواندن یا نوشتن روی پایگاه دادهٔ نامطمئن، برنامه تا راه‌اندازی مجدد متوقف شد. نسخهٔ ایمنی در «{}» نگهداری می‌شود؛ آن را دستی بازیابی کنید. خطای بازیابی: {error} | خطای بازگردانی: {rollback_error}",
                        safety_path.display()
                    );
                    *self
                        .failed
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(reason.clone());
                    return Err(reason);
                }
                return Err(format!(
                    "بازیابی انجام نشد و پایگاه قبلی بازگردانده شد: {error}"
                ));
            }
            Ok(safety_path.clone())
        })
    }
}

const SCHEMA_TRIGGERS: [&str; 11] = [
    "vouchers_positive_number_insert",
    "vouchers_positive_number_update",
    "voucher_lines_valid_insert",
    "voucher_lines_valid_update",
    "voucher_lines_account_pair_insert",
    "voucher_lines_account_pair_update",
    "vouchers_permanent_update",
    "vouchers_permanent_delete",
    "voucher_lines_permanent_insert",
    "voucher_lines_permanent_update",
    "voucher_lines_permanent_delete",
];

/// `SCHEMA` uses `CREATE TRIGGER IF NOT EXISTS`, so a restored database could
/// keep a tampered trigger that happens to use an allowed name. Drop the
/// known triggers first and recreate them from the application's own schema.
fn reset_schema_triggers(conn: &Connection) -> Result<(), String> {
    for trigger in SCHEMA_TRIGGERS {
        conn.execute(&format!("DROP TRIGGER IF EXISTS \"{trigger}\""), [])
            .map_err(db_error)?;
    }
    apply_base_schema(conn)?;
    ensure_voucher_account_pair_guards(conn)?;
    ensure_permanent_voucher_guards(conn)
}

#[cfg(test)]
pub(crate) fn validate_backup_connection(conn: &Connection) -> Result<(), String> {
    validate_backup_connection_with_options(conn, SnapshotKind::Regular)
}

fn validate_backup_connection_with_options(
    conn: &Connection,
    kind: SnapshotKind,
) -> Result<(), String> {
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(db_error)?;
    if integrity != "ok" {
        return Err(format!("سلامت فایل SQLite تأیید نشد: {integrity}"));
    }
    let unexpected_trigger: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master
                            WHERE type = 'trigger'
                                AND name NOT IN (
                                        'vouchers_positive_number_insert',
                                        'vouchers_positive_number_update',
                                        'voucher_lines_valid_insert',
                                        'voucher_lines_valid_update',
                                        'voucher_lines_account_pair_insert',
                                        'voucher_lines_account_pair_update',
                                        'vouchers_permanent_update',
                                        'vouchers_permanent_delete',
                                        'voucher_lines_permanent_insert',
                                        'voucher_lines_permanent_update',
                                        'voucher_lines_permanent_delete'
                                )
                            LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_error)?;
    if let Some(name) = unexpected_trigger {
        return Err(format!(
            "فایل پشتیبان دارای trigger ناشناخته «{name}» است و قابل بازیابی نیست."
        ));
    }
    let version: String = conn
        .query_row(
            "SELECT value FROM app_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    let version_number = version
        .parse::<i64>()
        .map_err(|_| "نسخه ساختار فایل پشتیبان معتبر نیست.".to_string())?;
    // Version 4 backups are accepted as migration inputs. They are upgraded
    // only after copying into the active DB, where restore's safety snapshot
    // can roll everything back if v5's level-4/reference preflight fails.
    if !(4..=CURRENT_SCHEMA_VERSION).contains(&version_number) {
        return Err("نسخه ساختار فایل پشتیبان با این نسخه برنامه سازگار نیست.".to_string());
    }
    for table in [
        "app_meta",
        "settings",
        "account_groups",
        "accounts",
        "vouchers",
        "voucher_lines",
    ] {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                params![table],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if !exists {
            return Err(format!(
                "فایل انتخاب‌شده پشتیبان معتبر نیست؛ جدول {table} وجود ندارد."
            ));
        }
    }

    const REQUIRED_COLUMNS: [(&str, &[&str]); 6] = [
        ("app_meta", &["key", "value"]),
        ("settings", &["key", "value"]),
        (
            "account_groups",
            &[
                "id",
                "code",
                "name",
                "type",
                "default_section",
                "created_at",
                "updated_at",
            ],
        ),
        (
            "accounts",
            &[
                "id",
                "code",
                "name",
                "level",
                "group_code",
                "nature",
                "parent_code",
                "parent_name",
                "is_postable",
                "is_active",
                "created_at",
                "updated_at",
            ],
        ),
        (
            "vouchers",
            &[
                "id",
                "number",
                "date",
                "type",
                "description",
                "created_at",
                "updated_at",
            ],
        ),
        (
            "voucher_lines",
            &[
                "id",
                "voucher_id",
                "row_no",
                "account_code",
                "description",
                "debit",
                "credit",
            ],
        ),
    ];
    for (table, required) in REQUIRED_COLUMNS {
        let columns = backup_table_columns(conn, table)?;
        for column in required {
            if !columns.iter().any(|name| name == column) {
                return Err(format!(
                    "فایل انتخاب‌شده پشتیبان معتبر نیست؛ ستون {table}.{column} وجود ندارد."
                ));
            }
        }
        if table == "voucher_lines" && version_number >= 5 {
            for column in ["account_id", "detail1_code"] {
                if !columns.iter().any(|name| name == column) {
                    return Err(format!(
                        "فایل پشتیبان نسخه ۵ معتبر نیست؛ ستون voucher_lines.{column} وجود ندارد."
                    ));
                }
            }
        }
    }

    {
        let mut statement = conn.prepare("PRAGMA foreign_key_check").map_err(db_error)?;
        let mut rows = statement.query([]).map_err(db_error)?;
        if let Some(row) = rows.next().map_err(db_error)? {
            let table: String = row.get(0).map_err(db_error)?;
            let row_id: i64 = row.get(1).map_err(db_error)?;
            return Err(format!(
                "فایل پشتیبان رابطه کلید خارجی نامعتبر دارد (جدول {table}، ردیف {row_id})."
            ));
        }
    }

    if let Some(code) = backup_first_text(
        conn,
        "SELECT code FROM account_groups
          WHERE code = '' OR length(code) > 50 OR code GLOB '*[^0-9]*' OR trim(code, '0') = ''
          LIMIT 1",
    )? {
        return Err(format!("کد گروه «{code}» در فایل پشتیبان معتبر نیست."));
    }
    if let Some(code) = backup_first_text(
        conn,
        "SELECT code FROM account_groups
          WHERE type NOT IN ('دارایی', 'بدهی', 'سرمایه', 'درآمد', 'هزینه', 'سایر')
          LIMIT 1",
    )? {
        return Err(format!("نوع گروه «{code}» در فایل پشتیبان معتبر نیست."));
    }
    if let Some(name) = backup_first_text(
        conn,
        "SELECT name FROM account_groups
          WHERE trim(name) = '' OR length(name) > 255
          LIMIT 1",
    )? {
        return Err(format!(
            "نام گروه «{name}» در فایل پشتیبان خالی یا بیش از حد بلند است."
        ));
    }

    if let Some(code) = backup_first_text(
        conn,
        "SELECT code FROM accounts
          WHERE code = '' OR length(code) > 50 OR code GLOB '*[^0-9]*' OR trim(code, '0') = ''
          LIMIT 1",
    )? {
        return Err(format!("کد حساب «{code}» در فایل پشتیبان معتبر نیست."));
    }
    if let Some(name) = backup_first_text(
        conn,
        "SELECT name FROM accounts
          WHERE trim(name) = '' OR length(name) > 255
          LIMIT 1",
    )? {
        return Err(format!(
            "نام حساب «{name}» در فایل پشتیبان خالی یا بیش از حد بلند است."
        ));
    }
    if let Some(code) = backup_first_text(
        conn,
        &format!(
            "SELECT code FROM accounts
          WHERE level NOT BETWEEN 1 AND {}
             OR is_postable NOT IN (0, 1)
             OR nature NOT IN ('debtor', 'creditor', 'dual')
          LIMIT 1",
            if !(5..9).contains(&version_number) {
                4
            } else {
                3
            }
        ),
    )? {
        return Err(format!(
            "سطح، ماهیت یا وضعیت ثبت حساب «{code}» در فایل پشتیبان معتبر نیست."
        ));
    }
    if let Some(code) = backup_first_text(
        conn,
        "SELECT code FROM accounts
          WHERE (level = 1 AND parent_code IS NOT NULL AND parent_code <> '')
             OR (level > 1 AND (parent_code IS NULL OR parent_code = ''))
          LIMIT 1",
    )? {
        return Err(format!(
            "حساب «{code}» در فایل پشتیبان حساب بالاتر معتبری ندارد."
        ));
    }
    let hierarchy_check = if kind.allow_legacy_level4_hierarchy() {
        // A legacy level-4 account keeps its level-3 parent but may still carry
        // a different group code until the operator maps it explicitly.
        "SELECT a.code
           FROM accounts a
           JOIN accounts p ON p.code = a.parent_code
          WHERE (a.level <> p.level + 1 OR a.group_code <> p.group_code)
            AND NOT (a.level = 4 AND p.level = 3)
          LIMIT 1"
    } else {
        "SELECT a.code
           FROM accounts a
           JOIN accounts p ON p.code = a.parent_code
          WHERE a.level <> p.level + 1
             OR a.group_code <> p.group_code
          LIMIT 1"
    };
    if let Some(code) = backup_first_text(conn, hierarchy_check)? {
        return Err(format!(
            "ساختار حساب «{code}» با حساب بالاتر آن در فایل پشتیبان سازگار نیست."
        ));
    }

    if let Some(number) = backup_first_text(
        conn,
        &format!(
            "SELECT CAST(number AS TEXT) FROM vouchers
              WHERE number <= 0 OR number > {MAX_VOUCHER_NUMBER}
              LIMIT 1"
        ),
    )? {
        return Err(format!(
            "شماره سند «{number}» در فایل پشتیبان خارج از محدوده مجاز است."
        ));
    }
    if let Some(number) = backup_first_text(
        conn,
        "SELECT CAST(number AS TEXT) FROM vouchers
          GROUP BY number HAVING COUNT(*) > 1
          LIMIT 1",
    )? {
        return Err(format!("شماره سند «{number}» در فایل پشتیبان تکراری است."));
    }

    {
        let mut statement = conn
            .prepare("SELECT number, date, type, description FROM vouchers ORDER BY id")
            .map_err(db_error)?;
        let mut rows = statement.query([]).map_err(db_error)?;
        while let Some(row) = rows.next().map_err(db_error)? {
            let number: i64 = row.get(0).map_err(db_error)?;
            let date: String = row.get(1).map_err(db_error)?;
            let voucher_type: String = row.get(2).map_err(db_error)?;
            let description: String = row.get(3).map_err(db_error)?;
            let shown_number = to_fa_digits(number.to_string());
            if crate::vouchers::validate_voucher_date(&date).is_err() {
                return Err(format!(
                    "تاریخ سند شماره {shown_number} در فایل پشتیبان معتبر نیست."
                ));
            }
            if !crate::vouchers::VOUCHER_TYPES.contains(&voucher_type.as_str()) {
                return Err(format!(
                    "نوع سند شماره {shown_number} در فایل پشتیبان معتبر نیست."
                ));
            }
            if description.chars().count() > crate::vouchers::MAX_DESCRIPTION {
                return Err(format!(
                    "شرح سند شماره {shown_number} در فایل پشتیبان بیش از حد مجاز است."
                ));
            }
        }
    }

    if let Some(line) = backup_first_text(
        conn,
        "SELECT CAST(id AS TEXT) FROM voucher_lines
          WHERE account_code = '' OR length(account_code) > 50
             OR account_code GLOB '*[^0-9]*'
          LIMIT 1",
    )? {
        return Err(format!("کد حساب ردیف {line} در فایل پشتیبان معتبر نیست."));
    }
    if let Some(line) = backup_first_text(
        conn,
        &format!(
            "SELECT CAST(id AS TEXT) FROM voucher_lines
              WHERE row_no <= 0
                 OR debit < 0 OR credit < 0
                 OR debit > {} OR credit > {}
                 OR (debit > 0 AND credit > 0)
                 OR (debit = 0 AND credit = 0)
                 OR length(description) > {}
              LIMIT 1",
            crate::vouchers::MAX_AMOUNT,
            crate::vouchers::MAX_AMOUNT,
            crate::vouchers::MAX_LINE_DESCRIPTION
        ),
    )? {
        return Err(format!(
            "مبلغ، شماره ردیف یا شرح ردیف {line} در فایل پشتیبان معتبر نیست."
        ));
    }
    let account_reference_check = if version_number >= 9 {
        "SELECT CAST(l.id AS TEXT)
           FROM voucher_lines l
           LEFT JOIN accounts a ON a.id = l.account_id AND a.code = l.account_code
           LEFT JOIN accounts d ON d.code = l.detail1_code
          WHERE NOT (
                  l.legacy_level4_pending = 1
                  AND l.account_id IS NULL
                  AND EXISTS (
                      SELECT 1 FROM accounts legacy
                       WHERE legacy.code = l.account_code AND legacy.level = 4
                  )
                )
            AND (a.id IS NULL OR a.level NOT IN (1,2) OR a.is_postable = 0
              OR (l.detail1_code IS NOT NULL AND l.detail1_code <> '' AND
                  (d.id IS NULL OR d.level <> 3 OR a.level <> 2 OR d.parent_code <> a.code OR d.group_code <> a.group_code)))
          LIMIT 1"
    } else if version_number >= 5 {
        "SELECT CAST(l.id AS TEXT)
           FROM voucher_lines l
           LEFT JOIN accounts a ON a.id = l.account_id AND a.code = l.account_code
           LEFT JOIN accounts d ON d.code = l.detail1_code
          WHERE a.id IS NULL OR a.level NOT IN (1,2) OR a.is_postable = 0
             OR (l.detail1_code IS NOT NULL AND l.detail1_code <> '' AND
                 (d.id IS NULL OR d.level <> 3 OR a.level <> 2 OR d.parent_code <> a.code OR d.group_code <> a.group_code))
          LIMIT 1"
    } else {
        "SELECT CAST(l.id AS TEXT)
           FROM voucher_lines l
           LEFT JOIN accounts a ON a.code = l.account_code
          WHERE a.id IS NULL OR a.level NOT BETWEEN 1 AND 4
          LIMIT 1"
    };
    if let Some(line) = backup_first_text(conn, account_reference_check)? {
        return Err(format!(
            "حساب ردیف {line} در فایل پشتیبان وجود ندارد یا ساختار ارجاع آن معتبر نیست."
        ));
    }
    // A marked pre-delete snapshot is a reversible archive of the exact live
    // database state. It may contain an old voucher with an invalid aggregate
    // row count/balance; all SQLite, schema, foreign-key and line-level checks
    // above still run. Regular backups remain fully strict.
    if !kind.preserve_legacy_voucher_structure() {
        if let Some(number) = backup_first_text(
            conn,
            &format!(
                "SELECT CAST(v.number AS TEXT)
                   FROM vouchers v
                   LEFT JOIN voucher_lines l ON l.voucher_id = v.id
                  GROUP BY v.id
                 HAVING COUNT(l.id) < 2
                     OR COUNT(l.id) > {}
                     OR COUNT(DISTINCT l.row_no) <> COUNT(l.id)
                     OR COALESCE(SUM(l.debit), 0) = 0
                     OR COALESCE(SUM(l.debit), 0) <> COALESCE(SUM(l.credit), 0)
                  LIMIT 1",
                crate::vouchers::MAX_LINES
            ),
        )? {
            return Err(format!(
                "ساختار ردیف‌ها یا تراز سند شماره {number} در فایل پشتیبان معتبر نیست."
            ));
        }
    }

    Ok(())
}

fn backup_table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, String> {
    let mut statement = conn
        .prepare(&format!("PRAGMA table_info(\"{table}\")"))
        .map_err(db_error)?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(db_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db_error)?;
    Ok(columns)
}

fn backup_first_text(conn: &Connection, sql: &str) -> Result<Option<String>, String> {
    conn.query_row(sql, [], |row| row.get::<_, Option<String>>(0))
        .optional()
        .map_err(db_error)
        .map(Option::flatten)
}

pub(crate) fn unique_sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("database");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    path.with_file_name(format!(
        "{file_name}.{suffix}-{}-{stamp}",
        std::process::id()
    ))
}

/// Creates a file that only the current user can read. Portable backup code
/// writes plaintext database copies to such files before encrypting them, so a
/// world-readable temporary file would defeat the point of the encryption.
pub(crate) fn create_private_file(path: &Path) -> Result<std::fs::File, String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|error| format!("ساخت فایل موقت خصوصی ناموفق بود: {error}"))
}

/// Removes a temporary file on every exit path, including early returns and
/// panics. Plaintext database copies must never outlive the operation that
/// needed them.
pub(crate) struct PlaintextTempFile {
    path: PathBuf,
}

impl PlaintextTempFile {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PlaintextTempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Temporary file suffixes that may contain unencrypted database contents.
const PLAINTEXT_TEMP_MARKERS: [&str; 4] = [
    ".portable-backup-tmp-",
    ".portable-restore-tmp-",
    ".portable-tmp-",
    ".portable-plain-tmp-",
];

/// A crashed process can leave a plaintext portable-backup temporary file
/// behind because `Drop` does not run. Delete such leftovers at startup, but
/// only after an hour so a second instance's in-flight temp is never touched.
const STALE_PLAINTEXT_TEMP_AGE: Duration = Duration::from_secs(60 * 60);

pub(crate) fn cleanup_stale_plaintext_temps(database_path: &Path) -> Result<usize, String> {
    let directory = match database_path.parent() {
        Some(directory) => directory,
        None => return Ok(0),
    };
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(format!(
                "خواندن پوشه داده برای پاک‌سازی فایل‌های موقت ناموفق بود: {error}"
            ))
        }
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries {
        let entry = entry.map_err(|error| format!("خواندن فایل موقت ناموفق بود: {error}"))?;
        if !entry
            .file_type()
            .map_err(|error| format!("تعیین نوع فایل موقت ناموفق بود: {error}"))?
            .is_file()
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !PLAINTEXT_TEMP_MARKERS
            .iter()
            .any(|marker| name.contains(marker))
        {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH);
        if now.duration_since(modified).unwrap_or_default() < STALE_PLAINTEXT_TEMP_AGE {
            continue;
        }
        match std::fs::remove_file(entry.path()) {
            Ok(()) => removed += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "حذف فایل موقت حاوی دادهٔ رمزنگاری‌نشده ناموفق بود: {error}"
                ))
            }
        }
    }
    Ok(removed)
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

pub(crate) fn replace_file(temporary: &Path, destination: &Path) -> Result<(), String> {
    let previous = unique_sibling_path(destination, "previous");
    if destination.exists() {
        std::fs::rename(destination, &previous)
            .map_err(|error| format!("آماده‌سازی فایل قبلی ناموفق بود: {error}"))?;
    }
    if let Err(error) = std::fs::rename(temporary, destination) {
        if previous.exists() {
            let _ = std::fs::rename(&previous, destination);
        }
        let _ = std::fs::remove_file(temporary);
        return Err(format!("جایگزینی فایل پشتیبان ناموفق بود: {error}"));
    }
    let _ = std::fs::remove_file(previous);
    Ok(())
}

/// Keeps the newest regular backups so the backups folder cannot grow without
/// bound. Older files are removed only when they match the backup naming
/// pattern used by this application.
pub(crate) fn prune_backup_files(directory: &Path) -> Result<usize, String> {
    prune_matching_files(directory, BACKUP_RETENTION, |name| {
        name.starts_with("hesabdari-backup-") && name.ends_with(".sqlite3")
    })
}

/// Removes older `.before-restore-*` safety copies next to the active
/// database. The newest copies remain available for manual recovery.
pub(crate) fn prune_restore_safety_files(database_path: &Path) -> Result<usize, String> {
    let directory = database_path
        .parent()
        .ok_or_else(|| "مسیر پوشه پایگاه داده پیدا نشد.".to_string())?;
    prune_matching_files(directory, RESTORE_SAFETY_RETENTION, |name| {
        name.contains(".before-restore-")
    })
}

/// Keeps a bounded number of pre-delete and legacy-migration snapshots. These
/// snapshots are written by the application itself and have a signed sidecar
/// manifest; the manifest is removed together with its SQLite file so a
/// retained file never loses its proof of origin. Regular backups are handled
/// separately by [`prune_backup_files`].
pub(crate) fn prune_snapshot_files(directory: &Path) -> Result<usize, String> {
    let mut removed = prune_snapshot_group(directory, "pre_delete_", PRE_DELETE_RETENTION)?;
    removed += prune_snapshot_group(
        directory,
        "level4-migration-",
        LEGACY_MIGRATION_SNAPSHOT_RETENTION,
    )?;
    Ok(removed)
}

fn prune_snapshot_group(directory: &Path, prefix: &str, keep: usize) -> Result<usize, String> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(format!("خواندن پوشه snapshot ناموفق بود: {error}")),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("خواندن فایل snapshot ناموفق بود: {error}"))?;
        if !entry
            .file_type()
            .map_err(|error| format!("تعیین نوع فایل snapshot ناموفق بود: {error}"))?
            .is_file()
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(prefix) || !name.ends_with(".sqlite3") {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH);
        files.push((modified, name));
    }
    if files.len() <= keep {
        return Ok(0);
    }
    // Newest first. The name is a deterministic tie-breaker; both snapshot
    // naming schemes embed a high-resolution timestamp.
    files.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    let victims: Vec<_> = files.into_iter().skip(keep).collect();
    let total_bytes: u64 = victims
        .iter()
        .filter_map(|(_, name)| std::fs::metadata(directory.join(name)).ok())
        .map(|metadata| metadata.len())
        .sum();
    eprintln!(
        "[backup-retention] removing {} old '{prefix}*' snapshot(s), {} bytes",
        victims.len(),
        total_bytes
    );
    let mut removed = 0;
    for (_, name) in victims {
        let path = directory.join(&name);
        // The manifest is the proof of origin and is useless without its file;
        // delete the pair together. A missing manifest is fine (it may already
        // have been cleaned up).
        let manifest = directory.join(format!("{name}{SNAPSHOT_MANIFEST_SUFFIX}"));
        match std::fs::remove_file(&manifest) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "حذف manifest snapshot قدیمی «{}» ناموفق بود: {error}",
                    manifest.display()
                ));
            }
        }
        std::fs::remove_file(&path).map_err(|error| {
            format!(
                "حذف snapshot قدیمی «{}» ناموفق بود: {error}",
                path.display()
            )
        })?;
        removed += 1;
    }
    Ok(removed)
}

fn prune_matching_files<F>(directory: &Path, keep: usize, matches: F) -> Result<usize, String>
where
    F: Fn(&str) -> bool,
{
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(format!("خواندن پوشه پشتیبان ناموفق بود: {error}")),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("خواندن فایل پشتیبان ناموفق بود: {error}"))?;
        if !entry
            .file_type()
            .map_err(|error| format!("تعیین نوع فایل پشتیبان ناموفق بود: {error}"))?
            .is_file()
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !matches(&name) {
            continue;
        }
        // Both backup and safety names end in a nanosecond timestamp, so the
        // suffix orders them without depending on filesystem time precision.
        let stem = name.strip_suffix(".sqlite3").unwrap_or(&name);
        let stamp = stem
            .rsplit('-')
            .next()
            .and_then(|value| value.parse::<u128>().ok())
            .unwrap_or(0);
        files.push((stamp, name));
    }
    if files.len() <= keep {
        return Ok(0);
    }
    files.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    let mut removed = 0;
    for (_, name) in files.into_iter().skip(keep) {
        let path = directory.join(name);
        std::fs::remove_file(&path).map_err(|error| {
            format!("حذف پشتیبان قدیمی «{}» ناموفق بود: {error}", path.display())
        })?;
        removed += 1;
    }
    Ok(removed)
}

const BACKUP_RETENTION: usize = 30;
const RESTORE_SAFETY_RETENTION: usize = 5;
const PRE_DELETE_RETENTION: usize = 10;
const LEGACY_MIGRATION_SNAPSHOT_RETENTION: usize = 10;
