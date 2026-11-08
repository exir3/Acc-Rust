//! Print-preview commands.

use std::collections::HashMap;
use std::sync::Mutex;

use tauri::{AppHandle, State, WebviewWindow};

use crate::commands::state::{lock, report_meta, require_preview_window, require_window, AppState};
use crate::commands::windows::show_preview_window;
use crate::report;
use crate::vouchers::{IntegrityReport, ReportScope};

/// Official is the safe default for printed statements; temporary vouchers are
/// included only when the caller explicitly asks for a working report.
fn report_scope(scope: Option<String>) -> Result<ReportScope, String> {
    ReportScope::parse(scope.as_deref().unwrap_or("official"))
}

/// An official statement with integrity errors is not a printable final
/// document. Working/temporary previews stay available so the user can review
/// the same figures with the full health panel before fixing them.
fn ensure_printable(scope: ReportScope, integrity: &IntegrityReport) -> Result<(), String> {
    if scope == ReportScope::Official && !integrity.ok {
        return Err(
            "گزارش رسمی به دلیل وجود خطای سلامت حسابداری قابل چاپ نیست؛ ابتدا خطاهای Integrity را برطرف کنید یا گزارش را در حالت کاری تهیه کنید."
                .to_string(),
        );
    }
    Ok(())
}

/// The document belongs to the server side; the preview window may only ask
/// for a copy. It stays in state until its window is destroyed so saving the
/// file cannot substitute client HTML.
fn stored_doc(
    print_docs: &Mutex<HashMap<String, report::PrintDoc>>,
    label: &str,
) -> Result<report::PrintDoc, String> {
    lock(print_docs)
        .get(label)
        .cloned()
        .ok_or_else(|| "گزارشی برای پیش‌نمایش آماده نیست.".to_string())
}

/// Keeps the suggested save name a plain file name inside the folder the user
/// picks: no directory part, no parent component, no control character, and a
/// single `.html` extension.
fn safe_html_file_name(value: &str) -> String {
    let base = value.rsplit(['/', '\\']).next().unwrap_or("");
    let mut cleaned = String::with_capacity(base.len());
    for character in base.chars() {
        if character.is_control()
            || matches!(
                character,
                '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*'
            )
        {
            continue;
        }
        cleaned.push(character);
    }
    while cleaned.contains("..") {
        cleaned = cleaned.replace("..", ".");
    }
    cleaned = cleaned.trim().trim_matches('.').trim().to_string();
    let lower = cleaned.to_ascii_lowercase();
    let stem = if lower.ends_with(".html") {
        &cleaned[..cleaned.len() - 5]
    } else if lower.ends_with(".htm") {
        &cleaned[..cleaned.len() - 4]
    } else {
        cleaned.as_str()
    };
    let stem = stem.trim().trim_matches('.').trim();
    if stem.is_empty() {
        "گزارش.html".to_string()
    } else {
        format!("{stem}.html")
    }
}

/* ── Print preview ──────────────────────────────────────────── */

#[tauri::command(async)]
pub(crate) fn open_journal_preview(
    from: String,
    to: String,
    search: String,
    scope: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher"])?;
    let scope = report_scope(scope)?;
    let report = state
        .database
        .get_journal_scoped(&from, &to, &search, scope)?;
    let integrity = state
        .database
        .integrity_report(&report.from, &report.to, false, scope)?;
    let meta = report_meta(&state, &from, &to)?;
    show_preview_window(&app, report::journal_print_doc(&report, &meta, &integrity))
}

#[tauri::command(async)]
pub(crate) fn open_ledger_preview(
    account_code: String,
    from: String,
    to: String,
    scope: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher"])?;
    let scope = report_scope(scope)?;
    let report = state
        .database
        .get_ledger_scoped(&account_code, &from, &to, scope)?;
    let integrity = state
        .database
        .integrity_report(&report.from, &report.to, false, scope)?;
    let meta = report_meta(&state, &from, &to)?;
    show_preview_window(&app, report::ledger_print_doc(&report, &meta, &integrity))
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn open_trial_balance_preview(
    from: String,
    to: String,
    scope: Option<String>,
    account_from: Option<String>,
    account_to: Option<String>,
    show_zero: Option<bool>,
    multi_level: Option<bool>,
    balance_type: Option<String>,
    show_account_code: Option<bool>,
    show_account_name: Option<bool>,
    show_print_date_time: Option<bool>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher", "trial-balance"])?;
    let scope = report_scope(scope)?;
    let report = state.database.get_trial_balance_filtered_scoped(
        &from,
        &to,
        account_from.as_deref().unwrap_or(""),
        account_to.as_deref().unwrap_or(""),
        show_zero.unwrap_or(false),
        multi_level.unwrap_or(true),
        scope,
    )?;
    // A trial balance is an official statement: an unbalanced or otherwise
    // unhealthy book must not be printed as a final report. The working scope
    // remains available for review.
    let integrity = state
        .database
        .integrity_report(&report.from, &report.to, false, scope)?;
    ensure_printable(scope, &integrity)?;
    let mut meta = report_meta(&state, &from, &to)?;
    meta.show_print_date_time = show_print_date_time.unwrap_or(true);
    show_preview_window(
        &app,
        report::trial_balance_print_doc(
            &report,
            &meta,
            balance_type.as_deref().unwrap_or("شش ستونی"),
            multi_level.unwrap_or(true),
            show_account_code.unwrap_or(true),
            show_account_name.unwrap_or(true),
        )?,
    )
}

#[tauri::command(async)]
pub(crate) fn open_income_statement_preview(
    from: String,
    to: String,
    scope: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher"])?;
    let scope = report_scope(scope)?;
    let report = state
        .database
        .get_income_statement_scoped(&from, &to, scope)?;
    ensure_printable(scope, &report.integrity)?;
    let meta = report_meta(&state, &from, &to)?;
    show_preview_window(&app, report::income_statement_print_doc(&report, &meta))
}

#[tauri::command(async)]
pub(crate) fn open_balance_sheet_preview(
    as_of: String,
    scope: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher"])?;
    let scope = report_scope(scope)?;
    let report = state.database.get_balance_sheet_scoped(&as_of, scope)?;
    ensure_printable(scope, &report.integrity)?;
    let meta = report_meta(&state, &as_of, &as_of)?;
    let document = report::balance_sheet_print_doc(&report, &meta)?;
    show_preview_window(&app, document)
}

#[tauri::command(async)]
pub(crate) fn open_equity_changes_preview(
    as_of: String,
    scope: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher"])?;
    let scope = report_scope(scope)?;
    let report = state.database.get_equity_changes_scoped(&as_of, scope)?;
    ensure_printable(scope, &report.integrity)?;
    let meta = report_meta(&state, &as_of, &as_of)?;
    show_preview_window(&app, report::equity_changes_print_doc(&report, &meta))
}

#[tauri::command(async)]
pub(crate) fn open_cogs_preview(
    from: String,
    to: String,
    scope: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher"])?;
    let scope = report_scope(scope)?;
    let report = state.database.get_cogs_scoped(&from, &to, scope)?;
    ensure_printable(scope, &report.integrity)?;
    let meta = report_meta(&state, &report.from_date, &report.to_date)?;
    show_preview_window(&app, report::cogs_print_doc(&report, &meta))
}

#[tauri::command(async)]
pub(crate) fn open_profit_appropriation_preview(
    as_of: String,
    scope: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher"])?;
    let scope = report_scope(scope)?;
    let report = state
        .database
        .get_profit_appropriation_scoped(&as_of, scope)?;
    ensure_printable(scope, &report.integrity)?;
    let meta = report_meta(&state, &as_of, &as_of)?;
    show_preview_window(&app, report::appropriation_print_doc(&report, &meta))
}

#[tauri::command(async)]
pub(crate) fn open_cash_flow_preview(
    as_of: String,
    scope: Option<String>,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher"])?;
    let scope = report_scope(scope)?;
    let report = state.database.get_cash_flow_scoped(&as_of, scope)?;
    ensure_printable(scope, &report.integrity)?;
    let meta = report_meta(&state, &as_of, &as_of)?;
    show_preview_window(&app, report::cash_flow_print_doc(&report, &meta))
}

#[tauri::command(async)]
pub(crate) fn open_voucher_preview(
    number: i64,
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_window(&window, &["main", "voucher", "voucher-list"])?;
    let voucher = state
        .database
        .get_voucher(number)?
        .ok_or_else(|| "برای چاپ، ابتدا سند را ذخیره کنید.".to_string())?;
    // A single voucher is printed as a working document, never blocked; an
    // unhealthy book is disclosed as a warning bar in the printed page.
    let integrity = state.database.integrity_report(
        &voucher.date,
        &voucher.date,
        false,
        ReportScope::Working,
    )?;
    let meta = report_meta(&state, &voucher.date, &voucher.date)?;
    show_preview_window(&app, report::voucher_print_doc(&voucher, &meta, &integrity))
}

/// The preview window asks for the document it should render. The stored
/// document is only copied; it remains available until the window closes so a
/// later save writes the server-built HTML, not renderer input.
#[tauri::command]
pub(crate) fn get_print_doc(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<report::PrintDoc, String> {
    require_preview_window(&window)?;
    stored_doc(&state.print_docs, window.label())
}

/// Saves the standalone (browser-printable) HTML, used by the preview toolbar.
#[tauri::command]
pub(crate) fn save_report_html(
    default_name: String,
    html: String,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    require_preview_window(&window)?;
    let doc = stored_doc(&state.print_docs, window.label())?;
    // `html` comes from the WebView and is deliberately ignored. The file must
    // contain exactly the document Rust built for this window.
    let _ = html;
    let suggested = if default_name.trim().is_empty() {
        doc.file_name.as_str()
    } else {
        default_name.as_str()
    };
    let file_name = safe_html_file_name(suggested);
    let Some(picked) = rfd::FileDialog::new()
        .set_title("ذخیره گزارش قابل چاپ")
        .set_file_name(&file_name)
        .add_filter("HTML", &["html"])
        .save_file()
    else {
        return Ok(None);
    };
    let mut path = picked.to_string_lossy().to_string();
    if !path.to_lowercase().ends_with(".html") {
        path.push_str(".html");
    }
    std::fs::write(&path, &doc.html).map_err(|e| format!("ذخیره فایل ناموفق بود: {e}"))?;
    if let Ok(canonical) = std::fs::canonicalize(&path) {
        lock(&state.report_files).insert(canonical);
    }
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::report::PrintDoc;

    fn print_doc(html: &str) -> PrintDoc {
        PrintDoc {
            html: html.to_string(),
            title: "گزارش".to_string(),
            subtitle: String::new(),
            file_name: "گزارش.html".to_string(),
            paper_width_mm: 210.0,
            paper_height_mm: 297.0,
            margin_top_mm: 14.0,
            margin_right_mm: 12.0,
            margin_bottom_mm: 14.0,
            margin_left_mm: 12.0,
        }
    }

    #[test]
    fn safe_html_file_name_keeps_a_plain_basename_with_one_html_extension() {
        assert_eq!(
            safe_html_file_name("تراز-آزمایشی.html"),
            "تراز-آزمایشی.html"
        );
        assert_eq!(
            safe_html_file_name("../../secret/report.htm"),
            "report.html"
        );
        assert_eq!(safe_html_file_name("C:\\temp\\report.HTML"), "report.html");
        assert_eq!(safe_html_file_name("a..b"), "a.b.html");
        assert_eq!(safe_html_file_name(" .. "), "گزارش.html");
        assert_eq!(safe_html_file_name("re\u{0007}port"), "report.html");
    }

    #[test]
    fn stored_document_survives_repeated_reads() {
        let docs = Mutex::new(HashMap::new());
        docs.lock()
            .unwrap()
            .insert("print-preview-1".to_string(), print_doc("<p>server</p>"));
        let first = stored_doc(&docs, "print-preview-1").unwrap();
        let second = stored_doc(&docs, "print-preview-1").unwrap();
        assert_eq!(first.html, "<p>server</p>");
        assert_eq!(second.html, "<p>server</p>");
    }

    #[test]
    fn official_print_rejects_unhealthy_book_but_working_print_allows_it() {
        let database = db::memory_database();
        database
            .with_conn(|conn| {
                let cash_id: i64 = conn
                    .query_row("SELECT id FROM accounts WHERE code = '1101'", [], |row| {
                        row.get(0)
                    })
                    .map_err(db::db_error)?;
                conn.execute(
                    "INSERT INTO vouchers (number, date, type, is_temporary)
                     VALUES (1, '1405/01/01', ?1, 1)",
                    rusqlite::params![crate::vouchers::VOUCHER_TYPES[0]],
                )
                .map_err(db::db_error)?;
                let voucher_id = conn.last_insert_rowid();
                // One-sided line: bypasses form validation like legacy data
                // and makes the trial balance unbalanced.
                conn.execute(
                    "INSERT INTO voucher_lines
                        (voucher_id, row_no, account_id, account_code, description, debit, credit)
                     VALUES (?1, 1, ?2, '1101', '', 100, 0)",
                    rusqlite::params![voucher_id, cash_id],
                )
                .map_err(db::db_error)?;
                conn.execute(
                    "UPDATE vouchers SET is_temporary = 0 WHERE id = ?1",
                    rusqlite::params![voucher_id],
                )
                .map_err(db::db_error)?;
                Ok(())
            })
            .unwrap();

        let integrity = database
            .integrity_report("1405/01/01", "1405/12/29", false, ReportScope::Official)
            .unwrap();
        assert!(!integrity.ok);
        assert!(integrity
            .issues
            .iter()
            .any(|issue| issue.code == "trial_balance"));
        assert!(ensure_printable(ReportScope::Official, &integrity).is_err());
        assert!(ensure_printable(ReportScope::Working, &integrity).is_ok());
    }
}
