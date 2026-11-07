'use strict';

/* ── Statement of profit appropriation ──────────────────────── */

PAGES.appropriation = {
  actions: () => [
    {
      label: 'چاپ گزارش',
      icon: 'i-print',
      onClick: () => printWithFeedback('open_profit_appropriation_preview', {
        asOf: state.filters.appropriation.asOf,
        scope: normalizeReportScope(state.filters.appropriation.scope),
      }),
    },
    { label: 'به‌روزرسانی', icon: 'i-refresh', onClick: () => navigate('appropriation', { force: true }) },
  ],
  load: async () => {
    const f = state.filters.appropriation;
    return invoke('get_profit_appropriation_statement', { asOf: f.asOf, scope: normalizeReportScope(f.scope) });
  },
  render: renderAppropriation,
  bind: bindFilterOnly,
};

function appropriationFilterBar() {
  const f = state.filters.appropriation;
  return `
  <form class="filter-bar" data-filter="appropriation">
    <div class="field">
      <label for="apDate">تا تاریخ</label>
      <input id="apDate" class="input" name="asOf" data-date type="text" inputmode="numeric" value="${esc(f.asOf)}" placeholder="۱۴۰۵/۱۲/۲۹" />
    </div>
    ${reportScopeField('appropriation', 'apScope')}
    <button class="btn btn-primary" type="submit">${icon('i-search')}<span>محاسبه صورت</span></button>
    <button class="btn" type="button" data-reset>${icon('i-refresh')}<span>بازنشانی</span></button>
  </form>`;
}

function renderAppropriation(report) {
  const rows = report.rows || [];
  const integrity = report.integrity || { ok: true, counts: { error: 0, warning: 0, info: 0 }, issues: [] };
  const errorCount = Number(integrity.counts?.error || 0);
  const body = rows.map((row) => {
    const rowClass = row.kind === 'total' || row.kind === 'retained' ? ' class="row-total"' : '';
    const accounts = (row.accounts || []).map((account) => `
      <tr class="row-highlight">
        <td>حساب ${esc(toFa(account.accountCode))} — ${esc(account.accountName)}</td>
        <td class="money">${money(account.amount)}</td>
      </tr>`).join('');
    return `<tr${rowClass}>
      <td>${esc(row.label)}</td>
      <td class="money ${amountBigInt(row.amount) >= 0n ? 'debit' : 'credit'}">${money(row.amount)}</td>
    </tr>${accounts}`;
  }).join('');
  return `
  ${appropriationFilterBar()}
  <div class="report-summary">
    <div class="kpi"><span>سود (زیان) خالص دوره</span><strong>${money(report.totals.net)}</strong></div>
    <div class="kpi"><span>جمع تخصیص‌های شناسایی‌شده</span><strong>${money(report.totals.appropriated)}</strong></div>
    <div class="kpi" data-tone="${amountBigInt(report.totals.retained) >= 0n ? 'ok' : 'danger'}"><span>ماندهٔ محاسباتی پس از تخصیص</span><strong>${money(report.totals.retained)}</strong></div>
    <div class="kpi" data-tone="${integrity.ok ? 'ok' : 'danger'}"><span>کنترل سلامت</span><strong>${integrity.ok ? 'بدون خطا' : `${toFa(errorCount)} خطا`}</strong></div>
  </div>
  <div class="card">
    <div class="section-head">
      <h2>${icon('i-coins')} صورت تخصیص سود</h2>
      <span class="badge" data-tone="info">${toFa(rows.length)} ردیف</span>
    </div>
    <div class="note" data-tone="warn">فقط انتقال‌های مستقل و متوازن از سود انباشته به اندوخته یا سود سهام شناسایی می‌شوند؛ پرداخت سود، آورده و ثبت‌های مرکب در مبلغ تخصیص وارد نمی‌شوند.</div>
    <div class="table-wrap">
      <table class="data-table">
        <thead><tr><th>شرح</th><th style="width:170px">مبلغ</th></tr></thead>
        <tbody>${body || '<tr><td colspan="2" class="center muted">موردی ثبت نشده است.</td></tr>'}</tbody>
      </table>
    </div>
  </div>
  ${integrityPanel(integrity)}`;
}
