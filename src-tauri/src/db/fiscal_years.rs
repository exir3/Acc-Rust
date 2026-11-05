//! Independent fiscal-year entities and the open-period guard used by
//! voucher writes and reports.

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::db::audit;
use crate::db::common::{db_error, to_fa_digits};
use crate::db::model::{Database, FiscalYear, FiscalYearList};
use crate::vouchers::{normalize_date, normalize_filter_date};

pub(crate) const FISCAL_YEAR_STATUSES: [&str; 3] = ["open", "closed", "final"];

fn validate_status(status: &str) -> Result<String, String> {
    let value = status.trim();
    if value.is_empty() {
        return Ok("open".to_string());
    }
    if FISCAL_YEAR_STATUSES.contains(&value) {
        Ok(value.to_string())
    } else {
        Err("وضعیت سال مالی معتبر نیست.".to_string())
    }
}

fn validate_title(title: &str) -> Result<String, String> {
    let value = title.trim();
    if value.is_empty() {
        return Err("عنوان سال مالی الزامی است.".to_string());
    }
    if value.chars().count() > 60 {
        return Err("عنوان سال مالی نمی‌تواند بیشتر از ۶۰ کاراکتر باشد.".to_string());
    }
    Ok(value.to_string())
}

/// A finalized year is a statement that the books are done. Reopening it is
/// allowed, but only with a recorded reason and an explicit confirmation.
fn reopen_reason_for(
    before: &FiscalYear,
    new_status: &str,
    reason: Option<&str>,
    confirm: bool,
) -> Result<String, String> {
    if before.status != "final" || new_status == "final" {
        return Ok(String::new());
    }
    let reason = reason.unwrap_or("").trim();
    if reason.is_empty() {
        return Err("سال مالی نهایی‌شده فقط با ثبت دلیل بازگشایی قابل تغییر است.".to_string());
    }
    if reason.chars().count() > 500 {
        return Err("دلیل بازگشایی نمی‌تواند بیشتر از ۵۰۰ کاراکتر باشد.".to_string());
    }
    if !confirm {
        return Err(
            "بازگشایی سال مالی نهایی‌شده نیاز به تأیید صریح دارد؛ عملیات را تأیید کنید.".to_string(),
        );
    }
    Ok(reason.to_string())
}

/// Finalizing a year while unsaved/temporary vouchers remain would hide work
/// that still has to be completed, so it is refused.
fn ensure_no_temporary_vouchers(conn: &Connection, year_id: i64) -> Result<(), String> {
    let temporary: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM vouchers v
              WHERE v.is_temporary = 1
                AND (
                    v.fiscal_year_id = ?1
                    OR (
                        v.fiscal_year_id IS NULL
                        AND v.date BETWEEN (SELECT start_date FROM fiscal_years WHERE id = ?1)
                                       AND (SELECT end_date FROM fiscal_years WHERE id = ?1)
                    )
                )",
            params![year_id],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    if temporary > 0 {
        return Err(format!(
            "سال مالی {} سند موقت دارد؛ ابتدا اسناد موقت را دائمی یا حذف کنید و سپس سال را نهایی کنید.",
            to_fa_digits(temporary)
        ));
    }
    Ok(())
}

fn fetch_year(conn: &Connection, id: i64) -> Result<FiscalYear, String> {
    conn.query_row(
        "SELECT f.id, f.title, f.start_date, f.end_date, f.status, f.created_at, f.updated_at,
                (SELECT COUNT(*) FROM vouchers v WHERE v.fiscal_year_id = f.id)
           FROM fiscal_years f
          WHERE f.id = ?1",
        params![id],
        row_to_year,
    )
    .optional()
    .map_err(db_error)?
    .ok_or_else(|| "سال مالی یافت نشد.".to_string())
}

fn row_to_year(row: &rusqlite::Row<'_>) -> rusqlite::Result<FiscalYear> {
    Ok(FiscalYear {
        id: row.get(0)?,
        title: row.get(1)?,
        start_date: row.get(2)?,
        end_date: row.get(3)?,
        status: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        voucher_count: row.get(7)?,
    })
}

fn list_years(conn: &Connection) -> Result<Vec<FiscalYear>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT f.id, f.title, f.start_date, f.end_date, f.status, f.created_at, f.updated_at,
                    (SELECT COUNT(*) FROM vouchers v WHERE v.fiscal_year_id = f.id)
               FROM fiscal_years f
              ORDER BY f.start_date, f.id",
        )
        .map_err(db_error)?;
    let rows = stmt.query_map([], row_to_year).map_err(db_error)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)
}

fn check_overlap(
    conn: &Connection,
    start: &str,
    end: &str,
    except_id: Option<i64>,
) -> Result<(), String> {
    let overlap = conn
        .query_row(
            "SELECT title, start_date, end_date
               FROM fiscal_years
              WHERE id <> ?3 AND start_date <= ?2 AND end_date >= ?1
              LIMIT 1",
            params![start, end, except_id.unwrap_or(0)],
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
    if let Some((title, existing_start, existing_end)) = overlap {
        return Err(format!(
            "بازه انتخاب‌شده با سال مالی «{title}» ({} تا {}) هم‌پوشانی دارد.",
            to_fa_digits(existing_start),
            to_fa_digits(existing_end)
        ));
    }
    Ok(())
}

/// Parses the four-digit year from a normalized Jalali date.
fn year_of(date: &str) -> Result<i32, String> {
    let year = date
        .get(0..4)
        .and_then(|value| value.parse::<i32>().ok())
        .ok_or_else(|| "سال مالی تاریخ سند قابل تشخیص نیست.".to_string())?;
    if !(1..=3177).contains(&year) {
        return Err("سال تاریخ سند خارج از بازه پشتیبانی تقویم شمسی است.".to_string());
    }
    Ok(year)
}

fn standard_range(year: i32) -> (String, String) {
    let start = format!("{year:04}/01/01");
    let end = format!(
        "{year:04}/12/{:02}",
        if jalali_calendar::is_leap_year(year) {
            30
        } else {
            29
        }
    );
    (start, end)
}

/// Returns the fiscal year whose range contains `date`.
pub(crate) fn fiscal_year_for_date(
    conn: &Connection,
    date: &str,
) -> Result<Option<FiscalYear>, String> {
    conn.query_row(
        "SELECT f.id, f.title, f.start_date, f.end_date, f.status, f.created_at, f.updated_at,
                (SELECT COUNT(*) FROM vouchers v WHERE v.fiscal_year_id = f.id)
           FROM fiscal_years f
          WHERE ?1 BETWEEN f.start_date AND f.end_date
          ORDER BY f.id
          LIMIT 1",
        params![date],
        row_to_year,
    )
    .optional()
    .map_err(db_error)
}

/// Creates an open standard year for `date` when no configured year covers it.
/// A partial overlap is refused instead of silently creating an overlapping
/// period, because that would make period filtering ambiguous.
fn ensure_standard_year(conn: &Connection, date: &str) -> Result<i64, String> {
    if let Some(year) = fiscal_year_for_date(conn, date)? {
        return Ok(year.id);
    }
    let year = year_of(date)?;
    let (start, end) = standard_range(year);
    if let Some((title, existing_start, existing_end)) = conn
        .query_row(
            "SELECT title, start_date, end_date
               FROM fiscal_years
              WHERE start_date <= ?2 AND end_date >= ?1
              LIMIT 1",
            params![start, end],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(db_error)?
    {
        return Err(format!(
            "تاریخ سند در هیچ سال مالی باز نیست؛ بازه سال «{title}» ({} تا {}) با سال استاندارد هم‌پوشانی دارد. بازه سال‌های مالی را اصلاح کنید.",
            to_fa_digits(existing_start),
            to_fa_digits(existing_end)
        ));
    }
    conn.execute(
        "INSERT INTO fiscal_years (title, start_date, end_date, status)
         VALUES (?1, ?2, ?3, 'open')",
        params![format!("سال مالی {}", to_fa_digits(year)), start, end],
    )
    .map_err(db_error)?;
    let id = conn.last_insert_rowid();
    audit::log(
        conn,
        "create",
        "fiscal_year",
        Some(id),
        None,
        None,
        "سال مالی به‌صورت خودکار برای تاریخ سند ایجاد شد.",
    )?;
    Ok(id)
}

/// Returns the open fiscal year for `date`, creating a standard one when
/// necessary. Saving or editing a voucher in a closed/final year is refused.
pub(crate) fn ensure_open_period(conn: &Connection, date: &str) -> Result<i64, String> {
    match fiscal_year_for_date(conn, date)? {
        Some(year) if year.status == "open" => Ok(year.id),
        Some(year) => Err(format!(
            "سال مالی «{}» {} است؛ ثبت یا تغییر سند در این دوره مجاز نیست.",
            year.title,
            if year.status == "final" {
                "نهایی‌شده"
            } else {
                "بسته"
            }
        )),
        None => ensure_standard_year(conn, date),
    }
}

impl Database {
    /// The fiscal-year title that best describes a report's date range.
    ///
    /// A range fully inside one configured year shows that year's own title;
    /// when the range spans years (or no year was set up), the caller keeps the
    /// configured default. This keeps printed reports consistent with the
    /// dates they actually cover instead of always showing the settings value.
    pub fn fiscal_year_label_for_range(
        &self,
        from: &str,
        to: &str,
    ) -> Result<Option<String>, String> {
        let from = normalize_filter_date(from, "از تاریخ")?;
        let to = normalize_filter_date(to, "تا تاریخ")?;
        if !from.is_empty() && !to.is_empty() && from > to {
            return Err("تاریخ شروع نمی‌تواند بعد از تاریخ پایان باشد.".to_string());
        }
        if from.is_empty() && to.is_empty() {
            return Ok(None);
        }
        let anchor_from = if from.is_empty() {
            to.clone()
        } else {
            from.clone()
        };
        let anchor_to = if to.is_empty() {
            from.clone()
        } else {
            to.clone()
        };

        self.with_conn(move |conn| {
            let covering = conn
                .query_row(
                    "SELECT title FROM fiscal_years
                      WHERE start_date <= ?1 AND end_date >= ?2
                      ORDER BY id
                      LIMIT 1",
                    params![anchor_from, anchor_to],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(db_error)?;
            if covering.is_some() {
                return Ok(covering);
            }
            // A range that spans more than one configured year has no single
            // fiscal-year title. Returning an empty label makes the header show
            // "—" instead of an unrelated value from the editable settings. A
            // range outside every configured year keeps the caller's fallback.
            if conn
                .query_row(
                    "SELECT 1 FROM fiscal_years
                      WHERE ?1 BETWEEN start_date AND end_date
                      LIMIT 1",
                    params![anchor_from],
                    |_| Ok(()),
                )
                .optional()
                .map_err(db_error)?
                .is_some()
            {
                return Ok(Some(String::new()));
            }
            Ok(None)
        })
    }

    pub fn list_fiscal_years(&self) -> Result<FiscalYearList, String> {
        let years = self.with_conn(|conn| list_years(conn))?;
        Ok(FiscalYearList {
            years,
            database_path: self.path_string(),
        })
    }

    pub fn create_fiscal_year(
        &self,
        title: &str,
        start_date: &str,
        end_date: &str,
        status: &str,
    ) -> Result<FiscalYear, String> {
        let title = validate_title(title)?;
        let start_date = normalize_date(start_date, "شروع سال مالی")?;
        let end_date = normalize_date(end_date, "پایان سال مالی")?;
        if start_date > end_date {
            return Err("شروع سال مالی نمی‌تواند بعد از پایان آن باشد.".to_string());
        }
        let status = validate_status(status)?;

        self.with_conn(move |conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            check_overlap(&tx, &start_date, &end_date, None)?;
            tx.execute(
                "INSERT INTO fiscal_years (title, start_date, end_date, status)
                 VALUES (?1, ?2, ?3, ?4)",
                params![title, start_date, end_date, status],
            )
            .map_err(db_error)?;
            let id = tx.last_insert_rowid();
            let year = fetch_year(&tx, id)?;
            audit::log(
                &tx,
                "create",
                "fiscal_year",
                Some(id),
                None,
                Some(&serde_json::to_string(&year).map_err(|error| error.to_string())?),
                "",
            )?;
            tx.commit().map_err(db_error)?;
            Ok(year)
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_fiscal_year(
        &self,
        id: i64,
        title: &str,
        start_date: &str,
        end_date: &str,
        status: &str,
        reason: Option<&str>,
        confirm: bool,
    ) -> Result<FiscalYear, String> {
        if id <= 0 {
            return Err("شناسه سال مالی نامعتبر است.".to_string());
        }
        let title = validate_title(title)?;
        let start_date = normalize_date(start_date, "شروع سال مالی")?;
        let end_date = normalize_date(end_date, "پایان سال مالی")?;
        if start_date > end_date {
            return Err("شروع سال مالی نمی‌تواند بعد از پایان آن باشد.".to_string());
        }
        let status = validate_status(status)?;

        self.with_conn(move |conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            let before = fetch_year(&tx, id)?;
            let reopen_reason = reopen_reason_for(&before, &status, reason, confirm)?;
            if status == "final" && before.status != "final" {
                ensure_no_temporary_vouchers(&tx, id)?;
            }
            if (before.start_date != start_date || before.end_date != end_date)
                && before.voucher_count > 0
            {
                return Err(
                    "این سال مالی سند ثبت‌شده دارد؛ بازه آن قابل تغییر نیست. فقط عنوان یا وضعیت را تغییر دهید."
                        .to_string(),
                );
            }
            check_overlap(&tx, &start_date, &end_date, Some(id))?;
            tx.execute(
                "UPDATE fiscal_years
                    SET title = ?1, start_date = ?2, end_date = ?3, status = ?4,
                        updated_at = datetime('now', 'localtime')
                  WHERE id = ?5",
                params![title, start_date, end_date, status, id],
            )
            .map_err(db_error)?;
            let after = fetch_year(&tx, id)?;
            let mut details = String::new();
            if before.status != after.status {
                details = format!("وضعیت از {} به {} تغییر کرد.", before.status, after.status);
            }
            if !reopen_reason.is_empty() {
                if !details.is_empty() {
                    details.push(' ');
                }
                details.push_str(&format!("دلیل بازگشایی: {reopen_reason}"));
            }
            audit::log(
                &tx,
                "update",
                "fiscal_year",
                Some(id),
                Some(&serde_json::to_string(&before).map_err(|error| error.to_string())?),
                Some(&serde_json::to_string(&after).map_err(|error| error.to_string())?),
                &details,
            )?;
            tx.commit().map_err(db_error)?;
            Ok(after)
        })
    }

    pub fn set_fiscal_year_status(
        &self,
        id: i64,
        status: &str,
        reason: Option<&str>,
        confirm: bool,
    ) -> Result<FiscalYear, String> {
        if id <= 0 {
            return Err("شناسه سال مالی نامعتبر است.".to_string());
        }
        let status = validate_status(status)?;

        self.with_conn(move |conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            let before = fetch_year(&tx, id)?;
            let reopen_reason = reopen_reason_for(&before, &status, reason, confirm)?;
            if status == "final" && before.status != "final" {
                ensure_no_temporary_vouchers(&tx, id)?;
            }
            tx.execute(
                "UPDATE fiscal_years
                    SET status = ?1, updated_at = datetime('now', 'localtime')
                  WHERE id = ?2",
                params![status, id],
            )
            .map_err(db_error)?;
            let after = fetch_year(&tx, id)?;
            let mut details = format!("وضعیت از {} به {} تغییر کرد.", before.status, after.status);
            if !reopen_reason.is_empty() {
                details.push_str(&format!(" دلیل بازگشایی: {reopen_reason}"));
            }
            audit::log(
                &tx,
                "status",
                "fiscal_year",
                Some(id),
                Some(&serde_json::to_string(&before).map_err(|error| error.to_string())?),
                Some(&serde_json::to_string(&after).map_err(|error| error.to_string())?),
                &details,
            )?;
            tx.commit().map_err(db_error)?;
            Ok(after)
        })
    }

    pub fn delete_fiscal_year(&self, id: i64) -> Result<(), String> {
        if id <= 0 {
            return Err("شناسه سال مالی نامعتبر است.".to_string());
        }

        self.with_conn(move |conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            let before = fetch_year(&tx, id)?;
            if before.voucher_count > 0 {
                return Err(format!(
                    "این سال مالی {} سند دارد و قابل حذف نیست؛ در صورت نیاز آن را ببندید.",
                    to_fa_digits(before.voucher_count)
                ));
            }
            tx.execute("DELETE FROM fiscal_years WHERE id = ?1", params![id])
                .map_err(db_error)?;
            audit::log(
                &tx,
                "delete",
                "fiscal_year",
                Some(id),
                Some(&serde_json::to_string(&before).map_err(|error| error.to_string())?),
                None,
                "",
            )?;
            tx.commit().map_err(db_error)?;
            Ok(())
        })
    }
}
