//! Tauri command handlers, grouped by the feature they serve.

pub(crate) mod accounts;
pub(crate) mod database;
pub(crate) mod print;
pub(crate) mod reports;
pub(crate) mod settings;
pub(crate) mod state;
pub(crate) mod vouchers;
pub(crate) mod windows;

pub use state::AppState;

pub(crate) use accounts::*;
pub(crate) use database::*;
pub(crate) use print::*;
pub(crate) use reports::*;
pub(crate) use settings::*;
pub(crate) use vouchers::*;
pub(crate) use windows::*;
