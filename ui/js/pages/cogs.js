'use strict';

/* ── Statement of cost of goods sold ────────────────────────── */

PAGES.cogs = {
  actions: () => [
    {
      label: 'چاپ گزارش',
      icon: 'i-print',
      onClick: () => printWithFeedback('open_cogs_preview', {
        from: state.filters.cogs.from,
        to: state.filters.cogs.to,
        scope: normalizeReportScope(state.filters.cogs.scope),
      }),
    },
    { label: 'به‌روزرسانی', icon: 'i-refresh', onClick: () => navigate('cogs', { force: true }) },
  ],
  load: async () => {
    const f = state.filters.cogs;
    return invoke('get_cogs_statement', { from: f.from, to: f.to, scope: normalizeReportScope(f.scope) });
  },
  render: renderCogs,
  bind: bindFilterOnly,
};

function cogsFilterBar() {
  const f = state.filters.cogs;
  return `
  <form class="filter-bar" data-filter="cogs">
    <div class="field">
      <label for="cogsFrom">از تاریخ</label>
      <input id="cogsFrom" class="input" name="from" data-date type="text" inputmode="numeric" value="${esc(f.from)}" placeholder="۱۴۰۵/۰۱/۰۱" />
    </div>
    <div class="field">
      <label for="cogsTo">تا تاریخ</label>
      <input id="cogsTo" class="input" name="to" data-date type="text" inputmode="numeric" value="${esc(f.to)}" placeholder="۱۴۰۵/۱۲/۲۹" />
    </div>
    ${reportScopeField('cogs', 'cogsScope')}
    <button class="btn btn-primary" type="submit">${icon('i-search')}<span>محاسبه صورت</span></button>
    <button class="btn" type="button" data-reset>${icon('i-refresh')}<span>بازنشانی</span></button>
  </form>`;
}

function renderCogs(report) {
  const rows = (report.rows || []).filter((row) => row.kind !== 'total');
  const integrity = report.integrity || { ok: true, counts: { error: 0, warning: 0, info: 0 }, issues: [] };
  const errorCount = Number(integrity.counts?.error || 0);
  const inventoryChange = amountBigInt(report.totals.closingInventory) - amountBigInt(report.totals.openingInventory);
  const body = rows.map((row) => `
    <tr class="${row.kind === 'inventory' ? 'row-highlight' : ''}">
      <td class="center mono">${row.code ? esc(toFa(row.code)) : '—'}</td>
      <td>${esc(row.label)}${row.note ? `<div class="muted small">${esc(row.note)}</div>` : ''}</td>
      <td class="money ${amountBigInt(row.amount) >= 0n ? 'debit' : 'credit'}">${money(row.amount)}</td>
    </tr>`).join('');
  return `
  ${cogsFilterBar()}
  <div class="report-summary">
    <div class="kpi"><span>بهای تمام‌شدهٔ ثبت‌شده</span><strong>${money(report.totals.costs)}</strong></div>
    <div class="kpi"><span>موجودی ابتدای دوره</span><strong>${money(report.totals.openingInventory)}</strong></div>
    <div class="kpi"><span>موجودی پایان دوره</span><strong>${money(report.totals.closingInventory)}</strong></div>
    <div class="kpi" data-tone="${inventoryChange >= 0n ? 'info' : 'warn'}"><span>تغییر موجودی</span><strong>${money(inventoryChange)}</strong></div>
    <div class="kpi" data-tone="${integrity.ok ? 'ok' : 'danger'}"><span>کنترل سلامت</span><strong>${integrity.ok ? 'بدون خطا' : `${toFa(errorCount)} خطا`}</strong></div>
  </div>
  <div class="card">
    <div class="section-head">
      <h2>${icon('i-coins')} صورت بهای تمام‌شده کالای فروش‌رفته</h2>
      <span class="badge" data-tone="info">${report.detailed ? 'تفکیک‌شده' : 'جمع ثبت‌شده'}</span>
    </div>
    <div class="note" data-tone="warn">${esc(report.limitation)}</div>
    <div class="table-wrap">
      <table class="data-table">
        <thead><tr><th style="width:96px">کد</th><th>شرح</th><th style="width:150px">مبلغ</th></tr></thead>
        <tbody>${body || '<tr><td colspan="3" class="center muted">موردی ثبت نشده است.</td></tr>'}</tbody>
        <tfoot><tr><td colspan="2" class="bold">بهای تمام‌شدهٔ کالای فروش‌رفته</td><td class="num bold">${money(report.totals.cogs)}</td></tr></tfoot>
      </table>
    </div>
    <div class="note">تغییر موجودی فقط اطلاعات مکمل است و در جمع هزینهٔ فروش دوباره اعمال نمی‌شود.</div>
  </div>
  ${integrityPanel(integrity)}`;
}
