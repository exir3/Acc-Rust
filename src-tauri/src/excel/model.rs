//! Public Excel import/export result types.

use crate::vouchers::VoucherInput;
use serde::Serialize;

#[derive(Debug, Clone)]
pub struct BulkVoucherImport {
    pub file_name: String,
    pub sheet_name: String,
    pub vouchers: Vec<VoucherInput>,
    pub accounts: Vec<ImportedAccount>,
}

#[derive(Debug, Clone)]
pub struct ImportedAccount {
    pub code: String,
    pub name: String,
    pub parent_code: String,
    pub group_code: String,
    pub level: i64,
    pub nature: String,
    pub nature_is_explicit: bool,
    /// True when the workbook had no group-code column/value and the group was
    /// derived from the account-code prefix. The preview surfaces these rows
    /// and the import audit records them, so a guessed mapping is never silent.
    pub group_is_inferred: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InferredGroupMapping {
    pub account_code: String,
    pub account_name: String,
    pub level: i64,
    pub group_code: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkVoucherPreview {
    pub file_name: String,
    pub sheet_name: String,
    pub voucher_count: usize,
    pub line_count: usize,
    pub account_count: usize,
    pub new_account_count: usize,
    pub number_conflict_count: usize,
    pub conflicting_voucher_numbers: Vec<i64>,
    pub inferred_group_count: usize,
    pub inferred_group_mappings: Vec<InferredGroupMapping>,
}

#[derive(Default)]
pub(crate) struct BulkImportColumns {
    pub(crate) voucher_number: Option<usize>,
    pub(crate) voucher_date: Option<usize>,
    pub(crate) group_code: Option<usize>,
    pub(crate) general_code: Option<usize>,
    pub(crate) general_name: Option<usize>,
    pub(crate) general_nature: Option<usize>,
    pub(crate) subsidiary_code: Option<usize>,
    pub(crate) subsidiary_name: Option<usize>,
    pub(crate) subsidiary_nature: Option<usize>,
    pub(crate) detail1_code: Option<usize>,
    pub(crate) detail1_name: Option<usize>,
    pub(crate) detail1_nature: Option<usize>,
    pub(crate) voucher_type: Option<usize>,
    pub(crate) voucher_description: Option<usize>,
    pub(crate) line_description: Option<usize>,
    pub(crate) debit: Option<usize>,
    pub(crate) credit: Option<usize>,
}
