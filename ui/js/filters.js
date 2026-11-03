'use strict';

/* ── Date and filter defaults ───────────────────────────────── */

function currentYear() {
  const today = state.settings?.today || '';
  const year = toEn(today).slice(0, 4);
  return /^\d{4}$/.test(year) ? year : '';
}

function defaultFilters() {
  const today = state.settings?.today || '';
  const year = currentYear();
  const from = year ? `${year}/01/01` : '';
  return {
    vouchers: { from, to: today, search: '' },
    journal: { from, to: today, search: '' },
    ledger: { accountCode: '', from, to: today },
    trial: {
      from,
      to: today,
      accountFrom: '',
      accountTo: '',
      showZero: true,
      multiLevel: false,
      balanceType: 'شش ستونی',
      showAccountCode: true,
      showAccountName: true,
      showPrintDateTime: true,
    },
    income: { from, to: today },
    balance: { asOf: today },
  };
}

function filterValue(key, name) {
  const group = state.filters[key] || {};
  return group[name] ?? '';
}

