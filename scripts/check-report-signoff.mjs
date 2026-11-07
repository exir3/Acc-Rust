// Release gate for the independent accounting review of the report fixture.
//
// The signature block in Docs/Accounting-Report-Signoff.md is the only source
// of truth. A release build fails unless:
//   - Status is `approved`,
//   - Reviewer is filled in,
//   - Reviewed is a YYYY-MM-DD date,
//   - Fixture-SHA256 matches the current report_reference.json byte for byte.
//
// The hash check means a fixture edit automatically invalidates an old
// approval and forces a fresh review.
//
// Usage:
//   node scripts/check-report-signoff.mjs             # verify (CI gate)
//   node scripts/check-report-signoff.mjs --print-hash # print fixture hash

import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const fixturePath = path.join(
  root,
  'src-tauri/src/vouchers/reports/fixtures/report_reference.json',
);
const signoffPath = path.join(root, 'Docs/Accounting-Report-Signoff.md');

function fail(message) {
  console.error(`report sign-off: ${message}`);
  process.exit(1);
}

const fixtureBytes = await readFile(fixturePath);
const fixtureHash = createHash('sha256').update(fixtureBytes).digest('hex');

if (process.argv.includes('--print-hash')) {
  console.log(fixtureHash);
  process.exit(0);
}

const document = await readFile(signoffPath, 'utf8');
const block = document.match(/<!--\s*RELEASE-SIGNOFF([\s\S]*?)-->/);
if (!block) {
  fail(`no RELEASE-SIGNOFF block in ${signoffPath}`);
}

const fields = {};
for (const line of block[1].split(/\r?\n/)) {
  const match = line.match(/^\s*([A-Za-z][A-Za-z-]*):\s*(.*)$/);
  if (match) fields[match[1].toLowerCase()] = match[2].trim();
}

const status = (fields.status || '').toLowerCase();
if (status !== 'approved') {
  fail('accounting reports are not approved yet (set Status: approved after review)');
}
if (!fields.reviewer) {
  fail('Reviewer is empty');
}
if (!/^\d{4}-\d{2}-\d{2}$/.test(fields.reviewed || '')) {
  fail('Reviewed must be a YYYY-MM-DD date');
}
if ((fields['fixture-sha256'] || '').toLowerCase() !== fixtureHash) {
  fail(
    'Fixture-SHA256 does not match report_reference.json; '
      + `run with --print-hash (current hash ${fixtureHash}) and review the change`,
  );
}

console.log(
  `report sign-off valid: ${fields.reviewer} on ${fields.reviewed} `
    + `(fixture ${fixtureHash.slice(0, 12)}…)`,
);
