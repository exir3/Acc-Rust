'use strict';

/* ── Row toolbar actions ────────────────────────────────────── */

function addRow() {
  if (isLockedVoucher()) { toast('سند دائمی است و ردیف‌های آن قابل تغییر نیستند.', 'error'); return; }
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

function repeatRow() {
  if (isLockedVoucher()) { toast('سند دائمی است و ردیف‌های آن قابل تغییر نیستند.', 'error'); return; }
  if (gridRows().filter(rowHasValue).length >= MAX_VOUCHER_LINES) {
    toast(`حداکثر ${vfFa(MAX_VOUCHER_LINES)} ردیف برای هر سند مجاز است.`, 'warning');
    return;
  }
  const row = state.activeRow && state.activeRow.isConnected ? state.activeRow : null;
  if (!row) {
    toast('ابتدا ردیفی را انتخاب کنید.', 'info');
    return;
  }
  const above = row.previousElementSibling;
  if (!above || !rowHasValue(above)) {
    toast('ردیف بالایی وجود ندارد.', 'warning');
    return;
  }

  // Mirror the reference «تکرار سطر بالا»: copy account, detail and
  // description from the row above into a new row placed above the active
  // row. Amounts are deliberately not copied so a repeat never unbalances
  // the voucher on its own.
  const source = {
    code: above.querySelector('.vf-code-input').value,
    detail1: above.querySelector('.vf-detail1-input').value,
    description: above.querySelector('.vf-desc-input').value,
  };

  row.insertAdjacentHTML('beforebegin', voucherRowTemplate());
  const copy = row.previousElementSibling;
  copy.querySelector('.vf-code-input').value = source.code;
  copy.querySelector('.vf-detail1-input').value = source.detail1;
  copy.querySelector('.vf-desc-input').value = source.description;
  updateRowAccount(copy);
  markDirty();

  state.activeRow = copy;
  ensureTrailingBlank();
  syncVoucherRowCount();
  refreshFormState();
  copy.querySelector('.vf-debit-input')?.focus();
}

function deleteRow() {
  if (isLockedVoucher()) { toast('سند دائمی است و ردیف‌های آن قابل تغییر نیستند.', 'error'); return; }
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

