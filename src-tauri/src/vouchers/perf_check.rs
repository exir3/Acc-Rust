//! Manual performance check for the report queries (ignored by default).
//!
//! Builds a book with many accounts and voucher lines, runs the trial balance,
//! income statement and balance sheet, and prints each timing together with a
//! hash of the JSON output. Compare the hashes before and after a change to
//! prove the results did not move.
//!
//! Run in release mode, sized like your data:
//!   set PERF_ACCOUNTS=20000
//!   set PERF_VOUCHERS=60000
//!   cargo test --release perf_and_equivalence -- --ignored --nocapture
use crate::db;
use crate::vouchers::model::{ReportScope, VOUCHER_TYPES};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Instant;

fn env_n(k: &str, d: usize) -> usize {
    std::env::var(k)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(d)
}
fn h<T: serde::Serialize>(v: &T) -> (u64, usize) {
    let s = serde_json::to_string(v).unwrap();
    let mut x = DefaultHasher::new();
    s.hash(&mut x);
    (x.finish(), s.len())
}

#[test]
#[ignore]
fn perf_and_equivalence() {
    let n_detail = env_n("PERF_ACCOUNTS", 600);
    let n_vouchers = env_n("PERF_VOUCHERS", 20_000);
    let db = db::memory_database();
    let t = Instant::now();
    db.with_conn(|conn| {
        let tx = conn.transaction().map_err(db::db_error)?;
        let group: String = tx
            .query_row("SELECT code FROM account_groups WHERE type='دارایی' ORDER BY code LIMIT 1", [], |r| r.get(0))
            .unwrap();
        let rev_group: String = tx
            .query_row("SELECT code FROM account_groups WHERE type='درآمد' ORDER BY code LIMIT 1", [], |r| r.get(0))
            .unwrap();
        // two trees: assets (group) and revenue (rev_group)
        let mut base: Vec<(i64, String, String)> = Vec::new(); // (id, code, group)
        for (root, g) in [("8", &group), ("7", &rev_group)] {
            tx.execute("INSERT INTO accounts (code,name,level,group_code,parent_code) VALUES (?1,?2,1,?3,NULL)", rusqlite::params![root, format!("کل {root}"), g]).unwrap();
            for i in 0..40 {
                let code = format!("{root}{:02}", i);
                tx.execute("INSERT INTO accounts (code,name,level,group_code,parent_code) VALUES (?1,?2,2,?3,?4)", rusqlite::params![code, format!("معین {code}"), g, root]).unwrap();
                let id: i64 = tx.query_row("SELECT id FROM accounts WHERE code=?1", [&code], |r| r.get(0)).unwrap();
                base.push((id, code, g.to_string()));
            }
        }
        let mut details: Vec<(usize, String)> = Vec::new(); // (base idx, detail code)
        for i in 0..n_detail {
            let bi = i % base.len();
            let code = format!("{}{:06}", base[bi].1, i);
            tx.execute("INSERT INTO accounts (code,name,level,group_code,parent_code) VALUES (?1,?2,3,?3,?4)", rusqlite::params![code, format!("تفصیلی {i}"), base[bi].2, base[bi].1]).unwrap();
            details.push((bi, code));
        }
        let mut seed: u64 = 12345;
        let mut rnd = |m: u64| { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); (seed >> 33) % m };
        {
            let mut vs = tx.prepare("INSERT INTO vouchers (number,date,type,description,is_temporary) VALUES (?1,?2,?3,'',?4)").unwrap();
            let mut ls = tx.prepare("INSERT INTO voucher_lines (voucher_id,row_no,account_id,account_code,detail1_code,debit,credit) VALUES (?1,?2,?3,?4,?5,?6,?7)").unwrap();
            for v in 0..n_vouchers {
                let month = 1 + rnd(12); let day = 1 + rnd(28);
                let date = format!("1405/{:02}/{:02}", month, day);
                let typ = match v % 5000 { 1 => VOUCHER_TYPES[1], 2 => VOUCHER_TYPES[2], 3 => VOUCHER_TYPES[3], _ => VOUCHER_TYPES[0] };
                let temp = 1;
                vs.execute(rusqlite::params![v as i64 + 1, date, typ, temp]).unwrap();
                let vid = tx.last_insert_rowid();
                let amount = 1000 + rnd(1_000_000) as i64;
                for row in 0..5 {
                    let di = rnd(details.len() as u64) as usize;
                    let (bi, dcode) = &details[di];
                    let use_detail = rnd(4) != 0; // some lines post to the base account only
                    let (debit, credit) = if row < 4 { if row % 2 == 0 { (amount, 0) } else { (0, amount) } } else { (7, 0) };
                    let det: Option<&str> = if use_detail { Some(dcode.as_str()) } else { None };
                    ls.execute(rusqlite::params![vid, row + 1, base[*bi].0, base[*bi].1, det, debit, credit]).unwrap();
                }
            }
        }
        tx.execute("UPDATE vouchers SET is_temporary = 0 WHERE number % 10 <> 0", []).unwrap();
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_voucher_lines_effective_account ON voucher_lines(COALESCE(NULLIF(detail1_code, ''), account_code));
             CREATE INDEX IF NOT EXISTS idx_vouchers_date_number ON vouchers(date, number);
             CREATE INDEX IF NOT EXISTS idx_vouchers_scope_date_number ON vouchers(is_temporary, date, number);
             ANALYZE;",
        ).unwrap();
        tx.commit().map_err(db::db_error)?;
        Ok(())
    }).unwrap();
    println!(
        "PERF setup {:?} (accounts={}, vouchers={}, lines={})",
        t.elapsed(),
        n_detail + 82,
        n_vouchers,
        n_vouchers * 5
    );

    for (label, scope) in [
        ("working", ReportScope::Working),
        ("official", ReportScope::Official),
        ("temporary", ReportScope::Temporary),
    ] {
        let t = Instant::now();
        let r = db
            .get_trial_balance_filtered_scoped("", "", "", "", true, true, scope)
            .unwrap();
        println!(
            "PERF trial[{label}] all-dates   {:?} rows={} hash={:?}",
            t.elapsed(),
            r.rows.len(),
            h(&r)
        );
        let t = Instant::now();
        let r = db
            .get_trial_balance_filtered_scoped(
                "1405/03/01",
                "1405/09/30",
                "",
                "",
                false,
                true,
                scope,
            )
            .unwrap();
        println!(
            "PERF trial[{label}] range       {:?} rows={} hash={:?}",
            t.elapsed(),
            r.rows.len(),
            h(&r)
        );
        let t = Instant::now();
        let r = db
            .get_trial_balance_filtered_scoped(
                "1405/03/01",
                "1405/09/30",
                "8",
                "8",
                true,
                true,
                scope,
            )
            .unwrap();
        println!(
            "PERF trial[{label}] code-range  {:?} rows={} hash={:?}",
            t.elapsed(),
            r.rows.len(),
            h(&r)
        );
        let t = Instant::now();
        let r = db
            .get_income_statement_scoped("1405/01/01", "1405/12/29", scope)
            .unwrap();
        println!(
            "PERF income[{label}]            {:?} hash={:?}",
            t.elapsed(),
            h(&r)
        );
        let t = Instant::now();
        let r = db
            .get_income_statement_scoped("1405/04/01", "1405/06/31", scope)
            .unwrap();
        println!(
            "PERF income[{label}] sub-range  {:?} hash={:?}",
            t.elapsed(),
            h(&r)
        );
        let t = Instant::now();
        let r = db.get_balance_sheet_scoped("1405/12/29", scope).unwrap();
        println!(
            "PERF balance[{label}]           {:?} hash={:?}",
            t.elapsed(),
            h(&r)
        );
        let t = Instant::now();
        let r = db.get_balance_sheet_scoped("1405/06/15", scope).unwrap();
        println!(
            "PERF balance[{label}] mid-year  {:?} hash={:?}",
            t.elapsed(),
            h(&r)
        );
    }
}
