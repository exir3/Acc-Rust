# Account code policy

This document is the single source of truth for account and group codes. The
form, the Rust validation layer and Excel import/export must all follow it.

## Binding rules (enforced by Rust and the database)

- A code contains ASCII digits only. Persian/Arabic digits are normalized
  before validation.
- A code is at most 50 digits.
- A code must contain at least one non-zero digit (it cannot be all zeroes).
- A code is unique across all accounts, regardless of level or parent.
- A child account references its parent explicitly (`parent_code`) and must:
  - be exactly one level below the parent;
  - use the same `group_code` as the parent.
- An account marked postable may be level 1 or level 2. Level 3 accounts are
  detail accounts (`detail1_code`) and are never posted to directly.

These rules are implemented in `src-tauri/src/db/validation.rs` and
`src-tauri/src/db/accounts.rs`, and are also applied to restored backups in
`src-tauri/src/db/backup.rs`.

## Presentation convention (a suggestion, never a hard block)

The account definition form suggests the historical WebApp widths:

| Level | Suggested width | Suggested child suffix |
| ----- | --------------- | ---------------------- |
| کل (1) | 4 digits | — |
| معین (2) | parent + 2 digits | 2 digits |
| تفصیلی ۱ (3) | parent + 3 digits | 3 digits |

`nextAccountCode()` uses the convention to propose the next free code. When an
operator enters a valid code that does not follow the suggestion, the form
shows a warning and still saves it; the backend and Excel import never reject a
code for its width or prefix.

## Excel import and export

The import reads the explicit parent column/order from the workbook and accepts
any binding-rule-compliant code. Export writes codes unchanged. The old
prefix-match assumption is intentionally not part of the accounting model, so a
legacy chart with non-standard widths can be migrated and re-imported.

## Changing the policy

Changing a binding rule requires:

1. a migration plan for existing codes and voucher references;
2. updates to the Rust validator, the restore validator, the form and the Excel
   importer together;
3. tests covering a non-standard-width code at each level.
