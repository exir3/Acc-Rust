//! Local-only HTTP host for the accounting UI.
//!
//! The desktop app and this browser mode share the same SQLite schema and the
//! same Rust storage layer. The browser gets a small JSON command endpoint
//! that mirrors the part of the Tauri command surface needed by the local-web
//! slices so far: settings, the chart of accounts, fiscal years including
//! year-end closing and carry forward, the voucher list, the voucher form
//! including save, permanent, delete and line transfer, the main and
//! supplementary report pages, their print previews, and the browser
//! backup/restore transfer endpoints.
//!
//! The listener is bound to `127.0.0.1` only. There is no authentication and no
//! remote access; the server is meant for the same Windows session as the
//! desktop application.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Path as UrlPath, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tower_http::services::ServeDir;

use crate::commands::print::{ensure_printable, report_scope};
use crate::commands::state::report_meta_for;
use crate::db;
use crate::excel;
use crate::report::{self, PrintDoc};
use crate::vouchers::{self, ReportScope, VoucherInput, VoucherTransferInput};

const DEFAULT_PORT: u16 = 4174;

/// The local host builds print previews per request. Each document gets an
/// opaque token that only the preview tab receives through its URL.
static PRINT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
const MAX_STORED_PRINT_DOCS: usize = 16;

/// Backups and restore uploads are handed to the browser through one-time
/// links instead of exposing arbitrary paths on the loopback server.
const DOWNLOAD_TTL: Duration = Duration::from_secs(10 * 60);
const UPLOAD_TTL: Duration = Duration::from_secs(15 * 60);
const MAX_RESTORE_BYTES: usize = 512 * 1024 * 1024;
/// Excel workbooks are capped well below the backup limit because the parser
/// already rejects anything larger than 32 MB; the upload limit mirrors it.
const MAX_EXCEL_BYTES: usize = 32 * 1024 * 1024;
const EXCEL_EXTENSIONS: &[&str] = &["xlsx", "xlsm", "xls", "xlsb", "ods"];

#[derive(Clone)]
struct WebState {
    database: Arc<db::Database>,
    print_docs: Arc<Mutex<HashMap<String, PrintDoc>>>,
    transfers: Transfers,
}

/// State shared by every request. Besides the one-time download and upload
/// maps, it keeps the parsed Excel sessions between preview and confirmation,
/// exactly like the desktop `AppState` slots.
#[derive(Clone, Default)]
struct Transfers {
    downloads: Arc<Mutex<HashMap<String, DownloadEntry>>>,
    uploads: Arc<Mutex<HashMap<String, UploadEntry>>>,
    excel_uploads: Arc<Mutex<HashMap<String, ExcelUploadEntry>>>,
    restore_in_progress: Arc<Mutex<bool>>,
    sessions: Arc<Sessions>,
}

#[derive(Default)]
struct Sessions {
    account_import_preview: Mutex<Option<excel::AccountImportSession>>,
    bulk_voucher_import: Mutex<Option<excel::BulkVoucherImport>>,
    bulk_import_cancel: AtomicBool,
}

struct DownloadEntry {
    path: PathBuf,
    file_name: Option<String>,
    created_at: SystemTime,
}

struct UploadEntry {
    path: PathBuf,
    encrypted: bool,
    file_sha256: String,
    created_at: SystemTime,
}

struct ExcelUploadEntry {
    path: PathBuf,
    file_name: String,
    created_at: SystemTime,
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
struct PasswordArgs {
    password: String,
}

#[derive(Debug, Deserialize)]
struct RestoreArgs {
    token: String,
    password: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuditLogArgs {
    limit: Option<i64>,
    offset: Option<i64>,
    entity_type: Option<String>,
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
#[serde(rename_all = "camelCase")]
struct SetFiscalYearStatusArgs {
    id: i64,
    status: String,
    reason: Option<String>,
    confirm: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FiscalYearIdArgs {
    id: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClosingPreviewArgs {
    year_id: i64,
    as_of_date: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClosingRolesArgs {
    year_id: i64,
    income_summary_code: Option<String>,
    retained_earnings_code: Option<String>,
    closing_balance_code: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecuteClosingArgs {
    year_id: i64,
    as_of_date: Option<String>,
    income_summary_code: Option<String>,
    retained_earnings_code: Option<String>,
    closing_balance_code: Option<String>,
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReverseClosingArgs {
    year_id: i64,
    reason: Option<String>,
    force: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CarryForwardArgs {
    source_year_id: i64,
    target_year_id: i64,
    force: Option<bool>,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveSettingsArgs {
    company_name: String,
    #[serde(default)]
    fiscal_year: String,
}

/// Both single-voucher and account imports receive a short-lived upload token
/// instead of a server path chosen by the page.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExcelUploadArgs {
    upload_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BulkAnalyzeArgs {
    upload_token: String,
    date_system: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportVoucherArgs {
    input: VoucherInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportAccountArgs {
    preview_token: String,
    mode: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommitBulkArgs {
    #[serde(default)]
    renumber_conflicts: bool,
    group_overrides: Option<Vec<excel::GroupMappingOverride>>,
    confirm_inferred_groups: Option<bool>,
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
        transfers: Transfers::default(),
    };
    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/command", post(command))
        .route("/api/backup/{token}", get(download_backup))
        .route("/api/excel/{token}", get(download_excel))
        .route(
            "/api/restore/upload",
            post(upload_restore).layer(DefaultBodyLimit::max(MAX_RESTORE_BYTES)),
        )
        .route(
            "/api/excel/upload",
            post(upload_excel).layer(DefaultBodyLimit::max(MAX_EXCEL_BYTES)),
        )
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
    let transfers = state.transfers.clone();

    let result = tokio::task::spawn_blocking(move || {
        dispatch(&database, &print_docs, &transfers, &name, args, &window)
    })
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
    transfers: &Transfers,
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
        "save_settings" => {
            require_window(window, &["main"])?;
            let args: SaveSettingsArgs = parse_args(args)?;
            to_value(database.save_settings(&args.company_name, &args.fiscal_year)?)
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
        "set_fiscal_year_status" => {
            require_window(window, &["main"])?;
            let args: SetFiscalYearStatusArgs = parse_args(args)?;
            let year = database.set_fiscal_year_status(
                args.id,
                &args.status,
                args.reason.as_deref(),
                args.confirm.unwrap_or(false),
            )?;
            to_value(year)
        }
        "delete_fiscal_year" => {
            require_window(window, &["main"])?;
            let args: FiscalYearIdArgs = parse_args(args)?;
            database.delete_fiscal_year(args.id)?;
            Ok(Value::Null)
        }
        "get_closing_preview" => {
            require_window(window, &["main"])?;
            let args: ClosingPreviewArgs = parse_args(args)?;
            to_value(database.get_closing_preview(args.year_id, args.as_of_date.as_deref())?)
        }
        "assign_closing_roles" => {
            require_window(window, &["main"])?;
            let args: ClosingRolesArgs = parse_args(args)?;
            to_value(database.assign_closing_roles(
                args.year_id,
                args.income_summary_code.as_deref(),
                args.retained_earnings_code.as_deref(),
                args.closing_balance_code.as_deref(),
            )?)
        }
        "execute_closing" => {
            require_window(window, &["main"])?;
            let args: ExecuteClosingArgs = parse_args(args)?;
            to_value(database.execute_closing(
                args.year_id,
                args.as_of_date.as_deref(),
                args.income_summary_code.as_deref(),
                args.retained_earnings_code.as_deref(),
                args.closing_balance_code.as_deref(),
                args.reason.as_deref(),
            )?)
        }
        "reverse_closing" => {
            require_window(window, &["main"])?;
            let args: ReverseClosingArgs = parse_args(args)?;
            to_value(database.reverse_closing(
                args.year_id,
                args.reason.as_deref(),
                args.force.unwrap_or(false),
            )?)
        }
        "get_carry_forward_preview" => {
            require_window(window, &["main"])?;
            let args: CarryForwardArgs = parse_args(args)?;
            to_value(database.get_carry_forward_preview(args.source_year_id, args.target_year_id)?)
        }
        "execute_carry_forward" => {
            require_window(window, &["main"])?;
            let args: CarryForwardArgs = parse_args(args)?;
            to_value(database.execute_carry_forward(
                args.source_year_id,
                args.target_year_id,
                args.force.unwrap_or(false),
            )?)
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
        "export_account_excel" => {
            require_window(window, &["main", "account-groups", "account-definition"])?;
            let (path, file_name) = excel_export_path(database, "لیست-حساب‌ها.xlsx")?;
            let result = excel::export_accounts_to(database, &path)?;
            let mut value = excel_download(transfers, PathBuf::from(&result.path), file_name)?;
            value["skippedCodes"] = to_value(result.skipped_codes)?;
            Ok(value)
        }
        "preview_account_excel_import" => {
            require_window(window, &["main", "account-groups", "account-definition"])?;
            let args: ExcelUploadArgs = parse_args(args)?;
            let upload = pending_excel_upload(&transfers.excel_uploads, &args.upload_token)?;
            let (preview, session) = excel::preview_accounts_at(database, &upload.path)?;
            *lock(&transfers.sessions.account_import_preview) = Some(session);
            to_value(preview)
        }
        "import_account_excel" => {
            require_window(window, &["main", "account-groups", "account-definition"])?;
            let args: ImportAccountArgs = parse_args(args)?;
            // Consume the session only when the token matches, so a stale token
            // cannot apply a plan that was never previewed in this process.
            let session = {
                let mut guard = lock(&transfers.sessions.account_import_preview);
                match guard.as_ref() {
                    Some(session) if session.token == args.preview_token => guard.take(),
                    _ => None,
                }
            };
            let Some(session) = session else {
                return Err(
                    "پیش‌نمایش ورود اکسل معتبر نیست یا منقضی شده است؛ دوباره پیش‌نمایش بگیرید."
                        .to_string(),
                );
            };
            let upload_path = session.canonical_path.clone();
            let result = excel::import_accounts(database, session, &args.mode);
            remove_excel_upload(&transfers.excel_uploads, &upload_path);
            to_value(result?)
        }
        "export_voucher_excel" => {
            require_window(window, &["main", "voucher"])?;
            let args: ExportVoucherArgs = parse_args(args)?;
            database.validate_voucher_accounts(args.input.id, &args.input.lines)?;
            let accounts = database.list_accounts()?.accounts;
            let (path, file_name) =
                excel_export_path(database, &excel::voucher_export_file_name(&args.input))?;
            let saved = excel::export_voucher_to(&args.input, &accounts, &path)?;
            excel_download(transfers, PathBuf::from(saved), file_name)
        }
        "import_voucher_excel" => {
            require_window(window, &["main", "voucher"])?;
            let args: ExcelUploadArgs = parse_args(args)?;
            let upload = pending_excel_upload(&transfers.excel_uploads, &args.upload_token)?;
            let parsed = excel::import_voucher_from_path(&upload.path);
            remove_excel_upload(&transfers.excel_uploads, &upload.path);
            to_value(parsed?)
        }
        "analyze_bulk_voucher_excel" => {
            require_window(window, &["main"])?;
            let args: BulkAnalyzeArgs = parse_args(args)?;
            let date_system = excel::DateSystem::parse(args.date_system.as_deref())?;
            *lock(&transfers.sessions.bulk_voucher_import) = None;
            transfers
                .sessions
                .bulk_import_cancel
                .store(false, Ordering::Relaxed);
            let upload = pending_excel_upload(&transfers.excel_uploads, &args.upload_token)?;
            let sessions = transfers.sessions.clone();
            let parsed = excel::parse_bulk_vouchers_with_progress(
                &upload.path,
                date_system,
                &mut |_, _| {},
                &|| sessions.bulk_import_cancel.load(Ordering::Relaxed),
            );
            // The parsed import is held in memory; the uploaded workbook is no
            // longer needed once analysis succeeds or fails.
            remove_excel_upload(&transfers.excel_uploads, &upload.path);
            let import = parsed?;
            let preview = database.preview_bulk_import(&import)?;
            *lock(&transfers.sessions.bulk_voucher_import) = Some(import);
            to_value(preview)
        }
        "cancel_bulk_voucher_import" => {
            require_window(window, &["main"])?;
            transfers
                .sessions
                .bulk_import_cancel
                .store(true, Ordering::Relaxed);
            Ok(Value::Null)
        }
        "commit_bulk_voucher_excel" => {
            require_window(window, &["main"])?;
            let args: CommitBulkArgs = parse_args(args)?;
            let import = lock(&transfers.sessions.bulk_voucher_import)
                .clone()
                .ok_or_else(|| "ابتدا فایل اکسل را انتخاب و تحلیل کنید.".to_string())?;
            let group_overrides = args.group_overrides.unwrap_or_default();
            // A guessed group must be explicitly confirmed by the operator.
            // Every inferred account needs a final mapping, even when the value
            // is unchanged, so no mapping can be left unconfirmed.
            let inferred_codes: std::collections::HashSet<String> = import
                .accounts
                .iter()
                .filter(|account| account.group_is_inferred)
                .map(|account| crate::db::normalize_code(&account.code))
                .collect();
            if !inferred_codes.is_empty() {
                if !args.confirm_inferred_groups.unwrap_or(false) {
                    return Err(
                        "گروه حساب‌های حدس‌زده‌شده باید پیش از ثبت بررسی و تأیید شود.".to_string()
                    );
                }
                let provided: std::collections::HashSet<String> = group_overrides
                    .iter()
                    .map(|mapping| crate::db::normalize_code(&mapping.account_code))
                    .collect();
                if let Some(missing) = inferred_codes.iter().find(|code| !provided.contains(*code))
                {
                    return Err(format!(
                        "نگاشت گروه حساب {} تأیید نشده است؛ همه نگاشت‌های حدسی را بررسی کنید.",
                        crate::db::to_fa_digits(missing)
                    ));
                }
            }
            transfers
                .sessions
                .bulk_import_cancel
                .store(false, Ordering::Relaxed);
            let sessions = transfers.sessions.clone();
            let imported_count = database.import_vouchers_with_progress(
                import,
                args.renumber_conflicts,
                &group_overrides,
                &mut |_, _| {},
                &|| sessions.bulk_import_cancel.load(Ordering::Relaxed),
            )?;
            *lock(&transfers.sessions.bulk_voucher_import) = None;
            to_value(imported_count)
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
        "get_audit_logs" => {
            require_window(window, &["main"])?;
            let args: AuditLogArgs = parse_args(args)?;
            to_value(database.list_audit_logs(
                args.limit,
                args.offset,
                args.entity_type.as_deref(),
            )?)
        }
        "backup_database" => {
            require_window(window, &["main"])?;
            let (path, file_name) = create_backup(database, None)?;
            backup_download(transfers, path, file_name)
        }
        "backup_database_encrypted" => {
            require_window(window, &["main"])?;
            let args: PasswordArgs = parse_args(args)?;
            db::validate_portable_password(&args.password)?;
            let (path, file_name) = create_backup(database, Some(&args.password))?;
            backup_download(transfers, path, file_name)
        }
        "restore_database_file" => {
            require_window(window, &["main"])?;
            let args: RestoreArgs = parse_args(args)?;
            let _restore_guard = begin_restore(&transfers.restore_in_progress)?;
            let upload = pending_upload(&transfers.uploads, &args.token)?;
            let current_hash = db::sha256_file(&upload.path)?;
            if current_hash != upload.file_sha256 {
                return Err(
                    "فایل پشتیبان پس از انتخاب تغییر کرده است و بازیابی انجام نشد.".to_string(),
                );
            }
            let password = if upload.encrypted {
                Some(
                    args.password
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| {
                            "این پشتیبان رمزنگاری‌شده است؛ رمز عبور را وارد کنید.".to_string()
                        })?,
                )
            } else {
                None
            };
            let database_path = PathBuf::from(database.path_string());
            let safety_path = match password.as_deref() {
                Some(password) => database.restore_encrypted_from(&upload.path, password)?,
                None => database.restore_from(&upload.path)?,
            };
            remove_upload(&transfers.uploads, &upload.path);
            let _ = db::prune_restore_safety_files(&database_path);
            Ok(Value::String(safety_path.to_string_lossy().to_string()))
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

fn lock<T>(slot: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn create_backup(
    database: &db::Database,
    password: Option<&str>,
) -> Result<(PathBuf, String), String> {
    let database_path = PathBuf::from(database.path_string());
    let backup_dir = database_path
        .parent()
        .ok_or_else(|| "مسیر پوشه پایگاه داده پیدا نشد.".to_string())?
        .join("backups");
    std::fs::create_dir_all(&backup_dir)
        .map_err(|error| format!("ساخت پوشه پشتیبان ناموفق بود: {error}"))?;
    let stamp = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("زمان سیستم معتبر نیست: {error}"))?
        .as_nanos();
    let file_name = match password {
        Some(_) => format!("hesabdari-backup-{stamp}.{}", db::PORTABLE_EXTENSION),
        None => format!("hesabdari-backup-{stamp}.sqlite3"),
    };
    let path = backup_dir.join(&file_name);
    match password {
        Some(password) => database.backup_encrypted_to(&path, password)?,
        None => database.backup_to(&path)?,
    }
    let _ = db::prune_backup_files(&backup_dir);
    Ok((path, file_name))
}

/// Hands a freshly written backup to the browser once without exposing an
/// arbitrary file path on the server. The desktop UI still receives a path,
/// so the bridge downloads `webDownload` and returns `path` to the caller.
fn backup_download(
    transfers: &Transfers,
    path: PathBuf,
    file_name: String,
) -> Result<Value, String> {
    let token = format!("backup-{}", db::random_hex(16)?);
    {
        let mut downloads = lock(&transfers.downloads);
        prune_downloads(&mut downloads);
        downloads.insert(
            token.clone(),
            DownloadEntry {
                path: path.clone(),
                file_name: Some(file_name.clone()),
                created_at: SystemTime::now(),
            },
        );
    }
    Ok(json!({
        "path": path.to_string_lossy(),
        "fileName": file_name,
        "webDownload": format!("/api/backup/{token}"),
    }))
}

/// Creates a uniquely named workbook in the data folder. The name shown to the
/// browser stays clean while the file on disk never collides with a parallel
/// export or a download that is still in flight.
fn excel_export_path(
    database: &db::Database,
    base_name: &str,
) -> Result<(PathBuf, String), String> {
    let database_path = PathBuf::from(database.path_string());
    let directory = database_path
        .parent()
        .ok_or_else(|| "مسیر پوشه پایگاه داده پیدا نشد.".to_string())?
        .join("exports");
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("ساخت پوشه خروجی اکسل ناموفق بود: {error}"))?;
    let token = db::random_hex(8)?;
    Ok((
        directory.join(format!("export-{token}.xlsx")),
        base_name.to_string(),
    ))
}

/// Hands a freshly written workbook to the browser through a one-time link.
/// The returned `path` is the download name because the browser, not the
/// server, chooses the final folder.
fn excel_download(
    transfers: &Transfers,
    path: PathBuf,
    file_name: String,
) -> Result<Value, String> {
    let token = format!("excel-{}", db::random_hex(16)?);
    {
        let mut downloads = lock(&transfers.downloads);
        prune_downloads(&mut downloads);
        downloads.insert(
            token.clone(),
            DownloadEntry {
                path,
                file_name: Some(file_name.clone()),
                created_at: SystemTime::now(),
            },
        );
    }
    Ok(json!({
        "path": file_name,
        "fileName": file_name,
        "webDownload": format!("/api/excel/{token}"),
    }))
}

fn prune_downloads(entries: &mut HashMap<String, DownloadEntry>) {
    let now = SystemTime::now();
    entries.retain(|_, entry| {
        now.duration_since(entry.created_at)
            .map(|age| age <= DOWNLOAD_TTL)
            .unwrap_or(false)
    });
}

fn prune_uploads(entries: &mut HashMap<String, UploadEntry>) {
    let now = SystemTime::now();
    entries.retain(|_, entry| {
        let fresh = now
            .duration_since(entry.created_at)
            .map(|age| age <= UPLOAD_TTL)
            .unwrap_or(false);
        if !fresh {
            let _ = std::fs::remove_file(&entry.path);
        }
        fresh
    });
}

fn prune_excel_uploads(entries: &mut HashMap<String, ExcelUploadEntry>) {
    let now = SystemTime::now();
    entries.retain(|_, entry| {
        let fresh = now
            .duration_since(entry.created_at)
            .map(|age| age <= UPLOAD_TTL)
            .unwrap_or(false);
        if !fresh {
            let _ = std::fs::remove_file(&entry.path);
        }
        fresh
    });
}

fn pending_excel_upload(
    uploads: &Mutex<HashMap<String, ExcelUploadEntry>>,
    token: &str,
) -> Result<ExcelUploadEntry, String> {
    let mut entries = lock(uploads);
    prune_excel_uploads(&mut entries);
    let entry = entries
        .get(token)
        .filter(|entry| entry.path.exists())
        .ok_or_else(|| "فایل اکسل انتخاب‌شده منقضی شده است؛ دوباره انتخاب کنید.".to_string())?;
    Ok(ExcelUploadEntry {
        path: entry.path.clone(),
        file_name: entry.file_name.clone(),
        created_at: entry.created_at,
    })
}

fn remove_excel_upload(uploads: &Mutex<HashMap<String, ExcelUploadEntry>>, path: &Path) {
    let mut entries = lock(uploads);
    entries.retain(|_, entry| entry.path != path);
    let _ = std::fs::remove_file(path);
}

fn pending_upload(
    uploads: &Mutex<HashMap<String, UploadEntry>>,
    token: &str,
) -> Result<UploadEntry, String> {
    let mut entries = lock(uploads);
    prune_uploads(&mut entries);
    let entry = entries
        .get(token)
        .ok_or_else(|| "فایل پشتیبان انتخاب‌شده منقضی شده است؛ دوباره انتخاب کنید.".to_string())?;
    Ok(UploadEntry {
        path: entry.path.clone(),
        encrypted: entry.encrypted,
        file_sha256: entry.file_sha256.clone(),
        created_at: entry.created_at,
    })
}

fn remove_upload(uploads: &Mutex<HashMap<String, UploadEntry>>, path: &Path) {
    let mut entries = lock(uploads);
    entries.retain(|_, entry| entry.path != path);
    let _ = std::fs::remove_file(path);
}

/// Keeps `restore_in_progress` set for the whole restore, including early
/// errors, so two browser tabs cannot replace the database at the same time.
struct RestoreInProgress<'a>(&'a Mutex<bool>);

impl Drop for RestoreInProgress<'_> {
    fn drop(&mut self) {
        *lock(self.0) = false;
    }
}

fn begin_restore(slot: &Mutex<bool>) -> Result<RestoreInProgress<'_>, String> {
    let mut running = lock(slot);
    if *running {
        return Err("یک بازیابی دیگر در حال انجام است؛ تا پایان آن صبر کنید.".to_string());
    }
    *running = true;
    Ok(RestoreInProgress(slot))
}

fn decode_file_name(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_digit(bytes[index + 1]), hex_digit(bytes[index + 2]))
            {
                decoded.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).to_string()
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

/// The uploaded name is display metadata only; the server stores the bytes
/// under its own token. Strip path separators and control characters anyway.
fn sanitize_upload_name(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .filter(|ch| !ch.is_control() && *ch != '/' && *ch != '\\')
        .take(120)
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "backup".to_string()
    } else {
        trimmed.to_string()
    }
}

async fn download_backup(
    State(state): State<WebState>,
    UrlPath(token): UrlPath<String>,
) -> Response {
    serve_download(
        &state.transfers,
        &token,
        "پشتیبان درخواستی در دسترس نیست؛ دوباره پشتیبان بگیرید.",
        "فایل پشتیبان پیدا نشد.",
    )
    .await
}

async fn download_excel(
    State(state): State<WebState>,
    UrlPath(token): UrlPath<String>,
) -> Response {
    serve_download(
        &state.transfers,
        &token,
        "فایل اکسل درخواستی در دسترس نیست؛ دوباره خروجی بگیرید.",
        "فایل اکسل پیدا نشد.",
    )
    .await
}

async fn serve_download(
    transfers: &Transfers,
    token: &str,
    missing: &str,
    unreadable: &str,
) -> Response {
    let entry = {
        let mut downloads = lock(&transfers.downloads);
        prune_downloads(&mut downloads);
        downloads
            .get(token)
            .map(|entry| (entry.path.clone(), entry.file_name.clone()))
    };
    let Some((path, file_name)) = entry else {
        return transfer_error(StatusCode::NOT_FOUND, missing);
    };
    let file_name = file_name.unwrap_or_else(|| {
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("hesabdari-export")
            .to_string()
    });
    match tokio::task::spawn_blocking(move || std::fs::read(&path)).await {
        Ok(Ok(bytes)) => {
            let mut response = Response::new(Body::from(bytes));
            response.headers_mut().insert(
                CONTENT_TYPE,
                axum::http::HeaderValue::from_static("application/octet-stream"),
            );
            response.headers_mut().insert(
                CACHE_CONTROL,
                axum::http::HeaderValue::from_static("no-store"),
            );
            if let Some(value) = content_disposition(&file_name) {
                response.headers_mut().insert(CONTENT_DISPOSITION, value);
            }
            response
        }
        _ => transfer_error(StatusCode::NOT_FOUND, unreadable),
    }
}

/// Builds an RFC 6266 attachment header. Persian file names are percent-encoded
/// in `filename*`; a plain ASCII fallback keeps older browsers working.
fn content_disposition(file_name: &str) -> Option<axum::http::HeaderValue> {
    let mut encoded = String::new();
    for byte in file_name.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(*byte as char);
            }
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    axum::http::HeaderValue::from_str(&format!(
        "attachment; filename=\"export.bin\"; filename*=UTF-8''{encoded}"
    ))
    .ok()
}

async fn upload_restore(
    State(state): State<WebState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if body.is_empty() {
        return transfer_error(StatusCode::BAD_REQUEST, "فایل پشتیبان خالی است.");
    }
    let file_name = headers
        .get("x-file-name")
        .and_then(|value| value.to_str().ok())
        .map(decode_file_name)
        .map(|value| sanitize_upload_name(&value))
        .unwrap_or_else(|| "backup".to_string());
    let database_path = PathBuf::from(state.database.path_string());
    let Some(parent) = database_path.parent() else {
        return transfer_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "مسیر پوشه پایگاه داده پیدا نشد.",
        );
    };
    let directory = parent.join("restore-uploads");
    let Ok(token) = db::random_hex(16) else {
        return transfer_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "ساخت شناسهٔ بازیابی ناموفق بود.",
        );
    };
    let path = directory.join(format!("restore-{token}.upload"));
    let bytes = body.to_vec();
    let target = path.clone();
    let stored = tokio::task::spawn_blocking(move || -> Result<(String, bool), String> {
        std::fs::create_dir_all(&directory)
            .map_err(|error| format!("ساخت پوشهٔ بازیابی ناموفق بود: {error}"))?;
        // Uploads left behind by a crashing or closed tab must not accumulate.
        if let Ok(items) = std::fs::read_dir(&directory) {
            for item in items.flatten() {
                let stale = item
                    .metadata()
                    .ok()
                    .and_then(|meta| meta.modified().ok())
                    .and_then(|modified| modified.elapsed().ok())
                    .map(|age| age > UPLOAD_TTL)
                    .unwrap_or(false);
                if stale {
                    let _ = std::fs::remove_file(item.path());
                }
            }
        }
        std::fs::write(&target, &bytes)
            .map_err(|error| format!("ذخیرهٔ فایل پشتیبان ناموفق بود: {error}"))?;
        let file_sha256 = db::sha256_file(&target)?;
        let encrypted = db::is_encrypted_backup(&target)?;
        Ok((file_sha256, encrypted))
    })
    .await;
    let (file_sha256, encrypted) = match stored {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => return transfer_error(StatusCode::BAD_REQUEST, &error),
        Err(error) => {
            return transfer_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("خواندن فایل پشتیبان ناموفق بود: {error}"),
            )
        }
    };
    {
        let mut uploads = lock(&state.transfers.uploads);
        prune_uploads(&mut uploads);
        uploads.insert(
            token.clone(),
            UploadEntry {
                path,
                encrypted,
                file_sha256,
                created_at: SystemTime::now(),
            },
        );
    }
    Json(json!({
        "ok": true,
        "data": {
            "token": token,
            "encrypted": encrypted,
            "fileName": file_name,
        },
    }))
    .into_response()
}

fn transfer_error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "ok": false, "error": message }))).into_response()
}

/// Receives a workbook chosen in the browser. Only the extension and size are
/// checked here; the Excel parser performs its own archive and dimension
/// validation before reading any data.
async fn upload_excel(State(state): State<WebState>, headers: HeaderMap, body: Bytes) -> Response {
    if body.is_empty() {
        return transfer_error(StatusCode::BAD_REQUEST, "فایل اکسل خالی است.");
    }
    let file_name = headers
        .get("x-file-name")
        .and_then(|value| value.to_str().ok())
        .map(decode_file_name)
        .map(|value| sanitize_upload_name(&value))
        .unwrap_or_else(|| "workbook.xlsx".to_string());
    let extension = Path::new(&file_name)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    if !EXCEL_EXTENSIONS.contains(&extension.as_str()) {
        return transfer_error(
            StatusCode::BAD_REQUEST,
            "پسوند فایل اکسل باید یکی از xlsx، xlsm، xls، xlsb یا ods باشد.",
        );
    }
    let database_path = PathBuf::from(state.database.path_string());
    let Some(parent) = database_path.parent() else {
        return transfer_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "مسیر پوشه پایگاه داده پیدا نشد.",
        );
    };
    let directory = parent.join("excel-uploads");
    let Ok(token) = db::random_hex(16) else {
        return transfer_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "ساخت شناسهٔ فایل اکسل ناموفق بود.",
        );
    };
    let path = directory.join(format!("excel-{token}.{extension}"));
    let bytes = body.to_vec();
    let target = path.clone();
    let stored = tokio::task::spawn_blocking(move || -> Result<(), String> {
        std::fs::create_dir_all(&directory)
            .map_err(|error| format!("ساخت پوشهٔ فایل‌های اکسل ناموفق بود: {error}"))?;
        // Uploads left behind by a closed tab must not accumulate.
        if let Ok(items) = std::fs::read_dir(&directory) {
            for item in items.flatten() {
                let stale = item
                    .metadata()
                    .ok()
                    .and_then(|meta| meta.modified().ok())
                    .and_then(|modified| modified.elapsed().ok())
                    .map(|age| age > UPLOAD_TTL)
                    .unwrap_or(false);
                if stale {
                    let _ = std::fs::remove_file(item.path());
                }
            }
        }
        std::fs::write(&target, &bytes)
            .map_err(|error| format!("ذخیرهٔ فایل اکسل ناموفق بود: {error}"))
    })
    .await;
    match stored {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return transfer_error(StatusCode::BAD_REQUEST, &error),
        Err(error) => {
            return transfer_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("ذخیرهٔ فایل اکسل ناموفق بود: {error}"),
            )
        }
    }
    {
        let mut uploads = lock(&state.transfers.excel_uploads);
        prune_excel_uploads(&mut uploads);
        uploads.insert(
            token.clone(),
            ExcelUploadEntry {
                path,
                file_name: file_name.clone(),
                created_at: SystemTime::now(),
            },
        );
    }
    Json(json!({
        "ok": true,
        "data": {
            "token": token,
            "fileName": file_name,
        },
    }))
    .into_response()
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
        let transfers = Transfers::default();
        super::dispatch(database, &print_docs, &transfers, name, args, window)
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

        let closed = call(
            &database,
            "set_fiscal_year_status",
            json!({ "id": updated["id"], "status": "closed", "reason": null, "confirm": true }),
        )
        .unwrap();
        assert_eq!(closed["status"], "closed");

        let reopened = call(
            &database,
            "set_fiscal_year_status",
            json!({ "id": updated["id"], "status": "open", "reason": null, "confirm": true }),
        )
        .unwrap();
        assert_eq!(reopened["status"], "open");

        call(
            &database,
            "delete_fiscal_year",
            json!({ "id": reopened["id"] }),
        )
        .unwrap();
        let after_delete = call(&database, "list_fiscal_years", Value::Null).unwrap();
        assert_eq!(
            after_delete["years"].as_array().unwrap().len(),
            before_count
        );
    }

    #[test]
    fn closing_and_carry_forward_run_through_the_browser_commands() {
        let database = crate::db::memory_database();
        let year = call(
            &database,
            "create_fiscal_year",
            json!({
                "title": "سال مالی بستن وب",
                "startDate": "1410/01/01",
                "endDate": "1410/12/29",
                "status": "open",
            }),
        )
        .unwrap();
        let year_id = year["id"].as_i64().unwrap();
        let saved = save_cash_sale(&database, 1, "1410/01/05", 1500);
        call(
            &database,
            "set_voucher_permanent",
            json!({ "id": saved["id"], "expectedUpdatedAt": saved["updatedAt"] }),
        )
        .unwrap();

        let preview = call(
            &database,
            "get_closing_preview",
            json!({ "yearId": year_id }),
        )
        .unwrap();
        assert_eq!(preview["canExecute"], true, "{preview}");
        assert_eq!(preview["netIncome"], 1500);
        assert!(!preview["tempLines"].as_array().unwrap().is_empty());

        let roles = call(
            &database,
            "assign_closing_roles",
            json!({
                "yearId": year_id,
                "incomeSummaryCode": null,
                "retainedEarningsCode": null,
                "closingBalanceCode": null,
            }),
        )
        .unwrap();
        assert!(roles["incomeSummary"]["code"].is_string());
        assert!(roles["retainedEarnings"]["code"].is_string());

        let closing_args = json!({
            "yearId": year_id,
            "asOfDate": null,
            "incomeSummaryCode": null,
            "retainedEarningsCode": null,
            "closingBalanceCode": null,
            "reason": "بستن آزمون وب",
        });
        let result = call(&database, "execute_closing", closing_args.clone()).unwrap();
        assert_eq!(result["netIncome"], 1500);
        assert!(result["tempVoucher"]["number"].is_i64());
        assert!(result["closingVoucher"]["number"].is_i64());

        let closed = call(&database, "list_fiscal_years", Value::Null).unwrap();
        let closed_year = closed["years"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == json!(year_id))
            .unwrap();
        assert_eq!(closed_year["status"], "closed");

        let reversed = call(
            &database,
            "reverse_closing",
            json!({ "yearId": year_id, "reason": "برگشت آزمون وب", "force": false }),
        )
        .unwrap();
        assert_eq!(reversed["deletedVouchers"], 2);

        // Carry forward needs an executed close; close again, then plan and run it.
        let target = call(
            &database,
            "create_fiscal_year",
            json!({
                "title": "سال مالی انتقال وب",
                "startDate": "1411/01/01",
                "endDate": "1411/12/29",
                "status": "open",
            }),
        )
        .unwrap();
        let target_id = target["id"].as_i64().unwrap();
        call(&database, "execute_closing", closing_args).unwrap();

        let plan = call(
            &database,
            "get_carry_forward_preview",
            json!({ "sourceYearId": year_id, "targetYearId": target_id }),
        )
        .unwrap();
        assert!(plan["errors"].as_array().unwrap().is_empty(), "{plan}");
        assert_eq!(plan["totals"]["debit"], 1500);
        assert_eq!(plan["totals"]["credit"], 1500);

        let opening = call(
            &database,
            "execute_carry_forward",
            json!({ "sourceYearId": year_id, "targetYearId": target_id, "force": false }),
        )
        .unwrap();
        assert!(opening["voucherNumber"].is_i64());
        assert_eq!(opening["accountCount"], 2);

        // Closing commands stay limited to the main window.
        assert!(dispatch(
            &database,
            "get_closing_preview",
            json!({ "yearId": year_id }),
            "voucher",
        )
        .is_err());
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

    fn add_excel_upload(transfers: &Transfers, token: &str, path: PathBuf, file_name: &str) {
        lock(&transfers.excel_uploads).insert(
            token.to_string(),
            ExcelUploadEntry {
                path,
                file_name: file_name.to_string(),
                created_at: SystemTime::now(),
            },
        );
    }

    fn excel_download_path(transfers: &Transfers, download: &Value) -> PathBuf {
        let url = download["webDownload"].as_str().expect("webDownload");
        let token = url.strip_prefix("/api/excel/").expect("excel download url");
        lock(&transfers.downloads)
            .get(token)
            .expect("download entry")
            .path
            .clone()
    }

    fn write_bulk_workbook(path: &Path) {
        let mut workbook = rust_xlsxwriter::Workbook::new();
        let sheet = workbook.add_worksheet();
        let headers = [
            "شماره سند",
            "تاریخ",
            "نوع سند",
            "شرح سند",
            "کد گروه",
            "کد کل",
            "عنوان کل",
            "ماهیت کل",
            "کد معین",
            "عنوان معین",
            "ماهیت معین",
            "بدهکار",
            "بستانکار",
        ];
        for (column, header) in headers.iter().enumerate() {
            sheet
                .write_string(0, column as u16, *header)
                .expect("header");
        }
        let rows = [
            [
                "500",
                "1410/02/01",
                "",
                "ورود دسته‌جمعی وب",
                "1",
                "95",
                "کل آزمون وب",
                "بدهکار",
                "9501",
                "معین آزمون وب",
                "بدهکار",
                "1000",
                "0",
            ],
            [
                "500",
                "1410/02/01",
                "",
                "ورود دسته‌جمعی وب",
                "7",
                "45",
                "کل درآمد آزمون وب",
                "بستانکار",
                "4501",
                "معین درآمد آزمون وب",
                "بستانکار",
                "0",
                "1000",
            ],
        ];
        for (index, row) in rows.iter().enumerate() {
            for (column, value) in row.iter().enumerate() {
                sheet
                    .write_string(index as u32 + 1, column as u16, *value)
                    .expect("cell");
            }
        }
        workbook.save(path).expect("save workbook");
    }

    #[test]
    fn settings_are_editable_through_the_browser_command() {
        let database = crate::db::memory_database();
        let saved = call(
            &database,
            "save_settings",
            json!({ "companyName": "شرکت آزمون وب", "fiscalYear": "۱۴۱۰" }),
        )
        .unwrap();
        assert_eq!(saved["companyName"], "شرکت آزمون وب");
        let current = call(&database, "get_settings", Value::Null).unwrap();
        assert_eq!(current["companyName"], "شرکت آزمون وب");
        assert!(dispatch(
            &database,
            "save_settings",
            json!({ "companyName": "غیرمجاز", "fiscalYear": "۱۴۱۰" }),
            "voucher",
        )
        .is_err());
    }

    #[test]
    fn excel_export_upload_and_bulk_import_run_through_the_browser_commands() {
        let directory = crate::db::test_temp_dir("web-excel");
        let database_path = directory.join(crate::db::DB_FILE_NAME);
        let database = crate::db::file_database(&database_path);
        let print_docs = Mutex::new(HashMap::new());
        let transfers = Transfers::default();
        let saved = save_cash_sale(&database, 1, "1410/01/05", 1500);

        let exported = super::dispatch(
            &database,
            &print_docs,
            &transfers,
            "export_voucher_excel",
            json!({
                "input": {
                    "id": saved["id"],
                    "number": saved["number"],
                    "date": "1410/01/05",
                    "voucherType": vouchers::VOUCHER_TYPES[0],
                    "description": "فروش نقدی",
                    "fiscalYearId": null,
                    "lines": [
                        {
                            "id": saved["lines"][0]["id"],
                            "accountCode": "110101",
                            "detail1Code": null,
                            "description": "صندوق",
                            "debit": 1500,
                            "credit": 0,
                        },
                        {
                            "id": saved["lines"][1]["id"],
                            "accountCode": "410101",
                            "detail1Code": null,
                            "description": "فروش",
                            "debit": 0,
                            "credit": 1500,
                        },
                    ],
                },
            }),
            "voucher",
        )
        .unwrap();
        assert!(exported["fileName"].as_str().unwrap().ends_with(".xlsx"));
        let voucher_path = excel_download_path(&transfers, &exported);
        assert!(voucher_path.exists());

        // The exported workbook can be read back through the browser command.
        add_excel_upload(&transfers, "voucher-upload", voucher_path, "سند.xlsx");
        let imported = super::dispatch(
            &database,
            &print_docs,
            &transfers,
            "import_voucher_excel",
            json!({ "uploadToken": "voucher-upload" }),
            "voucher",
        )
        .unwrap();
        let lines = imported.as_array().unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["accountCode"], "110101");
        assert!(!lock(&transfers.excel_uploads).contains_key("voucher-upload"));

        let accounts_export = super::dispatch(
            &database,
            &print_docs,
            &transfers,
            "export_account_excel",
            Value::Null,
            "account-definition",
        )
        .unwrap();
        assert!(accounts_export["skippedCodes"].is_array());
        assert!(excel_download_path(&transfers, &accounts_export).exists());
        assert!(super::dispatch(
            &database,
            &print_docs,
            &transfers,
            "export_account_excel",
            Value::Null,
            "voucher",
        )
        .is_err());

        // A workbook with one new level-1 account previews and then imports.
        let account_workbook = directory.join("accounts-upload.xlsx");
        {
            let mut workbook = rust_xlsxwriter::Workbook::new();
            let sheet = workbook.add_worksheet();
            for (column, header) in ["کد گروه", "کد کل", "عنوان کل"].iter().enumerate()
            {
                sheet
                    .write_string(0, column as u16, *header)
                    .expect("header");
            }
            sheet.write_string(1, 0, "1").unwrap();
            sheet.write_string(1, 1, "1998").unwrap();
            sheet.write_string(1, 2, "حساب اکسل وب").unwrap();
            workbook.save(&account_workbook).unwrap();
        }
        add_excel_upload(
            &transfers,
            "accounts-upload",
            account_workbook,
            "accounts.xlsx",
        );
        let preview = super::dispatch(
            &database,
            &print_docs,
            &transfers,
            "preview_account_excel_import",
            json!({ "uploadToken": "accounts-upload" }),
            "account-definition",
        )
        .unwrap();
        assert_eq!(preview["createdCount"], 1);
        assert_eq!(preview["atomicReady"], true);
        let preview_token = preview["previewToken"].as_str().unwrap().to_string();
        let result = super::dispatch(
            &database,
            &print_docs,
            &transfers,
            "import_account_excel",
            json!({ "previewToken": preview_token, "mode": "atomic" }),
            "account-definition",
        )
        .unwrap();
        assert_eq!(result["createdCount"], 1);
        assert!(database
            .list_accounts()
            .unwrap()
            .accounts
            .iter()
            .any(|account| account.code == "1998"));
        // A session token is consumed by the first successful import.
        assert!(super::dispatch(
            &database,
            &print_docs,
            &transfers,
            "import_account_excel",
            json!({ "previewToken": preview_token, "mode": "atomic" }),
            "account-definition",
        )
        .is_err());

        // Bulk analysis keeps the parsed import for the commit command.
        let bulk_workbook = directory.join("bulk-upload.xlsx");
        write_bulk_workbook(&bulk_workbook);
        add_excel_upload(&transfers, "bulk-upload", bulk_workbook, "bulk.xlsx");
        let bulk_preview = super::dispatch(
            &database,
            &print_docs,
            &transfers,
            "analyze_bulk_voucher_excel",
            json!({ "uploadToken": "bulk-upload", "dateSystem": "jalali" }),
            "main",
        )
        .unwrap();
        assert_eq!(bulk_preview["voucherCount"], 1);
        let committed = super::dispatch(
            &database,
            &print_docs,
            &transfers,
            "commit_bulk_voucher_excel",
            json!({
                "renumberConflicts": false,
                "groupOverrides": [],
                "confirmInferredGroups": true,
            }),
            "main",
        )
        .unwrap();
        assert_eq!(committed, 1);
        assert_eq!(database.voucher_count().unwrap(), 2);
        // The parsed import is cleared after a successful commit.
        assert!(super::dispatch(
            &database,
            &print_docs,
            &transfers,
            "commit_bulk_voucher_excel",
            json!({
                "renumberConflicts": false,
                "groupOverrides": [],
                "confirmInferredGroups": true,
            }),
            "main",
        )
        .is_err());

        let _ = std::fs::remove_dir_all(&directory);
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
        let transfers = Transfers::default();

        let opened = super::dispatch(
            &database,
            &print_docs,
            &transfers,
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
            &transfers,
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
            &transfers,
            "get_print_doc",
            json!({ "token": token }),
            "main",
        )
        .is_err());

        let trial = super::dispatch(
            &database,
            &print_docs,
            &transfers,
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
            &transfers,
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
        let transfers = Transfers::default();

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
            let opened =
                super::dispatch(&database, &print_docs, &transfers, command, args, "main").unwrap();
            assert_eq!(opened["webName"], "print-preview");
            let url = opened["webOpen"].as_str().unwrap();
            let token = url
                .strip_prefix("/print.html?doc=")
                .expect("preview token in the url");
            let document = super::dispatch(
                &database,
                &print_docs,
                &transfers,
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
    fn browser_backup_creates_a_download_and_plain_restore_replaces_the_database() {
        let directory = crate::db::test_temp_dir("web-backup");
        let database_path = directory.join("hesabdari.sqlite3");
        let database = crate::db::file_database(&database_path);
        save_cash_sale(&database, 1, "1410/01/05", 1500);
        let print_docs = Mutex::new(HashMap::new());
        let transfers = Transfers::default();

        let backup = super::dispatch(
            &database,
            &print_docs,
            &transfers,
            "backup_database",
            Value::Null,
            "main",
        )
        .unwrap();
        let backup_path = PathBuf::from(backup["path"].as_str().unwrap());
        assert!(backup_path.exists());
        assert!(backup_path.starts_with(directory.join("backups")));
        assert!(backup["fileName"].as_str().unwrap().ends_with(".sqlite3"));
        let download = backup["webDownload"].as_str().unwrap();
        let download_token = download
            .strip_prefix("/api/backup/")
            .expect("download token in the url");
        {
            let downloads = lock(&transfers.downloads);
            assert_eq!(downloads.get(download_token).unwrap().path, backup_path);
        }

        // A backup created with one voucher must bring that voucher back.
        save_cash_sale(&database, 2, "1410/01/06", 700);
        assert_eq!(database.voucher_count().unwrap(), 2);
        let upload_token = "restore-plain-test".to_string();
        {
            let mut uploads = lock(&transfers.uploads);
            uploads.insert(
                upload_token.clone(),
                UploadEntry {
                    path: backup_path.clone(),
                    encrypted: false,
                    file_sha256: db::sha256_file(&backup_path).unwrap(),
                    created_at: SystemTime::now(),
                },
            );
        }
        let safety = super::dispatch(
            &database,
            &print_docs,
            &transfers,
            "restore_database_file",
            json!({ "token": upload_token, "password": null }),
            "main",
        )
        .unwrap();
        assert!(safety.as_str().unwrap().contains("before-restore"));
        assert_eq!(database.voucher_count().unwrap(), 1);
        assert!(!lock(&transfers.uploads).contains_key(&upload_token));

        // Backup and restore are limited to the main window.
        assert!(super::dispatch(
            &database,
            &print_docs,
            &transfers,
            "backup_database",
            Value::Null,
            "voucher",
        )
        .is_err());
        assert!(super::dispatch(
            &database,
            &print_docs,
            &transfers,
            "restore_database_file",
            json!({ "token": "missing" }),
            "trial-balance",
        )
        .is_err());

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn encrypted_browser_backup_requires_its_password_and_rejects_stale_uploads() {
        let directory = crate::db::test_temp_dir("web-backup-encrypted");
        let database_path = directory.join("hesabdari.sqlite3");
        let database = crate::db::file_database(&database_path);
        save_cash_sale(&database, 1, "1410/01/05", 1500);
        let print_docs = Mutex::new(HashMap::new());
        let transfers = Transfers::default();
        let password = "correct-horse";

        let backup = super::dispatch(
            &database,
            &print_docs,
            &transfers,
            "backup_database_encrypted",
            json!({ "password": password }),
            "main",
        )
        .unwrap();
        let backup_path = PathBuf::from(backup["path"].as_str().unwrap());
        assert!(db::is_encrypted_backup(&backup_path).unwrap());
        assert!(backup["fileName"].as_str().unwrap().ends_with(".hacc"));
        let stale_path = directory.join("stale.hacc");
        std::fs::copy(&backup_path, &stale_path).unwrap();

        let upload_token = "restore-encrypted-test".to_string();
        {
            let mut uploads = lock(&transfers.uploads);
            uploads.insert(
                upload_token.clone(),
                UploadEntry {
                    path: backup_path.clone(),
                    encrypted: true,
                    file_sha256: db::sha256_file(&backup_path).unwrap(),
                    created_at: SystemTime::now(),
                },
            );
        }
        assert!(super::dispatch(
            &database,
            &print_docs,
            &transfers,
            "restore_database_file",
            json!({ "token": upload_token.clone(), "password": null }),
            "main",
        )
        .is_err());
        assert!(super::dispatch(
            &database,
            &print_docs,
            &transfers,
            "restore_database_file",
            json!({ "token": upload_token.clone(), "password": "wrong-password" }),
            "main",
        )
        .is_err());
        // A wrong password keeps the selection, so the correct one can retry.
        super::dispatch(
            &database,
            &print_docs,
            &transfers,
            "restore_database_file",
            json!({ "token": upload_token, "password": password }),
            "main",
        )
        .unwrap();

        // Expired uploads are refused and their temporary file is removed.
        let stale_token = "restore-stale-test".to_string();
        {
            let mut uploads = lock(&transfers.uploads);
            uploads.insert(
                stale_token.clone(),
                UploadEntry {
                    path: stale_path.clone(),
                    encrypted: true,
                    file_sha256: db::sha256_file(&stale_path).unwrap(),
                    created_at: SystemTime::now()
                        .checked_sub(UPLOAD_TTL + Duration::from_secs(1))
                        .unwrap(),
                },
            );
        }
        let error = super::dispatch(
            &database,
            &print_docs,
            &transfers,
            "restore_database_file",
            json!({ "token": stale_token, "password": password }),
            "main",
        )
        .unwrap_err();
        assert!(error.contains("منقضی"));
        assert!(!stale_path.exists());

        let _ = std::fs::remove_dir_all(&directory);
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
