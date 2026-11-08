# Financial report reference fixture

`report_reference.json` is the expected output of
`src-tauri/src/vouchers/reports/reference_tests.rs`. The test builds the chart
and book in code, runs the trial balance, income statement and balance sheet,
and compares every total and row with this file. A classification change that
alters a report therefore fails CI until the fixture is updated deliberately.

## Scope of this fixture

This fixture currently pins the **trial balance, income statement and balance
sheet**. The journal and ledger have their own Rust unit tests. The four
supplementary statements (cash flow, changes in equity, cost of goods sold and
profit appropriation) are covered by computational tests in
`src-tauri/src/vouchers/tests.rs` and print-document tests in
`src-tauri/src/report/tests.rs`, but they are **not yet part of
`report_reference.json`**. They still require an independent accounting
reference or manual review before an official release.

## Dataset coverage

The reference book is small enough to audit by hand and covers:

- current and non-current assets, current and non-current liabilities, equity;
- revenue, contra-revenue (debit balance in the revenue group), cost of goods
  sold, selling and administrative expenses;
- other operating revenue/expense, non-operating revenue/expense, finance cost
  and income tax;
- explicit statement sections for all 14 section keys;
- the built-in dual groups `10` and `11` with both debit and credit balances;
- debtor, creditor and dual account natures;
- an opening voucher, a temporary closing voucher and a final closing voucher;
- working, official and temporary scopes (one deliberately temporary draft
  sale is included in working reports only);
- a fiscal-year boundary (`1405/12/29` to `1406/01/01`) and a leap-year date
  (`1403/12/30`);
- a level-3 detail account (`102001001`) and accounts without details;
- multi-period activity where February opening balances come from January and
  from the opening voucher.

## Key hand-checked totals

| Scenario | Opening debit/credit | Period debit/credit | Closing debit/credit |
| --- | ---: | ---: | ---: |
| Working, 1405/01/01–1405/02/28 | 0 / 0 | 2,056,000 / 2,056,000 | 1,753,000 / 1,753,000 |
| Working, 1405/02/01–1405/02/28 | 1,650,000 / 1,650,000 | 151,000 / 151,000 | 1,753,000 / 1,753,000 |
| Official, 1405/01/01–1405/02/28 | 0 / 0 | 2,006,000 / 2,006,000 | 1,703,000 / 1,703,000 |

| Income statement | Working | Official |
| --- | ---: | ---: |
| Revenue | 330,000 | 280,000 |
| Cost of goods sold | 180,000 | 180,000 |
| Gross profit | 150,000 | 100,000 |
| Operating profit | 82,000 | 32,000 |
| Profit before tax | 83,000 | 33,000 |
| Net profit | 75,000 | 25,000 |

| Balance sheet | Working before closing | Official before closing | Working at year end | Official at year end |
| --- | ---: | ---: | ---: | ---: |
| Assets | 1,425,000 | 1,375,000 | 1,425,000 | 1,375,000 |
| Liabilities | 450,000 | 450,000 | 450,000 | 450,000 |
| Equity | 900,000 | 900,000 | 925,000 | 925,000 |
| Profit | 75,000 | 25,000 | 50,000 | 0 |
| Balanced | yes | yes | yes | yes |

At year end the final closing voucher is hidden until the next opening
voucher; the official position therefore remains the pre-final-closing
position. After the `1406/01/01` opening voucher, the final closing and opening
cancel each other.

## Approval status

The values above were derived from the accounting identities of the dataset and
reviewed against the report code. They are an **internal reference, not yet a
formal accountant/auditor sign-off**. The four supplementary statements are
outside this fixture and need independent reference examples. Before an
official release, the responsible accountant should review the dataset,
expected values and supplementary statements, and record approval in
`Docs/Accounting-Report-Signoff.md`. The release CI job runs
`node scripts/check-report-signoff.mjs`; it fails unless that document says
`Status: approved`, names the reviewer and date, and its `Fixture-SHA256`
matches this directory's `report_reference.json`. Editing the fixture therefore
invalidates the previous approval. CI already enforces that no report change
passes without an intentional fixture update.

## Updating the fixture

Do not regenerate the file merely to make a failing test pass. First decide
whether the classification change is correct, update the hand-checked tables
above, then replace `report_reference.json` with the reviewed output and run:

```text
cargo test --lib vouchers::reports::reference_tests
```
