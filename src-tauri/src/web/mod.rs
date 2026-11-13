//! Local-only HTTP host for the accounting UI.
//!
//! The desktop app and this browser mode share the same SQLite schema and the
//! same Rust storage layer. The browser gets a small JSON command endpoint
//! that mirrors the part of the Tauri command surface needed by the first
//! local-web slice: settings, the chart of accounts, fiscal years, the voucher
//! list and the trial balance.
//!
//! The listener is bound to `127.0.0.1` only. There is no authentication and no
//! remote access; the server is meant for the same Windows session as the
//! desktop application.

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tower_http::services::ServeDir;

use crate::db;
use crate::vouchers::{self, ReportScope};

const DEFAULT_PORT: u16 = 4174;

#[derive(Clone)]
struct WebState {
    database: Arc<db::Database>,
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

    let result =
        tokio::task::spawn_blocking(move || dispatch(&database, &name, args, &window)).await;
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
            require_window(window, &["main", "trial-balance", "voucher-list"])?;
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
    fn window_commands_return_browser_urls() {
        let database = crate::db::memory_database();

        let trial = call(&database, "open_trial_balance_window", Value::Null).unwrap();
        assert_eq!(trial["webOpen"], "/index.html?popup=trial-balance");
        assert_eq!(trial["webName"], "trial-balance");

        let groups = call(&database, "open_account_groups_window", Value::Null).unwrap();
        assert_eq!(groups["webOpen"], "/account-groups.html");
        assert_eq!(groups["webName"], "account-groups");
    }

    #[test]
    fn unknown_commands_and_foreign_windows_are_rejected() {
        let database = crate::db::memory_database();

        assert!(call(&database, "save_voucher", Value::Null).is_err());
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
