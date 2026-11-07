'use strict';

/* ── Statement of changes in equity ─────────────────────────── */

PAGES.equity = {
  actions: () => [
    {
      label: 'چاپ گزارش',
      icon: 'i-print',
      onClick: () => printWithFeedback('open_equity_changes_preview', {
        asOf: state.filters.equity.asOf,
        scope: normalizeReportScope(state.filters.equity.scope),
      }),
    },
    { label: 'به‌روزرسانی', icon: 'i-refresh', onClick: () => navigate('equity', { force: true }) },
  ],
  load: async () => {
    const f = state.filters.equity;
    return invoke('get_equity_changes_statement', { asOf: f.asOf, scope: normalizeReportScope(f.scope) });
  },
  render: renderEquityChanges,
  bind: bindFilterOnly,
};

function equityFilterBar() {
  const f = state.filters.equity;
  return `
  <form class="filter-bar" data-filter="equity">
    <div class="field">
      <label for="eqDate">تا تاریخ</label>
      <input id="eqDate" class="input" name="asOf" data-date type="text" inputmode="numeric" value="${esc(f.asOf)}" placeholder="۱۴۰۵/۱۲/۲۹" />
    </div>
    ${reportScopeField('equity', 'eqScope')}
    <button class="btn btn-primary" type="submit">${icon('i-search')}<span>محاسبه صورت</span></button>
    <button class="btn" type="button" data-reset>${icon('i-refresh')}<span>بازنشانی</span></button>
  </form>`;
}

function renderEquityChanges(report) {
  const rows = report.rows || [];
  const integrity = report.integrity || { ok: true, counts: { error: 0, warning: 0, info: 0 }, issues: [] };
  const errorCount = Number(integrity.counts?.error || 0);
  const body = rows.map((row) => {
    const accounts = (row.accounts || []).length
      ? `<div class="muted small">${row.accounts.map((account) => `${esc(toFa(account.accountCode))} ${esc(account.accountName)}: ${money(account.amount)}`).join('، ')}</div>`
      : '';
    const isTotal = row.kind === 'total';
    const isProfit = row.kind === 'profit';
    return `<tr class="${isTotal ? 'row-total' : ''}${isProfit ? ' row-highlight' : ''}">
      <td>${esc(row.title)}${accounts}</td>
      <td class="money">${money(row.opening)}</td>
      <td class="money ${amountBigInt(row.change) >= 0n ? 'debit' : 'credit'}">${money(row.change)}</td>
      <td class="money ${amountBigInt(row.closing) >= 0n ? 'debit' : 'credit'}">${money(row.closing)}</td>
    </tr>`;
  }).join('');
  return `
  ${equityFilterBar()}
  <div class="report-summary">
    <div class="kpi"><span>حقوق مالکانه ابتدای دوره</span><strong>${money(report.totals.opening)}</strong></div>
    <div class="kpi"><span>تغییر دوره</span><strong>${money(report.totals.change)}</strong></div>
    <div class="kpi" data-tone="ok"><span>حقوق مالکانه پایان دوره</span><strong>${money(report.totals.closing)}</strong></div>
    <div class="kpi"><span>سود (زیان) خالص دوره</span><strong>${money(report.totals.netIncome)}</strong></div>
    <div class="kpi" data-tone="${integrity.ok ? 'ok' : 'danger'}"><span>کنترل سلامت</span><strong>${integrity.ok ? 'بدون خطا' : `${toFa(errorCount)} خطا`}</strong></div>
  </div>
  <div class="card">
    <div class="section-head">
      <h2>${icon('i-balance')} تغییرات حقوق مالکانه</h2>
      <span class="badge" data-tone="info">${toFa(rows.length)} ردیف</span>
    </div>
    <div class="note">مانده ابتدایی از آخرین سند افتتاحیه تا تاریخ گزارش گرفته می‌شود؛ اسناد افتتاحیهٔ قبلی دوباره جمع نمی‌شوند.</div>
    <div class="table-wrap">
      <table class="data-table">
        <thead><tr><th>شرح</th><th style="width:150px">ابتدای دوره</th><th style="width:150px">تغییر دوره</th><th style="width:150px">پایان دوره</th></tr></thead>
        <tbody>${body || '<tr><td colspan="4" class="center muted">موردی ثبت نشده است.</td></tr>'}</tbody>
      </table>
    </div>
  </div>
  ${integrityPanel(integrity)}`;
}
