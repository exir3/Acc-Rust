//! Application settings stored in the `settings` table.

use rusqlite::{params, Connection, OptionalExtension};

use crate::db::audit;
use crate::db::common::{db_error, today_jalali};
use crate::db::model::{Database, Limits, Settings};
use crate::db::schema::DEFAULT_SETTINGS;
use crate::db::validation::validate_name;

impl Database {
    pub fn get_settings(&self) -> Result<Settings, String> {
        self.with_conn(|conn| {
            let company_name = get_setting(conn, "company_name")?
                .unwrap_or_else(|| DEFAULT_SETTINGS[0].1.to_string());
            let fiscal_year = get_setting(conn, "fiscal_year")?
                .unwrap_or_else(|| DEFAULT_SETTINGS[1].1.to_string());
            Ok(Settings {
                company_name,
                fiscal_year,
                today: today_jalali(),
                limits: Limits::current(),
            })
        })
    }

    pub fn save_settings(&self, company_name: &str, fiscal_year: &str) -> Result<Settings, String> {
        let company_name = validate_name(company_name, "نام شرکت")?;
        let fiscal_year = fiscal_year.trim();
        if fiscal_year.chars().count() > 40 {
            return Err("عنوان سال مالی نمی‌تواند بیشتر از ۴۰ کاراکتر باشد.".to_string());
        }
        if !fiscal_year.is_empty()
            && (!fiscal_year.chars().any(char::is_numeric)
                || !fiscal_year
                    .chars()
                    .all(|ch| ch.is_numeric() || matches!(ch, '/' | '-' | ' ' | '\u{200c}')))
        {
            return Err("عنوان سال مالی باید فقط شامل رقم و جداکننده‌های / یا - باشد.".to_string());
        }

        self.with_conn(|conn| {
            let tx = conn.transaction().map_err(db_error)?;
            let before = serde_json::json!({
                "companyName": get_setting(&tx, "company_name")?,
                "fiscalYear": get_setting(&tx, "fiscal_year")?,
            });
            set_setting(&tx, "company_name", &company_name)?;
            set_setting(&tx, "fiscal_year", fiscal_year)?;
            let after = serde_json::json!({
                "companyName": &company_name,
                "fiscalYear": fiscal_year,
            });
            audit::log(
                &tx,
                "update",
                "settings",
                None,
                Some(&before.to_string()),
                Some(&after.to_string()),
                "",
            )?;
            tx.commit().map_err(db_error)?;
            Ok(Settings {
                company_name,
                fiscal_year: fiscal_year.to_string(),
                today: today_jalali(),
                limits: Limits::current(),
            })
        })
    }
}

pub(crate) fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(db_error)
}

fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(db_error)?;
    Ok(())
}
