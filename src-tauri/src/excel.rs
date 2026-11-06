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

pub use accounts::{export_accounts, import_accounts, AccountExportResult, AccountImportResult};
pub use export::export_voucher;
pub use import::{import_voucher, import_vouchers};
pub use model::{BulkVoucherImport, BulkVoucherPreview, ImportedAccount, InferredGroupMapping};

#[cfg(test)]
mod tests;
