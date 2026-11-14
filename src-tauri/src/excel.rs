//! Excel import and export for vouchers and the chart of accounts.
//!
//! - [`model`] holds the public preview/result types.
//! - [`common`] owns workbook size limits and shared cell parsing.
//! - [`export`] writes one voucher as a real `.xlsx` workbook.
//! - [`import`] reads one voucher or a bulk-import workbook.
//! - [`accounts`] imports and exports the chart of accounts.
//!
//! The workbook layouts are unchanged and remain compatible with the WebApp
//! templates.

mod accounts;
mod common;
mod export;
mod import;
mod model;

pub(crate) use accounts::AccountImportSession;
pub use accounts::{
    export_accounts, import_accounts, preview_accounts, AccountExportResult, AccountImportPreview,
    AccountImportResult,
};
#[cfg(feature = "web")]
pub(crate) use accounts::{export_accounts_to, preview_accounts_at};
pub(crate) use common::parse_code;
#[cfg(feature = "web")]
pub(crate) use common::DateSystem;
pub use export::export_voucher;
#[cfg(feature = "web")]
pub(crate) use export::{export_voucher_to, voucher_export_file_name};
pub use import::{import_voucher, import_vouchers};
#[cfg(feature = "web")]
pub(crate) use import::{import_voucher_from_path, parse_bulk_vouchers_with_progress};
pub(crate) use import::{import_vouchers_with_progress, BULK_IMPORT_CANCELLED};
pub use model::{
    BulkVoucherImport, BulkVoucherPreview, GroupMappingOverride, ImportedAccount,
    InferredGroupMapping,
};

#[cfg(test)]
mod tests;
