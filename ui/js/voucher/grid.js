'use strict';

/* ── Voucher grid ───────────────────────────────────────────── */

function voucherRowTemplate() {
  return `
    <tr>
      <td class="vf-rowno"></td>
      <td><input class="vf-code-input" type="text" dir="ltr" inputmode="numeric" autocomplete="off" aria-label="کد حساب پایه" title="برای انتخاب حساب Space یا F4 را بزنید" /></td>
      <td class="vf-detail1"><input class="vf-detail1-input" type="text" dir="rtl" readonly autocomplete="off" aria-label="تفصیلی ۱" title="برای انتخاب تفصیلی Space یا F4 را بزنید" /></td>
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

function focusFirstDocumentRow() {
  const row = gridRows().find((item) => item.isConnected);
  if (!row) return;
  state.activeRow = row;
  row.querySelector('.vf-code-input')?.focus();
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
      debit += parseAmountInput(row.querySelector('.vf-debit-input')?.value || '') ?? 0n;
      credit += parseAmountInput(row.querySelector('.vf-credit-input')?.value || '') ?? 0n;
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
  const detailInput = row.querySelector('.vf-detail1-input');
  let detailCode = digitsOnly(detailInput?.value || '');
  let detail = detailCode ? accountForCode(detailCode) : null;

  // A saved row keeps its stable id only while the complete base/detail pair
  // remains unchanged. A changed pair is treated as a new posting.
  if (detailCode && (!detail || Number(detail.level) !== 3
      || digitsOnly(detail.parentCode) !== code || Number(account?.level) !== 2)) {
    if (detailInput) detailInput.value = '';
    detailCode = '';
    detail = null;
  }
  if (row.dataset.lineId && (row.dataset.savedAccount !== code
      || digitsOnly(row.dataset.savedDetail1 || '') !== detailCode)) {
    delete row.dataset.lineId;
    delete row.dataset.savedAccount;
    delete row.dataset.savedDetail1;
  }

  if (input) {
    input.title = account
      ? `${account.code} — ${account.name}${accountAllowedForNewLine(account, row) ? '' : !accountChainActive(account) ? ' (غیرفعال)' : ' (قابل ثبت نیست)'}`
      : 'برای انتخاب حساب پایه Space یا F4 را بزنید';
    if (accountAllowedForNewLine(account, row)) input.classList.remove('is-invalid');
    else if (!code) input.classList.remove('is-invalid');
    else if (final) input.classList.add('is-invalid');
  }
  if (detailInput) {
    const validDetail = !!detail && (accountChainActive(detail) ||
      (row.dataset.lineId && row.dataset.savedDetail1 === detailCode));
    detailInput.title = detail
      ? `${detail.code} — ${detail.name}${validDetail ? '' : ' (غیرفعال یا نامعتبر)'}`
      : account && Number(account.level) === 2
        ? 'برای انتخاب تفصیلی سطح ۳ از Space یا F4 استفاده کنید؛ انتخاب تفصیلی اختیاری است.'
        : 'تفصیلی فقط برای حساب معین قابل انتخاب است.';
    detailInput.classList.toggle('is-invalid', !!detailCode && !validDetail);
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
  const selectedDetailCode = digitsOnly(row.querySelector('.vf-detail1-input')?.value || '');
  const selectedDetail = selectedDetailCode ? accountForCode(selectedDetailCode) : null;
  const detailLabel = selectedDetail ? `${vfFa(selectedDetail.code)} — ${selectedDetail.name}` : '—';
  // Row 1 mirrors the reference «کل / معین» field (class and subsidiary);
  // row 2 is the level-3 detail. The third row is decorative in the
  // reference and stays empty here as well.
  const headings = [renderLevel([1, 2]), detailLabel];
  values.forEach((el, index) => {
    const value = headings[index] || '';
    el.textContent = value;
    if (value) el.title = value;
  });
}

function refreshFormState() {
  renumberRows();
  const locked = isLockedVoucher();
  $('.voucher-form')?.classList.toggle('is-locked', locked);
  gridRows().forEach((row) => {
    row.querySelectorAll('.vf-code-input, .vf-detail1-input, .vf-desc-input, .vf-debit-input, .vf-credit-input').forEach((input) => {
      input.readOnly = locked;
      input.setAttribute('aria-readonly', locked ? 'true' : 'false');
    });
  });
  ['#vfDocNo', '#vfDocDate', '#vfDocDesc'].forEach((selector) => {
    const input = $(selector);
    if (input) {
      input.readOnly = locked;
      input.setAttribute('aria-readonly', locked ? 'true' : 'false');
    }
  });
  const docType = $('#vfDocType');
  if (docType) docType.disabled = locked;
  const permanentBtn = $('#vfPermanentBtn');
  const changesBtn = $('#vfChangesBtn');
  if (permanentBtn) {
    permanentBtn.classList.toggle('is-active', locked || Boolean(state.makePermanent));
    permanentBtn.disabled = locked || state.busy;
    permanentBtn.title = locked
      ? 'این سند دائمی است و قابل تغییر نیست.'
      : 'پس از ذخیره، سند دائمی می‌شود و دیگر قابل ویرایش یا حذف نخواهد بود.';
  }
  if (changesBtn) {
    changesBtn.classList.toggle('is-active', !locked && !state.makePermanent);
    changesBtn.disabled = locked || state.busy;
    changesBtn.title = locked
      ? 'این سند دائمی است و قابل تغییر نیست.'
      : 'سند موقت بماند و پس از ذخیره قابل ویرایش باشد.';
  }
  const lockedBanner = $('#vfLockedBanner');
  if (lockedBanner) lockedBanner.hidden = !locked;
  const mutating = ['save', 'add-row', 'repeat-row', 'delete-row', 'transfer-document', 'transfer-move', 'excel-import', 'delete-voucher'];
  $$('[data-action]').forEach((button) => {
    if (mutating.includes(button.dataset.action)) button.disabled = locked || state.busy;
  });
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

