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
#[cfg(test)]
mod perf_check;
mod query;
mod reorganization;
mod reports;
mod storage;
mod transfer;
mod validation;

pub(crate) use model::{
    serialize_exact_amount, CLOSING_FINAL, CLOSING_TEMPORARY, MAX_DESCRIPTION, MAX_LINES,
    MAX_LINE_DESCRIPTION, TOUCHED_AT,
};
pub use model::{
    AmountRow, BalanceSheetReport, BulkVoucherDeleteDateRange, BulkVoucherDeleteInput,
    BulkVoucherDeleteNumberRange, BulkVoucherDeletePreview, BulkVoucherDeleteResult,
    BulkVoucherDeleteSample, IncomeStatementReport, IntegrityCounts, IntegrityIssue,
    IntegrityReport, JournalReport, JournalRow, LedgerReport, LedgerRow, ReportScope,
    TrialBalanceReport, TrialBalanceRow, Voucher, VoucherCounters, VoucherInput, VoucherLine,
    VoucherLineInput, VoucherList, VoucherMergePreview, VoucherNumberChange,
    VoucherReorganizationPreview, VoucherReorganizationResult, VoucherSummary,
    VoucherTransferInput, VoucherTransferResult, DEFAULT_JOURNAL_PAGE_SIZE,
    DEFAULT_VOUCHER_PAGE_SIZE, INTEGRITY_ERROR, INTEGRITY_INFO, INTEGRITY_WARNING, MAX_AMOUNT,
    MAX_BULK_DELETE_TARGETS, MAX_PAGE_SIZE, MAX_VOUCHER_NUMBER, VOUCHER_TYPES,
};
pub(crate) use validation::normalize_date;
pub use validation::{format_amount, normalize_filter_date, validate_voucher_date};

#[cfg(test)]
mod tests;
