//! Voucher persistence and the accounting reports that are built from it.
//!
//! The implementation is split by responsibility:
//!
//! - [`model`] holds the wire types and limits shared by every layer.
//! - [`validation`] normalizes dates, descriptions and account eligibility.
//! - [`storage`] reads, saves, pages and deletes vouchers.
//! - [`transfer`] copies or moves persisted rows between vouchers.
//! - [`import`] loads vouchers from parsed Excel workbooks.
//! - [`reorganization`] merges same-date vouchers and renumbers the book.
//! - [`bulk_delete`] previews and deletes vouchers by date or number range.
//! - [`reports`] builds the journal, ledger and financial statements.
//!
//! Amounts are stored as whole Rials (`i64`). Every voucher must balance
//! before it is written, so the reports can rely on a sound ledger. The
//! database schema and the public `crate::vouchers::*` API are unchanged.

mod accounting;
mod bulk_delete;
mod import;
mod model;
mod query;
mod reorganization;
mod reports;
mod storage;
mod transfer;
mod validation;

pub use model::{
    AmountRow, BalanceSheetReport, BulkVoucherDeleteDateRange, BulkVoucherDeleteInput,
    BulkVoucherDeleteNumberRange, BulkVoucherDeletePreview, BulkVoucherDeleteResult,
    BulkVoucherDeleteSample, IncomeStatementReport, JournalReport, JournalRow, LedgerReport,
    LedgerRow, TrialBalanceReport, TrialBalanceRow, Voucher, VoucherInput, VoucherLine,
    VoucherLineInput, VoucherList, VoucherMergePreview, VoucherNumberChange,
    VoucherReorganizationPreview, VoucherReorganizationResult, VoucherSummary,
    VoucherTransferInput, VoucherTransferResult, DEFAULT_JOURNAL_PAGE_SIZE,
    DEFAULT_VOUCHER_PAGE_SIZE, MAX_AMOUNT, MAX_BULK_DELETE_TARGETS, MAX_PAGE_SIZE,
    MAX_VOUCHER_NUMBER, VOUCHER_TYPES,
};
pub(crate) use model::{MAX_DESCRIPTION, MAX_LINES, MAX_LINE_DESCRIPTION};
pub use validation::{format_amount, normalize_filter_date, validate_voucher_date};

#[cfg(test)]
mod tests;
