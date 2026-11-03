'use strict';

/* ── State ──────────────────────────────────────────────────── */

const state = {
  accounts: [],
  accountByCode: new Map(),
  settings: { companyName: '', fiscalYear: '', today: '' },
  voucher: null,
  overflowVoucherLines: 0,
  nextNumber: 1,
  dirty: false,
  restoredConflict: false,
  busy: false,
  allowClose: false,
  ready: false,
  activeRow: null,
  ledger: { account: null, report: null, from: '', to: '', query: '', returnFocus: null, request: 0 },
  transfer: { mode: 'copy', source: null, selectedIds: new Set() },
  excelPreview: null,
  accountPickerPath: [],
  accountPickerShowAll: false,
};

const accountForCode = (code) => state.accountByCode.get(digitsOnly(code)) || null;

// Accounts already saved on the open voucher stay usable even after being
// marked inactive; new lines may only pick an active definition.
function voucherAlreadyUsesAccount(code) {
  const target = digitsOnly(code);
  return (state.voucher?.lines || []).some(
    (savedLine) => digitsOnly(savedLine.accountCode) === target
  );
}

function accountAllowedForNewLine(account, row = null) {
  if (!account) return false;
  // Only the exact saved row linked to this account may use an inactive or
  // legacy (level-4) account. A new row, or a row whose account changed, must
  // pass the normal active/postable checks.
  const savedRow = !!row && !!row.dataset.lineId
    && row.dataset.savedAccount === digitsOnly(account.code);
  const activeOk = account.isActive !== false || savedRow;
  const postableOk = account.isPostable || (Number(account.level) > 3 && savedRow);
  return activeOk && postableOk;
}

function levelLabel(level) {
  return ({ 1: 'کل', 2: 'معین', 3: 'تفصیلی ۱', 4: 'تفصیلی ۲ (قدیمی)' })[Number(level)] || '—';
}

