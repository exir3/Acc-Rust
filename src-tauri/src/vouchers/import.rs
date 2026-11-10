//! Bulk voucher import from parsed Excel workbooks.

use crate::db::{self, audit, Database};
use crate::vouchers::model::{
    VoucherLineInput, MAX_AMOUNT, MAX_DESCRIPTION, MAX_LINES, MAX_LINE_DESCRIPTION,
    MAX_VOUCHER_NUMBER,
};
use crate::vouchers::storage::voucher_snapshot;
use crate::vouchers::validation::{
    assert_voucher_type_matches_lines, checked_sum, normalize_date,
    normalize_voucher_line_accounts, validate_description, validate_voucher_line_accounts,
    validate_voucher_type,
};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

/// Versioned domain separator for the canonical import fingerprint.
const PAYLOAD_HASH_PREFIX: &str = "sha256-v1:";

/// Receives the canonical field stream of a parsed workbook. Both the current
/// SHA-256 fingerprint and the legacy FNV-1a lookup use the same traversal, so
/// a change in one field can never be silently ignored by only one of them.
trait PayloadSink {
    fn text(&mut self, value: &str);
    fn number(&mut self, value: i64) {
        self.text(&value.to_string());
    }
    fn flag(&mut self, value: bool);
}

fn visit_import_payload(import: &crate::excel::BulkVoucherImport, sink: &mut impl PayloadSink) {
    sink.text(&import.file_name);
    sink.text(&import.sheet_name);
    for account in &import.accounts {
        sink.text(&account.code);
        sink.text(&account.name);
        sink.text(&account.parent_code);
        sink.text(&account.group_code);
        sink.number(account.level);
        sink.text(&account.nature);
        sink.flag(account.nature_is_explicit);
        sink.flag(account.group_is_inferred);
    }
    for voucher in &import.vouchers {
        sink.number(voucher.id.unwrap_or(0));
        sink.number(voucher.number.unwrap_or(0));
        sink.text(&voucher.date);
        sink.text(&voucher.voucher_type);
        sink.text(&voucher.description);
        for line in &voucher.lines {
            sink.number(line.id.unwrap_or(0));
            sink.text(&line.account_code);
            sink.text(line.detail1_code.as_deref().unwrap_or(""));
            sink.text(&line.description);
            sink.number(line.debit);
            sink.number(line.credit);
        }
    }
}

/// Collision-resistant, versioned fingerprint of the parsed workbook. It
/// identifies a retry of the same import; the version prefix makes a future
/// canonical-encoding change explicit instead of silently comparing hashes
/// produced by different algorithms.
struct Sha256PayloadHasher {
    hasher: Sha256,
}

impl Sha256PayloadHasher {
    fn new() -> Self {
        let mut hasher = Sha256::new();
        hasher.update(PAYLOAD_HASH_PREFIX.as_bytes());
        Self { hasher }
    }

    fn finish(self) -> String {
        format!(
            "{PAYLOAD_HASH_PREFIX}{}",
            db::to_hex(self.hasher.finalize().as_slice())
        )
    }
}

impl PayloadSink for Sha256PayloadHasher {
    fn text(&mut self, value: &str) {
        // Length-prefix each field so concatenation cannot create the same
        // stream from different field boundaries.
        self.hasher.update((value.len() as u64).to_le_bytes());
        self.hasher.update(value.as_bytes());
    }

    fn flag(&mut self, value: bool) {
        self.hasher.update([u8::from(value)]);
    }
}

/// Previous 64-bit FNV-1a fingerprint. New batches are written with SHA-256,
/// but existing `import_batches` rows must still be recognized after upgrade.
struct LegacyFnvHasher {
    state: u64,
}

impl LegacyFnvHasher {
    fn new() -> Self {
        Self {
            state: 0xcbf2_9ce4_8422_2325,
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.state ^= u64::from(*byte);
            self.state = self.state.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn finish(self) -> String {
        format!("{:016x}", self.state)
    }
}

impl PayloadSink for LegacyFnvHasher {
    fn text(&mut self, value: &str) {
        self.write(value.as_bytes());
        self.write(&[0]);
    }

    fn flag(&mut self, value: bool) {
        self.write(&[u8::from(value)]);
    }
}

pub(super) fn import_payload_hash(import: &crate::excel::BulkVoucherImport) -> String {
    let mut hasher = Sha256PayloadHasher::new();
    visit_import_payload(import, &mut hasher);
    hasher.finish()
}

/// Read-only compatibility path so a workbook imported before the SHA-256
/// change is still treated as a replay instead of being imported twice.
pub(super) fn legacy_import_payload_hash(import: &crate::excel::BulkVoucherImport) -> String {
    let mut hasher = LegacyFnvHasher::new();
    visit_import_payload(import, &mut hasher);
    hasher.finish()
}

/// One audited group decision. `source` is `file`, `inferred` or `manual`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GroupMappingRecord {
    account_code: String,
    account_name: String,
    level: i64,
    group_code: String,
    source: &'static str,
    reason: String,
}

/// Applies the operator's editable mappings to the parsed accounts. A direct
/// override is only accepted for an account whose group was inferred from the
/// code prefix; an explicit value from the workbook is never silently
/// rewritten. The chosen group then flows down the same branch to inferred
/// children so the parent/child group invariant stays true.
fn apply_group_overrides(
    accounts: &mut [crate::excel::ImportedAccount],
    overrides: &[crate::excel::GroupMappingOverride],
) -> Result<Vec<GroupMappingRecord>, String> {
    if overrides.is_empty() {
        return Ok(Vec::new());
    }

    let mut direct: HashMap<String, String> = HashMap::new();
    for mapping in overrides {
        let code = db::normalize_code(&mapping.account_code);
        if code.is_empty() {
            return Err("کد حساب در نگاشت گروه خالی است.".to_string());
        }
        let group_code = crate::excel::parse_code(&mapping.group_code, "کد گروه")?;
        if group_code.chars().count() > 50 {
            return Err("کد گروه در نگاشت نمی‌تواند بیشتر از ۵۰ رقم باشد.".to_string());
        }
        if let Some(previous) = direct.insert(code.clone(), group_code.clone()) {
            if previous != group_code {
                return Err(format!(
                    "برای حساب {} دو گروه متفاوت انتخاب شده است.",
                    db::to_fa_digits(&code)
                ));
            }
        }
    }

    for code in direct.keys() {
        let Some(account) = accounts
            .iter()
            .find(|account| db::normalize_code(&account.code) == *code)
        else {
            return Err(format!(
                "حساب {} در فایل اکسل پیدا نشد؛ نگاشت گروه قابل اعمال نیست.",
                db::to_fa_digits(code)
            ));
        };
        if !account.group_is_inferred {
            return Err(format!(
                "گروه حساب {} از فایل اکسل خوانده شده است و با نگاشت دستی قابل جایگزینی نیست.",
                db::to_fa_digits(code)
            ));
        }
    }

    let mut order: Vec<usize> = (0..accounts.len()).collect();
    order.sort_by_key(|&index| accounts[index].level);
    let mut manual: HashMap<String, GroupMappingRecord> = HashMap::new();
    for index in order {
        let code = db::normalize_code(&accounts[index].code);
        let parent_code = db::normalize_code(&accounts[index].parent_code);
        let (group_code, reason) = if let Some(group_code) = direct.get(&code) {
            (
                group_code.clone(),
                "گروه پیشنهادی توسط کاربر اصلاح یا تأیید شد.".to_string(),
            )
        } else if accounts[index].group_is_inferred {
            match manual.get(&parent_code) {
                Some(parent) => (
                    parent.group_code.clone(),
                    format!(
                        "انتقال گروه از حساب بالاتر {}.",
                        db::to_fa_digits(&parent_code)
                    ),
                ),
                None => continue,
            }
        } else {
            // A group read from the workbook stays untouched; a conflicting
            // child is reported by the regular parent/group validation.
            continue;
        };
        accounts[index].group_code = group_code.clone();
        manual.insert(
            code.clone(),
            GroupMappingRecord {
                account_code: code,
                account_name: accounts[index].name.clone(),
                level: accounts[index].level,
                group_code,
                source: "manual",
                reason,
            },
        );
    }

    let mut records: Vec<GroupMappingRecord> = manual.into_values().collect();
    records.sort_by(|left, right| left.account_code.cmp(&right.account_code));
    Ok(records)
}

impl Database {
    pub fn preview_bulk_import(
        &self,
        import: &crate::excel::BulkVoucherImport,
    ) -> Result<crate::excel::BulkVoucherPreview, String> {
        let file_name = import.file_name.clone();
        let sheet_name = import.sheet_name.clone();
        let date_warning_count = import.date_warning_count;
        let date_warnings = import.date_warnings.clone();
        let account_rows = import.accounts.clone();
        let account_codes: HashSet<String> = account_rows
            .iter()
            .map(|account| db::normalize_code(&account.code))
            .collect();
        let referenced_codes: HashSet<String> = import
            .vouchers
            .iter()
            .flat_map(|voucher| voucher.lines.iter())
            .flat_map(|line| {
                [
                    db::normalize_code(&line.account_code),
                    line.detail1_code
                        .as_deref()
                        .map(db::normalize_code)
                        .unwrap_or_default(),
                ]
            })
            .filter(|code| !code.is_empty())
            .collect();
        let numbers: Vec<i64> = import
            .vouchers
            .iter()
            .filter_map(|voucher| voucher.number)
            .collect();
        let voucher_count = import.vouchers.len();
        let line_count = import
            .vouchers
            .iter()
            .map(|voucher| voucher.lines.len())
            .sum();
        let account_count = account_codes.len();
        // Accounts whose group was guessed from the code prefix, not read from
        // the workbook, are reported to the user before import and recorded in
        // the audit entry afterwards.
        let inferred_group_mappings: Vec<crate::excel::InferredGroupMapping> = account_rows
            .iter()
            .filter(|account| account.group_is_inferred)
            .map(|account| crate::excel::InferredGroupMapping {
                account_code: db::normalize_code(&account.code),
                account_name: account.name.clone(),
                level: account.level,
                group_code: db::normalize_code(&account.group_code),
                reason: "گروه از پیشوند کد حساب کل حدس زده شد.".to_string(),
            })
            .collect();
        let inferred_group_count = inferred_group_mappings.len();

        self.with_conn(move |conn| {
            // Validate all groups and existing account structures during preview,
            // not only after the user presses the final import button.
            let mut missing_groups: std::collections::BTreeMap<String, Vec<String>> =
                std::collections::BTreeMap::new();
            for account in &account_rows {
                let group_code = db::normalize_code(&account.group_code);
                let exists = conn
                    .query_row(
                        "SELECT 1 FROM account_groups WHERE code = ?1",
                        params![group_code],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(db::db_error)?
                    .is_some();
                if !exists {
                    missing_groups
                        .entry(group_code)
                        .or_default()
                        .push(account.code.clone());
                }
            }
            if !missing_groups.is_empty() {
                let detail = missing_groups
                    .iter()
                    .map(|(group, codes)| {
                        let shown = codes
                            .iter()
                            .take(5)
                            .map(db::to_fa_digits)
                            .collect::<Vec<_>>()
                            .join("، ");
                        let more = if codes.len() > 5 {
                            format!(" و {} حساب دیگر", db::to_fa_digits(codes.len() - 5))
                        } else {
                            String::new()
                        };
                        format!("گروه «{}» برای حساب‌های {shown}{more}", db::to_fa_digits(group))
                    })
                    .collect::<Vec<_>>()
                    .join(" | ");
                return Err(format!(
                    "پیش از ورود اسناد، گروه حساب‌های نامعتبر را اصلاح کنید: {detail}"
                ));
            }

            let mut new_account_count = 0;
            for account in &account_rows {
                let code = db::normalize_code(&account.code);
                let group_code = db::normalize_code(&account.group_code);
                let parent_code = db::normalize_code(&account.parent_code);
                let existing = conn
                    .query_row(
                        "SELECT level, group_code, COALESCE(parent_code, ''), nature
                           FROM accounts WHERE code = ?1",
                        params![code],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(db::db_error)?;

                if let Some((level, existing_group, existing_parent, existing_nature)) = existing {
                    if level != account.level
                        || existing_group != group_code
                        || existing_parent != parent_code
                    {
                        return Err(format!(
                            "حساب {} از قبل با گروه، سطح یا حساب بالاتر متفاوت ثبت شده است.",
                            db::to_fa_digits(&code)
                        ));
                    }
                    if account.nature_is_explicit && existing_nature != account.nature {
                        return Err(format!(
                            "ماهیت حساب {} با مقدار واردشده در فایل اکسل متفاوت است.",
                            db::to_fa_digits(&code)
                        ));
                    }
                } else {
                    if !matches!(account.nature.as_str(), "debtor" | "creditor" | "dual") {
                        return Err(format!(
                            "ماهیت حساب جدید {} مشخص نیست؛ برای گروه‌های سایر درآمدها و هزینه‌ها، ستون ماهیت مربوط به حساب را با مقدار بدهکار، بستانکار یا دوگانه تکمیل کنید.",
                            db::to_fa_digits(&code)
                        ));
                    }
                    new_account_count += 1;
                    if account.level > 1 && !parent_code.is_empty() {
                        let parent = conn
                            .query_row(
                                "SELECT level, group_code, is_active FROM accounts WHERE code = ?1",
                                params![parent_code],
                                |row| {
                                    Ok((
                                        row.get::<_, i64>(0)?,
                                        row.get::<_, String>(1)?,
                                        row.get::<_, i64>(2)?,
                                    ))
                                },
                            )
                            .optional()
                            .map_err(db::db_error)?;
                        if let Some((parent_level, parent_group, parent_is_active)) = parent {
                            if parent_level != account.level - 1 || parent_group != group_code {
                                return Err(format!(
                                    "گروه یا سطح حساب بالاتر {} با حساب {} سازگار نیست.",
                                    db::to_fa_digits(&parent_code),
                                    db::to_fa_digits(&code)
                                ));
                            }
                            if parent_is_active == 0 {
                                return Err(format!(
                                    "حساب بالاتر {} غیرفعال است؛ ابتدا آن را فعال کنید.",
                                    db::to_fa_digits(&parent_code)
                                ));
                            }
                        } else if !account_codes.contains(&parent_code) {
                            return Err(format!(
                                "حساب بالاتر {} برای حساب {} در فایل یا پایگاه داده پیدا نشد.",
                                db::to_fa_digits(&parent_code),
                                db::to_fa_digits(&code)
                            ));
                        }
                    }
                }
            }

            // Base and detail references must each exist in the live chart or
            // be declared in this workbook. Pair compatibility is validated
            // after all missing accounts have been created in the import tx.
            for code in &referenced_codes {
                let exists = conn.query_row(
                    "SELECT 1 FROM accounts WHERE code = ?1", params![code], |_| Ok(())
                ).optional().map_err(db::db_error)?.is_some();
                if !exists && !account_codes.contains(code) {
                    return Err(format!(
                        "حساب ردیف سند با کد {} در فایل یا پایگاه داده پیدا نشد.",
                        db::to_fa_digits(code)
                    ));
                }
            }

            let mut conflicting_voucher_numbers = Vec::new();
            for number in numbers {
                let exists = conn
                    .query_row(
                        "SELECT 1 FROM vouchers WHERE number = ?1",
                        params![number],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(db::db_error)?
                    .is_some();
                if exists {
                    conflicting_voucher_numbers.push(number);
                }
            }
            conflicting_voucher_numbers.sort_unstable();
            conflicting_voucher_numbers.dedup();
            Ok(crate::excel::BulkVoucherPreview {
                file_name,
                sheet_name,
                voucher_count,
                line_count,
                account_count,
                new_account_count,
                number_conflict_count: conflicting_voucher_numbers.len(),
                conflicting_voucher_numbers,
                inferred_group_count,
                inferred_group_mappings,
                date_warning_count,
                date_warnings,
            })
        })
    }

    pub fn import_vouchers(
        &self,
        import: crate::excel::BulkVoucherImport,
        renumber_conflicts: bool,
    ) -> Result<usize, String> {
        self.import_vouchers_with_progress(import, renumber_conflicts, &[], &mut |_, _| {}, &|| {
            false
        })
    }

    /// Imports a parsed workbook while reporting per-voucher progress and
    /// honouring an operator cancellation. Everything runs in one SQLite
    /// transaction, so a cancellation surfaces as an error and rolls the whole
    /// workbook back instead of leaving a partial import behind.
    #[allow(clippy::too_many_arguments)]
    pub fn import_vouchers_with_progress(
        &self,
        mut import: crate::excel::BulkVoucherImport,
        renumber_conflicts: bool,
        group_overrides: &[crate::excel::GroupMappingOverride],
        on_progress: &mut dyn FnMut(usize, usize),
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<usize, String> {
        if import.vouchers.is_empty() {
            return Err("فایلی برای ورود وجود ندارد.".to_string());
        }
        if is_cancelled() {
            return Err(crate::excel::BULK_IMPORT_CANCELLED.to_string());
        }

        // Apply the operator's confirmed mappings before hashing or writing, so
        // the transaction, the retry fingerprint and the audit all see the
        // same final groups.
        let manual_group_mappings = apply_group_overrides(&mut import.accounts, group_overrides)?;
        let payload_hash = import_payload_hash(&import);
        let legacy_payload_hash = legacy_import_payload_hash(&import);
        let file_name = import.file_name.clone();
        let sheet_name = import.sheet_name.clone();

        let mut prepared = Vec::with_capacity(import.vouchers.len());
        let total_vouchers = import.vouchers.len();
        let mut numbers = HashSet::new();
        for (index, input) in import.vouchers.into_iter().enumerate() {
            if index % 32 == 0 {
                if is_cancelled() {
                    return Err(crate::excel::BULK_IMPORT_CANCELLED.to_string());
                }
                on_progress(index, total_vouchers);
            }
            let date = normalize_date(&input.date, "تاریخ سند")?;
            let voucher_type = validate_voucher_type(&input.voucher_type)?;
            let description = validate_description(&input.description, "شرح سند", MAX_DESCRIPTION)?;
            let number = input.number.unwrap_or(0);
            if number <= 0 || number > MAX_VOUCHER_NUMBER || !numbers.insert(number) {
                return Err(format!(
                    "شماره سند {} نامعتبر یا تکراری است.",
                    db::to_fa_digits(number)
                ));
            }
            if input.lines.len() > MAX_LINES {
                return Err(format!(
                    "تعداد ردیف‌های سند {} از حد مجاز بیشتر است.",
                    db::to_fa_digits(number)
                ));
            }

            let mut lines = Vec::with_capacity(input.lines.len());
            for line in input.lines {
                let account_code = db::normalize_code(&line.account_code);
                let line_description =
                    validate_description(&line.description, "شرح ردیف", MAX_LINE_DESCRIPTION)?;
                if account_code.is_empty() {
                    return Err(format!(
                        "کد حساب در سند {} خالی است.",
                        db::to_fa_digits(number)
                    ));
                }
                if line.debit < 0
                    || line.credit < 0
                    || line.debit > MAX_AMOUNT
                    || line.credit > MAX_AMOUNT
                {
                    return Err(format!(
                        "مبلغ سند {} خارج از محدوده مجاز است.",
                        db::to_fa_digits(number)
                    ));
                }
                if (line.debit == 0 && line.credit == 0) || (line.debit > 0 && line.credit > 0) {
                    return Err(format!(
                        "مبلغ ردیف سند {} نامعتبر است.",
                        db::to_fa_digits(number)
                    ));
                }
                lines.push(VoucherLineInput {
                    id: line.id,
                    account_code,
                    detail1_code: line
                        .detail1_code
                        .as_deref()
                        .map(db::normalize_code)
                        .filter(|v| !v.is_empty()),
                    description: line_description,
                    debit: line.debit,
                    credit: line.credit,
                });
            }
            if lines.len() < 2 {
                return Err(format!(
                    "سند {} باید حداقل دو ردیف داشته باشد.",
                    db::to_fa_digits(number)
                ));
            }
            let total_debit = checked_sum(lines.iter().map(|line| line.debit), "جمع بدهکار سند")?;
            let total_credit =
                checked_sum(lines.iter().map(|line| line.credit), "جمع بستانکار سند")?;
            if total_debit == 0 || total_debit != total_credit {
                return Err(format!("سند {} تراز نیست.", db::to_fa_digits(number)));
            }
            prepared.push((number, date, voucher_type, description, lines));
        }

        let mut accounts = import.accounts;
        accounts.sort_by_key(|account| account.level);
        let manual_codes: HashSet<String> = manual_group_mappings
            .iter()
            .map(|mapping| mapping.account_code.clone())
            .collect();
        let inferred_group_mappings: Vec<crate::excel::InferredGroupMapping> = accounts
            .iter()
            .filter(|account| {
                account.group_is_inferred
                    && !manual_codes.contains(&db::normalize_code(&account.code))
            })
            .map(|account| crate::excel::InferredGroupMapping {
                account_code: db::normalize_code(&account.code),
                account_name: account.name.clone(),
                level: account.level,
                group_code: db::normalize_code(&account.group_code),
                reason: "گروه از پیشوند کد حساب کل حدس زده شد.".to_string(),
            })
            .collect();
        let inferred_group_count = inferred_group_mappings.len();
        let manual_group_count = manual_group_mappings.len();
        let file_group_count = accounts
            .iter()
            .filter(|account| {
                !account.group_is_inferred
                    && !manual_codes.contains(&db::normalize_code(&account.code))
            })
            .count();
        let imported_count = prepared.len();
        if is_cancelled() {
            return Err(crate::excel::BULK_IMPORT_CANCELLED.to_string());
        }
        on_progress(imported_count, imported_count);
        self.with_conn(move |conn| {
            let tx = conn.transaction().map_err(db::db_error)?;
            let previous = tx
                .query_row(
                    "SELECT voucher_count, imported_numbers
                       FROM import_batches
                      WHERE payload_hash IN (?1, ?2)",
                    params![&payload_hash, &legacy_payload_hash],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(db::db_error)?;
            if let Some((previous_count, previous_numbers)) = previous {
                let count = usize::try_from(previous_count)
                    .map_err(|_| "تعداد سندهای دسته ورودی معتبر نیست.".to_string())?;
                let recorded_numbers = previous_numbers
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::parse::<i64>)
                    .collect::<Result<Vec<_>, _>>()
                    .ok()
                    .filter(|numbers| !numbers.is_empty());
                if let Some(recorded_numbers) = recorded_numbers {
                    if recorded_numbers.len() != count || recorded_numbers.len() != prepared.len() {
                        // The stored progress cannot describe this payload
                        // any more; discard the marker and rebuild it below.
                        tx.execute(
                            "DELETE FROM import_batches WHERE payload_hash IN (?1, ?2)",
                            params![&payload_hash, &legacy_payload_hash],
                        )
                        .map_err(db::db_error)?;
                    } else {
                        // A replay is only safe while the vouchers recorded by
                        // the batch still exist with the same content. Without
                        // this check, deleting imported vouchers and importing
                        // the same workbook again reported success without
                        // restoring anything.
                        let mut existing = 0usize;
                        let mut first_mismatch = None;
                        {
                            let mut find_voucher = tx
                                .prepare(
                                    "SELECT id, date, type, description
                                       FROM vouchers WHERE number = ?1",
                                )
                                .map_err(db::db_error)?;
                            let mut find_lines = tx
                                .prepare(
                                    "SELECT description, debit, credit
                                       FROM voucher_lines
                                      WHERE voucher_id = ?1
                                      ORDER BY row_no, id",
                                )
                                .map_err(db::db_error)?;
                            for (index, stored_number) in recorded_numbers.iter().enumerate() {
                                let found = find_voucher
                                    .query_row(params![stored_number], |row| {
                                        Ok((
                                            row.get::<_, i64>(0)?,
                                            row.get::<_, String>(1)?,
                                            row.get::<_, String>(2)?,
                                            row.get::<_, String>(3)?,
                                        ))
                                    })
                                    .optional()
                                    .map_err(db::db_error)?;
                                let Some((voucher_id, date, voucher_type, description)) = found
                                else {
                                    continue;
                                };
                                existing += 1;
                                let (
                                    _,
                                    expected_date,
                                    expected_type,
                                    expected_description,
                                    expected_lines,
                                ) = &prepared[index];
                                let mut matches = date == *expected_date
                                    && voucher_type == *expected_type
                                    && description == *expected_description;
                                if matches {
                                    let mut line_count = 0usize;
                                    let rows = find_lines
                                        .query_map(params![voucher_id], |row| {
                                            Ok((
                                                row.get::<_, String>(0)?,
                                                row.get::<_, i64>(1)?,
                                                row.get::<_, i64>(2)?,
                                            ))
                                        })
                                        .map_err(db::db_error)?;
                                    for row in rows {
                                        let (line_description, debit, credit) =
                                            row.map_err(db::db_error)?;
                                        let Some(expected) = expected_lines.get(line_count) else {
                                            matches = false;
                                            break;
                                        };
                                        if line_description != expected.description
                                            || debit != expected.debit
                                            || credit != expected.credit
                                        {
                                            matches = false;
                                            break;
                                        }
                                        line_count += 1;
                                    }
                                    if line_count != expected_lines.len() {
                                        matches = false;
                                    }
                                }
                                if !matches {
                                    first_mismatch = Some(*stored_number);
                                }
                            }
                        }
                        if existing == count && first_mismatch.is_none() {
                            return Ok(count);
                        }
                        if let Some(number) = first_mismatch {
                            return Err(format!(
                                "سند شماره {} در پایگاه داده با محتوای همین فایل همخوانی ندارد؛ برای جلوگیری از ورود تکراری، ورود انجام نشد.",
                                db::to_fa_digits(number)
                            ));
                        }
                        if existing > 0 {
                            return Err(format!(
                                "بخشی از اسناد این ورود پیش‌تر حذف شده است ({} از {} سند موجود است). برای ورود دوباره، ابتدا همهٔ اسناد باقی‌ماندهٔ این ورود را حذف کنید و سپس فایل را دوباره وارد کنید.",
                                db::to_fa_digits(existing),
                                db::to_fa_digits(count)
                            ));
                        }
                        // None of the recorded vouchers exist any more: the
                        // batch marker is stale because its vouchers and maybe
                        // their accounts were deleted after the import. Drop
                        // the marker and import the workbook again below.
                        tx.execute(
                            "DELETE FROM import_batches WHERE payload_hash IN (?1, ?2)",
                            params![&payload_hash, &legacy_payload_hash],
                        )
                        .map_err(db::db_error)?;
                    }
                } else {
                    // Batches written before the number list was populated
                    // cannot be verified; keep the historical replay
                    // behaviour for them.
                    return Ok(count);
                }
            }
            if is_cancelled() {
                // Dropping the transaction rolls back; nothing is committed.
                return Err(crate::excel::BULK_IMPORT_CANCELLED.to_string());
            }
            let mut imported_numbers = Vec::with_capacity(prepared.len());
            let incoming_numbers: HashSet<i64> = prepared.iter().map(|item| item.0).collect();
            let mut used_numbers = HashSet::new();
            {
                let mut statement = tx.prepare("SELECT number FROM vouchers").map_err(db::db_error)?;
                let rows = statement.query_map([], |row| row.get::<_, i64>(0)).map_err(db::db_error)?;
                for row in rows {
                    used_numbers.insert(row.map_err(db::db_error)?);
                }
            }
            let mut next_number = used_numbers.iter().copied().max().unwrap_or(0);
            for account in accounts {
                let code = db::normalize_code(&account.code);
                let name = db::normalize_fa_text(&account.name)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                let parent_code = db::normalize_code(&account.parent_code);
                let group_code = db::normalize_code(&account.group_code);
                if code.is_empty()
                    || code.len() > 50
                    || !code.chars().all(|ch| ch.is_ascii_digit())
                    || name.is_empty()
                    || name.chars().count() > 255
                    || !(1..=db::MAX_ACCOUNT_LEVEL).contains(&account.level)
                {
                    return Err("اطلاعات یکی از حساب‌های فایل اکسل معتبر نیست.".to_string());
                }
                let existing = tx
                    .query_row(
                        "SELECT level, group_code, COALESCE(parent_code, ''), nature
                           FROM accounts WHERE code = ?1",
                        params![code],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(db::db_error)?;
                if let Some((level, existing_group, existing_parent, existing_nature)) = existing {
                    if level != account.level
                        || existing_group != group_code
                        || existing_parent != parent_code
                    {
                        return Err(format!(
                            "حساب {} از قبل با گروه، سطح یا حساب بالاتر متفاوت ثبت شده است.",
                            db::to_fa_digits(&code)
                        ));
                    }
                    if account.nature_is_explicit && existing_nature != account.nature {
                        return Err(format!(
                            "ماهیت حساب {} با مقدار واردشده در فایل اکسل متفاوت است.",
                            db::to_fa_digits(&code)
                        ));
                    }
                    continue;
                }
                if !matches!(account.nature.as_str(), "debtor" | "creditor" | "dual") {
                    return Err(format!(
                        "ماهیت حساب جدید {} مشخص نیست؛ برای گروه‌های سایر درآمدها و هزینه‌ها، ستون ماهیت مربوط به حساب را تکمیل کنید.",
                        db::to_fa_digits(&code)
                    ));
                }
                let group_exists = tx
                    .query_row("SELECT 1 FROM account_groups WHERE code = ?1", params![group_code], |_| Ok(()))
                    .optional()
                    .map_err(db::db_error)?
                    .is_some();
                if !group_exists {
                    return Err(format!("گروه حساب {} برای حساب {} پیدا نشد.", group_code, code));
                }

                let parent_name = if account.level == 1 {
                    if !parent_code.is_empty() {
                        return Err("حساب کل فایل اکسل نباید حساب بالاتر داشته باشد.".to_string());
                    }
                    String::new()
                } else {
                    if parent_code.is_empty() || code == parent_code {
                        return Err(format!("ساختار حساب {} در فایل اکسل معتبر نیست.", db::to_fa_digits(&code)));
                    }
                    let parent = tx
                        .query_row(
                            "SELECT name, level, group_code, is_active FROM accounts WHERE code = ?1",
                            params![parent_code],
                            |row| {
                                Ok((
                                    row.get::<_, String>(0)?,
                                    row.get::<_, i64>(1)?,
                                    row.get::<_, String>(2)?,
                                    row.get::<_, i64>(3)?,
                                ))
                            },
                        )
                        .optional()
                        .map_err(db::db_error)?
                        .ok_or_else(|| format!("حساب بالاتر {} برای {} پیدا نشد.", db::to_fa_digits(&parent_code), db::to_fa_digits(&code)))?;
                    if parent.1 != account.level - 1 || parent.2 != group_code {
                        return Err(format!("گروه یا سطح حساب بالاتر {} با حساب {} سازگار نیست.", db::to_fa_digits(&parent_code), db::to_fa_digits(&code)));
                    }
                    if parent.3 == 0 {
                        return Err(format!(
                            "حساب بالاتر {} غیرفعال است؛ ابتدا آن را فعال کنید.",
                            db::to_fa_digits(&parent_code)
                        ));
                    }
                    parent.0
                };
                let parent_code_value = if parent_code.is_empty() { None } else { Some(parent_code) };
                tx.execute(
                    "INSERT INTO accounts (code, name, level, group_code, nature, parent_code, parent_name, is_postable) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![code, name, account.level, group_code, account.nature, parent_code_value, parent_name, i64::from(account.level <= 2)],
                )
                .map_err(db::db_error)?;
            }

            // Read account ids once and reuse the two insert statements for
            // every voucher/line instead of preparing the same SQL again for
            // each row. Both are important for a 300k-row workbook.
            let account_ids: HashMap<String, i64> = {
                let mut statement = tx
                    .prepare("SELECT code, id FROM accounts")
                    .map_err(db::db_error)?;
                let rows = statement
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                    })
                    .map_err(db::db_error)?;
                rows.collect::<rusqlite::Result<HashMap<_, _>>>()
                    .map_err(db::db_error)?
            };
            // Most imports contain thousands of vouchers but only one or two
            // fiscal years. Remember the resolved year for each date instead of
            // running the period lookup for every voucher.
            let mut fiscal_year_by_date: HashMap<String, i64> = HashMap::new();
            let mut insert_voucher = tx
                .prepare_cached(
                    "INSERT INTO vouchers (number, date, type, description, fiscal_year_id)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                )
                .map_err(db::db_error)?;
            let mut insert_line = tx
                .prepare_cached(
                    "INSERT INTO voucher_lines
                        (voucher_id, row_no, account_id, account_code, detail1_code, description, debit, credit)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                )
                .map_err(db::db_error)?;

            for (index, (number, date, voucher_type, description, mut lines)) in
                prepared.into_iter().enumerate()
            {
                if is_cancelled() {
                    return Err(crate::excel::BULK_IMPORT_CANCELLED.to_string());
                }
                if index % 10 == 0 {
                    on_progress(index, imported_count);
                }
                normalize_voucher_line_accounts(&tx, &mut lines)?;
                validate_voucher_line_accounts(&tx, None, &lines)?;
                assert_voucher_type_matches_lines(&tx, &voucher_type, &lines)?;
                let mut stored_number = number;
                if used_numbers.contains(&number) {
                    if !renumber_conflicts {
                        return Err(format!("شماره سند {} از قبل در برنامه وجود دارد.", db::to_fa_digits(number)));
                    }
                    loop {
                        next_number = next_number.checked_add(1)
                            .ok_or_else(|| "شماره سند بعدی از محدوده عددی مجاز بیشتر است.".to_string())?;
                        if next_number > MAX_VOUCHER_NUMBER {
                            return Err(format!(
                                "شماره سند بعدی از حداکثر {} بیشتر است.",
                                db::to_fa_digits(MAX_VOUCHER_NUMBER.to_string())
                            ));
                        }
                        if !used_numbers.contains(&next_number) && !incoming_numbers.contains(&next_number) {
                            stored_number = next_number;
                            break;
                        }
                    }
                }
                used_numbers.insert(stored_number);
                let fiscal_year_id = match fiscal_year_by_date.get(&date) {
                    Some(id) => *id,
                    None => {
                        let id = db::ensure_open_period(&tx, &date)?;
                        fiscal_year_by_date.insert(date.clone(), id);
                        id
                    }
                };
                insert_voucher
                    .execute(params![
                        stored_number,
                        date,
                        voucher_type,
                        description,
                        fiscal_year_id
                    ])
                    .map_err(db::db_error)?;
                let voucher_id = tx.last_insert_rowid();
                for (index, line) in lines.iter().enumerate() {
                    let account_id = account_ids
                        .get(&line.account_code)
                        .copied()
                        .ok_or_else(|| {
                            format!(
                                "حساب {} هنگام ثبت ردیف سند یافت نشد.",
                                db::to_fa_digits(&line.account_code)
                            )
                        })?;
                    insert_line
                        .execute(params![
                            voucher_id,
                            index as i64 + 1,
                            account_id,
                            line.account_code,
                            line.detail1_code,
                            line.description,
                            line.debit,
                            line.credit
                        ])
                        .map_err(db::db_error)?;
                }
                imported_numbers.push(stored_number);
                let snapshot = voucher_snapshot(&tx, voucher_id)?;
                audit::log(
                    &tx,
                    "create",
                    "voucher",
                    Some(voucher_id),
                    None,
                    Some(&snapshot.to_string()),
                    "ورود گروهی از فایل اکسل",
                )?;
            }
            on_progress(imported_count, imported_count);
            let numbers_text = imported_numbers
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            tx.execute(
                "INSERT INTO import_batches
                    (payload_hash, file_name, sheet_name, voucher_count, imported_numbers)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    &payload_hash,
                    &file_name,
                    &sheet_name,
                    imported_count as i64,
                    numbers_text
                ],
            )
            .map_err(db::db_error)?;
            let after = serde_json::json!({
                "voucherCount": imported_count,
                "numbers": imported_numbers,
                "groupMappingCounts": {
                    "file": file_group_count,
                    "inferred": inferred_group_count,
                    "manual": manual_group_count,
                },
                "inferredGroupCount": inferred_group_count,
                "inferredGroupMappings": inferred_group_mappings,
                "manualGroupCount": manual_group_count,
                "manualGroupMappings": manual_group_mappings,
            });
            let mut details = format!(
                "ورود گروهی {} سند از فایل «{}»، برگه «{}».",
                db::to_fa_digits(imported_count),
                file_name,
                sheet_name
            );
            if inferred_group_count > 0 {
                details.push_str(&format!(
                    " گروه {} حساب بدون کد گروه در فایل و بر اساس پیشوند کد حدس زده شد؛ جزئیات در فیلد پس از عملیات ثبت شده است.",
                    db::to_fa_digits(inferred_group_count)
                ));
            }
            if manual_group_count > 0 {
                details.push_str(&format!(
                    " {} نگاشت گروه به‌صورت دستی توسط کاربر تأیید یا اصلاح شد؛ منبع هر نگاشت در فیلد پس از عملیات ثبت شده است.",
                    db::to_fa_digits(manual_group_count)
                ));
            }
            audit::log(
                &tx,
                "import",
                "voucher",
                None,
                None,
                Some(&after.to_string()),
                &details,
            )?;
            // A cancellation can arrive after the last per-voucher check and
            // while the batch/audit rows are being written. Check once more at
            // the last possible point so a requested cancel can never commit.
            if is_cancelled() {
                return Err(crate::excel::BULK_IMPORT_CANCELLED.to_string());
            }
            drop(insert_line);
            drop(insert_voucher);
            tx.commit().map_err(db::db_error)?;
            // A bulk load changes the table sizes by orders of magnitude, so
            // refresh SQLite's planner statistics once. It is a best-effort
            // optimisation: the import is already committed, and a failure
            // here must never turn a successful import into an error. The
            // analysis limit keeps it to a fraction of a second on big books.
            let _ = conn.execute_batch(
                "PRAGMA analysis_limit = 2000;
                 ANALYZE voucher_lines;
                 ANALYZE vouchers;
                 PRAGMA analysis_limit = 0;",
            );
            Ok(imported_count)
        })
    }
}
