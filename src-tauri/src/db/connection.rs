//! Opening the database file and managing the process-wide connection.

use rusqlite::{params, Connection, OpenFlags};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

use crate::db::common::{db_error, DB_FILE_NAME, LEGACY_DB_FILE_NAME, LEGACY_IDENTIFIER};
use crate::db::model::Database;
use crate::db::schema::{
    apply_base_schema, apply_schema_migrations, seed_default_accounts, seed_default_groups,
    seed_default_settings,
};

impl Database {
    pub fn attach(&self, connection: Connection, path: PathBuf) {
        // A panic in an earlier database operation must not make the
        // application panic again while attaching/reopening its connection.
        *self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(connection);
        *self
            .path
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(path);
    }

    pub fn path_string(&self) -> String {
        self.path
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    /// Re-enables database access after a failed restore, before reopening it.
    pub fn clear_failed(&self) {
        *self
            .failed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    /// Keeps the database blocked when reopening after a failed restore fails.
    pub fn mark_failed(&self, reason: String) {
        *self
            .failed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(reason);
    }

    pub(crate) fn with_conn<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, String>,
    ) -> Result<T, String> {
        if let Some(reason) = self
            .failed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            return Err(reason.clone());
        }
        // The connection remains owned and usable after mutex poisoning; a
        // panicking caller will have dropped any in-flight SQLite transaction.
        let mut guard = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let conn = guard
            .as_mut()
            .ok_or_else(|| "پایگاه داده هنوز آماده نشده است.".to_string())?;
        f(conn)
    }
}

/// Test-only data directory override used by the real Tauri smoke test.
///
/// Only debug builds honor it, so a shipped release can never be redirected by
/// an environment variable. The smoke test must never read or write the real
/// user database.
fn data_dir_override() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let value = std::env::var_os("HESABDARI_SMOKE_DATA_DIR")?;
    if value.is_empty() {
        return None;
    }
    let path = PathBuf::from(value);
    path.is_absolute().then_some(path)
}

pub fn initialize(app: &AppHandle) -> Result<(Connection, PathBuf), String> {
    let executable_dir = std::env::current_exe()
        .map_err(|e| format!("مسیر فایل اجرایی برنامه در دسترس نیست: {e}"))?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "پوشه فایل اجرایی برنامه پیدا نشد.".to_string())?;
    let override_dir = data_dir_override();
    let data_dir = match &override_dir {
        Some(dir) => dir.clone(),
        None => app
            .path()
            .app_data_dir()
            .map_err(|error| format!("مسیر داده‌های برنامه در دسترس نیست: {error}"))?,
    };
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| format!("ساخت پوشه پایگاه داده ناموفق بود: {e}"))?;

    let path = data_dir.join(DB_FILE_NAME);
    // A crash can leave an unencrypted portable-backup temp behind. Remove
    // stale leftovers before the database is opened; failure here is only
    // logged because a leftover file must not stop the application from
    // starting.
    if let Err(error) = super::cleanup_stale_plaintext_temps(&path) {
        eprintln!("[hesabdari] stale plaintext temp cleanup failed: {error}");
    }
    if cfg!(debug_assertions) {
        // The smoke test only sees the child process; make the effective paths
        // visible before the first fallible database step.
        eprintln!("[hesabdari] data directory: {}", data_dir.display());
        eprintln!("[hesabdari] database path: {}", path.display());
        if let Some(value) = std::env::var_os("HESABDARI_SMOKE_DATA_DIR") {
            eprintln!(
                "[hesabdari] HESABDARI_SMOKE_DATA_DIR={}",
                PathBuf::from(value).display()
            );
        }
    }

    // Preserve databases created beside older executable builds while moving
    // the active database to the writable per-user application data directory.
    // The smoke-test override starts from an empty folder on purpose, so it
    // must not import anything from the developer machine.
    if override_dir.is_none() {
        migrate_legacy_database(&data_dir, &path, Some(&data_dir), Some(&executable_dir))?;
    }

    let mut connection =
        Connection::open(&path).map_err(|e| format!("باز کردن پایگاه داده ناموفق بود: {e}"))?;

    connection
        .execute_batch(
            // Read-mostly desktop workload: keep a larger page cache, sort in
            // memory instead of spilling to a temp file, and memory-map the
            // database so large reports do not copy every page through the
            // SQLite page cache first.
            //
            // `synchronous` intentionally stays at SQLite's FULL default.
            // NORMAL is faster for writes but can drop the last committed
            // transactions after a power loss; accounting records must not
            // take that trade-off.
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;
             PRAGMA cache_size = -32768;
             PRAGMA temp_store = MEMORY;
             PRAGMA mmap_size = 268435456;
             PRAGMA journal_size_limit = 67108864;",
        )
        .map_err(db_error)?;
    apply_base_schema(&connection)?;
    apply_schema_migrations(&mut connection)?;
    seed_default_groups(&mut connection).map_err(db_error)?;
    seed_default_accounts(&mut connection).map_err(db_error)?;
    seed_default_settings(&connection).map_err(db_error)?;

    Ok((connection, path))
}

/// Copies the previous application's database into `destination` once. The
/// old file may still have most of its data in a `-wal` sidecar, so copying
/// only the main file would silently lose it. `VACUUM INTO` writes a single
/// consistent file that includes everything the source connection can see.
pub(crate) fn migrate_legacy_database(
    data_dir: &Path,
    destination: &Path,
    legacy_data_dir: Option<&Path>,
    legacy_executable_dir: Option<&Path>,
) -> Result<(), String> {
    if destination.exists() {
        return Ok(());
    }

    let mut candidates = vec![data_dir.join(LEGACY_DB_FILE_NAME)];
    if let Some(legacy_data_dir) = legacy_data_dir {
        candidates.push(legacy_data_dir.join(DB_FILE_NAME));
        candidates.push(legacy_data_dir.join(LEGACY_DB_FILE_NAME));
        if let Some(parent) = legacy_data_dir.parent() {
            candidates.push(parent.join(LEGACY_IDENTIFIER).join(LEGACY_DB_FILE_NAME));
        }
    }
    if let Some(legacy_executable_dir) = legacy_executable_dir {
        candidates.push(legacy_executable_dir.join("database").join(DB_FILE_NAME));
        candidates.push(
            legacy_executable_dir
                .join("database")
                .join(LEGACY_DB_FILE_NAME),
        );
    }
    if let Some(parent) = data_dir.parent() {
        candidates.push(parent.join(LEGACY_IDENTIFIER).join(LEGACY_DB_FILE_NAME));
    }

    let mut existing: Vec<PathBuf> = candidates
        .into_iter()
        .filter(|path| path.exists())
        .collect();
    if existing.is_empty() {
        return Ok(());
    }
    // Prefer the most recently written database; a stale copy in the old
    // identifier folder must not win over newer data in the app data folder.
    existing.sort_by(|left, right| {
        let modified = |path: &Path| {
            std::fs::metadata(path)
                .and_then(|meta| meta.modified())
                .ok()
        };
        modified(right).cmp(&modified(left))
    });

    let temporary = destination.with_extension("sqlite3.migrating");
    let mut failures = Vec::new();
    for source in existing {
        let _ = std::fs::remove_file(&temporary);
        match copy_sqlite_database(&source, &temporary) {
            Ok(()) => {
                std::fs::rename(&temporary, destination).map_err(|error| {
                    let _ = std::fs::remove_file(&temporary);
                    format!(
                        "پایگاه داده قدیمی کپی شد اما جایگزینی امن فایل مقصد ناموفق بود: {error}"
                    )
                })?;
                return Ok(());
            }
            Err(error) => {
                let _ = std::fs::remove_file(&temporary);
                failures.push(format!("{}: {}", source.display(), error));
            }
        }
    }

    Err(format!(
        "انتقال پایگاه داده قدیمی ناموفق بود؛ برای جلوگیری از ساخت پایگاه داده خالی و پنهان‌شدن اطلاعات قبلی، اجرای برنامه متوقف شد. جزئیات: {}",
        failures.join(" | ")
    ))
}

fn copy_sqlite_database(source: &Path, destination: &Path) -> Result<(), String> {
    let connection = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "VACUUM INTO ?1",
            params![destination.to_string_lossy().into_owned()],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}
