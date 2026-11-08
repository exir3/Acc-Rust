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
  const active = state.activeFiscalYear;
  const start = active?.startDate || (year ? `${year}/01/01` : '');
  let to = today;
  if (active) {
    const current = toEn(today || '');
    const startValue = toEn(active.startDate || '');
    const endValue = toEn(active.endDate || '');
    if (!current || current < startValue || current > endValue) to = active.endDate || today;
  }
  const from = start || to;
  return {
    vouchers: { from, to, search: '' },
    journal: { from, to, search: '', scope: 'official' },
    ledger: { accountCode: '', from, to, scope: 'official' },
    trial: {
      from,
      to,
      scope: 'official',
      accountFrom: '',
      accountTo: '',
      showZero: true,
      multiLevel: false,
      balanceType: 'شش ستونی',
      showAccountCode: true,
      showAccountName: true,
      showPrintDateTime: true,
    },
    income: { from, to, scope: 'official' },
    balance: { asOf: to, scope: 'official' },
    equity: { asOf: to, scope: 'official' },
    cashflow: { asOf: to, scope: 'official' },
    cogs: { from, to, scope: 'official' },
    appropriation: { asOf: to, scope: 'official' },
  };
}

/* ── Report scope (official / working / temporary) ─────────── */

const REPORT_SCOPES = [
  { value: 'official', label: 'رسمی', hint: 'فقط اسناد دائمی' },
  { value: 'working', label: 'کاری', hint: 'اسناد دائمی و موقت' },
  { value: 'temporary', label: 'فقط موقت', hint: 'فقط اسناد موقت' },
];

function normalizeReportScope(value) {
  const text = String(value ?? '').trim();
  return REPORT_SCOPES.some((scope) => scope.value === text) ? text : 'official';
}

function reportScopeField(filterKey, id, label = 'محدوده اسناد') {
  const group = state.filters[filterKey] || {};
  const selected = normalizeReportScope(group.scope);
  const options = REPORT_SCOPES.map((scope) => `<option value="${scope.value}" title="${esc(scope.hint)}"${scope.value === selected ? ' selected' : ''}>${esc(scope.label)}</option>`).join('');
  return `<div class="field">
      <label for="${id}">${label}</label>
      <select id="${id}" class="input" name="scope">${options}</select>
    </div>`;
}

function filterValue(key, name) {
  const group = state.filters[key] || {};
  return group[name] ?? '';
}

