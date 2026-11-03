'use strict';

/* ============================================================
   Accounting voucher window
   ============================================================
   Opens in its own OS window and writes real vouchers to the
   local SQLite database. The window can load an existing
   voucher, start a new one, copy, print and delete.

   Amounts are held as whole Rials. Persian and Arabic-Indic
   digits typed or pasted are converted to ASCII on the way in;
   the FD font draws ASCII digits as Persian.
   ============================================================ */

const TAURI = window.__TAURI__ || null;
const invoke = TAURI && TAURI.core ? TAURI.core.invoke.bind(TAURI.core) : null;
const appWindow = TAURI && TAURI.window && TAURI.window.getCurrentWindow
  ? TAURI.window.getCurrentWindow()
  : null;

const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => Array.from(root.querySelectorAll(sel));

const VF_FA_DIGITS = '۰۱۲۳۴۵۶۷۸۹';
const VF_AR_DIGITS = '٠١٢٣٤٥٦٧٨٩';

const VOUCHER_TYPES = [
  'سند حسابداری عادی',
  'سند حسابداری افتتاحیه',
  'سند حسابداری اختتامیه',
  'سند بستن حساب‌های موقت',
];

const vfFa = (value) => String(value).replace(/\d/g, (d) => VF_FA_DIGITS[Number(d)]);

/* Every value kept in the form/model is ASCII; the FD font draws them
   as Persian digits. Persian and Arabic-Indic digits typed or pasted
   are converted on the way in. */
function toAsciiDigits(value) {
  return String(value)
    .replace(/[۰-۹]/g, (d) => String(VF_FA_DIGITS.indexOf(d)))
    .replace(/[٠-٩]/g, (d) => String(VF_AR_DIGITS.indexOf(d)));
}

const digitsOnly = (value) => toAsciiDigits(value).replace(/[^\d]/g, '');

/* Search should not care about digit script, Arabic yeh/kaf or ZWNJ. */
function normalizeSearch(value) {
  return toAsciiDigits(String(value || ''))
    .replace(/[يى]/g, 'ی')
    .replace(/ك/g, 'ک')
    .replace(/\u200c/g, '')
    .replace(/\s+/g, ' ')
    .trim()
    .toLowerCase();
}

function errorText(error) {
  if (typeof error === 'string') return error;
  if (error && typeof error.message === 'string') return error.message;
  return String(error ?? 'خطای نامشخص');
}

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

/* ── Small in-window notice ─────────────────────────────────── */

let toastTimer = 0;

function toast(message, tone = 'info') {
  const el = $('#vfToast');
  if (!el) return;
  el.textContent = message;
  el.dataset.tone = tone;
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { el.hidden = true; }, 3600);
}

function setStatus(tone, text) {
  const box = $('#vfDocStatus');
  const label = $('#vfDocStatusText');
  if (box) box.dataset.tone = tone;
  if (label) label.textContent = text;
}

/* ── Amounts ────────────────────────────────────────────────── */

const groupDigits = (digits) => String(digits).replace(/\B(?=(\d{3})+(?!\d))/g, ',');

function formatAmount(value) {
  const big = typeof value === 'bigint' ? value : BigInt(digitsOnly(value) || '0');
  const negative = big < 0n;
  const digits = (negative ? -big : big).toString();
  return `${negative ? '-' : ''}${groupDigits(digits)}`;
}

const parseAmount = (value) => BigInt(digitsOnly(value) || '0');

// Fallbacks mirror the backend. `loadSettings` replaces them with the values
// returned by `get_settings`, so a limit change needs no front-end edit.
let MAX_AMOUNT = 999_999_999_999_999n;
let MAX_VOUCHER_LINES = 1000;
let MAX_VOUCHER_NUMBER = Number.MAX_SAFE_INTEGER;
let MAX_DESCRIPTION = 1000;
let MAX_LINE_DESCRIPTION = 500;

function applyLimits(limits) {
  if (!limits) return;
  const lines = Number(limits.maxVoucherLines);
  if (Number.isSafeInteger(lines) && lines > 0) MAX_VOUCHER_LINES = lines;
  const maxAmount = Number(limits.maxAmount);
  if (Number.isSafeInteger(maxAmount) && maxAmount > 0) MAX_AMOUNT = BigInt(maxAmount);
  const maxNumber = Number(limits.maxVoucherNumber);
  if (Number.isSafeInteger(maxNumber) && maxNumber > 0) MAX_VOUCHER_NUMBER = maxNumber;

  const maxDescription = Number(limits.maxDescription);
  if (Number.isSafeInteger(maxDescription) && maxDescription > 0) {
    MAX_DESCRIPTION = maxDescription;
    const area = $('#vfDocDesc');
    if (area) area.maxLength = maxDescription;
    updateDescCounter();
  }
  const maxLineDescription = Number(limits.maxLineDescription);
  if (Number.isSafeInteger(maxLineDescription) && maxLineDescription > 0) {
    MAX_LINE_DESCRIPTION = maxLineDescription;
    $$('.vf-desc-input').forEach((input) => { input.maxLength = maxLineDescription; });
  }
}

// Mirrors jalali_calendar's 33-year leap rule so the form can reject impossible
// dates (e.g. 31 Mehr or 30 Esfand in a common year) before a network round-trip.
function isJalaliLeapYear(year) {
  const countLeapDays = (value) => Math.trunc(value / 33) * 8
    + Math.trunc((((value % 33) + 3) / 4));
  const shifted = year + 1595;
  return countLeapDays(shifted + 1) - countLeapDays(shifted) === 1;
}

function jalaliMonthLength(year, month) {
  if (month >= 1 && month <= 6) return 31;
  if (month >= 7 && month <= 11) return 30;
  if (month === 12) return isJalaliLeapYear(year) ? 30 : 29;
  return 0;
}

/* ── Voucher grid ───────────────────────────────────────────── */

function voucherRowTemplate() {
  return `
    <tr>
      <td class="vf-rowno"></td>
      <td><input class="vf-code-input" type="text" dir="ltr" inputmode="numeric" autocomplete="off" aria-label="کد حساب" title="برای انتخاب حساب Space یا F4 را بزنید" /></td>
      <td class="vf-detail1" aria-label="تفصیلی ۱"></td>
      <td><input class="vf-desc-input" type="text" dir="rtl" maxlength="${MAX_LINE_DESCRIPTION}" autocomplete="off" aria-label="شرح" /></td>
      <td><input class="vf-amount-input vf-debit-input" type="text" dir="ltr" inputmode="numeric" autocomplete="off" aria-label="بدهکار" /></td>
      <td><input class="vf-amount-input vf-credit-input" type="text" dir="ltr" inputmode="numeric" autocomplete="off" aria-label="بستانکار" /></td>
    </tr>`;
}

function gridRows() {
  const tbody = $('#voucherRows');
  return tbody ? Array.from(tbody.children) : [];
}

function rowHasValue(row) {
  return !!row && Array.from(row.querySelectorAll('input')).some((input) => input.value.trim() !== '');
}

function appendGridRow() {
  const tbody = $('#voucherRows');
  if (!tbody) return null;
  if (gridRows().filter(rowHasValue).length >= MAX_VOUCHER_LINES) return null;
  tbody.insertAdjacentHTML('beforeend', voucherRowTemplate());
  return tbody.lastElementChild;
}

// Row numbers count only rows that actually carry a value, so a gap in the
// middle never leaves a hole in the numbering.
function renumberRows() {
  const filled = gridRows().filter(rowHasValue);
  const numbers = new Map(filled.map((row, index) => [row, index + 1]));
  gridRows().forEach((row) => {
    const number = numbers.get(row);
    const rowNo = row.querySelector('.vf-rowno');
    if (rowNo) rowNo.textContent = number ? vfFa(number) : '';
    const suffix = number ? ` ردیف ${vfFa(number)}` : '';
    const labels = {
      '.vf-code-input': 'کد حساب',
      '.vf-desc-input': 'شرح',
      '.vf-debit-input': 'بدهکار',
      '.vf-credit-input': 'بستانکار',
    };
    Object.entries(labels).forEach(([selector, label]) => {
      const input = row.querySelector(selector);
      if (input) input.setAttribute('aria-label', `${label}${suffix}`);
    });
  });
  return filled.length;
}

function syncVoucherRowCount() {
  const wrap = $('#vfGridWrap');
  const tbody = $('#voucherRows');
  if (!wrap || !tbody) return;

  const headHeight = wrap.querySelector('thead')?.offsetHeight || 37;
  const rowHeight = tbody.firstElementChild?.offsetHeight || 37;
  const visibleCount = Math.ceil((wrap.clientHeight - headHeight) / rowHeight);

  // Enough rows to fill the window, never fewer than the rows already in
  // use, and always one empty row at the end for the next entry.
  const filled = gridRows().filter(rowHasValue).length;
  const target = Math.min(MAX_VOUCHER_LINES, Math.max(5, visibleCount, filled + (filled < MAX_VOUCHER_LINES ? 1 : 0)));

  while (tbody.children.length < target) {
    tbody.insertAdjacentHTML('beforeend', voucherRowTemplate());
  }
  while (tbody.children.length > target) {
    const last = tbody.lastElementChild;
    if (!last || rowHasValue(last)) break;
    last.remove();
  }
}

function ensureTrailingBlank() {
  const tbody = $('#voucherRows');
  if (!tbody) return;
  const filled = gridRows().filter(rowHasValue).length;
  if (filled >= MAX_VOUCHER_LINES) return;
  const last = tbody.lastElementChild;
  if (!last || rowHasValue(last)) {
    tbody.insertAdjacentHTML('beforeend', voucherRowTemplate());
  }
}

function totalsFor(untilRow) {
  let debit = 0n;
  let credit = 0n;
  for (const row of gridRows()) {
    if (rowHasValue(row)) {
      debit += parseAmount(row.querySelector('.vf-debit-input')?.value || '');
      credit += parseAmount(row.querySelector('.vf-credit-input')?.value || '');
    }
    if (untilRow && row === untilRow) break;
  }
  return { debit, credit };
}

function setAmountCell(selector, value) {
  const cell = $(selector);
  if (cell) cell.textContent = vfFa(formatAmount(value));
}

function updateTotals() {
  const runningRow = state.activeRow && state.activeRow.isConnected ? state.activeRow : null;
  const all = totalsFor(null);
  const running = runningRow ? totalsFor(runningRow) : all;

  setAmountCell('#vfTotalDebit', all.debit);
  setAmountCell('#vfTotalCredit', all.credit);
  // «مغایرت» is the signed difference of this column against the opposite
  // column, so a balanced voucher shows ۰ in both cells.
  setAmountCell('#vfDiffDebit', all.debit - all.credit);
  setAmountCell('#vfDiffCredit', all.credit - all.debit);

  setAmountCell('#vfRunDebit', running.debit);
  setAmountCell('#vfRunCredit', running.credit);
  setAmountCell('#vfRunDiffDebit', running.debit - running.credit);
  setAmountCell('#vfRunDiffCredit', running.credit - running.debit);
}

function accountHierarchy(account) {
  const chain = [];
  const seen = new Set();
  let current = account;
  while (current && !seen.has(String(current.code))) {
    seen.add(String(current.code));
    chain.unshift(current);
    current = current.parentCode ? accountForCode(current.parentCode) : null;
  }
  return chain;
}

function updateRowAccount(row, final = false) {
  if (!row) return null;
  const input = row.querySelector('.vf-code-input');
  const code = digitsOnly(input?.value || '');
  // A saved row keeps its stable id only while it still points at the account
  // it was saved with. Changing the account turns it into a new posting, so the
  // "already saved" exceptions in the backend must not apply to it anymore.
  if (row.dataset.lineId && row.dataset.savedAccount !== code) {
    delete row.dataset.lineId;
    delete row.dataset.savedAccount;
  }
  const account = accountForCode(code);
  const detailCell = row.querySelector('.vf-detail1');
  const detail1 = account ? accountHierarchy(account).find((item) => Number(item.level) === 3) : null;

  if (detailCell) {
    detailCell.textContent = detail1 ? `${vfFa(detail1.code)} — ${detail1.name}` : '—';
    detailCell.title = detail1 ? `${detail1.code} — ${detail1.name}` : '';
  }
  if (input) {
    input.title = account
      ? `${account.code} — ${account.name}${accountAllowedForNewLine(account, row) ? '' : account.isActive === false ? ' (غیرفعال)' : ' (قابل ثبت نیست)'}`
      : 'برای انتخاب حساب Space یا F4 را بزنید';
    if (accountAllowedForNewLine(account, row)) input.classList.remove('is-invalid');
    else if (!code) input.classList.remove('is-invalid');
    else if (final) input.classList.add('is-invalid');
  }
  return account;
}

function updateLedger(row) {
  const values = $$('.vf-ledger-value');
  if (!values.length) return;
  values.forEach((el) => {
    el.textContent = '';
    el.removeAttribute('title');
  });
  if (!row) return;

  const code = digitsOnly(row.querySelector('.vf-code-input')?.value || '');
  if (!code) return;
  const account = accountForCode(code);
  if (!account) {
    values[0].textContent = vfFa(code);
    values[0].title = vfFa(code);
    return;
  }

  const hierarchy = accountHierarchy(account);
  const renderLevel = (levels) => hierarchy
    .filter((item) => levels.includes(Number(item.level)))
    .map((item) => `${vfFa(item.code)} — ${item.name}`)
    .join(' / ') || '—';
  const accountGroup = account.groupName ? `گروه: ${account.groupName}` : '';
  const headings = [[accountGroup, renderLevel([1, 2])].filter(Boolean).join(' / '), renderLevel([3])];
  values.forEach((el, index) => {
    const value = headings[index] || '—';
    el.textContent = value;
    el.title = value;
  });
}

function refreshFormState() {
  renumberRows();
  updateTotals();
  updateLedger(state.activeRow && state.activeRow.isConnected ? state.activeRow : null);
  applyVoucherGridSearch();
}

function markAllCodesFinal() {
  gridRows().forEach((row) => updateRowAccount(row, true));
}

function markDirty() {
  if (state.busy) return;
  state.dirty = true;
  setStatus('warn', 'تغییرات ذخیره‌نشده');
}

/* ── Grid editing ───────────────────────────────────────────── */

function bindVoucherGrid() {
  const tbody = $('#voucherRows');
  if (!tbody) return;

  tbody.addEventListener('focusin', (ev) => {
    const row = ev.target.closest('tr');
    if (!row) return;
    state.activeRow = row;
    const input = ev.target;
    if (input.classList.contains('vf-amount-input')) {
      // Drop the thousands separators while editing.
      input.value = digitsOnly(input.value);
    }
    updateRowAccount(row, input.classList.contains('vf-code-input'));
    updateLedger(row);
    updateTotals();
  });

  tbody.addEventListener('focusout', (ev) => {
    const input = ev.target;
    if (input.classList && input.classList.contains('vf-amount-input') && input.value.trim() !== '') {
      input.value = formatAmount(parseAmount(input.value));
    }
    if (input.classList && input.classList.contains('vf-code-input')) {
      updateRowAccount(input.closest('tr'), true);
    }
  });

  tbody.addEventListener('input', (ev) => {
    const input = ev.target;
    if (!input.matches('input')) return;

    if (input.classList.contains('vf-code-input') || input.classList.contains('vf-amount-input')) {
      const rawValue = toAsciiDigits(input.value);
      if (input.classList.contains('vf-amount-input') && /[-−]/.test(rawValue)) {
        input.value = '';
        toast('مبلغ منفی مجاز نیست؛ از ستون مقابل استفاده کنید.', 'error');
      } else if (input.classList.contains('vf-amount-input') && /[./٫]/.test(rawValue)) {
        input.value = '';
        toast('مبلغ باید عدد صحیح ریالی باشد؛ اعشار مجاز نیست.', 'error');
      }
      let cleaned = digitsOnly(input.value);
      if (input.classList.contains('vf-amount-input') && parseAmount(cleaned) > MAX_AMOUNT) {
        input.value = cleaned;
        input.classList.add('is-invalid');
        input.setAttribute('aria-invalid', 'true');
        toast(`مبلغ هر ردیف حداکثر ${vfFa(formatAmount(MAX_AMOUNT))} ریال می‌تواند باشد.`, 'error');
      } else {
        if (input.classList.contains('vf-amount-input')) {
          input.classList.remove('is-invalid');
          input.removeAttribute('aria-invalid');
        }
        if (input.value !== cleaned) input.value = cleaned;
      }
    }

    const row = input.closest('tr');
    if (!row) return;

    if (input.classList.contains('vf-code-input')) {
      updateRowAccount(row);
      if (state.activeRow === row) updateLedger(row);
    }

    // Debit and credit are mutually exclusive on one line.
    if (input.classList.contains('vf-amount-input') && input.value !== '') {
      const other = input.classList.contains('vf-debit-input')
        ? row.querySelector('.vf-credit-input')
        : row.querySelector('.vf-debit-input');
      if (other && other.value !== '') other.value = '';
    }

    ensureTrailingBlank();
    refreshFormState();
  });

  tbody.addEventListener('dblclick', (ev) => {
    const input = ev.target.closest('.vf-code-input');
    if (input) {
      ev.preventDefault();
      openAccountPicker(input);
    }
  });

  tbody.addEventListener('keydown', (ev) => {
    const input = ev.target;
    if (!input.matches || !input.matches('input')) return;

    const isSpace = ev.code === 'Space' || ev.key === ' ' || ev.key === 'Spacebar';
    if ((isSpace || ev.key === 'F4') && input.classList.contains('vf-code-input')) {
      ev.preventDefault();
      openAccountPicker(input);
      return;
    }

    if (ev.key === 'Enter' && !ev.ctrlKey && !ev.altKey && !ev.metaKey) {
      ev.preventDefault();
      moveGridFocus(input, input.closest('tr'), ev.shiftKey ? -1 : 1);
    }
  });

  window.addEventListener('resize', () => {
    syncVoucherRowCount();
    refreshFormState();
  });

  const form = $('.voucher-form');
  if (form) {
    const markFormDirty = (ev) => {
      // Typing in the voucher search box does not change the voucher.
      if (ev.target && ev.target.id === 'vfVoucherSearch') return;
      markDirty();
    };
    form.addEventListener('input', markFormDirty);
    form.addEventListener('change', markFormDirty);
  }
}

// Enter walks the row: code → description → debit → credit → next row's code.
function moveGridFocus(input, row, step) {
  if (!row) return;
  const cells = [
    row.querySelector('.vf-code-input'),
    row.querySelector('.vf-desc-input'),
    row.querySelector('.vf-debit-input'),
    row.querySelector('.vf-credit-input'),
  ].filter(Boolean);
  const index = cells.indexOf(input);
  if (index === -1) return;

  const nextIndex = index + step;
  if (nextIndex >= 0 && nextIndex < cells.length) {
    cells[nextIndex].focus();
    return;
  }

  if (step > 0) {
    ensureTrailingBlank();
    const nextRow = row.nextElementSibling;
    if (nextRow) nextRow.querySelector('.vf-code-input')?.focus();
    else if (gridRows().filter(rowHasValue).length >= MAX_VOUCHER_LINES) {
      toast(`حداکثر ${vfFa(MAX_VOUCHER_LINES)} ردیف برای هر سند مجاز است.`, 'warning');
    }
  } else {
    const prevRow = row.previousElementSibling;
    if (prevRow) prevRow.querySelector('.vf-credit-input')?.focus();
  }
}

/* ── Row toolbar actions ────────────────────────────────────── */

function addRow() {
  const tbody = $('#voucherRows');
  if (!tbody) return;
  const gridSearch = $('#vfGridSearch');
  if (gridSearch && gridSearch.value) { gridSearch.value = ''; applyVoucherGridSearch(); }
  if (gridRows().filter(rowHasValue).length >= MAX_VOUCHER_LINES) {
    toast(`حداکثر ${vfFa(MAX_VOUCHER_LINES)} ردیف برای هر سند مجاز است.`, 'warning');
    return;
  }

  const last = tbody.lastElementChild;
  let anchor = state.activeRow && state.activeRow.isConnected && state.activeRow.parentElement === tbody
    ? state.activeRow
    : null;
  // Keep the empty row at the end empty.
  if (anchor === last && !rowHasValue(anchor)) anchor = anchor.previousElementSibling;

  if (anchor) anchor.insertAdjacentHTML('afterend', voucherRowTemplate());
  else tbody.insertAdjacentHTML('beforeend', voucherRowTemplate());

  const row = anchor ? anchor.nextElementSibling : tbody.lastElementChild;
  state.activeRow = row;
  ensureTrailingBlank();
  syncVoucherRowCount();
  refreshFormState();
  row?.querySelector('.vf-code-input')?.focus();
}

function repeatRow(includeAmounts = false) {
  if (gridRows().filter(rowHasValue).length >= MAX_VOUCHER_LINES) {
    toast(`حداکثر ${vfFa(MAX_VOUCHER_LINES)} ردیف برای هر سند مجاز است.`, 'warning');
    return;
  }
  const row = state.activeRow && state.activeRow.isConnected ? state.activeRow : null;
  if (!row) {
    toast('ابتدا ردیفی را انتخاب کنید.', 'info');
    return;
  }

  const source = {
    code: row.querySelector('.vf-code-input').value,
    description: row.querySelector('.vf-desc-input').value,
    debit: row.querySelector('.vf-debit-input').value,
    credit: row.querySelector('.vf-credit-input').value,
  };

  row.insertAdjacentHTML('afterend', voucherRowTemplate());
  const copy = row.nextElementSibling;
  copy.querySelector('.vf-code-input').value = source.code;
  copy.querySelector('.vf-desc-input').value = source.description;
  if (includeAmounts) {
    copy.querySelector('.vf-debit-input').value = source.debit;
    copy.querySelector('.vf-credit-input').value = source.credit;
  }
  updateRowAccount(copy);
  markDirty();

  state.activeRow = copy;
  ensureTrailingBlank();
  syncVoucherRowCount();
  refreshFormState();
  copy.querySelector('.vf-debit-input')?.focus();
}

function deleteRow() {
  const row = state.activeRow && state.activeRow.isConnected ? state.activeRow : null;
  if (!row) {
    toast('ابتدا ردیفی را انتخاب کنید.', 'info');
    return;
  }

  const next = row.nextElementSibling || row.previousElementSibling;
  row.remove();
  state.activeRow = null;
  syncVoucherRowCount();
  refreshFormState();
  markDirty();
  if (next && next.isConnected) {
    state.activeRow = next;
    next.querySelector('.vf-code-input')?.focus();
    updateLedger(next);
  }
}

/* ── Draft / persistence ────────────────────────────────────── */

function setDocType(value) {
  const select = $('#vfDocType');
  if (!select) return;
  const wanted = value || VOUCHER_TYPES[0];
  let option = Array.from(select.options).find((item) => item.value === wanted || item.textContent === wanted);
  if (!option) {
    option = document.createElement('option');
    option.value = wanted;
    option.textContent = wanted;
    select.append(option);
  }
  select.value = option.value;
}

function currentNumber() {
  const typed = Number(digitsOnly($('#vfDocNo')?.value || ''));
  return typed > 0 ? typed : (state.voucher?.number || state.nextNumber);
}

function fillGrid(lines) {
  const tbody = $('#voucherRows');
  if (!tbody) return;
  state.overflowVoucherLines = Math.max(0, (lines || []).length - MAX_VOUCHER_LINES);
  if (state.overflowVoucherLines) {
    toast(`این سند ${vfFa(lines.length)} ردیف دارد و ظرفیت فرم ${vfFa(MAX_VOUCHER_LINES)} ردیف است؛ سند فقط‌خواندنی باز شد.`, 'error');
  }
  tbody.textContent = '';
  const rendered = (lines || []).slice(0, MAX_VOUCHER_LINES);
  if (rendered.length) {
    // Build all rows in one HTML insertion. Calling appendGridRow per line made
    // every call rescan the existing rows, turning a 1000-line voucher into a
    // quadratic amount of DOM work.
    tbody.insertAdjacentHTML('beforeend', rendered.map(() => voucherRowTemplate()).join(''));
    const rows = Array.from(tbody.children);
    rendered.forEach((line, index) => {
      const row = rows[index];
      if (!row) return;
      // Keep the stable id of the saved line so the backend can tell an edited
      // saved row from a newly added row using the same (possibly inactive or
      // legacy) account.
      if (line.id != null) {
        row.dataset.lineId = String(line.id);
        row.dataset.savedAccount = digitsOnly(line.accountCode || '');
      }
      row.querySelector('.vf-code-input').value = line.accountCode || '';
      row.querySelector('.vf-desc-input').value = line.description || '';
      if (Number(line.debit) > 0) {
        row.querySelector('.vf-debit-input').value = formatAmount(line.debit);
      }
      if (Number(line.credit) > 0) {
        row.querySelector('.vf-credit-input').value = formatAmount(line.credit);
      }
      updateRowAccount(row);
    });
  }
  ensureTrailingBlank();
  syncVoucherRowCount();
  refreshFormState();
}

function startNew(number) {
  state.voucher = null;
  state.restoredConflict = false;
  state.overflowVoucherLines = 0;
  state.dirty = false;
  state.activeRow = null;
  $('#vfDocNo').value = number > 0 ? String(number) : '';
  $('#vfDocDate').value = state.settings.today || '';
  setDocType(VOUCHER_TYPES[0]);
  $('#vfDocDesc').value = '';
  updateDescCounter();
  fillGrid([]);
  setStatus('info', 'سند جدید — ذخیره نشده');
  requestAnimationFrame(() => $('#voucherRows .vf-code-input')?.focus());
}

function loadVoucher(voucher) {
  state.voucher = voucher;
  state.restoredConflict = false;
  state.dirty = false;
  state.activeRow = null;
  $('#vfDocNo').value = String(voucher.number);
  $('#vfDocDate').value = voucher.date || state.settings.today || '';
  setDocType(voucher.type);
  $('#vfDocDesc').value = voucher.description || '';
  updateDescCounter();
  fillGrid(voucher.lines || []);
  setStatus('ok', `سند شماره ${vfFa(voucher.number)} ذخیره‌شده`);
}

function collectDraft() {
  const lines = [];
  for (const row of gridRows()) {
    const accountCode = digitsOnly(row.querySelector('.vf-code-input')?.value || '');
    const description = (row.querySelector('.vf-desc-input')?.value || '').trim();
    const debit = Number(parseAmount(row.querySelector('.vf-debit-input')?.value || ''));
    const credit = Number(parseAmount(row.querySelector('.vf-credit-input')?.value || ''));
    if (!accountCode && !description && !debit && !credit) continue;
    const lineId = Number(row.dataset.lineId || 0);
    lines.push({ id: lineId > 0 ? lineId : null, accountCode, description, debit, credit });
  }

  const numberValue = Number(digitsOnly($('#vfDocNo')?.value || ''));
  return {
    id: state.voucher ? state.voucher.id : null,
    number: numberValue > 0 ? numberValue : null,
    date: toAsciiDigits($('#vfDocDate')?.value || '').replace(/-/g, '/').replace(/[^\d/]/g, ''),
    voucherType: $('#vfDocType')?.value || VOUCHER_TYPES[0],
    description: $('#vfDocDesc')?.value || '',
    lines,
  };
}

function validateDraft(draft) {
  if (state.overflowVoucherLines) {
    return `این سند ${vfFa(state.overflowVoucherLines + MAX_VOUCHER_LINES)} ردیف دارد و به‌دلیل ظرفیت فرم قابل ذخیره نیست.`;
  }
  if (draft.number != null && (!Number.isSafeInteger(draft.number) || draft.number > MAX_VOUCHER_NUMBER)) {
    return `شماره سند باید عدد صحیح و حداکثر ${vfFa(MAX_VOUCHER_NUMBER)} باشد.`;
  }
  if (!draft.date) return 'تاریخ سند را وارد کنید.';
  if (draft.lines.length > MAX_VOUCHER_LINES) {
    return `تعداد ردیف‌های سند نمی‌تواند بیشتر از ${vfFa(MAX_VOUCHER_LINES)} باشد.`;
  }
  const dateParts = /^(\d{4})\/(\d{1,2})\/(\d{1,2})$/.exec(draft.date);
  if (!dateParts) return 'تاریخ سند باید به شکل ۱۴۰۵/۰۱/۰۱ وارد شود.';
  const year = Number(dateParts[1]);
  const month = Number(dateParts[2]);
  const day = Number(dateParts[3]);
  if (year < 1 || year > 3177 || month < 1 || month > 12 || day < 1
      || day > jalaliMonthLength(year, month)) {
    return 'سال، ماه یا روز تاریخ سند معتبر نیست.';
  }
  // The Rust/Jalali calendar remains authoritative and is still called before
  // saving; this check only avoids a round-trip for impossible dates.
  if (draft.lines.length < 2) return 'سند حسابداری باید حداقل دو ردیف داشته باشد.';

  let debit = 0n;
  let credit = 0n;
  for (let index = 0; index < draft.lines.length; index += 1) {
    const line = draft.lines[index];
    const rowNo = vfFa(index + 1);
    if (!line.accountCode) return `کد حساب ردیف ${rowNo} را وارد کنید.`;
    const account = accountForCode(line.accountCode);
    if (!account) return `حساب با کد ${vfFa(line.accountCode)} یافت نشد.`;
    const savedLine = line.id == null
      ? null
      : (state.voucher?.lines || []).find((saved) => saved.id === line.id) || null;
    const savedAccountMatches = !!savedLine
      && digitsOnly(savedLine.accountCode) === line.accountCode;
    if (!savedAccountMatches && account.isActive === false) {
      return `حساب «${account.name}» غیرفعال است و برای ردیف جدید سند قابل انتخاب نیست.`;
    }
    const legacyAccountAlreadyOnThisVoucher = Number(account.level) > 3 && savedAccountMatches;
    if (!account.isPostable && !legacyAccountAlreadyOnThisVoucher) {
      return `حساب «${account.name}» (سطح ${levelLabel(account.level)}) قابل ثبت نیست. لطفاً یک حساب معین یا تفصیلی ۱ قابل ثبت انتخاب کنید.`;
    }
    if (!line.debit && !line.credit) return `مبلغ ردیف ${rowNo} را وارد کنید.`;
    if (line.debit && line.credit) return `در ردیف ${rowNo} فقط یکی از بدهکار یا بستانکار را وارد کنید.`;
    if (line.debit < 0 || line.credit < 0) return `مبلغ ردیف ${rowNo} نمی‌تواند منفی باشد.`;
    if (!Number.isSafeInteger(line.debit) || !Number.isSafeInteger(line.credit)
      || BigInt(line.debit) > MAX_AMOUNT || BigInt(line.credit) > MAX_AMOUNT) {
      return `مبلغ ردیف ${rowNo} باید عدد صحیح و حداکثر ${vfFa(formatAmount(MAX_AMOUNT))} ریال باشد.`;
    }
    debit += BigInt(line.debit);
    credit += BigInt(line.credit);
  }

  if (debit === 0n) return 'مبلغ سند باید بزرگ‌تر از صفر باشد.';
  if (debit !== credit) {
    return `سند تراز نیست: جمع بدهکار ${vfFa(formatAmount(debit))} و جمع بستانکار ${vfFa(formatAmount(credit))} است.`;
  }
  return '';
}

async function saveVoucher(options = {}) {
  if (state.restoredConflict) {
    toast('پایگاه داده بازیابی شده است؛ برای حفظ تغییرات فرم، آن را ذخیره نکنید. پنجره را ببندید و سند را دوباره باز کنید.', 'error');
    return null;
  }
  if (state.busy) return null;
  if (!invoke) {
    toast('ذخیره‌سازی فقط در نسخه دسکتاپ ممکن است.', 'error');
    return null;
  }

  const draft = collectDraft();
  const draftSnapshot = JSON.stringify(draft);
  const problem = validateDraft(draft);
  if (problem) {
    markAllCodesFinal();
    toast(problem, 'error');
    return null;
  }

  state.busy = true;
  setStatus('info', 'در حال اعتبارسنجی و ذخیره…');
  try {
    // The Rust/Jalali calendar implementation is authoritative, including
    // leap-year validation. This command gives the form early feedback.
    const normalizedDate = await invoke('validate_voucher_date', { date: draft.date });
    // Check the original form before writing the normalized date back. Doing
    // this in the opposite order could overwrite a date typed during the
    // asynchronous validation and accidentally save the stale draft.
    if (JSON.stringify(collectDraft()) !== draftSnapshot) {
      state.busy = false;
      state.dirty = true;
      setStatus('warn', 'فرم تغییر کرده است');
      toast('اطلاعات فرم هنگام اعتبارسنجی تغییر کرد؛ لطفاً دوباره ذخیره کنید.', 'warning');
      return null;
    }
    const dateInput = $('#vfDocDate');
    if (dateInput && typeof normalizedDate === 'string') dateInput.value = normalizedDate;
    draft.date = normalizedDate;
    dateInput?.classList.remove('is-invalid');
    dateInput?.removeAttribute('aria-invalid');
    if (dateInput) dateInput.title = '';
  } catch (error) {
    state.busy = false;
    if (JSON.stringify(collectDraft()) !== draftSnapshot) state.dirty = true;
    const dateInput = $('#vfDocDate');
    dateInput?.classList.add('is-invalid');
    dateInput?.setAttribute('aria-invalid', 'true');
    if (dateInput) dateInput.title = errorText(error);
    setStatus('error', 'تاریخ نامعتبر');
    toast(errorText(error), 'error');
    return null;
  }

  let saveSnapshot = '';
  try {
    saveSnapshot = JSON.stringify(collectDraft());
    const saved = await invoke('save_voucher', {
      input: draft,
      expectedUpdatedAt: state.voucher?.updatedAt || null,
    });
    state.busy = false;
    const currentDraft = collectDraft();
    if (JSON.stringify(currentDraft) !== saveSnapshot) {
      state.voucher = saved;
      if (currentDraft.number === draft.number) $('#vfDocNo').value = String(saved.number);
      state.dirty = true;
      setStatus('warn', 'تغییرات ذخیره‌نشده');
      toast('نسخه اولیه سند ذخیره شد؛ ویرایش‌های جدید فرم حفظ شده‌اند و هنوز ذخیره نشده‌اند.', 'warning');
    } else {
      loadVoucher(saved);
    }

    if (!options.silent) {
      toast(`سند شماره ${vfFa(saved.number)} ذخیره شد.`, 'ok');
    }
    try {
      await refreshNextNumber();
    } catch (refreshError) {
      state.nextNumber = Math.max(state.nextNumber || 1, Number(saved.number) + 1);
      toast(`سند ذخیره شد؛ دریافت شماره بعدی ناموفق بود: ${errorText(refreshError)}`, 'warning');
    }
    return saved;
  } catch (error) {
    state.busy = false;
    if (saveSnapshot && JSON.stringify(collectDraft()) !== saveSnapshot) state.dirty = true;
    setStatus('error', 'ذخیره ناموفق');
    toast(errorText(error), 'error');
    return null;
  }
}

function resetNumberInput() {
  const value = state.voucher ? state.voucher.number : state.nextNumber;
  const input = $('#vfDocNo');
  if (input) input.value = String(value);
}

function draftChangedSince(snapshot) {
  return JSON.stringify(collectDraft()) !== snapshot;
}

function warnDraftChangedDuringLoad() {
  state.dirty = true;
  setStatus('warn', 'تغییرات ذخیره‌نشده');
  toast('فرم هنگام بارگذاری سند تغییر کرد؛ اطلاعات حفظ شد و جابه‌جایی انجام نشد.', 'warning');
}

async function openNumber(number, options = {}) {
  if (state.busy || !invoke) return;
  const target = Number(number);
  if (!Number.isFinite(target) || target <= 0) {
    toast('شماره سند معتبر نیست.', 'error');
    resetNumberInput();
    return;
  }

  if (state.voucher && !state.dirty && state.voucher.number === target) return;
  if (!(await guardUnsaved())) {
    resetNumberInput();
    return;
  }

  const snapshot = JSON.stringify(collectDraft());
  state.busy = true;
  try {
    const voucher = await invoke('get_voucher', { number: target });
    state.busy = false;
    if (draftChangedSince(snapshot)) {
      warnDraftChangedDuringLoad();
      return;
    }
    if (voucher) {
      loadVoucher(voucher);
      return;
    }
    await refreshNextNumber();
    if (draftChangedSince(snapshot)) {
      warnDraftChangedDuringLoad();
      return;
    }
    startNew(target);
    if (!options.quiet) {
      toast(`سند شماره ${vfFa(target)} یافت نشد؛ سند جدید با این شماره آغاز شد.`, 'info');
    }
  } catch (error) {
    state.busy = false;
    resetNumberInput();
    toast(errorText(error), 'error');
  }
}

async function navigatePrev() {
  if (state.busy || !invoke) return;
  const base = currentNumber();
  if (base <= 1) {
    toast('سند قبلی وجود ندارد.', 'info');
    return;
  }
  if (!(await guardUnsaved())) {
    resetNumberInput();
    return;
  }

  const snapshot = JSON.stringify(collectDraft());
  state.busy = true;
  try {
    const target = await invoke('adjacent_voucher_number', { number: base, direction: 'previous' });
    if (target == null) {
      state.busy = false;
      toast('سند قبلی وجود ندارد.', 'info');
      return;
    }
    const voucher = await invoke('get_voucher', { number: target });
    state.busy = false;
    if (draftChangedSince(snapshot)) {
      warnDraftChangedDuringLoad();
      return;
    }
    if (voucher) loadVoucher(voucher);
    else toast(`سند شماره ${vfFa(target)} یافت نشد.`, 'info');
  } catch (error) {
    state.busy = false;
    toast(errorText(error), 'error');
  }
}

async function navigateNext() {
  if (state.busy || !invoke) return;
  if (!(await guardUnsaved())) {
    resetNumberInput();
    return;
  }

  const snapshot = JSON.stringify(collectDraft());
  const base = currentNumber();
  state.busy = true;
  try {
    const target = await invoke('adjacent_voucher_number', { number: base, direction: 'next' });
    if (draftChangedSince(snapshot)) {
      state.busy = false;
      warnDraftChangedDuringLoad();
      return;
    }
    if (target == null) {
      await refreshNextNumber();
      if (draftChangedSince(snapshot)) {
        state.busy = false;
        warnDraftChangedDuringLoad();
        return;
      }
      state.busy = false;
      startNew(state.nextNumber);
      toast('سند بعدی وجود ندارد؛ سند جدید آغاز شد.', 'info');
      return;
    }
    const voucher = await invoke('get_voucher', { number: target });
    if (draftChangedSince(snapshot)) {
      state.busy = false;
      warnDraftChangedDuringLoad();
      return;
    }
    if (voucher) {
      state.busy = false;
      loadVoucher(voucher);
      return;
    }
    await refreshNextNumber();
    if (draftChangedSince(snapshot)) {
      state.busy = false;
      warnDraftChangedDuringLoad();
      return;
    }
    state.busy = false;
    if (target >= state.nextNumber) {
      startNew(state.nextNumber);
      toast('سند بعدی وجود ندارد؛ سند جدید آغاز شد.', 'info');
    } else {
      toast(`سند شماره ${vfFa(target)} یافت نشد.`, 'info');
    }
  } catch (error) {
    state.busy = false;
    toast(errorText(error), 'error');
  }
}

async function newVoucher() {
  if (state.busy) return;
  if (!(await guardUnsaved())) return;
  const draftSnapshot = JSON.stringify(collectDraft());
  state.busy = true;
  try {
    await refreshNextNumber();
    if (JSON.stringify(collectDraft()) !== draftSnapshot) {
      state.dirty = true;
      toast('فرم هنگام دریافت شماره تغییر کرد؛ اطلاعات حفظ شد و سند جدید باز نشد.', 'warning');
      return;
    }
    startNew(state.nextNumber);
    toast('سند جدید آماده است.', 'info');
  } catch (error) {
    toast(`دریافت شماره سند جدید ناموفق بود: ${errorText(error)}`, 'error');
  } finally {
    state.busy = false;
  }
}

async function copyVoucher() {
  if (state.busy) return;
  if (state.overflowVoucherLines) {
    toast('کپی این سند از فرم محدودشده ممکن نیست.', 'error');
    return;
  }
  const draft = collectDraft();
  const draftSnapshot = JSON.stringify(draft);
  if (!draft.lines.length) {
    toast('سندی برای کپی وجود ندارد.', 'info');
    return;
  }

  state.busy = true;
  try {
    await refreshNextNumber();
  } catch (error) {
    toast(errorText(error), 'error');
    return;
  } finally {
    state.busy = false;
  }
  if (JSON.stringify(collectDraft()) !== draftSnapshot) {
    state.dirty = true;
    toast('فرم هنگام دریافت شماره تغییر کرد؛ اطلاعات حفظ شد و کپی ساخته نشد.', 'warning');
    return;
  }

  const copy = {
    date: draft.date || state.settings.today || '',
    voucherType: draft.voucherType,
    description: draft.description,
    lines: draft.lines.map((line) => ({ ...line })),
  };

  state.voucher = null;
  state.dirty = false;
  state.activeRow = null;
  $('#vfDocNo').value = String(state.nextNumber);
  $('#vfDocDate').value = copy.date;
  setDocType(copy.voucherType);
  $('#vfDocDesc').value = copy.description;
  updateDescCounter();
  fillGrid(copy.lines);
  markDirty();
  toast(`سند جدید از روی سند جاری ساخته شد (شماره ${vfFa(state.nextNumber)}).`, 'info');
  requestAnimationFrame(() => $('#voucherRows .vf-code-input')?.focus());
}

async function deleteVoucher() {
  if (state.busy) return;
  if (state.overflowVoucherLines) {
    toast('حذف این سند از فرم محدودشده ممکن نیست.', 'error');
    return;
  }
  const voucher = state.voucher;
  if (!voucher) {
    toast('ابتدا یک سند ذخیره‌شده را باز کنید.', 'info');
    return;
  }

  const ok = await confirmAction(
    state.dirty
      ? `سند شماره ${vfFa(voucher.number)} برای همیشه حذف شود؟ تغییرات ذخیره‌نشدهٔ فرم نیز از بین می‌رود و این کار قابل بازگشت نیست.`
      : `سند شماره ${vfFa(voucher.number)} برای همیشه حذف شود؟ این کار قابل بازگشت نیست.`,
    'حذف سند'
  );
  if (!ok) return;

  state.busy = true;
  try {
    await invoke('delete_voucher', {
      id: voucher.id,
      expectedUpdatedAt: voucher.updatedAt || null,
    });
    state.busy = false;
    // The deletion has committed. Do not report it as failed just because
    // refreshing the suggested next number has a transient error.
    try {
      await refreshNextNumber();
    } catch (refreshError) {
      state.nextNumber = Math.max(1, state.nextNumber || Number(voucher.number));
      startNew(state.nextNumber);
      toast(`سند شماره ${vfFa(voucher.number)} حذف شد. دریافت شماره بعدی ناموفق بود: ${errorText(refreshError)}`, 'warning');
      return;
    }
    startNew(state.nextNumber);
    toast(`سند شماره ${vfFa(voucher.number)} حذف شد.`, 'ok');
  } catch (error) {
    state.busy = false;
    toast(errorText(error), 'error');
  }
}

async function printVoucher() {
  if (state.busy) return;
  if (!invoke) {
    toast('چاپ فقط در نسخه دسکتاپ ممکن است.', 'error');
    return;
  }

  let voucher = state.voucher;
  if (!voucher || state.dirty) {
    voucher = await saveVoucher({ silent: true });
    if (!voucher) return;
  }

  try {
    await invoke('open_voucher_preview', { number: voucher.number });
  } catch (error) {
    toast(errorText(error), 'error');
  }
}

function checkBalance() {
  const draft = collectDraft();
  if (draft.lines.length === 0) {
    toast('سند هنوز ردیفی ندارد.', 'info');
    return;
  }
  const problem = validateDraft(draft);
  if (problem) {
    markAllCodesFinal();
    toast(problem, 'error');
    return;
  }
  toast(`سند تراز است؛ جمع بدهکار و بستانکار ${vfFa(formatAmount(totalsFor(null).debit))} ریال.`, 'ok');
}

/* ── Account picker ─────────────────────────────────────────── */

let apTarget = null;
let apIndex = 0;
let apVisible = [];

const accountPickerOpen = () => {
  const modal = $('#accountPickerModal');
  return !!modal && !modal.hidden;
};

function filterAccountPicker(term) {
  const q = normalizeSearch(term);
  const parent = state.accountPickerPath[state.accountPickerPath.length - 1] || null;
  const selectable = state.accounts.filter((account) => account.isActive !== false || voucherAlreadyUsesAccount(account.code));
  let source = selectable;
  if (!q && !state.accountPickerShowAll) {
    source = parent
      ? selectable.filter((account) => digitsOnly(account.parentCode) === digitsOnly(parent.code))
      : selectable.filter((account) => Number(account.level) === 1 || !account.parentCode);
  }
  if (!q) return source.slice();
  return selectable.filter((account) =>
    normalizeSearch(account.code).includes(q) ||
    normalizeSearch(account.name).includes(q) ||
    normalizeSearch(levelLabel(account.level)).includes(q) ||
    normalizeSearch(account.parentName).includes(q) ||
    normalizeSearch(account.groupName).includes(q)
  );
}

function renderAccountPickerPath() {
  const path = $('#accountPickerPath');
  if (!path) return;
  const labels = ['همه حساب‌های کل'].concat(state.accountPickerPath.map((account) =>
    vfFa(account.code) + ' — ' + account.name
  ));
  path.textContent = labels.join('  ‹  ');
}

function apCell(text, className, dir) {
  const cell = document.createElement('div');
  cell.className = `ap-cell ${className}`;
  if (dir) cell.dir = dir;
  cell.setAttribute('role', 'gridcell');
  cell.textContent = text;
  return cell;
}

function renderAccountPickerRows(resetIndex = true) {
  const box = $('#accountPickerRows');
  if (!box) return;
  const term = $('#accountPickerSearch') ? $('#accountPickerSearch').value : '';
  apVisible = filterAccountPicker(term);
  renderAccountPickerPath();
  if (resetIndex) apIndex = 0;
  if (apIndex > apVisible.length - 1) apIndex = Math.max(0, apVisible.length - 1);

  box.textContent = '';
  const fragment = document.createDocumentFragment();

  if (!state.accounts.length) {
    box.append(apCell('هنوز حسابی تعریف نشده است؛ از صفحه «حساب‌ها» حساب ایجاد کنید.', 'ap-nostate-cell'));
    return;
  }

  if (!apVisible.length) {
    const row = document.createElement('div');
    row.className = 'ap-row ap-nostate';
    row.setAttribute('role', 'row');
    const cell = document.createElement('div');
    cell.className = 'ap-cell ap-nostate-cell';
    cell.setAttribute('role', 'gridcell');
    cell.textContent = 'نتیجه‌ای برای این جستجو یافت نشد';
    row.append(cell);
    fragment.append(row);
    box.append(fragment);
    return;
  }

  apVisible.forEach((account, i) => {
    const row = document.createElement('div');
    row.className = 'ap-row';
    if (i === apIndex) row.classList.add('selected');
    if (!account.isPostable) row.classList.add('not-postable');
    if (account.isActive === false) row.classList.add('inactive');
    row.setAttribute('role', 'row');
    row.tabIndex = -1;
    row.dataset.pickerIndex = String(i);
    row.setAttribute('aria-selected', i === apIndex ? 'true' : 'false');
    row.title = account.isActive === false
      ? 'این حساب غیرفعال است و فقط برای ردیف‌های ذخیره‌شدهٔ همین سند قابل استفاده است'
      : account.isPostable
        ? `${account.code} — ${account.name}`
        : 'این حساب قابل ثبت نیست؛ یک حساب معین یا تفصیلی ۱ قابل ثبت انتخاب کنید';

    row.append(
      apCell(vfFa(i + 1), 'rownum center'),
      apCell(vfFa(account.code), 'code center', 'ltr'),
      apCell(account.name, 'start'),
      apCell(levelLabel(account.level), 'level center'),
      apCell(account.parentName || '—', 'start')
    );
    fragment.append(row);
  });

  // Rows that keep the table pattern even when the filter is narrow.
  const fillers = Math.max(0, 8 - apVisible.length);
  for (let i = 0; i < fillers; i += 1) {
    const row = document.createElement('div');
    row.className = 'ap-row empty';
    row.setAttribute('role', 'presentation');
    row.setAttribute('aria-hidden', 'true');
    for (let c = 0; c < 5; c += 1) {
      const cell = document.createElement('div');
      cell.className = 'ap-cell';
      row.append(cell);
    }
    fragment.append(row);
  }

  box.append(fragment);

  const selected = box.querySelector('.ap-row.selected');
  if (selected) selected.scrollIntoView({ block: 'nearest' });
}

function highlightAccountPickerRow(index) {
  const box = $('#accountPickerRows');
  if (!box) return;
  const rows = $$('.ap-row[data-picker-index]', box);
  if (!rows.length) return;
  apIndex = Math.max(0, Math.min(index, rows.length - 1));
  rows.forEach((row, i) => {
    row.classList.toggle('selected', i === apIndex);
    row.setAttribute('aria-selected', i === apIndex ? 'true' : 'false');
  });
  rows[apIndex].scrollIntoView({ block: 'nearest' });
}

function openAccountPicker(input) {
  const modal = $('#accountPickerModal');
  if (!modal) return;
  if (!state.accounts.length) {
    toast('هنوز حسابی تعریف نشده است؛ ابتدا از صفحه «حساب‌ها» حساب بسازید.', 'info');
    return;
  }

  apTarget = input || null;
  state.accountPickerPath = [];
  state.accountPickerShowAll = false;
  const search = $('#accountPickerSearch');
  const code = input && input.classList.contains('vf-code-input') ? digitsOnly(input.value) : '';
  if (search) search.value = code;

  apIndex = 0;
  renderAccountPickerRows();
  const exact = apVisible.findIndex((account) => account.code === code);
  if (exact > 0) {
    apIndex = exact;
    renderAccountPickerRows(false);
  }

  modal.hidden = false;
  const main = $('.voucher-window');
  if (main) main.inert = true;

  if (search) {
    search.focus();
    search.select();
  }
  const selected = $('#accountPickerRows .ap-row.selected');
  if (selected) selected.scrollIntoView({ block: 'nearest' });
}

function drillOrChooseAccountPickerRow(account) {
  if (!account) return;
  const children = state.accounts.filter((item) =>
    digitsOnly(item.parentCode) === digitsOnly(account.code)
  );
  if (children.length) {
    state.accountPickerPath.push(account);
    const search = $('#accountPickerSearch');
    if (search) search.value = '';
    apIndex = 0;
    renderAccountPickerRows();
    search?.focus();
    return;
  }
  chooseAccountPickerRow();
}

function closeAccountPicker(refocus = false) {
  const modal = $('#accountPickerModal');
  if (!modal || modal.hidden) return;
  modal.hidden = true;
  const main = $('.voucher-window');
  if (main) main.inert = false;

  const target = apTarget;
  apTarget = null;
  if (refocus && target && target.isConnected && !target.disabled) target.focus();
}

function chooseAccountPickerRow() {
  const account = apVisible[apIndex];
  if (!account) return;

  if (!accountAllowedForNewLine(account, apTarget?.closest?.('tr') || null)) {
    const hasChildren = state.accounts.some((item) =>
      digitsOnly(item.parentCode) === digitsOnly(account.code)
    );
    if (hasChildren) {
      drillOrChooseAccountPickerRow(account);
      return;
    }
    const reason = account.isActive === false
      ? 'غیرفعال است'
      : `${levelLabel(account.level)} قابل ثبت نیست`;
    toast(`حساب «${account.name}» ${reason}؛ یک حساب معین یا تفصیلی ۱ فعال و قابل ثبت انتخاب کنید.`, 'error');
    return;
  }

  const target = apTarget;
  if (target && target.isConnected) {
    target.value = account.code; // ASCII; the FD font draws Persian digits
    target.dispatchEvent(new Event('input', { bubbles: true }));
    updateRowAccount(target.closest('tr'), true);
    updateLedger(target.closest('tr'));
  }
  closeAccountPicker(true);
}

function trapPickerFocus(ev) {
  if (ev.key !== 'Tab') return;
  const focusables = $$(
    '.ap-window button:not([disabled]), .ap-window input:not([disabled]), .ap-window [tabindex]:not([tabindex="-1"])',
    $('#accountPickerModal')
  ).filter((el) => el.offsetParent !== null);
  if (!focusables.length) return;
  const first = focusables[0];
  const last = focusables[focusables.length - 1];
  if (ev.shiftKey && document.activeElement === first) {
    ev.preventDefault();
    last.focus();
  } else if (!ev.shiftKey && document.activeElement === last) {
    ev.preventDefault();
    first.focus();
  }
}

function bindAccountPicker() {
  const modal = $('#accountPickerModal');
  const rows = $('#accountPickerRows');
  const search = $('#accountPickerSearch');
  if (!modal || !rows) return;

  const closeBtn = $('#btnAccountPickerClose');
  if (closeBtn) closeBtn.addEventListener('click', () => closeAccountPicker(true));
  const backBtn = $('#accountPickerBack');
  if (backBtn) backBtn.addEventListener('click', () => {
    if (state.accountPickerPath.length) {
      state.accountPickerPath.pop();
      state.accountPickerShowAll = false;
      if (search) search.value = '';
      apIndex = 0;
      renderAccountPickerRows();
      search?.focus();
    } else {
      closeAccountPicker(true);
    }
  });
  modal.addEventListener('click', (ev) => {
    if (ev.target.id === 'accountPickerModal') closeAccountPicker(true);
  });

  if (search) {
    let apSearchDebounceTimer = 0;
    search.addEventListener('input', () => {
      clearTimeout(apSearchDebounceTimer);
      apSearchDebounceTimer = setTimeout(() => renderAccountPickerRows(), 120);
    });
  }

  rows.addEventListener('click', (ev) => {
    const row = ev.target.closest('.ap-row[data-picker-index]');
    if (!row) return;
    apIndex = Number(row.dataset.pickerIndex);
    highlightAccountPickerRow(apIndex);
  });
  rows.addEventListener('dblclick', (ev) => {
    const row = ev.target.closest('.ap-row[data-picker-index]');
    if (!row) return;
    apIndex = Number(row.dataset.pickerIndex);
    highlightAccountPickerRow(apIndex);
    drillOrChooseAccountPickerRow(apVisible[apIndex]);
  });

  const selectBtn = $('#accountPickerSelect');
  if (selectBtn) selectBtn.addEventListener('click', chooseAccountPickerRow);

  const clearBtn = modal.querySelector('[data-picker-act="clear"]');
  const showAllAccounts = () => {
    state.accountPickerPath = [];
    state.accountPickerShowAll = true;
    if (search) search.value = '';
    renderAccountPickerRows();
    if (search) search.focus();
  };
  if (clearBtn) {
    clearBtn.addEventListener('click', showAllAccounts);
  }

  modal.addEventListener('keydown', (ev) => {
    trapPickerFocus(ev);
    if (ev.defaultPrevented || ev.ctrlKey || ev.altKey || ev.metaKey) return;

    const inField = ev.target.closest && ev.target.closest('input, textarea, select');
    const nativeControl = ev.target.closest && ev.target.closest('button, a[href], [role="button"]');
    if (nativeControl && (ev.key === 'Enter' || ev.key === ' ')) return;

    const step = Math.max(1, Math.floor(rows.clientHeight / 37) - 1);
    switch (ev.key) {
      case 'ArrowDown':
        ev.preventDefault();
        highlightAccountPickerRow(apIndex + 1);
        break;
      case 'ArrowUp':
        ev.preventDefault();
        highlightAccountPickerRow(apIndex - 1);
        break;
      case 'PageDown':
        ev.preventDefault();
        highlightAccountPickerRow(apIndex + step);
        break;
      case 'PageUp':
        ev.preventDefault();
        highlightAccountPickerRow(apIndex - step);
        break;
      case 'Home':
        if (!inField && (!search || !search.value)) {
          ev.preventDefault();
          highlightAccountPickerRow(0);
        }
        break;
      case 'End':
        if (!inField && (!search || !search.value)) {
          ev.preventDefault();
          highlightAccountPickerRow(apVisible.length - 1);
        }
        break;
      case 'Enter':
        ev.preventDefault();
        chooseAccountPickerRow();
        break;
      case 'F2':
        ev.preventDefault();
        if (apVisible[apIndex] && state.accounts.some((item) =>
          digitsOnly(item.parentCode) === digitsOnly(apVisible[apIndex].code))) {
          drillOrChooseAccountPickerRow(apVisible[apIndex]);
        } else {
          chooseAccountPickerRow();
        }
        break;
      case 'F5':
        ev.preventDefault();
        if (search) search.focus();
        break;
      case 'F6':
        ev.preventDefault();
        showAllAccounts();
        break;
      default:
        break;
    }
  });
}

/* ── Confirmation dialog ────────────────────────────────────── */

let confirmResolve = null;
let confirmReturnFocus = null;
let confirmUnderlyingModal = null;
let confirmMainWasInert = false;
let excelReturnFocus = null;
let excelMainWasInert = false;
let transferReturnFocus = null;
let transferMainWasInert = false;

const confirmOpen = () => {
  const modal = $('#vfConfirmModal');
  return !!modal && !modal.hidden;
};

function confirmAction(message, confirmLabel = 'تأیید') {
  const modal = $('#vfConfirmModal');
  const text = $('#vfConfirmText');
  const okButton = $('#vfConfirmOk');
  if (!modal || !text || !okButton) {
    return Promise.resolve(window.confirm(message));
  }

  return new Promise((resolve) => {
    if (confirmResolve) {
      const previous = confirmResolve;
      confirmResolve = null;
      previous(false);
    }
    confirmResolve = resolve;
    confirmReturnFocus = document.activeElement;
    text.textContent = message;
    okButton.textContent = confirmLabel;
    modal.hidden = false;
    const main = $('.voucher-window');
    confirmMainWasInert = Boolean(main?.inert);
    if (main) main.inert = true;
    confirmUnderlyingModal = transferModalOpen()
      ? $('#vfTransferModal')
      : (excelPreviewOpen() ? $('#vfExcelPreviewModal') : null);
    if (confirmUnderlyingModal) confirmUnderlyingModal.inert = true;
    okButton.focus();
  });
}

function settleConfirm(value) {
  const modal = $('#vfConfirmModal');
  if (!modal || modal.hidden) return;
  modal.hidden = true;
  const main = $('.voucher-window');
  if (main) main.inert = confirmMainWasInert;
  if (confirmUnderlyingModal) confirmUnderlyingModal.inert = false;
  confirmUnderlyingModal = null;
  const resolve = confirmResolve;
  confirmResolve = null;
  if (resolve) resolve(value);
  const returnFocus = confirmReturnFocus;
  confirmReturnFocus = null;
  if (returnFocus?.isConnected && !returnFocus.closest('[inert]')) returnFocus.focus();
}

function bindConfirm() {
  const modal = $('#vfConfirmModal');
  if (!modal) return;
  $('#vfConfirmOk')?.addEventListener('click', () => settleConfirm(true));
  $('#vfConfirmCancel')?.addEventListener('click', () => settleConfirm(false));
  modal.addEventListener('click', (ev) => {
    if (ev.target.id === 'vfConfirmModal') settleConfirm(false);
  });
  modal.addEventListener('keydown', (ev) => {
    if (ev.key === 'Escape') {
      ev.preventDefault();
      settleConfirm(false);
    } else if (ev.key === 'Enter' && ev.target.tagName !== 'BUTTON') {
      ev.preventDefault();
      settleConfirm(true);
    } else if (ev.key === 'Tab') {
      const focusable = $$('#vfConfirmModal button:not([disabled])', modal);
      if (!focusable.length) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (ev.shiftKey && document.activeElement === first) {
        ev.preventDefault();
        last.focus();
      } else if (!ev.shiftKey && document.activeElement === last) {
        ev.preventDefault();
        first.focus();
      }
    }
  });
}

async function guardUnsaved() {
  if (!state.dirty) return true;
  return confirmAction('تغییرات ذخیره‌نشده این سند از بین می‌رود. ادامه می‌دهید؟', 'ادامه بدون ذخیره');
}

/* ── Document meta and description ──────────────────────────── */

function updateDescCounter() {
  const area = $('#vfDocDesc');
  const output = $('#vfDocDescCount');
  if (!area || !output) return;
  let chars = Array.from(area.value);
  if (chars.length > MAX_DESCRIPTION) {
    area.value = chars.slice(0, MAX_DESCRIPTION).join('');
    chars = chars.slice(0, MAX_DESCRIPTION);
  }
  output.textContent = `${vfFa(chars.length)}/${vfFa(MAX_DESCRIPTION)}`;
}

function bindDocMeta() {
  const docNo = $('#vfDocNo');
  const date = $('#vfDocDate');
  const desc = $('#vfDocDesc');

  if (docNo) {
    docNo.addEventListener('input', () => {
      const cleaned = digitsOnly(docNo.value);
      if (docNo.value !== cleaned) docNo.value = cleaned;
    });
    docNo.addEventListener('keydown', (ev) => {
      if (ev.key === 'Enter') {
        ev.preventDefault();
        openNumber(digitsOnly(docNo.value));
      }
    });
  }

  if (date) {
    let validationSequence = 0;
    date.addEventListener('input', () => {
      validationSequence += 1;
      date.value = toAsciiDigits(date.value).replace(/-/g, '/').replace(/[^\d/]/g, '').slice(0, 10);
      date.classList.remove('is-invalid');
      date.removeAttribute('aria-invalid');
      date.title = '';
    });
    date.addEventListener('blur', async () => {
      const value = toAsciiDigits(date.value).replace(/-/g, '/').replace(/[^\d/]/g, '');
      if (!value || !invoke) return;
      const request = ++validationSequence;
      try {
        const normalizedDate = await invoke('validate_voucher_date', { date: value });
        if (request !== validationSequence || date.value !== value) return;
        if (typeof normalizedDate === 'string') date.value = normalizedDate;
        date.classList.remove('is-invalid');
        date.removeAttribute('aria-invalid');
        date.title = '';
      } catch (error) {
        if (request !== validationSequence || date.value !== value) return;
        date.classList.add('is-invalid');
        date.setAttribute('aria-invalid', 'true');
        date.title = errorText(error);
        toast(errorText(error), 'error');
      }
    });
  }

  if (desc) desc.addEventListener('input', updateDescCounter);

  $('#vfPrevBtn')?.addEventListener('click', navigatePrev);
  $('#vfNextBtn')?.addEventListener('click', navigateNext);
}

/* ── Voucher search ─────────────────────────────────────────── */

let searchTimer = 0;
let searchRequest = 0;
let searchResults = [];
let searchIndex = -1;

const voucherSearchInput = () => $('#vfVoucherSearch');
const voucherSearchBox = () => $('#vfVoucherSearchResults');

const isVoucherNumberTerm = (term) => /^[0-9۰-۹٠-٩]+$/.test(toAsciiDigits(String(term)).trim());

function hideVoucherSearchResults() {
  const box = voucherSearchBox();
  const input = voucherSearchInput();
  if (box) {
    box.hidden = true;
    box.textContent = '';
  }
  if (input) input.setAttribute('aria-expanded', 'false');
  searchResults = [];
  searchIndex = -1;
}

function positionVoucherSearchResults() {
  const input = voucherSearchInput();
  const box = voucherSearchBox();
  if (!input || !box || box.hidden) return;

  const rect = input.getBoundingClientRect();
  const width = Math.max(rect.width, 360);
  let left = rect.right - width;
  if (left + width > window.innerWidth - 8) left = window.innerWidth - width - 8;
  if (left < 8) left = 8;

  box.style.width = `${width}px`;
  box.style.left = `${left}px`;
  box.style.top = `${rect.bottom + 4}px`;
}

function highlightVoucherSearchResult(index) {
  const box = voucherSearchBox();
  if (!box) return;
  const items = $$('.vf-search-result', box);
  if (!items.length) {
    searchIndex = -1;
    return;
  }
  searchIndex = (index + items.length) % items.length;
  items.forEach((item, itemIndex) => {
    item.classList.toggle('is-active', itemIndex === searchIndex);
    item.setAttribute('aria-selected', itemIndex === searchIndex ? 'true' : 'false');
  });
  items[searchIndex].scrollIntoView({ block: 'nearest' });
}

function showVoucherSearchMessage(message) {
  const box = voucherSearchBox();
  const input = voucherSearchInput();
  if (!box) return;
  box.textContent = '';
  const empty = document.createElement('div');
  empty.className = 'vf-search-result-empty';
  empty.textContent = message;
  box.append(empty);
  box.hidden = false;
  input?.setAttribute('aria-expanded', 'true');
  positionVoucherSearchResults();
}

function renderVoucherSearchResults(results) {
  const box = voucherSearchBox();
  const input = voucherSearchInput();
  if (!box) return;

  searchResults = results;
  searchIndex = results.length ? 0 : -1;
  box.textContent = '';

  if (!results.length) {
    const empty = document.createElement('div');
    empty.className = 'vf-search-result-empty';
    empty.textContent = 'سندی برای این جستجو یافت نشد.';
    box.append(empty);
  } else {
    results.forEach((voucher, index) => {
      const item = document.createElement('button');
      item.type = 'button';
      item.className = `vf-search-result${index === 0 ? ' is-active' : ''}`;
      item.dataset.index = String(index);
      item.setAttribute('role', 'option');
      item.setAttribute('aria-selected', index === 0 ? 'true' : 'false');
      item.title = `${vfFa(voucher.number)} — ${vfFa(voucher.date)} — ${voucher.type || ''}`;

      const number = document.createElement('span');
      number.className = 'vf-search-result-no';
      number.textContent = `سند ${vfFa(voucher.number)}`;

      const date = document.createElement('span');
      date.className = 'vf-search-result-date';
      date.textContent = vfFa(voucher.date || '');

      const description = document.createElement('span');
      description.className = 'vf-search-result-desc';
      description.textContent = voucher.description || voucher.type || '—';

      item.append(number, date, description);
      box.append(item);
    });
  }

  box.hidden = false;
  input?.setAttribute('aria-expanded', 'true');
  positionVoucherSearchResults();
}

async function runVoucherSearch(term) {
  if (!invoke) return;
  const wanted = term.trim();
  if (!wanted) {
    hideVoucherSearchResults();
    return;
  }

  const request = ++searchRequest;
  try {
    const payload = await invoke('list_vouchers', { from: '', to: '', search: wanted });
    if (request !== searchRequest) return;
    if (normalizeSearch(voucherSearchInput()?.value || '') !== normalizeSearch(wanted)) return;

    const allResults = Array.isArray(payload?.vouchers) ? payload.vouchers : [];
    let results;
    if (isVoucherNumberTerm(wanted)) {
      const exactNumber = digitsOnly(wanted);
      const exact = allResults.find((voucher) => String(voucher.number) === exactNumber);
      results = exact
        ? [exact, ...allResults.filter((voucher) => voucher !== exact).slice(0, 7)]
        : allResults.slice(0, 8);
    } else {
      results = allResults.slice(0, 8);
    }
    renderVoucherSearchResults(results);
  } catch (error) {
    if (request !== searchRequest) return;
    showVoucherSearchMessage(errorText(error));
  }
}

function chooseVoucherSearchResult(index = searchIndex) {
  const voucher = searchResults[index];
  if (!voucher) return;
  hideVoucherSearchResults();
  const input = voucherSearchInput();
  if (input) input.value = '';
  openNumber(voucher.number);
}

function bindVoucherSearch() {
  const input = voucherSearchInput();
  const box = voucherSearchBox();
  if (!input || !box) return;

  const search = () => {
    const term = input.value.trim();
    window.clearTimeout(searchTimer);
    searchRequest += 1;
    if (!term) {
      hideVoucherSearchResults();
      return;
    }
    searchTimer = window.setTimeout(() => runVoucherSearch(term), 180);
  };

  input.addEventListener('input', search);
  input.addEventListener('focus', () => {
    if (input.value.trim() && searchResults.length) {
      box.hidden = false;
      input.setAttribute('aria-expanded', 'true');
      positionVoucherSearchResults();
    }
  });

  input.addEventListener('keydown', (ev) => {
    if (ev.key === 'ArrowDown' || ev.key === 'ArrowUp') {
      ev.preventDefault();
      if (box.hidden) {
        if (input.value.trim()) runVoucherSearch(input.value.trim());
        return;
      }
      highlightVoucherSearchResult(searchIndex + (ev.key === 'ArrowDown' ? 1 : -1));
      return;
    }

    if (ev.key === 'Enter') {
      ev.preventDefault();
      if (searchResults.length) {
        chooseVoucherSearchResult();
        return;
      }
      const term = input.value.trim();
      if (!term) return;
      if (isVoucherNumberTerm(term)) {
        hideVoucherSearchResults();
        input.value = '';
        openNumber(digitsOnly(term));
      } else {
        runVoucherSearch(term);
      }
      return;
    }

    if (ev.key === 'Escape' && !box.hidden) {
      ev.preventDefault();
      hideVoucherSearchResults();
    }
  });

  box.addEventListener('mousedown', (ev) => ev.preventDefault());
  box.addEventListener('click', (ev) => {
    const item = ev.target.closest('.vf-search-result');
    if (!item) return;
    chooseVoucherSearchResult(Number(item.dataset.index));
  });

  document.addEventListener('pointerdown', (ev) => {
    if (box.hidden) return;
    if (box.contains(ev.target) || input === ev.target) return;
    hideVoucherSearchResults();
  });

  window.addEventListener('resize', positionVoucherSearchResults);
  $('.vf-meta')?.addEventListener('scroll', positionVoucherSearchResults, { passive: true });
}
/* ── Excel and account ledger ───────────────────────────────── */

async function importVoucherFromExcel() {
  if (state.busy) return;
  if (!invoke) {
    toast('دریافت از اکسل فقط در نسخه دسکتاپ ممکن است.', 'error');
    return;
  }

  state.busy = true;
  try {
    const imported = await invoke('import_voucher_excel');
    state.busy = false;
    if (!imported) return;
    if (!Array.isArray(imported) || !imported.length) {
      toast('ردیفی برای افزودن از فایل اکسل پیدا نشد.', 'info');
      return;
    }

    renderExcelPreview(imported);
  } catch (error) {
    state.busy = false;
    toast(errorText(error), 'error');
  }
}

async function exportVoucherToExcel() {
  if (state.busy) return;
  if (state.overflowVoucherLines) {
    toast('خروجی سند بزرگ از فرم محدودشده ممکن نیست؛ برای جلوگیری از فایل ناقص، عملیات متوقف شد.', 'error');
    return;
  }
  if (!invoke) {
    toast('خروجی اکسل فقط در نسخه دسکتاپ ممکن است.', 'error');
    return;
  }

  const draft = collectDraft();
  if (!draft.lines.length) {
    toast('سند هنوز ردیفی برای خروجی ندارد.', 'info');
    return;
  }

  state.busy = true;
  try {
    const saved = await invoke('export_voucher_excel', { input: draft });
    state.busy = false;
    if (!saved) return;
    const fileName = String(saved).split(/[\\/]/).pop();
    toast(`فایل اکسل «${fileName}» ذخیره شد.`, 'ok');
  } catch (error) {
    state.busy = false;
    toast(errorText(error), 'error');
  }
}

async function openAccountLedger() {
  if (state.busy) return;
  if (!invoke) {
    toast('گردش حساب فقط در نسخه دسکتاپ ممکن است.', 'error');
    return;
  }

  const row = state.activeRow && state.activeRow.isConnected
    ? state.activeRow
    : gridRows().find(rowHasValue);
  const code = digitsOnly(row?.querySelector('.vf-code-input')?.value || '');
  if (!code) {
    toast('ابتدا ردیفی با کد حساب انتخاب کنید.', 'info');
    return;
  }

  const account = accountForCode(code);
  if (!account) {
    toast(`حساب با کد ${vfFa(code)} یافت نشد.`, 'error');
    return;
  }

  state.ledger = {
    account,
    report: null,
    from: '',
    to: '',
    query: '',
    returnFocus: document.activeElement,
    request: 0,
  };
  $('#accountLedgerFrom').value = '';
  $('#accountLedgerTo').value = '';
  $('#accountLedgerSearch').value = '';
  $('#accountLedgerAccount').textContent = `${vfFa(account.code)} — ${account.name}`;
  $('#accountLedgerModal').hidden = false;
  $('.voucher-window')?.setAttribute('inert', '');
  $('#accountLedgerFrom').focus();
  await loadAccountLedger();
}

function closeAccountLedger() {
  const modal = $('#accountLedgerModal');
  if (!modal || modal.hidden) return;
  modal.hidden = true;
  $('.voucher-window')?.removeAttribute('inert');
  state.ledger.request += 1;
  const trigger = state.ledger.returnFocus;
  state.ledger.returnFocus = null;
  if (trigger && trigger.isConnected) trigger.focus();
}

function ledgerModalOpen() {
  return !$('#accountLedgerModal')?.hidden;
}

async function loadAccountLedger() {
  const account = state.ledger.account;
  if (!account || !invoke) return;
  const request = ++state.ledger.request;

  const fromInput = $('#accountLedgerFrom');
  const toInput = $('#accountLedgerTo');
  const fromText = toAsciiDigits(fromInput.value).replace(/-/g, '/').trim();
  const toText = toAsciiDigits(toInput.value).replace(/-/g, '/').trim();
  const message = $('#accountLedgerMessage');
  const rows = $('#accountLedgerRows');
  const foot = $('#accountLedgerFoot');
  let invalidDateField = '';

  try {
    fromInput.classList.remove('is-invalid');
    toInput.classList.remove('is-invalid');
    state.ledger.from = '';
    state.ledger.to = '';
    let from = '';
    let to = '';
    if (fromText) {
      try {
        from = await invoke('validate_voucher_date', { date: fromText });
      } catch (error) {
        invalidDateField = 'from';
        throw error;
      }
    }
    if (toText) {
      try {
        to = await invoke('validate_voucher_date', { date: toText });
      } catch (error) {
        invalidDateField = 'to';
        throw error;
      }
    }
    if (request !== state.ledger.request) return;
    if (from && to && from > to) throw new Error('تاریخ شروع نباید پس از تاریخ پایان باشد.');

    state.ledger.from = from;
    state.ledger.to = to;
    state.ledger.report = null;
    rows.replaceChildren();
    foot.replaceChildren();
    $('#accountLedgerSummary').replaceChildren();
    $('#accountLedgerMeta').textContent = '';
    message.textContent = 'در حال دریافت گردش حساب…';
    message.dataset.tone = 'info';

    const report = await invoke('get_ledger', {
      accountCode: account.code,
      from,
      to,
    });
    if (request !== state.ledger.request) return;
    state.ledger.report = report;
    renderAccountLedger();
  } catch (error) {
    if (request !== state.ledger.request) return;
    if (invalidDateField === 'from') fromInput.classList.add('is-invalid');
    if (invalidDateField === 'to') toInput.classList.add('is-invalid');
    state.ledger.report = null;
    rows.replaceChildren();
    foot.replaceChildren();
    $('#accountLedgerSummary').replaceChildren();
    $('#accountLedgerMeta').textContent = '';
    message.textContent = errorText(error);
    message.dataset.tone = 'error';
  }
}

function ledgerBalance(debit, credit) {
  const net = BigInt(String(debit || 0)) - BigInt(String(credit || 0));
  const side = net > 0n ? 'بدهکار' : net < 0n ? 'بستانکار' : 'تراز';
  return `${vfFa(formatAmount(net))} ${side}`;
}

function ledgerCell(row, value, className = '') {
  const cell = document.createElement('td');
  cell.textContent = String(value ?? '—');
  if (className) cell.className = className;
  row.append(cell);
  return cell;
}

function visibleLedgerRows() {
  const report = state.ledger.report;
  if (!report) return [];
  const query = normalizeSearch(state.ledger.query);
  return report.rows.filter((row) => {
    if (!query) return true;
    return normalizeSearch([
      row.number,
      row.date,
      row.lineDescription,
      row.voucherDescription,
      row.debit,
      row.credit,
      row.runningDebit,
      row.runningCredit,
    ].join(' ')).includes(query);
  });
}

function renderAccountLedger() {
  const report = state.ledger.report;
  if (!report) return;

  const summary = $('#accountLedgerSummary');
  summary.replaceChildren();
  const stats = [
    ['مانده افتتاحیه', ledgerBalance(report.openingDebit, report.openingCredit)],
    ['گردش بدهکار', vfFa(formatAmount(BigInt(String(report.totalDebit || 0))))],
    ['گردش بستانکار', vfFa(formatAmount(BigInt(String(report.totalCredit || 0))))],
    ['مانده اختتامیه', ledgerBalance(report.closingDebit, report.closingCredit)],
  ];
  stats.forEach(([label, value]) => {
    const item = document.createElement('div');
    item.className = 'ledger-stat';
    const caption = document.createElement('span');
    caption.textContent = label;
    const amount = document.createElement('strong');
    amount.textContent = value;
    item.append(caption, amount);
    summary.append(item);
  });

  const dateRange = `${report.from ? vfFa(report.from) : 'ابتدا'} تا ${report.to ? vfFa(report.to) : 'انتها'}`;
  $('#accountLedgerMeta').textContent = `گروه: ${report.groupName || '—'}  |  دوره: ${dateRange}${report.includesChildren ? '  |  شامل زیرحساب‌ها' : ''}`;
  const rowsHost = $('#accountLedgerRows');
  rowsHost.replaceChildren();

  const opening = document.createElement('tr');
  opening.className = 'ledger-opening-row';
  ledgerCell(opening, '—', 'nowrap');
  ledgerCell(opening, '—', 'center');
  ledgerCell(opening, 'مانده افتتاحیه');
  ledgerCell(opening, vfFa(formatAmount(BigInt(String(report.openingDebit || 0)))), 'money debit');
  ledgerCell(opening, vfFa(formatAmount(BigInt(String(report.openingCredit || 0)))), 'money credit');
  ledgerCell(opening, '—', 'money');
  ledgerCell(opening, '—', 'money');
  rowsHost.append(opening);

  const filtered = visibleLedgerRows();
  filtered.forEach((item) => {
    const row = document.createElement('tr');
    ledgerCell(row, vfFa(item.date), 'nowrap');
    const numberCell = ledgerCell(row, '', 'center');
    const numberButton = document.createElement('button');
    numberButton.type = 'button';
    numberButton.className = 'ledger-voucher-link';
    numberButton.dataset.ledgerVoucher = String(item.number);
    numberButton.textContent = vfFa(item.number);
    numberButton.title = `بازکردن سند ${vfFa(item.number)}`;
    numberCell.append(numberButton);
    ledgerCell(row, item.lineDescription || item.voucherDescription || '—', 'ledger-description');
    ledgerCell(row, vfFa(formatAmount(BigInt(String(item.debit || 0)))), 'money debit');
    ledgerCell(row, vfFa(formatAmount(BigInt(String(item.credit || 0)))), 'money credit');
    ledgerCell(row, vfFa(formatAmount(BigInt(String(item.runningDebit || 0)))), 'money');
    ledgerCell(row, vfFa(formatAmount(BigInt(String(item.runningCredit || 0)))), 'money');
    rowsHost.append(row);
  });

  const foot = $('#accountLedgerFoot');
  foot.replaceChildren();
  const total = document.createElement('tr');
  const searching = normalizeSearch(state.ledger.query).length > 0;
  if (searching) {
    // The closing balance belongs to the whole period, not to the searched
    // subset, so show sums of the visible rows instead of mixing the two.
    const filteredDebit = filtered.reduce((sum, item) => sum + BigInt(String(item.debit || 0)), 0n);
    const filteredCredit = filtered.reduce((sum, item) => sum + BigInt(String(item.credit || 0)), 0n);
    ledgerCell(total, `جمع ردیف‌های فیلترشده (${vfFa(filtered.length)} از ${vfFa((report.rows || []).length)})`, 'ledger-total-label').colSpan = 3;
    ledgerCell(total, vfFa(formatAmount(filteredDebit)), 'money bold');
    ledgerCell(total, vfFa(formatAmount(filteredCredit)), 'money bold');
    ledgerCell(total, '—', 'money bold');
    ledgerCell(total, '—', 'money bold');
  } else {
    ledgerCell(total, `گردش دوره و مانده اختتامیه (${vfFa(filtered.length)} ردیف)`, 'ledger-total-label').colSpan = 3;
    ledgerCell(total, vfFa(formatAmount(BigInt(String(report.totalDebit || 0)))), 'money bold');
    ledgerCell(total, vfFa(formatAmount(BigInt(String(report.totalCredit || 0)))), 'money bold');
    ledgerCell(total, vfFa(formatAmount(BigInt(String(report.closingDebit || 0)))), 'money bold');
    ledgerCell(total, vfFa(formatAmount(BigInt(String(report.closingCredit || 0)))), 'money bold');
  }
  foot.append(total);

  const message = $('#accountLedgerMessage');
  message.textContent = filtered.length ? '' : 'در این بازه ردیفی برای این حساب ثبت نشده است.';
  message.dataset.tone = 'info';
}

function exportAccountLedger() {
  const report = state.ledger.report;
  if (!report) return;
  const csvCell = (value) => {
    let text = String(value ?? '');
    if (/^[=+\-@]/.test(text.trim())) text = `'${text}`;
    return `"${text.replace(/"/g, '""')}"`;
  };
  const values = [
    ['تاریخ', 'شماره سند', 'شرح', 'بدهکار', 'بستانکار', 'مانده بدهکار', 'مانده بستانکار'],
    ['', '', 'مانده افتتاحیه', report.openingDebit, report.openingCredit, '', ''],
    ...visibleLedgerRows().map((row) => [
      row.date, row.number, row.lineDescription || row.voucherDescription,
      row.debit, row.credit, row.runningDebit, row.runningCredit,
    ]),
  ];
  const csv = `\uFEFF${values.map((row) => row.map(csvCell).join(',')).join('\r\n')}`;
  const url = URL.createObjectURL(new Blob([csv], { type: 'text/csv;charset=utf-8' }));
  const link = document.createElement('a');
  link.href = url;
  link.download = `ledger-${state.ledger.account.code}.csv`;
  link.click();
  URL.revokeObjectURL(url);
}

async function printAccountLedger() {
  const { account, from, to, report } = state.ledger;
  if (!account || !report || !invoke) return;
  try {
    await invoke('open_ledger_preview', { accountCode: account.code, from, to });
  } catch (error) {
    toast(errorText(error), 'error');
  }
}

function bindAccountLedger() {
  $('#accountLedgerFilters')?.addEventListener('submit', (event) => {
    event.preventDefault();
    loadAccountLedger();
  });
  $('#accountLedgerSearch')?.addEventListener('input', (event) => {
    state.ledger.query = event.currentTarget.value;
    if (state.ledger.report) renderAccountLedger();
  });
  $('#accountLedgerReset')?.addEventListener('click', () => {
    $('#accountLedgerFrom').value = '';
    $('#accountLedgerTo').value = '';
    $('#accountLedgerSearch').value = '';
    state.ledger.query = '';
    loadAccountLedger();
  });
  $('#accountLedgerRefresh')?.addEventListener('click', loadAccountLedger);
  $('#accountLedgerExport')?.addEventListener('click', exportAccountLedger);
  $('#accountLedgerPrint')?.addEventListener('click', printAccountLedger);
  $('#btnAccountLedgerClose')?.addEventListener('click', closeAccountLedger);
  $('#accountLedgerRows')?.addEventListener('click', (event) => {
    const button = event.target.closest('[data-ledger-voucher]');
    if (!button) return;
    const number = Number(button.dataset.ledgerVoucher);
    closeAccountLedger();
    openNumber(number);
  });
  const modal = $('#accountLedgerModal');
  modal?.addEventListener('click', (event) => {
    if (event.target === event.currentTarget) closeAccountLedger();
  });
  modal?.addEventListener('keydown', (event) => {
    if (event.key !== 'Tab') return;
    const focusable = $$('button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex="-1"])', modal)
      .filter((element) => !element.hidden && element.getClientRects().length);
    if (!focusable.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  });
}


function applyVoucherGridSearch() {
  const input = $('#vfGridSearch');
  const count = $('#vfGridSearchCount');
  const query = normalizeSearch(input ? input.value : '');
  let visible = 0;
  let filled = 0;
  for (const row of gridRows()) {
    if (!rowHasValue(row)) { row.hidden = !!query; continue; }
    filled += 1;
    const code = digitsOnly(row.querySelector('.vf-code-input')?.value || '');
    const account = accountForCode(code);
    const haystack = normalizeSearch([
      code, account ? account.name : '',
      row.querySelector('.vf-desc-input')?.value || '',
      row.querySelector('.vf-debit-input')?.value || '',
      row.querySelector('.vf-credit-input')?.value || '',
    ].join(' '));
    const activeEditing = row === state.activeRow &&
      document.activeElement?.closest?.('#voucherRows tr') === row;
    const match = !query || haystack.includes(query) || activeEditing;
    row.hidden = !match;
    if (match) visible += 1;
  }
  if (count) count.textContent = query
    ? vfFa(visible) + ' از ' + vfFa(filled) + ' ردیف'
    : vfFa(filled) + ' ردیف';
}

function focusedVoucherRow() {
  const active = document.activeElement;
  const row = active && active.closest ? active.closest('#voucherRows tr') : null;
  return row || (state.activeRow && state.activeRow.isConnected ? state.activeRow : null);
}
function writeClipboardText(text) {
  if (navigator.clipboard && navigator.clipboard.writeText) return navigator.clipboard.writeText(text);
  return new Promise((resolve, reject) => {
    const area = document.createElement('textarea');
    area.value = text; area.style.position = 'fixed'; area.style.opacity = '0';
    document.body.append(area); area.select();
    const ok = document.execCommand('copy'); area.remove();
    if (ok) resolve(); else reject(new Error('دسترسی به کلیپ‌بورد ممکن نیست.'));
  });
}
async function copyVoucherRow() {
  const row = focusedVoucherRow();
  if (!row || !rowHasValue(row)) { toast('ابتدا یک ردیف دارای اطلاعات را انتخاب کنید.', 'info'); return; }
  const code = digitsOnly(row.querySelector('.vf-code-input')?.value || '');
  const description = row.querySelector('.vf-desc-input')?.value || '';
  const debit = digitsOnly(row.querySelector('.vf-debit-input')?.value || '');
  const credit = digitsOnly(row.querySelector('.vf-credit-input')?.value || '');
  const rowNumber = gridRows().filter(rowHasValue).indexOf(row) + 1;
  const line = [rowNumber, code, '', description, debit, credit].join('\t');
  try { await writeClipboardText(line); state.activeRow = row; toast('ردیف جاری در کلیپ‌بورد کپی شد.', 'ok'); }
  catch (error) { toast(errorText(error), 'error'); }
}
function parseVoucherClipboardRows(raw) {
  const lines = String(raw || '').split(/\r?\n/).filter((line) => line.trim());
  if (!lines.length) return [];
  const isHeader = (cells) => {
    const words = cells.map(normalizeSearch).join(' ');
    const hasCode = words.includes('کد') || words.includes('code') || words.includes('account');
    const hasDebit = words.includes('بدهکار') || words.includes('debit');
    const hasCredit = words.includes('بستانکار') || words.includes('credit');
    return hasCode && hasDebit && hasCredit;
  };
  const start = isHeader(lines[0].split('\t')) ? 1 : 0;
  const parsed = [];
  for (const line of lines.slice(start)) {
    const cells = line.split('\t').map((cell) => cell.trim());
    let code = '', description = '', debitText = '', creditText = '';
    if (cells.length >= 6) {
      code = cells[1] || '';
      description = cells[3] || '';
      debitText = cells[4] || '';
      creditText = cells[5] || '';
    } else if (cells.length >= 5) {
      code = cells[1] && accountForCode(cells[1]) ? cells[1] : (cells[0] || '');
      description = cells[2] || ''; debitText = cells[3] || ''; creditText = cells[4] || '';
    } else {
      code = cells[0] || ''; description = cells[1] || '';
      debitText = cells[2] || ''; creditText = cells[3] || '';
    }
    const accountCode = digitsOnly(code);
    const descriptionLength = [...description].length;
    if (descriptionLength > MAX_LINE_DESCRIPTION) {
      throw new Error(`شرح ردیف کلیپ‌بورد بیش از ${vfFa(MAX_LINE_DESCRIPTION)} نویسه است (${vfFa(descriptionLength)} نویسه).`);
    }
    const parseClipboardAmount = (text) => {
      const normalized = toAsciiDigits(text).replace(/[,٬\s\u200c\u00a0]/g, '');
      if (/[-−]/.test(normalized)) throw new Error('مبلغ منفی مجاز نیست؛ از ستون مقابل استفاده کنید.');
      if (/[./٫]/.test(normalized)) throw new Error('مبلغ باید عدد صحیح ریالی باشد؛ اعشار مجاز نیست.');
      if (!normalized) return 0;
      if (!/^\d+$/.test(normalized)) throw new Error(`مبلغ «${text}» معتبر نیست.`);
      const amount = Number(normalized);
      if (!Number.isSafeInteger(amount) || amount > Number(MAX_AMOUNT))
        throw new Error('یکی از مبالغ کلیپ‌بورد خارج از محدوده مجاز است.');
      return amount;
    };
    const debit = parseClipboardAmount(debitText);
    const credit = parseClipboardAmount(creditText);
    if (!accountCode && !description && !debit && !credit) continue;
    if (!Number.isSafeInteger(debit) || !Number.isSafeInteger(credit) ||
        debit > Number(MAX_AMOUNT) || credit > Number(MAX_AMOUNT))
      throw new Error('یکی از مبالغ کلیپ‌بورد خارج از محدوده مجاز است.');
    if (debit > 0 && credit > 0)
      throw new Error('در ردیف چسبانده‌شده، هم‌زمان بدهکار و بستانکار پر شده است.');
    parsed.push({ accountCode, description, debit, credit });
  }
  return parsed;
}
async function pasteVoucherRows() {
  try {
    if (!navigator.clipboard || !navigator.clipboard.readText)
      throw new Error('خواندن کلیپ‌بورد در این پنجره مجاز نیست.');
    const parsed = parseVoucherClipboardRows(await navigator.clipboard.readText());
    if (!parsed.length) { toast('ردیف قابل استفاده‌ای در کلیپ‌بورد پیدا نشد.', 'info'); return; }
    const tbody = $('#voucherRows'); if (!tbody) return;
    const existingCount = gridRows().filter(rowHasValue).length;
    const availableCount = Math.max(0, MAX_VOUCHER_LINES - existingCount);
    if (parsed.length > availableCount) {
      toast(`ظرفیت باقی‌مانده ${vfFa(availableCount)} ردیف است، اما ${vfFa(parsed.length)} ردیف درخواست شده؛ هیچ ردیفی وارد نشد.`, 'warning');
      return;
    }
    const filter = $('#vfGridSearch'); if (filter) filter.value = '';
    let anchor = focusedVoucherRow();
    if (anchor && !rowHasValue(anchor)) anchor = anchor.previousElementSibling;
    if (!anchor || anchor.parentElement !== tbody) {
      const filled = gridRows().filter(rowHasValue);
      anchor = filled.length ? filled[filled.length - 1] : null;
    }
    let inserted = 0;
    for (const line of parsed) {
      if (anchor) anchor.insertAdjacentHTML('afterend', voucherRowTemplate());
      else tbody.insertAdjacentHTML('afterbegin', voucherRowTemplate());
      const row = anchor ? anchor.nextElementSibling : tbody.firstElementChild;
      if (!row) break;
      row.querySelector('.vf-code-input').value = line.accountCode;
      row.querySelector('.vf-desc-input').value = line.description;
      row.querySelector('.vf-debit-input').value = line.debit ? formatAmount(line.debit) : '';
      row.querySelector('.vf-credit-input').value = line.credit ? formatAmount(line.credit) : '';
      updateRowAccount(row, true); anchor = row; inserted += 1;
    }
    if (!inserted) { toast('ظرفیت مجاز ردیف‌های سند تکمیل است.', 'warning'); return; }
    state.activeRow = anchor; ensureTrailingBlank(); syncVoucherRowCount();
    refreshFormState(); markDirty();
    toast(vfFa(inserted) + ' ردیف از کلیپ‌بورد چسبانده شد.', 'ok');
    anchor.querySelector('.vf-code-input')?.focus();
  } catch (error) { toast(errorText(error), 'error'); }
}
function transferModalOpen() {
  const modal = $('#vfTransferModal'); return !!modal && !modal.hidden;
}
function excelPreviewOpen() {
  const modal = $('#vfExcelPreviewModal'); return !!modal && !modal.hidden;
}
function renderVoucherTransferSummary() {
  const source = state.transfer.source, summary = $('#vfTransferSummary');
  if (!source || !summary) return;
  const chosen = source.lines.filter((line) => state.transfer.selectedIds.has(line.id));
  const debit = chosen.reduce((sum, line) => sum + BigInt(line.debit || 0), 0n);
  const credit = chosen.reduce((sum, line) => sum + BigInt(line.credit || 0), 0n);
  const difference = debit > credit ? debit - credit : credit - debit;
  summary.textContent = vfFa(chosen.length) + ' ردیف | بدهکار ' +
    vfFa(formatAmount(debit)) + ' | بستانکار ' + vfFa(formatAmount(credit)) +
    ' | مغایرت ' + vfFa(formatAmount(difference));
}
function renderVoucherTransferRows() {
  const tbody = $('#vfTransferRows'), source = state.transfer.source;
  if (!tbody || !source) return;
  tbody.textContent = '';
  for (const line of source.lines) {
    const row = document.createElement('tr');
    const pickCell = document.createElement('td'); pickCell.className = 'vf-center';
    const checkbox = document.createElement('input');
    checkbox.type = 'checkbox'; checkbox.className = 'vf-transfer-check';
    checkbox.dataset.lineId = String(line.id);
    checkbox.checked = state.transfer.selectedIds.has(line.id);
    checkbox.setAttribute('aria-label', 'انتخاب ردیف ' + vfFa(line.rowNo));
    pickCell.append(checkbox); row.append(pickCell);
    const values = [vfFa(line.rowNo), vfFa(line.accountCode), line.accountName || '',
      line.description || '', vfFa(formatAmount(line.debit || 0)), vfFa(formatAmount(line.credit || 0))];
    values.forEach((value, index) => {
      const cell = document.createElement('td'); cell.textContent = value;
      if (index >= 4) cell.className = 'vf-money'; row.append(cell);
    });
    if (checkbox.checked) row.classList.add('selected');
    checkbox.addEventListener('change', () => {
      if (checkbox.checked) state.transfer.selectedIds.add(line.id);
      else state.transfer.selectedIds.delete(line.id);
      row.classList.toggle('selected', checkbox.checked); renderVoucherTransferSummary();
    });
    row.addEventListener('click', (event) => {
      if (event.target === checkbox) return;
      checkbox.checked = !checkbox.checked;
      checkbox.dispatchEvent(new Event('change', { bubbles: true }));
    });
    tbody.append(row);
  }
  renderVoucherTransferSummary();
}
async function openVoucherTransfer(mode) {
  if (state.busy || !invoke || (mode !== 'copy' && mode !== 'move')) return;
  let source = state.voucher;
  if (!source) { toast('برای کپی یا انتقال ردیف‌ها، ابتدا یک سند ذخیره‌شده را باز کنید.', 'warning'); return; }
  if (state.dirty) {
    source = await saveVoucher({ silent: true });
    if (!source) return;
  }
  const draftSnapshot = JSON.stringify(collectDraft());
  state.busy = true;
  let next;
  try {
    next = await invoke('next_voucher_number');
  } catch (error) {
    toast(errorText(error), 'error');
    return;
  } finally {
    state.busy = false;
  }
  if (JSON.stringify(collectDraft()) !== draftSnapshot) {
    state.dirty = true;
    toast('فرم هنگام آماده‌سازی انتقال تغییر کرد؛ اطلاعات حفظ شد و عملیات باز نشد.', 'warning');
    return;
  }

  state.transfer.mode = mode; state.transfer.source = source;
  state.transfer.selectedIds = new Set();
  const target = $('#vfTransferTarget'); if (target) target.value = String(next);
  const title = $('#vfTransferTitle'), modeLabel = $('#vfTransferModeLabel');
  if (title) title.textContent = mode === 'move' ? 'انتقال ردیف‌های سند به سند دیگر' : 'کپی ردیف‌های سند به سند دیگر';
  if (modeLabel) modeLabel.textContent = mode === 'move'
    ? 'حالت عملیات: انتقال (حذف از مبدأ پس از تأیید)'
    : 'حالت عملیات: کپی (سند مبدأ حفظ می‌شود)';
  renderVoucherTransferRows();
  const modal = $('#vfTransferModal');
  transferReturnFocus = document.activeElement;
  const main = $('.voucher-window');
  transferMainWasInert = Boolean(main?.inert);
  modal.hidden = false;
  if (main) main.inert = true;
  target?.focus(); target?.select();
  void updateTransferTargetInfo();
}
function closeVoucherTransfer() {
  if (state.busy) return;
  const modal = $('#vfTransferModal'); if (!modal || modal.hidden) return;
  modal.hidden = true;
  const main = $('.voucher-window'); if (main) main.inert = transferMainWasInert;
  state.transfer.source = null; state.transfer.selectedIds = new Set();
  const returnFocus = transferReturnFocus;
  transferReturnFocus = null;
  if (returnFocus?.isConnected && !returnFocus.closest('[inert]')) returnFocus.focus();
}
let transferTargetInfoSeq = 0;
function transferTargetNumber() {
  const text = toAsciiDigits($('#vfTransferTarget')?.value || '').trim();
  if (!/^\d+$/.test(text)) return { text, number: null };
  const number = Number(text);
  if (!Number.isSafeInteger(number) || number <= 0) return { text, number: null };
  return { text, number };
}
function setTransferTargetInfo(message, tone = '') {
  const info = $('#vfTransferTargetInfo');
  if (!info) return;
  info.textContent = message;
  if (tone) info.dataset.tone = tone;
  else delete info.dataset.tone;
}
async function updateTransferTargetInfo() {
  const { text, number } = transferTargetNumber();
  if (!text) {
    setTransferTargetInfo('');
    return;
  }
  if (number == null) {
    setTransferTargetInfo('شماره سند مقصد باید فقط شامل رقم باشد و بزرگ‌تر از صفر.', 'error');
    return;
  }
  if (!invoke) return;
  const seq = ++transferTargetInfoSeq;
  setTransferTargetInfo('در حال بررسی سند مقصد…');
  try {
    const voucher = await invoke('get_voucher', { number });
    if (seq !== transferTargetInfoSeq || !transferModalOpen()) return;
    if (voucher) {
      setTransferTargetInfo(
        `سند مقصد موجود است: شماره ${vfFa(voucher.number)} · تاریخ ${vfFa(voucher.date)} · ${voucher.type}`,
        'ok'
      );
    } else {
      setTransferTargetInfo(
        `سند شماره ${vfFa(number)} وجود ندارد؛ با اجرای عملیات، سند جدیدی با این شماره ساخته می‌شود.`,
        'info'
      );
    }
  } catch (error) {
    if (seq !== transferTargetInfoSeq) return;
    setTransferTargetInfo(`بررسی سند مقصد ناموفق بود: ${errorText(error)}`, 'error');
  }
}
async function submitVoucherTransfer() {
  if (state.busy) return;
  const source = state.transfer.source;
  const lineIds = Array.from(state.transfer.selectedIds);
  const { text: targetText, number: targetNumber } = transferTargetNumber();
  if (!source || lineIds.length < 2) { toast('حداقل دو ردیف مرتبط را انتخاب کنید.', 'warning'); return; }
  if (!targetText || targetNumber == null) {
    toast('شماره سند مقصد باید فقط شامل رقم باشد و بزرگ‌تر از صفر.', 'error'); return;
  }
  if (targetNumber === Number(source.number)) { toast('شماره سند مقصد باید با سند مبدأ متفاوت باشد.', 'error'); return; }

  state.busy = true;
  let targetVoucher;
  try {
    targetVoucher = await invoke('get_voucher', { number: targetNumber });
  } catch (error) {
    state.busy = false;
    toast(`بررسی سند مقصد ناموفق بود: ${errorText(error)}`, 'error');
    return;
  }
  state.busy = false;

  const targetLabel = targetVoucher
    ? `سند مقصد: شماره ${vfFa(targetVoucher.number)} · تاریخ ${vfFa(targetVoucher.date)} · ${targetVoucher.type}`
    : `سند مقصد: شماره ${vfFa(targetNumber)} (سند جدید)`;
  setTransferTargetInfo(targetLabel, targetVoucher ? 'ok' : 'info');

  if (transferTargetNumber().number !== targetNumber) {
    state.busy = false;
    toast('شماره سند مقصد حین بررسی تغییر کرد؛ عملیات متوقف شد.', 'warning');
    return;
  }

  if (state.transfer.mode === 'move') {
    const ok = await confirmAction(
      `${targetLabel} · ردیف‌های انتخاب‌شده از سند مبدأ حذف و به سند مقصد افزوده می‌شوند. فقط در صورتی عملیات انجام می‌شود که هر دو سند پس از انتقال تراز بمانند. ادامه می‌دهید؟`,
      'انتقال ردیف‌ها'
    );
    if (!ok) return;
    if (transferTargetNumber().number !== targetNumber) {
      toast('شماره سند مقصد حین تأیید تغییر کرد؛ عملیات متوقف شد.', 'warning');
      return;
    }
  }
  const operationMode = state.transfer.mode;
  state.busy = true; setStatus('info', 'در حال کپی یا انتقال ردیف‌ها…');
  try {
    const result = await invoke('transfer_voucher_lines', { input: {
      sourceVoucherId: source.id,
      sourceVoucherNumber: source.number,
      targetVoucherNumber: targetNumber,
      lineIds, mode: operationMode,
    }});
    state.busy = false; closeVoucherTransfer(); loadVoucher(result.target);
    toast(operationMode === 'move'
      ? 'ردیف‌ها به سند شماره ' + vfFa(result.target.number) + ' منتقل شدند.'
      : 'ردیف‌ها در سند شماره ' + vfFa(result.target.number) + ' کپی شدند.', 'ok');
  } catch (error) {
    state.busy = false; setStatus('error', 'عملیات ردیف‌ها انجام نشد'); toast(errorText(error), 'error');
  }
}
function bindVoucherTransfer() {
  $('#vfTransferClose')?.addEventListener('click', closeVoucherTransfer);
  $('#vfTransferCancel')?.addEventListener('click', closeVoucherTransfer);
  $('#vfTransferSubmit')?.addEventListener('click', submitVoucherTransfer);
  $('#vfTransferTarget')?.addEventListener('change', () => void updateTransferTargetInfo());
  $('#vfTransferModal')?.addEventListener('click', (event) => {
    if (event.target.id === 'vfTransferModal') closeVoucherTransfer();
  });
  $('#vfTransferModal')?.addEventListener('keydown', (event) => {
    if (event.key !== 'Tab') return;
    const modal = $('#vfTransferModal');
    const focusable = $$('button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex="-1"])', modal)
      .filter((element) => !element.hidden && element.getClientRects().length > 0);
    if (!focusable.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  });
  $('#vfTransferSelectAll')?.addEventListener('click', () => {
    if (!state.transfer.source) return;
    state.transfer.selectedIds = new Set(state.transfer.source.lines.map((line) => line.id));
    renderVoucherTransferRows();
  });
  $('#vfTransferSelectNone')?.addEventListener('click', () => {
    state.transfer.selectedIds = new Set(); renderVoucherTransferRows();
  });
}
function closeExcelPreview() {
  const modal = $('#vfExcelPreviewModal'); if (!modal || modal.hidden) return;
  modal.hidden = true;
  const main = $('.voucher-window'); if (main) main.inert = excelMainWasInert;
  state.excelPreview = null;
  const returnFocus = excelReturnFocus;
  excelReturnFocus = null;
  if (returnFocus?.isConnected && !returnFocus.closest('[inert]')) returnFocus.focus();
}
function renderExcelPreview(imported) {
  const tbody = $('#vfExcelPreviewRows'), summary = $('#vfExcelPreviewSummary');
  if (!tbody || !summary) return;
  const items = imported.map((line, index) => {
    const code = digitsOnly(line.accountCode || ''), account = accountForCode(code);
    const debit = Number(line.debit || 0), credit = Number(line.credit || 0), errors = [];
    if (!code) errors.push('کد حساب خالی');
    else if (!account) errors.push('حساب یافت نشد');
    else if (!account.isPostable) errors.push('حساب قابل ثبت نیست');
    if ([...String(line.description || '')].length > MAX_LINE_DESCRIPTION) errors.push(`شرح بیش از ${vfFa(MAX_LINE_DESCRIPTION)} نویسه`);
    if (!Number.isSafeInteger(debit) || !Number.isSafeInteger(credit) ||
        debit < 0 || credit < 0 || debit > Number(MAX_AMOUNT) || credit > Number(MAX_AMOUNT))
      errors.push('مبلغ نامعتبر');
    else if ((debit === 0) === (credit === 0))
      errors.push('دقیقاً یکی از بدهکار/بستانکار باید مثبت باشد');
    return { index, line: { accountCode: code, description: String(line.description || ''), debit, credit }, account, errors };
  });
  const validLines = items.filter((item) => !item.errors.length).map((item) => item.line);
  state.excelPreview = { lines: validLines, total: items.length, invalid: items.length - validLines.length };
  tbody.textContent = '';
  items.forEach((item) => {
    const row = document.createElement('tr'); if (item.errors.length) row.classList.add('invalid');
    const values = [vfFa(item.index + 1), vfFa(item.line.accountCode),
      item.account ? item.account.name : '—', item.line.description,
      vfFa(formatAmount(item.line.debit)), vfFa(formatAmount(item.line.credit)),
      item.errors.length ? item.errors.join('، ') : 'معتبر'];
    values.forEach((value, index) => {
      const cell = document.createElement('td'); cell.textContent = value;
      if (index === 4 || index === 5) cell.className = 'vf-money';
      if (index === 6) cell.className = item.errors.length ? 'vf-import-error' : 'vf-import-ok';
      row.append(cell);
    });
    tbody.append(row);
  });
  summary.textContent = vfFa(items.length) + ' ردیف از اکسل خوانده شد؛ ' +
    vfFa(validLines.length) + ' ردیف معتبر و ' + vfFa(state.excelPreview.invalid) +
    ' ردیف دارای خطا. فقط ردیف‌های معتبر قابل افزودن هستند.';
  $('#vfExcelAppend').disabled = !validLines.length;
  $('#vfExcelReplace').disabled = !validLines.length;
  const modal = $('#vfExcelPreviewModal');
  if (modal.hidden) {
    excelReturnFocus = document.activeElement;
    const main = $('.voucher-window');
    excelMainWasInert = Boolean(main?.inert);
    if (main) main.inert = true;
    modal.hidden = false;
    ($('#vfExcelAppend').disabled ? $('#vfExcelPreviewCancel') : $('#vfExcelAppend'))?.focus();
  }
}
async function commitExcelPreview(replaceExisting) {
  const preview = state.excelPreview; if (!preview || !preview.lines.length) return;
  const existingCount = collectDraft().lines.length;
  const resultCount = replaceExisting ? preview.lines.length : existingCount + preview.lines.length;
  if (resultCount > MAX_VOUCHER_LINES) {
    const availableCount = replaceExisting ? MAX_VOUCHER_LINES : Math.max(0, MAX_VOUCHER_LINES - existingCount);
    toast(`ظرفیت ${vfFa(availableCount)} ردیف است، اما ${vfFa(preview.lines.length)} ردیف معتبر انتخاب شده؛ هیچ تغییری اعمال نشد.`, 'warning');
    return;
  }
  if (replaceExisting && collectDraft().lines.length) {
    const ok = await confirmAction(
      'ردیف‌های فعلی سند با ردیف‌های معتبر فایل اکسل جایگزین می‌شوند. این تغییر تا زمان ذخیره سند قابل بازگشت است. ادامه می‌دهید؟',
      'جایگزینی ردیف‌ها'
    );
    if (!ok) return;
  }
  const lines = replaceExisting ? preview.lines : collectDraft().lines.concat(preview.lines);
  fillGrid(lines); state.dirty = true; closeExcelPreview(); markDirty();
  toast(vfFa(preview.lines.length) + ' ردیف معتبر از پیش‌نمایش اکسل ' +
    (replaceExisting ? 'جایگزین' : 'به سند افزوده') + ' شد.', 'ok');
}
function bindExcelPreview() {
  $('#vfExcelPreviewClose')?.addEventListener('click', closeExcelPreview);
  $('#vfExcelPreviewCancel')?.addEventListener('click', closeExcelPreview);
  $('#vfExcelAppend')?.addEventListener('click', () => commitExcelPreview(false));
  $('#vfExcelReplace')?.addEventListener('click', () => commitExcelPreview(true));
  $('#vfExcelPreviewModal')?.addEventListener('click', (event) => {
    if (event.target.id === 'vfExcelPreviewModal') closeExcelPreview();
  });
  $('#vfExcelPreviewModal')?.addEventListener('keydown', (event) => {
    if (event.key !== 'Tab') return;
    const modal = $('#vfExcelPreviewModal');
    const focusable = $$('button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex="-1"])', modal)
      .filter((element) => !element.hidden && element.getClientRects().length > 0);
    if (!focusable.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  });
}
function bindVoucherGridSearch() {
  $('#vfGridSearch')?.addEventListener('input', applyVoucherGridSearch);
  applyVoucherGridSearch();
}
/* ── Toolbar and keyboard ───────────────────────────────────── */

function bindToolbar() {
  $$('[data-action]').forEach((button) => {
    button.addEventListener('click', async () => {
      if (state.busy) return;
      switch (button.dataset.action) {
        case 'save':
          await saveVoucher();
          break;
        case 'new':
          await newVoucher();
          break;
        case 'copy':
          await copyVoucher();
          break;
        case 'print':
          await printVoucher();
          break;
        case 'add-row':
          addRow();
          break;
        case 'repeat-row':
          repeatRow(false);
          break;
        case 'delete-row':
          deleteRow();
          break;
        case 'ledger':
          await openAccountLedger();
          break;
        case 'excel-import':
          await importVoucherFromExcel();
          break;
        case 'excel-export':
          await exportVoucherToExcel();
          break;
        case 'balance':
          checkBalance();
          break;
        case 'delete-voucher':
          await deleteVoucher();
          break;
        case 'transfer-copy':
          await openVoucherTransfer('copy');
          break;
        case 'transfer-move':
          await openVoucherTransfer('move');
          break;
        case 'copy-row':
          await copyVoucherRow();
          break;
        case 'paste-row':
          await pasteVoucherRows();
          break;
        default:
          break;
      }
    });
  });
}

function bindWindowKeys() {
  document.addEventListener('keydown', (ev) => {
    if (confirmOpen()) return;
    if (ev.defaultPrevented) return;
    if (transferModalOpen() || excelPreviewOpen()) {
      if (ev.key === 'Escape') {
        ev.preventDefault();
        if (transferModalOpen()) closeVoucherTransfer();
        else closeExcelPreview();
      }
      return;
    }

    if (ev.key === 'Escape') {
      if (ledgerModalOpen()) {
        ev.preventDefault();
        closeAccountLedger();
        return;
      }
      // Escape only backs out of the account picker. The OS close button
      // closes the window; a stray Escape must not throw the entry away.
      if (accountPickerOpen()) {
        ev.preventDefault();
        closeAccountPicker(true);
      }
      return;
    }

    if (accountPickerOpen()) return;

    if (ledgerModalOpen()) {
      if (ev.ctrlKey && ev.key.toLowerCase() === 'p') {
        ev.preventDefault();
        printAccountLedger();
      }
      return;
    }

    const gridInput = ev.target?.closest?.('#voucherRows input');
    if (gridInput && ev.ctrlKey && !ev.altKey && !ev.metaKey) {
      const hasTextSelection = typeof gridInput.selectionStart === 'number' &&
        gridInput.selectionStart !== gridInput.selectionEnd;
      const key = ev.key.toLowerCase();
      if (key === 'c' && !hasTextSelection) {
        ev.preventDefault(); copyVoucherRow(); return;
      }
      if (key === 'v' && !hasTextSelection) {
        ev.preventDefault(); pasteVoucherRows(); return;
      }
    }

    if (ev.key === 'F2' && !accountPickerOpen()) {
      ev.preventDefault();
      saveVoucher();
      return;
    }

    if (ev.ctrlKey && !ev.altKey && !ev.metaKey) {
      const key = ev.key.toLowerCase();
      if (key === 's') {
        ev.preventDefault();
        saveVoucher();
        return;
      }
      if (key === 'n') {
        ev.preventDefault();
        newVoucher();
        return;
      }
      if (key === 'p') {
        ev.preventDefault();
        printVoucher();
        return;
      }
      if (key === 'f') {
        ev.preventDefault();
        const search = voucherSearchInput();
        if (search) {
          search.focus();
          search.select();
        }
        return;
      }
    }

    const reloadKey = ev.key === 'F5' || (ev.ctrlKey && (ev.key === 'r' || ev.key === 'R'));
    if (reloadKey && state.dirty) {
      ev.preventDefault();
      toast('برای جلوگیری از از دست رفتن اطلاعات، تازه‌سازی غیرفعال است.', 'info');
    }
  });

  window.addEventListener('beforeunload', (ev) => {
    if (state.allowClose || !state.dirty) return;
    ev.preventDefault();
    ev.returnValue = '';
  });
}

function bindCloseGuard() {
  if (!appWindow || typeof appWindow.onCloseRequested !== 'function') return;
  try {
    appWindow.onCloseRequested(async (event) => {
      if (state.allowClose || !state.dirty) return;
      event.preventDefault();
      const ok = await confirmAction('تغییرات ذخیره‌نشده از بین می‌رود. پنجره بسته شود؟', 'بستن بدون ذخیره');
      if (ok) {
        state.allowClose = true;
        await appWindow.close();
      }
    });
  } catch {
    // The close guard is a convenience; closing still works without it.
  }
}

/* ── Data loading ───────────────────────────────────────────── */

async function loadAccounts() {
  if (!invoke) return;
  const payload = await invoke('list_accounts');
  state.accounts = Array.isArray(payload?.accounts) ? payload.accounts : [];
  state.accountByCode = new Map(state.accounts.map((account) => [String(account.code), account]));
  if (state.ready) gridRows().forEach((row) => updateRowAccount(row));
}

async function loadSettings() {
  if (!invoke) return;
  const settings = await invoke('get_settings');
  if (settings) {
    state.settings = settings;
    applyLimits(settings.limits);
  }
}

async function refreshNextNumber() {
  if (!invoke) return;
  const value = await invoke('next_voucher_number');
  state.nextNumber = Number(value) > 0 ? Number(value) : 1;
}

let queuedVoucherRequest;
let hasQueuedVoucherRequest = false;
let handlingVoucherRequest = false;

async function drainVoucherRequest() {
  if (!hasQueuedVoucherRequest || handlingVoucherRequest || state.busy || !state.ready) return;
  const number = queuedVoucherRequest;
  queuedVoucherRequest = undefined;
  hasQueuedVoucherRequest = false;
  await handleVoucherRequest(number);
}

async function handleVoucherRequest(number) {
  if (handlingVoucherRequest || state.busy) {
    queuedVoucherRequest = number;
    hasQueuedVoucherRequest = true;
    return;
  }
  handlingVoucherRequest = true;
  try {
    if (!(await guardUnsaved())) return;
    const draftSnapshot = JSON.stringify(collectDraft());
    state.busy = true;

    if (number && Number(number) > 0) {
      const voucher = await invoke('get_voucher', { number: Number(number) });
      if (hasQueuedVoucherRequest) return;
      if (JSON.stringify(collectDraft()) !== draftSnapshot) {
        state.dirty = true;
        toast('فرم هنگام بازکردن سند تغییر کرد؛ اطلاعات حفظ شد.', 'warning');
        return;
      }
      if (voucher) loadVoucher(voucher);
      else {
        await refreshNextNumber();
        if (hasQueuedVoucherRequest) return;
        if (JSON.stringify(collectDraft()) !== draftSnapshot) {
          state.dirty = true;
          toast('فرم هنگام بازکردن سند تغییر کرد؛ اطلاعات حفظ شد.', 'warning');
          return;
        }
        startNew(Number(number));
      }
    } else {
      await refreshNextNumber();
      if (hasQueuedVoucherRequest) return;
      if (JSON.stringify(collectDraft()) !== draftSnapshot) {
        state.dirty = true;
        toast('فرم هنگام بازکردن سند تغییر کرد؛ اطلاعات حفظ شد.', 'warning');
        return;
      }
      startNew(state.nextNumber);
    }

    if (appWindow) {
      try {
        await appWindow.setFocus();
      } catch {
        // Focus is best-effort.
      }
    }
  } catch (error) {
    toast(errorText(error), 'error');
  } finally {
    state.busy = false;
    handlingVoucherRequest = false;
    if (hasQueuedVoucherRequest) void drainVoucherRequest();
  }
}

async function bindVoucherRequests() {
  if (!TAURI || !TAURI.event || typeof TAURI.event.listen !== 'function') return;
  try {
    await TAURI.event.listen('voucher-request', (event) => {
      const number = event && event.payload;
      if (state.busy || handlingVoucherRequest || !state.ready) {
        queuedVoucherRequest = number;
        hasQueuedVoucherRequest = true;
        return;
      }
      void handleVoucherRequest(number);
    });
  } catch {
    // Events are an optimisation; opening the window still loads a voucher.
  }
}

async function handleDatabaseRestored() {
  await loadAccounts();
  if (!state.voucher) return;
  if (state.dirty) {
    state.restoredConflict = true;
    setStatus('error', 'پایگاه داده بازیابی شده؛ ذخیره مسدود است');
    toast('پایگاه داده هنگام ویرایش این سند بازیابی شد. برای جلوگیری از بازنویسی داده‌ها، ذخیره مسدود است؛ پنجره را ببندید و سند را دوباره باز کنید.', 'error');
    return;
  }
  const voucher = await invoke('get_voucher', { number: state.voucher.number });
  if (voucher) {
    loadVoucher(voucher);
  } else {
    state.restoredConflict = true;
    setStatus('error', 'سند در پایگاه بازیابی‌شده موجود نیست');
    toast('این سند در پایگاه بازیابی‌شده وجود ندارد؛ پنجره را ببندید و سند را دوباره انتخاب کنید.', 'error');
  }
}

/* ── Init ───────────────────────────────────────────────────── */

async function init() {
  bindVoucherGrid();
  bindAccountPicker();
  bindVoucherGridSearch();
  bindVoucherTransfer();
  bindExcelPreview();
  bindConfirm();
  bindDocMeta();
  bindVoucherSearch();
  bindToolbar();
  bindAccountLedger();
  bindWindowKeys();
  bindCloseGuard();
  updateDescCounter();

  if (!invoke) {
    setStatus('error', 'نسخه دسکتاپ در دسترس نیست');
    toast('این فرم فقط در برنامه دسکتاپ کار می‌کند.', 'error');
    $$('[data-action]').forEach((button) => {
      if (['save', 'new', 'copy', 'print', 'delete-voucher', 'ledger', 'excel-import', 'excel-export'].includes(button.dataset.action)) {
        button.disabled = true;
      }
    });
    return;
  }

  setStatus('info', 'در حال بارگذاری…');
  state.busy = true;
  try {
    await bindVoucherRequests();
    if (typeof TAURI.event?.listen === 'function') {
      await TAURI.event.listen('accounts-changed', () => {
        loadAccounts().catch(() => {});
      });
      await TAURI.event.listen('database-restored', () => {
        handleDatabaseRestored().catch((error) => toast(errorText(error), 'error'));
      });
    }
    await Promise.all([loadSettings(), loadAccounts(), refreshNextNumber()]);
    const requested = await invoke('take_voucher_request');
    state.ready = true;

    if (requested && Number(requested) > 0) {
      const voucher = await invoke('get_voucher', { number: Number(requested) });
      if (voucher) {
        loadVoucher(voucher);
      } else {
        startNew(Number(requested));
        toast(`سند شماره ${vfFa(requested)} یافت نشد؛ سند جدید با این شماره آغاز شد.`, 'info');
      }
    } else {
      startNew(state.nextNumber);
    }

    state.busy = false;
    await drainVoucherRequest();

    window.addEventListener('focus', () => {
      if (!state.ready) return;
      loadAccounts().catch(() => {});
    });
  } catch (error) {
    state.busy = false;
    setStatus('error', 'بارگذاری ناموفق');
    toast(errorText(error), 'error');
    startNew(state.nextNumber);
  }

  syncVoucherRowCount();
  refreshFormState();
}

document.addEventListener('DOMContentLoaded', init);
