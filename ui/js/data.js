'use strict';

/* ── Data loading ───────────────────────────────────────────── */

async function ensureAccounts(force = false) {
  if (!state.accountsLoaded || force) {
    const payload = await invoke('list_accounts');
    state.accounts = Array.isArray(payload?.accounts) ? payload.accounts : [];
    state.databasePath = payload?.databasePath || state.databasePath;
    state.accountsLoaded = true;
  }
  return state.accounts;
}

async function ensureGroups(force = false) {
  if (!state.groupsLoaded || force) {
    const payload = await invoke('list_account_groups');
    state.groups = Array.isArray(payload?.groups) ? payload.groups : [];
    state.databasePath = payload?.databasePath || state.databasePath;
    state.groupsLoaded = true;
  }
  return state.groups;
}

async function loadSettings() {
  const settings = await invoke('get_settings');
  state.settings = settings;
  applyLimits(settings?.limits);
  updateBranding(settings);
  return settings;
}

function applyLimits(limits) {
  if (!limits) return;
  const voucherPage = Number(limits.voucherPageSize);
  if (Number.isSafeInteger(voucherPage) && voucherPage > 0) VOUCHER_PAGE_SIZE = voucherPage;
  const journalPage = Number(limits.journalPageSize);
  if (Number.isSafeInteger(journalPage) && journalPage > 0) JOURNAL_PAGE_SIZE = journalPage;
}

// Loads a page and, when the current offset no longer exists (for example
// after deleting the last voucher of the last page), retries the last page
// instead of showing an empty list.
async function loadPagedList(kind, step, fetchPage, totalOf, rowsOf) {
  let report = await fetchPage(state.listOffsets[kind] || 0);
  const total = Number(totalOf(report) || 0);
  const shown = rowsOf(report).length;
  if (!total) {
    state.listOffsets[kind] = 0;
    return report;
  }
  if ((state.listOffsets[kind] || 0) > 0 && shown === 0) {
    state.listOffsets[kind] = Math.max(0, Math.floor((total - 1) / step) * step);
    report = await fetchPage(state.listOffsets[kind]);
  }
  return report;
}

function updateBranding(settings) {
  if (!settings) return;
  const company = settings.companyName || 'واحد اقتصادی';
  const selected = state.activeFiscalYear?.title || settings.fiscalYear || '—';
  // Fiscal-year titles often already read «سال مالی ۱۴۰۵»; don't repeat the label.
  const fiscal = String(selected).replace(/^سال مالی\s*/, '') || '—';
  const today = faDate(settings.today || '');
  const statusCompany = $('#statusCompany');
  const statusFiscal = $('#statusFiscal');
  const statusDate = $('#statusDate');
  if (statusCompany) statusCompany.textContent = `شرکت: ${company}`;
  if (statusFiscal) statusFiscal.textContent = `سال مالی: ${toFa(fiscal)}`;
  if (statusDate) statusDate.textContent = `تاریخ: ${today}`;
}

/* ── Page actions ───────────────────────────────────────────── */

function renderActions(actions) {
  const bar = $('#pageActions');
  if (!bar) return;
  bar.innerHTML = '';
  (actions || []).forEach((action) => {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = `btn${action.primary ? ' btn-primary' : ''}`;
    if (action.title) button.title = action.title;
    if (action.icon) button.innerHTML = icon(action.icon);
    const label = document.createElement('span');
    label.textContent = action.label;
    button.append(label);
    button.addEventListener('click', () => action.onClick?.());
    bar.append(button);
  });
}

