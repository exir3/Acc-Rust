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
  nextNumber: 1,
  dirty: false,
  busy: false,
  allowClose: false,
  ready: false,
  activeRow: null,
};

const accountForCode = (code) => state.accountByCode.get(digitsOnly(code)) || null;

function levelLabel(level) {
  return ({ 1: 'کل', 2: 'معین', 3: 'تفصیلی ۱', 4: 'تفصیلی ۲' })[Number(level)] || '—';
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

// Largest amount per line, same as the backend (1e15 − 1 Rials). Beyond it
// JavaScript numbers lose precision.
const MAX_AMOUNT_DIGITS = 15;
const MAX_VOUCHER_LINES = 500;

/* ── Voucher grid ───────────────────────────────────────────── */

function voucherRowTemplate() {
  return `
    <tr>
      <td class="vf-rowno"></td>
      <td><input class="vf-code-input" type="text" dir="ltr" inputmode="numeric" autocomplete="off" aria-label="کد حساب" title="برای انتخاب حساب Space یا F4 را بزنید" /></td>
      <td class="vf-detail2" aria-label="تفصیلی 2"></td>
      <td><input class="vf-desc-input" type="text" dir="rtl" maxlength="500" autocomplete="off" aria-label="شرح" /></td>
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
  const account = accountForCode(code);
  const detailCell = row.querySelector('.vf-detail2');
  const level4 = account ? accountHierarchy(account).find((item) => Number(item.level) === 4) : null;

  if (detailCell) {
    detailCell.textContent = level4 ? `${vfFa(level4.code)} — ${level4.name}` : '—';
    detailCell.title = level4 ? `${level4.code} — ${level4.name}` : '';
  }
  if (input) {
    input.title = account
      ? `${account.code} — ${account.name}${account.isPostable ? '' : ' (قابل ثبت نیست)'}`
      : 'برای انتخاب حساب Space یا F4 را بزنید';
    if (account && account.isPostable) input.classList.remove('is-invalid');
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
  const headings = [renderLevel([1, 2]), renderLevel([3]), renderLevel([4])];
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
      let cleaned = digitsOnly(input.value);
      if (input.classList.contains('vf-amount-input') && cleaned.length > MAX_AMOUNT_DIGITS) {
        cleaned = cleaned.slice(0, MAX_AMOUNT_DIGITS);
        toast('مبلغ هر ردیف حداکثر ۱۵ رقم می‌تواند باشد.', 'error');
      }
      if (input.value !== cleaned) input.value = cleaned;
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
  tbody.textContent = '';
  (lines || []).forEach((line) => {
    const row = appendGridRow();
    if (!row) return;
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
  ensureTrailingBlank();
  syncVoucherRowCount();
  refreshFormState();
}

function startNew(number) {
  state.voucher = null;
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
    lines.push({ accountCode, description, debit, credit });
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
  if (!draft.date) return 'تاریخ سند را وارد کنید.';
  if (draft.lines.length > MAX_VOUCHER_LINES) {
    return `تعداد ردیف‌های سند نمی‌تواند بیشتر از ${vfFa(MAX_VOUCHER_LINES)} باشد.`;
  }
  const dateParts = /^(\d{4})\/(\d{1,2})\/(\d{1,2})$/.exec(draft.date);
  if (!dateParts) return 'تاریخ سند باید به شکل ۱۴۰۵/۰۱/۰۱ وارد شود.';
  const month = Number(dateParts[2]);
  const day = Number(dateParts[3]);
  const maxDay = month >= 1 && month <= 6 ? 31 : 30;
  if (month < 1 || month > 12 || day < 1 || day > maxDay) {
    return 'ماه یا روز تاریخ سند معتبر نیست.';
  }
  if (draft.lines.length < 2) return 'سند حسابداری باید حداقل دو ردیف داشته باشد.';

  let debit = 0n;
  let credit = 0n;
  for (let index = 0; index < draft.lines.length; index += 1) {
    const line = draft.lines[index];
    const rowNo = vfFa(index + 1);
    if (!line.accountCode) return `کد حساب ردیف ${rowNo} را وارد کنید.`;
    const account = accountForCode(line.accountCode);
    if (!account) return `حساب با کد ${vfFa(line.accountCode)} یافت نشد.`;
    if (!account.isPostable) return `حساب «${account.name}» قابل ثبت نیست؛ حساب معین یا تفصیلی انتخاب کنید.`;
    if (!line.debit && !line.credit) return `مبلغ ردیف ${rowNo} را وارد کنید.`;
    if (line.debit && line.credit) return `در ردیف ${rowNo} فقط یکی از بدهکار یا بستانکار را وارد کنید.`;
    if (line.debit < 0 || line.credit < 0) return `مبلغ ردیف ${rowNo} نمی‌تواند منفی باشد.`;
    if (!Number.isSafeInteger(line.debit) || !Number.isSafeInteger(line.credit)
      || line.debit > 999999999999999 || line.credit > 999999999999999) {
      return `مبلغ ردیف ${rowNo} باید عدد صحیح و حداکثر ۹۹۹٬۹۹۹٬۹۹۹٬۹۹۹٬۹۹۹ ریال باشد.`;
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
  if (state.busy) return null;
  if (!invoke) {
    toast('ذخیره‌سازی فقط در نسخه دسکتاپ ممکن است.', 'error');
    return null;
  }

  const draft = collectDraft();
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
    const dateInput = $('#vfDocDate');
    if (dateInput && typeof normalizedDate === 'string') dateInput.value = normalizedDate;
    draft.date = normalizedDate;
    // The form remains editable while the asynchronous calendar check runs.
    // Abort rather than saving a stale snapshot if the user changed anything.
    if (JSON.stringify(collectDraft()) !== JSON.stringify(draft)) {
      state.busy = false;
      setStatus('warn', 'فرم تغییر کرده است');
      toast('اطلاعات فرم هنگام اعتبارسنجی تغییر کرد؛ لطفاً دوباره ذخیره کنید.', 'warning');
      return null;
    }
    dateInput?.classList.remove('is-invalid');
    dateInput?.removeAttribute('aria-invalid');
    if (dateInput) dateInput.title = '';
  } catch (error) {
    state.busy = false;
    const dateInput = $('#vfDocDate');
    dateInput?.classList.add('is-invalid');
    dateInput?.setAttribute('aria-invalid', 'true');
    if (dateInput) dateInput.title = errorText(error);
    setStatus('error', 'تاریخ نامعتبر');
    toast(errorText(error), 'error');
    return null;
  }

  try {
    const saved = await invoke('save_voucher', { input: draft });
    state.busy = false;
    loadVoucher(saved);

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

  state.busy = true;
  try {
    const voucher = await invoke('get_voucher', { number: target });
    state.busy = false;
    if (voucher) {
      loadVoucher(voucher);
      return;
    }
    await refreshNextNumber();
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

  const target = base - 1;
  state.busy = true;
  try {
    const voucher = await invoke('get_voucher', { number: target });
    state.busy = false;
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

  const base = currentNumber();
  const target = base + 1;
  state.busy = true;
  try {
    const voucher = await invoke('get_voucher', { number: target });
    if (voucher) {
      state.busy = false;
      loadVoucher(voucher);
      return;
    }
    await refreshNextNumber();
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
  try {
    await refreshNextNumber();
    startNew(state.nextNumber);
    toast('سند جدید آماده است.', 'info');
  } catch (error) {
    toast(`دریافت شماره سند جدید ناموفق بود: ${errorText(error)}`, 'error');
  }
}

async function copyVoucher() {
  if (state.busy) return;
  const draft = collectDraft();
  if (!draft.lines.length) {
    toast('سندی برای کپی وجود ندارد.', 'info');
    return;
  }

  try {
    await refreshNextNumber();
  } catch (error) {
    toast(errorText(error), 'error');
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
  const voucher = state.voucher;
  if (!voucher) {
    toast('ابتدا یک سند ذخیره‌شده را باز کنید.', 'info');
    return;
  }

  const ok = await confirmAction(
    `سند شماره ${vfFa(voucher.number)} برای همیشه حذف شود؟ این کار قابل بازگشت نیست.`,
    'حذف سند'
  );
  if (!ok) return;

  state.busy = true;
  try {
    await invoke('delete_voucher', { id: voucher.id });
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
  if (!q) return state.accounts.slice();
  return state.accounts.filter((account) =>
    normalizeSearch(account.code).includes(q) ||
    normalizeSearch(account.name).includes(q) ||
    normalizeSearch(levelLabel(account.level)).includes(q) ||
    normalizeSearch(account.parentName).includes(q) ||
    normalizeSearch(account.groupName).includes(q)
  );
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
    row.setAttribute('role', 'row');
    row.tabIndex = -1;
    row.dataset.pickerIndex = String(i);
    row.setAttribute('aria-selected', i === apIndex ? 'true' : 'false');
    row.title = account.isPostable
      ? `${account.code} — ${account.name}`
      : 'این حساب قابل ثبت نیست؛ حساب معین یا تفصیلی انتخاب کنید';

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

  if (!account.isPostable) {
    toast('این حساب قابل ثبت نیست؛ حساب معین یا تفصیلی انتخاب کنید.', 'error');
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
  if (backBtn) backBtn.addEventListener('click', () => closeAccountPicker(true));
  modal.addEventListener('click', (ev) => {
    if (ev.target.id === 'accountPickerModal') closeAccountPicker(true);
  });

  if (search) search.addEventListener('input', () => renderAccountPickerRows());

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
    chooseAccountPickerRow();
  });

  const selectBtn = $('#accountPickerSelect');
  if (selectBtn) selectBtn.addEventListener('click', chooseAccountPickerRow);

  const clearBtn = modal.querySelector('[data-picker-act="clear"]');
  if (clearBtn) {
    clearBtn.addEventListener('click', () => {
      if (search) search.value = '';
      renderAccountPickerRows();
      if (search) search.focus();
    });
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
        chooseAccountPickerRow();
        break;
      case 'F5':
        ev.preventDefault();
        if (search) search.focus();
        break;
      case 'F6':
        ev.preventDefault();
        if (search) search.value = '';
        renderAccountPickerRows();
        if (search) search.focus();
        break;
      default:
        break;
    }
  });
}

/* ── Confirmation dialog ────────────────────────────────────── */

let confirmResolve = null;

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
    text.textContent = message;
    okButton.textContent = confirmLabel;
    modal.hidden = false;
    const main = $('.voucher-window');
    if (main) main.inert = true;
    okButton.focus();
  });
}

function settleConfirm(value) {
  const modal = $('#vfConfirmModal');
  if (!modal || modal.hidden) return;
  modal.hidden = true;
  const main = $('.voucher-window');
  if (main) main.inert = false;
  const resolve = confirmResolve;
  confirmResolve = null;
  if (resolve) resolve(value);
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
  if (chars.length > 1000) {
    area.value = chars.slice(0, 1000).join('');
    chars = chars.slice(0, 1000);
  }
  output.textContent = `${vfFa(chars.length)}/۱۰۰۰`;
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

    const existing = collectDraft().lines;
    fillGrid([...existing, ...imported]);
    markDirty();
    toast(`${vfFa(imported.length)} ردیف از فایل اکسل به سند اضافه شد.`, 'ok');
  } catch (error) {
    state.busy = false;
    toast(errorText(error), 'error');
  }
}

async function exportVoucherToExcel() {
  if (state.busy) return;
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

  state.busy = true;
  try {
    await invoke('open_ledger_preview', { accountCode: account.code, from: '', to: '' });
    state.busy = false;
  } catch (error) {
    state.busy = false;
    toast(errorText(error), 'error');
  }
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

    if (ev.key === 'Escape') {
      // Escape only backs out of the account picker. The OS close button
      // closes the window; a stray Escape must not throw the entry away.
      if (accountPickerOpen()) {
        ev.preventDefault();
        closeAccountPicker(true);
      }
      return;
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
  if (settings) state.settings = settings;
}

async function refreshNextNumber() {
  if (!invoke) return;
  const value = await invoke('next_voucher_number');
  state.nextNumber = Number(value) > 0 ? Number(value) : 1;
}

async function bindVoucherRequests() {
  if (!TAURI || !TAURI.event || typeof TAURI.event.listen !== 'function') return;
  try {
    await TAURI.event.listen('voucher-request', async (event) => {
      if (state.busy) return;
      if (!(await guardUnsaved())) return;
      const number = event && event.payload;

      if (number && Number(number) > 0) {
        try {
          const voucher = await invoke('get_voucher', { number: Number(number) });
          if (voucher) loadVoucher(voucher);
          else {
            await refreshNextNumber();
            startNew(Number(number));
          }
        } catch (error) {
          toast(errorText(error), 'error');
        }
      } else {
        await refreshNextNumber();
        startNew(state.nextNumber);
      }

      if (appWindow) {
        try {
          await appWindow.setFocus();
        } catch {
          // Focus is best-effort.
        }
      }
    });
  } catch {
    // Events are an optimisation; opening the window still loads a voucher.
  }
}

/* ── Init ───────────────────────────────────────────────────── */

async function init() {
  bindVoucherGrid();
  bindAccountPicker();
  bindConfirm();
  bindDocMeta();
  bindVoucherSearch();
  bindToolbar();
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
    await Promise.all([loadSettings(), loadAccounts(), refreshNextNumber()]);
    const requested = await invoke('take_voucher_request');
    state.busy = false;
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

    await bindVoucherRequests();

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
