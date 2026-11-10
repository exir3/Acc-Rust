'use strict';

/* ── State ──────────────────────────────────────────────────── */

const state = {
  accounts: [],
  accountByCode: new Map(),
  settings: { companyName: '', fiscalYear: '', today: '' },
  activeFiscalYear: null,
  voucher: null,
  makePermanent: false,
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

function isLockedVoucher() {
  return Boolean(state.voucher && state.voucher.isTemporary === false);
}

// Accounts already saved on the open voucher stay usable even after being
// marked inactive; new lines may only pick an active definition.
function voucherAlreadyUsesAccount(code) {
  const target = digitsOnly(code);
  return (state.voucher?.lines || []).some(
    (savedLine) => digitsOnly(savedLine.accountCode) === target
      || digitsOnly(savedLine.detail1Code || '') === target
  );
}

// The grid's «کد حساب» column shows the full code the operator picked: a
// کل/معین code or a تفصیلی code. A تفصیلی is never posted directly; it is
// persisted as its parent معین plus detail1Code.
function accountPostingPair(account) {
  if (!account) return null;
  if (Number(account.level) === 3) {
    const parent = accountForCode(account.parentCode);
    if (!parent || Number(parent.level) !== 2) return null;
    return { base: parent, detail: account };
  }
  return { base: account, detail: null };
}

function accountAllowedForNewLine(account, row = null) {
  if (!account) return false;
  const pair = accountPostingPair(account);
  if (!pair) return false;
  // Only the exact saved row linked to this account may use an inactive or
  // legacy account. A new row, or a row whose account changed, must pass the
  // normal active/postable checks.
  const savedRow = !!row && !!row.dataset.lineId
    && row.dataset.savedAccount === digitsOnly(account.code);
  // An inactive ancestor makes every descendant unusable for a new line,
  // even when the leaf itself still says it is active.
  const activeOk = savedRow || accountChainActive(account);
  const postableOk = savedRow || pair.base.isPostable;
  return activeOk && postableOk;
}

/// True when the account and every ancestor up to the root are active.
function accountChainActive(account) {
  if (!account) return false;
  const seen = new Set();
  let current = account;
  while (current) {
    if (current.isActive === false) return false;
    const parentCode = digitsOnly(current.parentCode || '');
    if (!parentCode || seen.has(parentCode)) break;
    seen.add(parentCode);
    current = accountForCode(parentCode);
  }
  return true;
}

function levelLabel(level) {
  return ({ 1: 'کل', 2: 'معین', 3: 'تفصیلی ۱', 4: 'تفصیلی ۲ (قدیمی)' })[Number(level)] || '—';
}

