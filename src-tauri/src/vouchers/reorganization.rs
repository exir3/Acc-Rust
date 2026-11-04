//! Merging same-date vouchers and renumbering the book.

use crate::db::{self, Database};
use crate::vouchers::model::{
    VoucherMergePreview, VoucherNumberChange, VoucherReorganizationPreview,
    VoucherReorganizationResult, MAX_DESCRIPTION, MAX_LINES, MAX_VOUCHER_NUMBER, TOUCHED_AT,
    VOUCHER_TYPES,
};
use crate::vouchers::validation::{checked_sum, validate_voucher_date};
use rusqlite::{params, Connection};
use std::collections::{BTreeMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

#[derive(Debug, Clone, Hash)]
struct ReorganizationVoucher {
    id: i64,
    number: i64,
    date: String,
    voucher_type: String,
    description: String,
}

#[derive(Debug, Clone, Hash)]
struct ReorganizationLine {
    id: i64,
    voucher_id: i64,
    row_no: i64,
    account_code: String,
    detail1_code: Option<String>,
    description: String,
    debit: i64,
    credit: i64,
}

#[derive(Debug, Clone, Hash)]
struct ReorganizationSnapshot {
    vouchers: Vec<ReorganizationVoucher>,
    lines: Vec<ReorganizationLine>,
}

#[derive(Debug, Clone)]
struct ReorganizationMergeGroup {
    keep_id: i64,
    source_ids: Vec<i64>,
    description: String,
}

#[derive(Debug, Clone)]
struct ReorganizationPlan {
    preview: VoucherReorganizationPreview,
    merge_groups: Vec<ReorganizationMergeGroup>,
    number_changes: Vec<(i64, VoucherNumberChange)>,
}

fn validate_reorganization_operation(operation: &str) -> Result<(), String> {
    if matches!(operation, "sort" | "merge" | "merge-then-sort") {
        Ok(())
    } else {
        Err("عملیات مرتب‌سازی یا ادغام معتبر نیست.".to_string())
    }
}

fn read_reorganization_snapshot(conn: &Connection) -> Result<ReorganizationSnapshot, String> {
    let vouchers = conn
        .prepare("SELECT id, number, date, type, description FROM vouchers ORDER BY id")
        .map_err(db::db_error)?
        .query_map([], |row| {
            Ok(ReorganizationVoucher {
                id: row.get(0)?,
                number: row.get(1)?,
                date: row.get(2)?,
                voucher_type: row.get(3)?,
                description: row.get(4)?,
            })
        })
        .map_err(db::db_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db::db_error)?;
    let lines = conn
        .prepare(
            "SELECT id, voucher_id, row_no, account_code, NULLIF(detail1_code,''), description, debit, credit FROM voucher_lines ORDER BY id",
        )
        .map_err(db::db_error)?
        .query_map([], |row| {
            Ok(ReorganizationLine {
                id: row.get(0)?,
                voucher_id: row.get(1)?,
                row_no: row.get(2)?,
                account_code: row.get(3)?,
                detail1_code: row.get(4)?,
                description: row.get(5)?,
                debit: row.get(6)?,
                credit: row.get(7)?,
            })
        })
        .map_err(db::db_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db::db_error)?;
    Ok(ReorganizationSnapshot { vouchers, lines })
}

fn build_reorganization_plan(
    operation: &str,
    snapshot: &ReorganizationSnapshot,
) -> Result<ReorganizationPlan, String> {
    validate_reorganization_operation(operation)?;
    let do_merge = operation != "sort";
    let do_sort = operation != "merge";
    let mut lines_by_voucher = BTreeMap::<i64, Vec<&ReorganizationLine>>::new();
    for line in &snapshot.lines {
        lines_by_voucher
            .entry(line.voucher_id)
            .or_default()
            .push(line);
    }

    let mut grouped = BTreeMap::<(String, String), Vec<&ReorganizationVoucher>>::new();
    if do_merge {
        for voucher in snapshot
            .vouchers
            .iter()
            .filter(|voucher| voucher.voucher_type == VOUCHER_TYPES[0])
        {
            let date = validate_voucher_date(&voucher.date)?;
            grouped
                .entry((date, voucher.voucher_type.clone()))
                .or_default()
                .push(voucher);
        }
    }

    let mut merge_groups = Vec::new();
    let mut unbalanced_groups = Vec::new();
    let mut merge_plan = Vec::new();
    let mut removed_ids = HashSet::new();
    for ((date, _), mut vouchers) in grouped {
        if vouchers.len() < 2 {
            continue;
        }
        vouchers.sort_by_key(|voucher| (voucher.number, voucher.id));
        let all_lines: Vec<&ReorganizationLine> = vouchers
            .iter()
            .flat_map(|voucher| {
                lines_by_voucher
                    .get(&voucher.id)
                    .into_iter()
                    .flatten()
                    .copied()
            })
            .collect();
        let total_debit = checked_sum(
            all_lines.iter().map(|line| line.debit),
            "جمع بدهکار گروه ادغام",
        )?;
        let total_credit = checked_sum(
            all_lines.iter().map(|line| line.credit),
            "جمع بستانکار گروه ادغام",
        )?;
        let balanced = total_debit > 0
            && total_debit == total_credit
            && all_lines.iter().all(|line| {
                line.debit >= 0 && line.credit >= 0 && ((line.debit > 0) ^ (line.credit > 0))
            });
        let preview = VoucherMergePreview {
            date,
            source_voucher_numbers: vouchers.iter().map(|voucher| voucher.number).collect(),
            keep_voucher_number: vouchers[0].number,
            line_count: all_lines.len(),
            total_debit,
            total_credit,
            reason: String::new(),
        };
        let mut merged_description = vouchers[0].description.clone();
        for source in vouchers.iter().skip(1) {
            let separator = if merged_description.is_empty() {
                ""
            } else {
                "\n"
            };
            merged_description.push_str(&format!(
                "{separator}ادغام از سند شماره {}: {}",
                source.number, source.description
            ));
        }
        let mut rejection_reasons = Vec::new();
        if !balanced {
            rejection_reasons.push("ردیف‌های گروه تراز نیستند.".to_string());
        }
        if all_lines.len() > MAX_LINES {
            rejection_reasons.push(format!(
                "تعداد ردیف‌ها از ظرفیت {} ردیفی فرم بیشتر است.",
                db::to_fa_digits(MAX_LINES.to_string())
            ));
        }
        if merged_description.chars().count() > MAX_DESCRIPTION {
            rejection_reasons.push(format!(
                "شرح‌های سرآیند پس از ادغام از سقف {} نویسه بیشتر می‌شوند.",
                db::to_fa_digits(MAX_DESCRIPTION.to_string())
            ));
        }
        if !rejection_reasons.is_empty() {
            let mut preview = preview;
            preview.reason = rejection_reasons.join(" ");
            unbalanced_groups.push(preview);
            continue;
        }
        let keep_id = vouchers[0].id;
        let source_ids: Vec<i64> = vouchers.iter().skip(1).map(|voucher| voucher.id).collect();
        removed_ids.extend(source_ids.iter().copied());
        merge_plan.push(ReorganizationMergeGroup {
            keep_id,
            source_ids,
            description: merged_description,
        });
        merge_groups.push(preview);
    }

    let mut number_changes = Vec::new();
    if do_sort {
        let mut remaining: Vec<(&ReorganizationVoucher, String)> = snapshot
            .vouchers
            .iter()
            .filter(|voucher| !removed_ids.contains(&voucher.id))
            .map(|voucher| Ok((voucher, validate_voucher_date(&voucher.date)?)))
            .collect::<Result<_, String>>()?;
        remaining.sort_by(|(left, left_date), (right, right_date)| {
            left_date
                .cmp(right_date)
                .then_with(|| left.number.cmp(&right.number))
                .then_with(|| left.id.cmp(&right.id))
        });
        for (index, (voucher, date)) in remaining.into_iter().enumerate() {
            let new_number = i64::try_from(index + 1)
                .map_err(|_| "تعداد اسناد از محدوده شماره‌گذاری مجاز بیشتر است.".to_string())?;
            if voucher.number != new_number {
                let change = VoucherNumberChange {
                    old_number: voucher.number,
                    new_number,
                    date,
                };
                number_changes.push((voucher.id, change));
            }
        }
    }

    let mut hasher = DefaultHasher::new();
    operation.hash(&mut hasher);
    snapshot.hash(&mut hasher);
    let plan_token = format!("{:016x}", hasher.finish());
    let removed_voucher_count = removed_ids.len();
    let preview = VoucherReorganizationPreview {
        operation: operation.to_string(),
        total_vouchers: snapshot.vouchers.len(),
        total_vouchers_after: snapshot.vouchers.len() - removed_voucher_count,
        merge_groups,
        unbalanced_groups,
        removed_voucher_count,
        number_changes: number_changes
            .iter()
            .map(|(_, change)| change.clone())
            .collect(),
        plan_token,
    };
    Ok(ReorganizationPlan {
        preview,
        merge_groups: merge_plan,
        number_changes,
    })
}

impl Database {
    pub fn preview_voucher_reorganization(
        &self,
        operation: &str,
    ) -> Result<VoucherReorganizationPreview, String> {
        validate_reorganization_operation(operation)?;
        self.with_conn(|conn| {
            let snapshot = read_reorganization_snapshot(conn)?;
            Ok(build_reorganization_plan(operation, &snapshot)?.preview)
        })
    }

    pub fn apply_voucher_reorganization(
        &self,
        operation: &str,
        plan_token: &str,
    ) -> Result<VoucherReorganizationResult, String> {
        validate_reorganization_operation(operation)?;
        if plan_token.is_empty() {
            return Err("پیش‌نمایش منقضی یا نامعتبر است؛ دوباره پیش‌نمایش بگیرید.".to_string());
        }
        let operation = operation.to_string();
        let plan_token = plan_token.to_string();
        self.with_conn(move |conn| {
            let tx = conn.transaction().map_err(db::db_error)?;
            let snapshot = read_reorganization_snapshot(&tx)?;
            let plan = build_reorganization_plan(&operation, &snapshot)?;
            if plan.preview.plan_token != plan_token {
                return Err("اسناد پس از پیش‌نمایش تغییر کرده‌اند؛ پیش‌نمایش جدید بگیرید.".to_string());
            }

            let mut removed_voucher_count = 0;
            for group in &plan.merge_groups {
                let mut next_row: i64 = tx
                    .query_row(
                        "SELECT COALESCE(MAX(row_no), 0) FROM voucher_lines WHERE voucher_id = ?1",
                        params![group.keep_id],
                        |row| row.get(0),
                    )
                    .map_err(db::db_error)?;
                for source_id in &group.source_ids {
                    let mut statement = tx
                        .prepare("SELECT id FROM voucher_lines WHERE voucher_id = ?1 ORDER BY row_no, id")
                        .map_err(db::db_error)?;
                    let source_lines = statement
                        .query_map(params![source_id], |row| row.get::<_, i64>(0))
                        .map_err(db::db_error)?
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .map_err(db::db_error)?;
                    drop(statement);
                    for line_id in source_lines {
                        next_row = next_row.checked_add(1)
                            .ok_or_else(|| "شماره ردیف‌های سند از محدوده مجاز بیشتر است.".to_string())?;
                        tx.execute(
                            "UPDATE voucher_lines SET voucher_id = ?1, row_no = ?2 WHERE id = ?3",
                            params![group.keep_id, next_row, line_id],
                        )
                        .map_err(db::db_error)?;
                    }
                    tx.execute("DELETE FROM vouchers WHERE id = ?1", params![source_id])
                        .map_err(db::db_error)?;
                    removed_voucher_count += 1;
                }
                tx.execute(
                    &format!(
                        "UPDATE vouchers
                            SET description = ?1, updated_at = {TOUCHED_AT}
                          WHERE id = ?2"
                    ),
                    params![group.description, group.keep_id],
                )
                .map_err(db::db_error)?;
            }

            for (index, (voucher_id, _)) in plan.number_changes.iter().enumerate() {
                let offset = i64::try_from(index)
                    .map_err(|_| "تعداد اسناد برای بازشماره‌گذاری بیش از حد مجاز است.".to_string())?;
                let temporary_number = MAX_VOUCHER_NUMBER
                    .checked_add(1)
                    .and_then(|base| base.checked_add(offset))
                    .ok_or_else(|| "شماره موقت بازشماره‌گذاری از محدوده خارج است.".to_string())?;
                tx.execute(
                    "UPDATE vouchers SET number = ?1 WHERE id = ?2",
                    params![temporary_number, voucher_id],
                )
                .map_err(db::db_error)?;
            }
            for (voucher_id, change) in &plan.number_changes {
                tx.execute(
                    &format!(
                        "UPDATE vouchers
                            SET number = ?1, updated_at = {TOUCHED_AT}
                          WHERE id = ?2"
                    ),
                    params![change.new_number, voucher_id],
                )
                .map_err(db::db_error)?;
            }

            let result = VoucherReorganizationResult {
                operation,
                merged_group_count: plan.merge_groups.len(),
                removed_voucher_count,
                renumbered_count: plan.number_changes.len(),
            };
            tx.commit().map_err(db::db_error)?;
            Ok(result)
        })
    }
}
