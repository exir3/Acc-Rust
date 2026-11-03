//! Self-contained printable HTML documents for the accounting reports.
//!
//! The in-app preview window extracts the `<style>` element from each
//! document, so the screen view and the printed page share one layout. Every
//! report lives in its own module and uses the shared scaffolding in
//! [`layout`].

mod balance_sheet;
mod income_statement;
mod journal;
mod layout;
mod ledger;
mod model;
mod trial_balance;
mod voucher;

pub use balance_sheet::balance_sheet_print_doc;
pub use income_statement::income_statement_print_doc;
pub use journal::journal_print_doc;
pub use ledger::ledger_print_doc;
pub use model::{PrintDoc, ReportMeta};
pub use trial_balance::trial_balance_print_doc;
pub use voucher::voucher_print_doc;

#[cfg(test)]
mod tests;
