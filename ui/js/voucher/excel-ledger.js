'use strict';

/* ── Excel and account ledger ───────────────────────────────── */

async function importVoucherFromExcel() {
  if (isLockedVoucher()) {
    toast('سند دائمی است و ورود ردیف جدید در آن مجاز نیست.', 'error');
    return;
  }
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
      scope: 'working',
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
    await invoke('open_ledger_preview', { accountCode: account.code, from, to, scope: 'working' });
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


