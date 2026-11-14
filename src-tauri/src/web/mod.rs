//! Local-only HTTP host for the accounting UI.
//!
//! The desktop app and this browser mode share the same SQLite schema and the
//! same Rust storage layer. The browser gets a small JSON command endpoint
//! that mirrors the part of the Tauri command surface needed by the local-web
//! slices so far: settings, the chart of accounts, fiscal years, the voucher
//! list, the voucher form including save, permanent, delete and line transfer,
//! the main and supplementary report pages, and their print previews.
//!
//! The listener is bound to `127.0.0.1` only. There is no authentication and no
//! remote access; the server is meant for the same Windows session as the
//! desktop application.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tower_http::services::ServeDir;

use crate::commands::print::{ensure_printable, report_scope};
use crate::commands::state::report_meta_for;
use crate::db;
use crate::report::{self, PrintDoc};
use crate::vouchers::{self, ReportScope, VoucherInput, VoucherTransferInput};

const DEFAULT_PORT: u16 = 4174;

/// The local host builds print previews per request. Each document gets an
/// opaque token that only the preview tab receives through its URL.
static PRINT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
const MAX_STORED_PRINT_DOCS: usize = 16;

#[derive(Clone)]
struct WebState {
    database: Arc<db::Database>,
    print_docs: Arc<Mutex<HashMap<String, PrintDoc>>>,
}

#[derive(Debug, Deserialize)]
struct CommandRequest {
    name: String,
    #[serde(default)]
    args: Value,
    #[serde(default)]
    window: Option<String>,
}

#[derive(Debug, Serialize)]
struct CommandResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListVouchersArgs {
    #[serde(default)]
    from: String,
    #[serde(default)]
    to: String,
    #[serde(default)]
    search: String,
    offset: Option<i64>,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrialBalanceArgs {
    #[serde(default)]
    from: String,
    #[serde(default)]
    to: String,
    scope: Option<String>,
    account_from: Option<String>,
    account_to: Option<String>,
    show_zero: Option<bool>,
    multi_level: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JournalArgs {
    #[serde(default)]
    from: String,
    #[serde(default)]
    to: String,
    #[serde(default)]
    search: String,
    scope: Option<String>,
    offset: Option<i64>,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LedgerArgs {
    account_code: String,
    #[serde(default)]
    from: String,
    #[serde(default)]
    to: String,
    scope: Option<String>,
    offset: Option<i64>,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IncomeStatementArgs {
    #[serde(default)]
    from: String,
    #[serde(default)]
    to: String,
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BalanceSheetArgs {
    as_of: String,
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AsOfArgs {
    as_of: String,
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RangeArgs {
    #[serde(default)]
    from: String,
    #[serde(default)]
    to: String,
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JournalPrintArgs {
    #[serde(default)]
    from: String,
    #[serde(default)]
    to: String,
    #[serde(default)]
    search: String,
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LedgerPrintArgs {
    account_code: String,
    #[serde(default)]
    from: String,
    #[serde(default)]
    to: String,
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrialBalancePrintArgs {
    #[serde(default)]
    from: String,
    #[serde(default)]
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
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrintDocArgs {
    token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateFiscalYearArgs {
    title: String,
    start_date: String,
    end_date: String,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateFiscalYearArgs {
    id: i64,
    title: String,
    start_date: String,
    end_date: String,
    status: String,
    reason: Option<String>,
    confirm: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct DateArgs {
    date: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VoucherNumberArgs {
    number: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdjacentVoucherArgs {
    number: i64,
    direction: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenVoucherArgs {
    number: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveVoucherArgs {
    input: VoucherInput,
    expected_updated_at: Option<String>,
    idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VoucherIdArgs {
    id: i64,
    expected_updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TransferVoucherArgs {
    input: VoucherTransferInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateAccountArgs {
    code: String,
    name: String,
    level: i64,
    group_code: String,
    nature: String,
    #[serde(default)]
    parent_code: String,
    #[serde(default)]
    is_postable: bool,
    is_active: Option<bool>,
    fs_line: Option<String>,
    contra_of: Option<String>,
    contra_kind: Option<String>,
    opposite_fs_line: Option<String>,
    split_by_sign: Option<bool>,
    system_role: Option<String>,
}

/// Starts the loopback server and blocks until it stops.
pub fn run() -> Result<(), String> {
    let data_dir = data_dir_from_env()?;
    let ui_dir = ui_dir_from_env()?;
    let port = port_from_env()?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("hesabdari-web")
        .build()
        .map_err(|error| format!("راه‌اندازی سرور وب محلی ناموفق بود: {error}"))?;
    runtime.block_on(serve(data_dir, ui_dir, port))
}

async fn serve(data_dir: PathBuf, ui_dir: PathBuf, port: u16) -> Result<(), String> {
    let (connection, path) = db::initialize_at(&data_dir, None, false)?;
    let database = Arc::new(db::Database::default());
    database.attach(connection, path);

    let state = WebState {
        database: database.clone(),
        print_docs: Arc::new(Mutex::new(HashMap::new())),
    };
    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/command", post(command))
        .fallback_service(ServeDir::new(&ui_dir).append_index_html_on_directories(true))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .await
        .map_err(|error| format!("گوش دادن روی 127.0.0.1:{port} ناموفق بود: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("خواندن آدرس سرور وب محلی ناموفق بود: {error}"))?;
    let url = format!("http://{address}/");

    eprintln!("[hesabdari-web] local web mode: {url}");
    eprintln!("[hesabdari-web] ui directory: {}", ui_dir.display());
    eprintln!("[hesabdari-web] database: {}", database.path_string());
    if std::env::var_os("HESABDARI_NO_OPEN").is_none() {
        open_browser(&url);
    }

    axum::serve(listener, app)
        .await
        .map_err(|error| format!("سرور وب محلی متوقف شد: {error}"))
}

async fn health(State(state): State<WebState>) -> Json<Value> {
    Json(json!({
        "ok": true,
        "mode": "local-web",
        "version": env!("CARGO_PKG_VERSION"),
        "databasePath": state.database.path_string(),
    }))
}

async fn command(State(state): State<WebState>, Json(request): Json<CommandRequest>) -> Response {
    let CommandRequest { name, args, window } = request;
    let window = window.unwrap_or_else(|| "main".to_string());
    let database = state.database.clone();
    let print_docs = state.print_docs.clone();

    let result =
        tokio::task::spawn_blocking(move || dispatch(&database, &print_docs, &name, args, &window))
            .await;
    match result {
        Ok(Ok(data)) => Json(CommandResponse {
            ok: true,
            data: Some(data),
            error: None,
        })
        .into_response(),
        Ok(Err(error)) => (
            StatusCode::BAD_REQUEST,
            Json(CommandResponse {
                ok: false,
                data: None,
                error: Some(error),
            }),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(CommandResponse {
                ok: false,
                data: None,
                error: Some(format!("اجرای دستور با خطای داخلی متوقف شد: {error}")),
            }),
        )
            .into_response(),
    }
}

fn dispatch(
    database: &db::Database,
    print_docs: &Mutex<HashMap<String, PrintDoc>>,
    name: &str,
    args: Value,
    window: &str,
) -> Result<Value, String> {
    match name {
        "get_settings" => {
            require_window(
                window,
                &["main", "trial-balance", "voucher-list", "voucher"],
            )?;
            to_value(database.get_settings()?)
        }
        "list_account_groups" => {
            require_window(
                window,
                &[
                    "main",
                    "trial-balance",
                    "voucher-list",
                    "voucher",
                    "account-groups",
                    "account-definition",
                ],
            )?;
            to_value(database.list_account_groups()?)
        }
        "list_accounts" => {
            require_window(
                window,
                &[
                    "main",
                    "trial-balance",
                    "voucher-list",
                    "voucher",
                    "account-groups",
                    "account-definition",
                ],
            )?;
            to_value(database.list_accounts()?)
        }
        "list_fs_lines" => {
            require_window(
                window,
                &["main", "voucher", "account-groups", "account-definition"],
            )?;
            to_value(db::fs_catalog::fs_catalog())
        }
        "list_fiscal_years" => {
            require_window(window, &["main"])?;
            to_value(database.list_fiscal_years()?)
        }
        "create_fiscal_year" => {
            require_window(window, &["main"])?;
            let args: CreateFiscalYearArgs = parse_args(args)?;
            let year = database.create_fiscal_year(
                &args.title,
                &args.start_date,
                &args.end_date,
                args.status.as_deref().unwrap_or("open"),
            )?;
            to_value(year)
        }
        "update_fiscal_year" => {
            require_window(window, &["main"])?;
            let args: UpdateFiscalYearArgs = parse_args(args)?;
            let year = database.update_fiscal_year(
                args.id,
                &args.title,
                &args.start_date,
                &args.end_date,
                &args.status,
                args.reason.as_deref(),
                args.confirm.unwrap_or(false),
            )?;
            to_value(year)
        }
        "get_voucher_count" => {
            require_window(window, &["main", "trial-balance", "voucher-list"])?;
            to_value(database.voucher_count()?)
        }
        "list_vouchers" => {
            require_window(
                window,
                &["main", "voucher", "trial-balance", "voucher-list"],
            )?;
            let args: ListVouchersArgs = parse_args(args)?;
            let list = database.list_vouchers_page(
                &args.from,
                &args.to,
                &args.search,
                args.offset,
                args.limit.or(Some(vouchers::DEFAULT_VOUCHER_PAGE_SIZE)),
            )?;
            to_value(list)
        }
        "validate_voucher_date" => {
            require_window(window, &["main", "voucher", "bulk-voucher-delete"])?;
            let args: DateArgs = parse_args(args)?;
            to_value(vouchers::validate_voucher_date(&args.date)?)
        }
        "next_voucher_number" => {
            require_window(window, &["main", "voucher"])?;
            to_value(database.next_voucher_number()?)
        }
        "adjacent_voucher_number" => {
            require_window(window, &["main", "voucher"])?;
            let args: AdjacentVoucherArgs = parse_args(args)?;
            to_value(database.adjacent_voucher_number(args.number, &args.direction)?)
        }
        "get_voucher" => {
            require_window(window, &["main", "voucher", "voucher-list"])?;
            let args: VoucherNumberArgs = parse_args(args)?;
            to_value(database.get_voucher(args.number)?)
        }
        "save_voucher" => {
            require_window(window, &["main", "voucher"])?;
            let args: SaveVoucherArgs = parse_args(args)?;
            let saved = database.save_voucher_idempotent(
                args.input,
                args.expected_updated_at.as_deref(),
                args.idempotency_key.as_deref(),
            )?;
            to_value(saved)
        }
        "set_voucher_permanent" => {
            require_window(window, &["main", "voucher"])?;
            let args: VoucherIdArgs = parse_args(args)?;
            let permanent =
                database.set_voucher_permanent(args.id, args.expected_updated_at.as_deref())?;
            to_value(permanent)
        }
        "delete_voucher" => {
            require_window(window, &["main", "voucher", "voucher-list"])?;
            let args: VoucherIdArgs = parse_args(args)?;
            database.delete_voucher(args.id, args.expected_updated_at.as_deref())?;
            to_value(())
        }
        "transfer_voucher_lines" => {
            require_window(window, &["main", "voucher"])?;
            let args: TransferVoucherArgs = parse_args(args)?;
            to_value(database.transfer_voucher_lines(args.input)?)
        }
        "create_account" => {
            require_window(
                window,
                &["main", "voucher", "account-groups", "account-definition"],
            )?;
            let args: CreateAccountArgs = parse_args(args)?;
            let account = db::NewAccount::new(
                &args.code,
                &args.name,
                args.level,
                &args.group_code,
                &args.nature,
                &args.parent_code,
                args.is_postable,
            )
            .with_active(args.is_active.unwrap_or(true))
            .with_fs_line(args.fs_line.as_deref().unwrap_or(""))
            .with_classification(
                args.contra_of.as_deref().unwrap_or(""),
                args.contra_kind.as_deref().unwrap_or(""),
                args.opposite_fs_line.as_deref().unwrap_or(""),
                args.split_by_sign.unwrap_or(false),
                args.system_role.as_deref().unwrap_or(""),
            );
            let created = database
                .create_accounts_atomic(vec![account])?
                .pop()
                .ok_or_else(|| "ساخت حساب انجام نشد.".to_string())?;
            to_value(created)
        }
        "get_trial_balance" => {
            require_window(window, &["main", "trial-balance", "voucher-list"])?;
            let args: TrialBalanceArgs = parse_args(args)?;
            let scope = ReportScope::parse(args.scope.as_deref().unwrap_or("official"))?;
            let report = database.get_trial_balance_filtered_scoped(
                &args.from,
                &args.to,
                args.account_from.as_deref().unwrap_or(""),
                args.account_to.as_deref().unwrap_or(""),
                args.show_zero.unwrap_or(false),
                args.multi_level.unwrap_or(true),
                scope,
            )?;
            to_value(report)
        }
        "get_journal" => {
            require_window(window, &["main", "voucher-list", "trial-balance"])?;
            let args: JournalArgs = parse_args(args)?;
            let scope = report_scope(args.scope)?;
            let report = database.get_journal_page_scoped(
                &args.from,
                &args.to,
                &args.search,
                args.offset,
                args.limit.or(Some(vouchers::DEFAULT_JOURNAL_PAGE_SIZE)),
                scope,
            )?;
            to_value(report)
        }
        "get_ledger" => {
            require_window(
                window,
                &["main", "voucher", "voucher-list", "trial-balance"],
            )?;
            let args: LedgerArgs = parse_args(args)?;
            let report = database.get_ledger_page_scoped(
                &args.account_code,
                &args.from,
                &args.to,
                args.offset,
                args.limit,
                report_scope(args.scope)?,
            )?;
            to_value(report)
        }
        "get_income_statement" => {
            require_window(window, &["main", "voucher-list", "trial-balance"])?;
            let args: IncomeStatementArgs = parse_args(args)?;
            to_value(database.get_income_statement_scoped(
                &args.from,
                &args.to,
                report_scope(args.scope)?,
            )?)
        }
        "get_balance_sheet" => {
            require_window(window, &["main", "voucher-list", "trial-balance"])?;
            let args: BalanceSheetArgs = parse_args(args)?;
            to_value(database.get_balance_sheet_scoped(&args.as_of, report_scope(args.scope)?)?)
        }
        "get_equity_changes_statement" => {
            require_window(window, &["main", "voucher-list", "trial-balance"])?;
            let args: AsOfArgs = parse_args(args)?;
            to_value(database.get_equity_changes_scoped(&args.as_of, report_scope(args.scope)?)?)
        }
        "get_cogs_statement" => {
            require_window(window, &["main", "voucher-list", "trial-balance"])?;
            let args: RangeArgs = parse_args(args)?;
            to_value(database.get_cogs_scoped(&args.from, &args.to, report_scope(args.scope)?)?)
        }
        "get_profit_appropriation_statement" => {
            require_window(window, &["main", "voucher-list", "trial-balance"])?;
            let args: AsOfArgs = parse_args(args)?;
            to_value(
                database.get_profit_appropriation_scoped(&args.as_of, report_scope(args.scope)?)?,
            )
        }
        "get_cash_flow_statement" => {
            require_window(window, &["main", "voucher-list", "trial-balance"])?;
            let args: AsOfArgs = parse_args(args)?;
            to_value(database.get_cash_flow_scoped(&args.as_of, report_scope(args.scope)?)?)
        }
        "open_voucher_preview" => {
            require_window(window, &["main", "voucher", "voucher-list"])?;
            let args: VoucherNumberArgs = parse_args(args)?;
            let voucher = database
                .get_voucher(args.number)?
                .ok_or_else(|| "برای چاپ، ابتدا سند را ذخیره کنید.".to_string())?;
            let integrity = database.integrity_report(
                &voucher.date,
                &voucher.date,
                false,
                ReportScope::Working,
            )?;
            let meta = report_meta_for(database, &voucher.date, &voucher.date)?;
            let document = report::voucher_print_doc(&voucher, &meta, &integrity);
            Ok(preview_window(print_docs, document))
        }
        "open_ledger_preview" => {
            require_window(window, &["main", "voucher"])?;
            let args: LedgerPrintArgs = parse_args(args)?;
            let scope = report_scope(args.scope)?;
            let ledger =
                database.get_ledger_scoped(&args.account_code, &args.from, &args.to, scope)?;
            let integrity = database.integrity_report(&ledger.from, &ledger.to, false, scope)?;
            let meta = report_meta_for(database, &args.from, &args.to)?;
            let document = report::ledger_print_doc(&ledger, &meta, &integrity);
            Ok(preview_window(print_docs, document))
        }
        "open_journal_preview" => {
            require_window(window, &["main", "voucher"])?;
            let args: JournalPrintArgs = parse_args(args)?;
            let scope = report_scope(args.scope)?;
            let journal = database.get_journal_scoped(&args.from, &args.to, &args.search, scope)?;
            let integrity = database.integrity_report(&journal.from, &journal.to, false, scope)?;
            let meta = report_meta_for(database, &args.from, &args.to)?;
            let document = report::journal_print_doc(&journal, &meta, &integrity);
            Ok(preview_window(print_docs, document))
        }
        "open_trial_balance_preview" => {
            require_window(window, &["main", "voucher", "trial-balance"])?;
            let args: TrialBalancePrintArgs = parse_args(args)?;
            let scope = report_scope(args.scope)?;
            let trial = database.get_trial_balance_filtered_scoped(
                &args.from,
                &args.to,
                args.account_from.as_deref().unwrap_or(""),
                args.account_to.as_deref().unwrap_or(""),
                args.show_zero.unwrap_or(false),
                args.multi_level.unwrap_or(true),
                scope,
            )?;
            let integrity = database.integrity_report(&trial.from, &trial.to, false, scope)?;
            ensure_printable(scope, &integrity)?;
            let mut meta = report_meta_for(database, &args.from, &args.to)?;
            meta.show_print_date_time = args.show_print_date_time.unwrap_or(true);
            let document = report::trial_balance_print_doc(
                &trial,
                &meta,
                args.balance_type.as_deref().unwrap_or("شش ستونی"),
                args.multi_level.unwrap_or(true),
                args.show_account_code.unwrap_or(true),
                args.show_account_name.unwrap_or(true),
            )?;
            Ok(preview_window(print_docs, document))
        }
        "open_income_statement_preview" => {
            require_window(window, &["main", "voucher"])?;
            let args: IncomeStatementArgs = parse_args(args)?;
            let scope = report_scope(args.scope)?;
            let statement = database.get_income_statement_scoped(&args.from, &args.to, scope)?;
            ensure_printable(scope, &statement.integrity)?;
            let meta = report_meta_for(database, &args.from, &args.to)?;
            Ok(preview_window(
                print_docs,
                report::income_statement_print_doc(&statement, &meta),
            ))
        }
        "open_balance_sheet_preview" => {
            require_window(window, &["main", "voucher"])?;
            let args: BalanceSheetArgs = parse_args(args)?;
            let scope = report_scope(args.scope)?;
            let statement = database.get_balance_sheet_scoped(&args.as_of, scope)?;
            ensure_printable(scope, &statement.integrity)?;
            let meta = report_meta_for(database, &args.as_of, &args.as_of)?;
            Ok(preview_window(
                print_docs,
                report::balance_sheet_print_doc(&statement, &meta)?,
            ))
        }
        "open_equity_changes_preview" => {
            require_window(window, &["main", "voucher"])?;
            let args: AsOfArgs = parse_args(args)?;
            let scope = report_scope(args.scope)?;
            let statement = database.get_equity_changes_scoped(&args.as_of, scope)?;
            ensure_printable(scope, &statement.integrity)?;
            let meta = report_meta_for(database, &args.as_of, &args.as_of)?;
            Ok(preview_window(
                print_docs,
                report::equity_changes_print_doc(&statement, &meta),
            ))
        }
        "open_cogs_preview" => {
            require_window(window, &["main", "voucher"])?;
            let args: RangeArgs = parse_args(args)?;
            let scope = report_scope(args.scope)?;
            let statement = database.get_cogs_scoped(&args.from, &args.to, scope)?;
            ensure_printable(scope, &statement.integrity)?;
            let meta = report_meta_for(database, &statement.from_date, &statement.to_date)?;
            Ok(preview_window(
                print_docs,
                report::cogs_print_doc(&statement, &meta),
            ))
        }
        "open_profit_appropriation_preview" => {
            require_window(window, &["main", "voucher"])?;
            let args: AsOfArgs = parse_args(args)?;
            let scope = report_scope(args.scope)?;
            let statement = database.get_profit_appropriation_scoped(&args.as_of, scope)?;
            ensure_printable(scope, &statement.integrity)?;
            let meta = report_meta_for(database, &args.as_of, &args.as_of)?;
            Ok(preview_window(
                print_docs,
                report::appropriation_print_doc(&statement, &meta),
            ))
        }
        "open_cash_flow_preview" => {
            require_window(window, &["main", "voucher"])?;
            let args: AsOfArgs = parse_args(args)?;
            let scope = report_scope(args.scope)?;
            let statement = database.get_cash_flow_scoped(&args.as_of, scope)?;
            ensure_printable(scope, &statement.integrity)?;
            let meta = report_meta_for(database, &args.as_of, &args.as_of)?;
            Ok(preview_window(
                print_docs,
                report::cash_flow_print_doc(&statement, &meta),
            ))
        }
        "get_print_doc" => {
            if !window.starts_with("print-preview-") {
                return Err("این فرمان در این پنجره مجاز نیست.".to_string());
            }
            let args: PrintDocArgs = parse_args(args)?;
            to_value(stored_print_doc(print_docs, &args.token)?)
        }
        "open_voucher_window" => {
            require_window(window, &["main", "voucher-list"])?;
            let args: OpenVoucherArgs = parse_args(args)?;
            let url = match args.number.filter(|number| *number > 0) {
                Some(number) => format!("/voucher.html?number={number}"),
                None => "/voucher.html".to_string(),
            };
            Ok(web_window(&url, "voucher"))
        }
        "open_trial_balance_window" => {
            require_window(window, &["main"])?;
            Ok(web_window(
                "/index.html?popup=trial-balance",
                "trial-balance",
            ))
        }
        "open_voucher_list_window" => {
            require_window(window, &["main"])?;
            Ok(web_window("/index.html?popup=voucher-list", "voucher-list"))
        }
        "open_account_groups_window" => {
            require_window(window, &["main"])?;
            Ok(web_window("/account-groups.html", "account-groups"))
        }
        "open_account_definition_window" => {
            require_window(window, &["main"])?;
            Ok(web_window("/account-definition.html", "account-definition"))
        }
        _ => Err(format!(
            "دستور «{name}» در نسخه وب محلی هنوز پشتیبانی نمی‌شود."
        )),
    }
}

fn require_window(window: &str, allowed: &[&str]) -> Result<(), String> {
    if allowed.contains(&window) {
        Ok(())
    } else {
        Err("این فرمان در این پنجره مجاز نیست.".to_string())
    }
}

fn parse_args<T>(args: Value) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let args = if args.is_null() {
        Value::Object(Map::new())
    } else {
        args
    };
    serde_json::from_value(args).map_err(|error| format!("پارامترهای دستور نامعتبر است: {error}"))
}

fn to_value<T: Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| format!("تبدیل پاسخ به JSON ناموفق بود: {error}"))
}

fn web_window(url: &str, name: &str) -> Value {
    json!({ "webOpen": url, "webName": name })
}

/// Keeps the printable document on the server and returns only an opaque
/// token. The preview tab asks for the stored HTML; the page never supplies it.
fn store_print_doc(print_docs: &Mutex<HashMap<String, PrintDoc>>, document: PrintDoc) -> String {
    let mut docs = print_docs
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if docs.len() >= MAX_STORED_PRINT_DOCS {
        docs.clear();
    }
    let token = format!("doc-{}", PRINT_SEQUENCE.fetch_add(1, Ordering::Relaxed));
    docs.insert(token.clone(), document);
    token
}

fn stored_print_doc(
    print_docs: &Mutex<HashMap<String, PrintDoc>>,
    token: &str,
) -> Result<PrintDoc, String> {
    print_docs
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(token)
        .cloned()
        .ok_or_else(|| "گزارشی برای پیش‌نمایش آماده نیست.".to_string())
}

fn preview_window(print_docs: &Mutex<HashMap<String, PrintDoc>>, document: PrintDoc) -> Value {
    let token = store_print_doc(print_docs, document);
    web_window(&format!("/print.html?doc={token}"), "print-preview")
}

fn data_dir_from_env() -> Result<PathBuf, String> {
    match std::env::var_os("HESABDARI_DATA_DIR") {
        Some(value) if !value.is_empty() => {
            let path = PathBuf::from(value);
            if !path.is_absolute() {
                return Err("HESABDARI_DATA_DIR باید یک مسیر مطلق باشد.".to_string());
            }
            Ok(path)
        }
        _ => default_data_dir(),
    }
}

#[cfg(target_os = "windows")]
fn default_data_dir() -> Result<PathBuf, String> {
    let app_data = std::env::var_os("APPDATA")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "متغیر محیطی APPDATA پیدا نشد؛ HESABDARI_DATA_DIR را تنظیم کنید.".to_string()
        })?;
    Ok(PathBuf::from(app_data).join("ir.hesabdari.software"))
}

#[cfg(not(target_os = "windows"))]
fn default_data_dir() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(value).join("hesabdari"));
    }
    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "متغیر محیطی HOME پیدا نشد؛ HESABDARI_DATA_DIR را تنظیم کنید.".to_string()
        })?;
    Ok(PathBuf::from(home).join(".local/share/hesabdari"))
}

fn ui_dir_from_env() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os("HESABDARI_UI_DIR") {
        if !value.is_empty() {
            return Ok(PathBuf::from(value));
        }
    }

    let mut candidates = Vec::new();
    if let Ok(current) = std::env::current_dir() {
        candidates.push(current.join("ui"));
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            candidates.push(directory.join("ui"));
            candidates.push(directory.join("..").join("ui"));
        }
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("ui"),
    );

    for candidate in candidates {
        if candidate.join("index.html").is_file() {
            return Ok(candidate);
        }
    }

    Err("پوشه رابط کاربری (ui) پیدا نشد. آن را کنار فایل اجرایی بگذارید یا HESABDARI_UI_DIR را تنظیم کنید.".to_string())
}

fn port_from_env() -> Result<u16, String> {
    let raw = match std::env::var("HESABDARI_WEB_PORT") {
        Ok(value) => value,
        Err(_) => return Ok(DEFAULT_PORT),
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(DEFAULT_PORT);
    }
    let port = raw
        .parse::<u16>()
        .map_err(|_| "HESABDARI_WEB_PORT باید یک شماره پورت معتبر باشد.".to_string())?;
    if port == 0 {
        return Err("HESABDARI_WEB_PORT نمی‌تواند صفر باشد.".to_string());
    }
    Ok(port)
}

fn open_browser(url: &str) {
    #[cfg(target_os = "windows")]
    {
        if let Err(error) = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
        {
            eprintln!("[hesabdari-web] could not open the browser: {error}");
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        if let Err(error) = std::process::Command::new(opener).arg(url).spawn() {
            eprintln!("[hesabdari-web] could not open the browser: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dispatch(
        database: &db::Database,
        name: &str,
        args: Value,
        window: &str,
    ) -> Result<Value, String> {
        let print_docs = Mutex::new(HashMap::new());
        super::dispatch(database, &print_docs, name, args, window)
    }

    fn call(database: &db::Database, name: &str, args: Value) -> Result<Value, String> {
        dispatch(database, name, args, "main")
    }

    #[test]
    fn settings_and_chart_are_served_from_the_shared_database() {
        let database = crate::db::memory_database();

        let settings = call(&database, "get_settings", Value::Null).unwrap();
        assert!(settings["companyName"].is_string());

        let groups = call(&database, "list_account_groups", Value::Null).unwrap();
        assert!(!groups["groups"].as_array().unwrap().is_empty());

        let accounts = call(&database, "list_accounts", Value::Null).unwrap();
        assert!(!accounts["accounts"].as_array().unwrap().is_empty());

        let lines = call(&database, "list_fs_lines", Value::Null).unwrap();
        assert!(!lines["lines"].as_array().unwrap().is_empty());
    }

    #[test]
    fn fiscal_year_create_and_update_use_the_shared_storage_rules() {
        let database = crate::db::memory_database();
        let before = call(&database, "list_fiscal_years", Value::Null).unwrap();
        let before_count = before["years"].as_array().unwrap().len();

        let created = call(
            &database,
            "create_fiscal_year",
            json!({
                "title": "سال مالی آزمون وب",
                "startDate": "1410/01/01",
                "endDate": "1410/12/29",
                "status": "open",
            }),
        )
        .unwrap();
        assert_eq!(created["title"], "سال مالی آزمون وب");
        assert_eq!(created["startDate"], "1410/01/01");

        let after = call(&database, "list_fiscal_years", Value::Null).unwrap();
        assert_eq!(after["years"].as_array().unwrap().len(), before_count + 1);

        let updated = call(
            &database,
            "update_fiscal_year",
            json!({
                "id": created["id"],
                "title": "سال مالی آزمون وب ۲",
                "startDate": "1410/01/01",
                "endDate": "1410/12/29",
                "status": "open",
                "reason": null,
                "confirm": false,
            }),
        )
        .unwrap();
        assert_eq!(updated["title"], "سال مالی آزمون وب ۲");
    }

    #[test]
    fn voucher_count_list_and_trial_balance_are_available() {
        let database = crate::db::memory_database();

        let count = call(&database, "get_voucher_count", Value::Null).unwrap();
        assert_eq!(count, json!(0));

        let list = call(
            &database,
            "list_vouchers",
            json!({
                "from": "1410/01/01",
                "to": "1410/12/29",
                "search": "",
                "offset": 0,
                "limit": 50,
            }),
        )
        .unwrap();
        assert!(list["vouchers"].as_array().unwrap().is_empty());
        assert_eq!(list["totalCount"], json!(0));

        let report = call(
            &database,
            "get_trial_balance",
            json!({
                "from": "1410/01/01",
                "to": "1410/12/29",
                "scope": "official",
                "accountFrom": "",
                "accountTo": "",
                "showZero": true,
                "multiLevel": true,
            }),
        )
        .unwrap();
        assert_eq!(report["from"], "1410/01/01");
        assert!(report["rows"].is_array());
    }

    #[test]
    fn voucher_form_saves_reads_updates_transfers_and_deletes() {
        let database = crate::db::memory_database();

        let next = call(&database, "next_voucher_number", Value::Null).unwrap();
        assert_eq!(next, json!(1));

        let normalized = dispatch(
            &database,
            "validate_voucher_date",
            json!({ "date": "۱۴۱۰/۱/۵" }),
            "voucher",
        )
        .unwrap();
        assert_eq!(normalized, json!("1410/01/05"));

        let first = dispatch(
            &database,
            "save_voucher",
            json!({
                "input": {
                    "id": null,
                    "number": 1,
                    "date": "1410/01/05",
                    "voucherType": vouchers::VOUCHER_TYPES[0],
                    "description": "سند آزمون وب",
                    "fiscalYearId": null,
                    "lines": [
                        {
                            "id": null,
                            "accountCode": "110101",
                            "detail1Code": null,
                            "description": "صندوق",
                            "debit": 1500,
                            "credit": 0,
                        },
                        {
                            "id": null,
                            "accountCode": "410101",
                            "detail1Code": null,
                            "description": "فروش",
                            "debit": 0,
                            "credit": 1500,
                        },
                    ],
                },
                "expectedUpdatedAt": null,
                "idempotencyKey": "web-save-1",
            }),
            "voucher",
        )
        .unwrap();
        assert_eq!(first["number"], json!(1));
        assert_eq!(first["totalDebit"], json!(1500));
        assert_eq!(first["isTemporary"], json!(true));

        let fetched =
            dispatch(&database, "get_voucher", json!({ "number": 1 }), "voucher").unwrap();
        assert_eq!(fetched["description"], "سند آزمون وب");

        let edited = dispatch(
            &database,
            "save_voucher",
            json!({
                "input": {
                    "id": first["id"],
                    "number": first["number"],
                    "date": first["date"],
                    "voucherType": first["type"],
                    "description": "سند ویرایش‌شده",
                    "fiscalYearId": null,
                    "lines": first["lines"].clone(),
                },
                "expectedUpdatedAt": first["updatedAt"],
                "idempotencyKey": "web-save-2",
            }),
            "voucher",
        )
        .unwrap();
        assert_eq!(edited["description"], "سند ویرایش‌شده");
        assert_eq!(edited["number"], json!(1));

        let permanent = call(
            &database,
            "set_voucher_permanent",
            json!({ "id": edited["id"], "expectedUpdatedAt": edited["updatedAt"] }),
        )
        .unwrap();
        assert_eq!(permanent["isTemporary"], json!(false));

        let second = dispatch(
            &database,
            "save_voucher",
            json!({
                "input": {
                    "id": null,
                    "number": 2,
                    "date": "1410/02/01",
                    "voucherType": vouchers::VOUCHER_TYPES[0],
                    "description": "سند مقصد انتقال",
                    "fiscalYearId": null,
                    "lines": [
                        {
                            "id": null,
                            "accountCode": "110101",
                            "detail1Code": null,
                            "description": "صندوق",
                            "debit": 2000,
                            "credit": 0,
                        },
                        {
                            "id": null,
                            "accountCode": "410101",
                            "detail1Code": null,
                            "description": "فروش",
                            "debit": 0,
                            "credit": 2000,
                        },
                    ],
                },
                "expectedUpdatedAt": null,
                "idempotencyKey": "web-save-3",
            }),
            "voucher",
        )
        .unwrap();
        assert_eq!(second["isTemporary"], json!(true));

        let source_lines: Vec<i64> = edited["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line["id"].as_i64().unwrap())
            .collect();
        let transferred = dispatch(
            &database,
            "transfer_voucher_lines",
            json!({
                "input": {
                    "sourceVoucherId": first["id"],
                    "sourceVoucherNumber": first["number"],
                    "targetVoucherNumber": second["number"],
                    "lineIds": source_lines,
                    "mode": "copy",
                },
            }),
            "voucher",
        )
        .unwrap();
        assert_eq!(transferred["createdTarget"], json!(false));
        assert_eq!(transferred["target"]["lines"].as_array().unwrap().len(), 4);

        let list = dispatch(
            &database,
            "list_vouchers",
            json!({
                "from": "1410/01/01",
                "to": "1410/12/29",
                "search": "",
                "offset": 0,
                "limit": 50,
            }),
            "voucher",
        )
        .unwrap();
        assert_eq!(list["totalCount"], json!(2));

        // A permanent voucher is locked and cannot be deleted.
        assert!(call(
            &database,
            "delete_voucher",
            json!({ "id": permanent["id"], "expectedUpdatedAt": permanent["updatedAt"] }),
        )
        .is_err());

        call(
            &database,
            "delete_voucher",
            json!({
                "id": second["id"],
                "expectedUpdatedAt": transferred["target"]["updatedAt"],
            }),
        )
        .unwrap();
        assert_eq!(
            call(&database, "get_voucher", json!({ "number": 2 })).unwrap(),
            Value::Null
        );
        assert_eq!(
            call(&database, "get_voucher_count", Value::Null).unwrap(),
            json!(1)
        );
    }

    fn save_cash_sale(database: &db::Database, number: i64, date: &str, amount: i64) -> Value {
        dispatch(
            database,
            "save_voucher",
            json!({
                "input": {
                    "id": null,
                    "number": number,
                    "date": date,
                    "voucherType": vouchers::VOUCHER_TYPES[0],
                    "description": "فروش نقدی",
                    "fiscalYearId": null,
                    "lines": [
                        {
                            "id": null,
                            "accountCode": "110101",
                            "detail1Code": null,
                            "description": "صندوق",
                            "debit": amount,
                            "credit": 0,
                        },
                        {
                            "id": null,
                            "accountCode": "410101",
                            "detail1Code": null,
                            "description": "فروش",
                            "debit": 0,
                            "credit": amount,
                        },
                    ],
                },
                "expectedUpdatedAt": null,
                "idempotencyKey": format!("report-save-{number}"),
            }),
            "voucher",
        )
        .unwrap()
    }

    #[test]
    fn report_pages_read_journal_ledger_income_and_balance() {
        let database = crate::db::memory_database();
        save_cash_sale(&database, 1, "1410/01/05", 1500);

        let journal = call(
            &database,
            "get_journal",
            json!({
                "from": "1410/01/01",
                "to": "1410/12/29",
                "search": "",
                "scope": "working",
                "offset": 0,
                "limit": 100,
            }),
        )
        .unwrap();
        assert_eq!(journal["rowCount"], json!(2));
        assert_eq!(journal["rows"].as_array().unwrap().len(), 2);

        let ledger = call(
            &database,
            "get_ledger",
            json!({
                "accountCode": "110101",
                "from": "1410/01/01",
                "to": "1410/12/29",
                "scope": "working",
                "offset": 0,
                "limit": 100,
            }),
        )
        .unwrap();
        assert_eq!(ledger["accountCode"], "110101");
        assert_eq!(ledger["rows"].as_array().unwrap().len(), 1);
        assert_eq!(ledger["closingDebit"], json!(1500));

        let income = call(
            &database,
            "get_income_statement",
            json!({
                "from": "1410/01/01",
                "to": "1410/12/29",
                "scope": "working",
            }),
        )
        .unwrap();
        assert_eq!(income["revenueTotal"], json!(1500));

        let balance = call(
            &database,
            "get_balance_sheet",
            json!({ "asOf": "1410/12/29", "scope": "working" }),
        )
        .unwrap();
        assert_eq!(balance["asOf"], "1410/12/29");
        assert!(!balance["assets"].as_array().unwrap().is_empty());
    }

    #[test]
    fn print_previews_store_the_server_document_for_the_preview_window() {
        let database = crate::db::memory_database();
        save_cash_sale(&database, 1, "1410/01/05", 1500);
        let print_docs = Mutex::new(HashMap::new());

        let opened = super::dispatch(
            &database,
            &print_docs,
            "open_voucher_preview",
            json!({ "number": 1 }),
            "voucher",
        )
        .unwrap();
        assert_eq!(opened["webName"], "print-preview");
        let url = opened["webOpen"].as_str().unwrap();
        let token = url.rsplit("doc=").next().unwrap();

        let document = super::dispatch(
            &database,
            &print_docs,
            "get_print_doc",
            json!({ "token": token }),
            "print-preview-web",
        )
        .unwrap();
        assert!(document["html"].as_str().unwrap().contains("سند حسابداری"));
        assert!(document["fileName"].as_str().unwrap().ends_with(".html"));

        // Only a preview tab receives the stored document.
        assert!(super::dispatch(
            &database,
            &print_docs,
            "get_print_doc",
            json!({ "token": token }),
            "main",
        )
        .is_err());

        let trial = super::dispatch(
            &database,
            &print_docs,
            "open_trial_balance_preview",
            json!({
                "from": "1410/01/01",
                "to": "1410/12/29",
                "scope": "working",
                "accountFrom": "",
                "accountTo": "",
                "showZero": true,
                "multiLevel": true,
                "balanceType": "شش ستونی",
                "showAccountCode": true,
                "showAccountName": true,
                "showPrintDateTime": true,
            }),
            "main",
        )
        .unwrap();
        assert_eq!(trial["webName"], "print-preview");

        let ledger = super::dispatch(
            &database,
            &print_docs,
            "open_ledger_preview",
            json!({
                "accountCode": "110101",
                "from": "1410/01/01",
                "to": "1410/12/29",
                "scope": "working",
            }),
            "voucher",
        )
        .unwrap();
        assert_eq!(ledger["webName"], "print-preview");
    }

    #[test]
    fn supplementary_statements_and_their_previews_use_the_shared_reports() {
        let database = crate::db::memory_database();
        save_cash_sale(&database, 1, "1410/01/05", 1500);
        let print_docs = Mutex::new(HashMap::new());

        let equity = call(
            &database,
            "get_equity_changes_statement",
            json!({ "asOf": "1410/12/29", "scope": "working" }),
        )
        .unwrap();
        assert_eq!(equity["toDate"], json!("1410/12/29"));
        assert!(!equity["statement"].as_str().unwrap().is_empty());

        let cogs = call(
            &database,
            "get_cogs_statement",
            json!({
                "from": "1410/01/01",
                "to": "1410/12/29",
                "scope": "working",
            }),
        )
        .unwrap();
        assert_eq!(cogs["toDate"], json!("1410/12/29"));

        let appropriation = call(
            &database,
            "get_profit_appropriation_statement",
            json!({ "asOf": "1410/12/29", "scope": "working" }),
        )
        .unwrap();
        assert_eq!(appropriation["toDate"], json!("1410/12/29"));

        let cash_flow = call(
            &database,
            "get_cash_flow_statement",
            json!({ "asOf": "1410/12/29", "scope": "working" }),
        )
        .unwrap();
        assert_eq!(cash_flow["toDate"], json!("1410/12/29"));

        let previews = [
            (
                "open_equity_changes_preview",
                json!({ "asOf": "1410/12/29", "scope": "working" }),
                "صورت تغییرات حقوق مالکانه",
            ),
            (
                "open_cogs_preview",
                json!({
                    "from": "1410/01/01",
                    "to": "1410/12/29",
                    "scope": "working",
                }),
                "بهای تمام‌شدهٔ کالای فروش‌رفته",
            ),
            (
                "open_profit_appropriation_preview",
                json!({ "asOf": "1410/12/29", "scope": "working" }),
                "صورت تخصیص سود",
            ),
            (
                "open_cash_flow_preview",
                json!({ "asOf": "1410/12/29", "scope": "working" }),
                "صورت جریان‌های نقدی",
            ),
        ];

        for (command, args, expected_title) in previews {
            let opened = super::dispatch(&database, &print_docs, command, args, "main").unwrap();
            assert_eq!(opened["webName"], "print-preview");
            let url = opened["webOpen"].as_str().unwrap();
            let token = url
                .strip_prefix("/print.html?doc=")
                .expect("preview token in the url");
            let document = super::dispatch(
                &database,
                &print_docs,
                "get_print_doc",
                json!({ "token": token }),
                "print-preview-web",
            )
            .unwrap();
            assert!(document["html"].as_str().unwrap().contains(expected_title));
            assert!(document["fileName"].as_str().unwrap().ends_with(".html"));
        }
    }

    #[test]
    fn quick_account_creation_is_limited_to_account_aware_windows() {
        let database = crate::db::memory_database();

        let account = dispatch(
            &database,
            "create_account",
            json!({
                "code": "110103",
                "name": "حساب آزمون وب",
                "level": 2,
                "groupCode": "1",
                "nature": "debtor",
                "parentCode": "1101",
                "isPostable": true,
                "isActive": true,
            }),
            "voucher",
        )
        .unwrap();
        assert_eq!(account["code"], "110103");
        assert_eq!(account["name"], "حساب آزمون وب");

        assert!(dispatch(
            &database,
            "create_account",
            json!({
                "code": "110104",
                "name": "حساب غیرمجاز",
                "level": 2,
                "groupCode": "1",
                "nature": "debtor",
                "parentCode": "1101",
                "isPostable": true,
            }),
            "trial-balance",
        )
        .is_err());
    }

    #[test]
    fn window_commands_return_browser_urls() {
        let database = crate::db::memory_database();

        let trial = call(&database, "open_trial_balance_window", Value::Null).unwrap();
        assert_eq!(trial["webOpen"], "/index.html?popup=trial-balance");
        assert_eq!(trial["webName"], "trial-balance");

        let groups = call(&database, "open_account_groups_window", Value::Null).unwrap();
        assert_eq!(groups["webOpen"], "/account-groups.html");
        assert_eq!(groups["webName"], "account-groups");

        let voucher = call(&database, "open_voucher_window", json!({ "number": 7 })).unwrap();
        assert_eq!(voucher["webOpen"], "/voucher.html?number=7");
        assert_eq!(voucher["webName"], "voucher");

        let new_voucher = dispatch(
            &database,
            "open_voucher_window",
            json!({ "number": null }),
            "voucher-list",
        )
        .unwrap();
        assert_eq!(new_voucher["webOpen"], "/voucher.html");

        assert!(dispatch(
            &database,
            "open_voucher_window",
            json!({ "number": 7 }),
            "trial-balance",
        )
        .is_err());
    }

    #[test]
    fn unknown_commands_and_foreign_windows_are_rejected() {
        let database = crate::db::memory_database();

        assert!(call(&database, "save_voucher_excel", Value::Null).is_err());
        assert!(dispatch(
            &database,
            "create_fiscal_year",
            json!({
                "title": "سال مالی نامعتبر",
                "startDate": "1411/01/01",
                "endDate": "1411/12/30",
            }),
            "trial-balance",
        )
        .is_err());
    }
}
