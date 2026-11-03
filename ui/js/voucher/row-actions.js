'use strict';

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

