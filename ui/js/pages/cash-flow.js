'use strict';

/* ── Statement of cash flows (indirect method) ──────────────── */

PAGES.cashflow = {
  actions: () => [
    {
      label: 'چاپ گزارش',
      icon: 'i-print',
      onClick: () => printWithFeedback('open_cash_flow_preview', {
        asOf: state.filters.cashflow.asOf,
        scope: normalizeReportScope(state.filters.cashflow.scope),
      }),
    },
    { label: 'به‌روزرسانی', icon: 'i-refresh', onClick: () => navigate('cashflow', { force: true }) },
  ],
  load: async () => {
    const f = state.filters.cashflow;
    return invoke('get_cash_flow_statement', { asOf: f.asOf, scope: normalizeReportScope(f.scope) });
  },
  render: renderCashFlow,
  bind: bindFilterOnly,
};

function cashFlowFilterBar() {
  const f = state.filters.cashflow;
  return `
  <form class="filter-bar" data-filter="cashflow">
    <div class="field">
      <label for="cfDate">تا تاریخ</label>
      <input id="cfDate" class="input" name="asOf" data-date type="text" inputmode="numeric" value="${esc(f.asOf)}" placeholder="۱۴۰۵/۱۲/۲۹" />
    </div>
    ${reportScopeField('cashflow', 'cfScope')}
    <button class="btn btn-primary" type="submit">${icon('i-search')}<span>محاسبه صورت</span></button>
    <button class="btn" type="button" data-reset>${icon('i-refresh')}<span>بازنشانی</span></button>
  </form>`;
}

function cashFlowSectionCard(section) {
  const lines = (section.lines || []).map((line) => `
    <tr>
      <td>${esc(line.label)}${line.note ? `<div class="muted small">${esc(line.note)}</div>` : ''}</td>
      <td class="money ${amountBigInt(line.amount) >= 0n ? 'debit' : 'credit'}">${money(line.amount)}</td>
    </tr>`).join('');
  return `<div class="card">
    <div class="section-head">
      <h2>${icon('i-trend')} ${esc(section.title)}</h2>
      <span class="badge" data-tone="info">${money(section.total)}</span>
    </div>
    <div class="table-wrap">
      <table class="data-table">
        <thead><tr><th>شرح</th><th style="width:170px">مبلغ</th></tr></thead>
        <tbody>${lines || '<tr><td colspan="2" class="center muted">جریان نقدی در این بخش ثبت نشده است.</td></tr>'}</tbody>
        <tfoot><tr><td class="bold">جمع ${esc(section.title)}</td><td class="num bold">${money(section.total)}</td></tr></tfoot>
      </table>
    </div>
  </div>`;
}

function renderCashFlow(report) {
  const sections = report.sections || [];
  const integrity = report.integrity || { ok: true, counts: { error: 0, warning: 0, info: 0 }, issues: [] };
  const errorCount = Number(integrity.counts?.error || 0);
  const unresolved = report.unresolvedVouchers || [];
  const variance = amountBigInt(report.variance);
  const unresolvedRows = unresolved.map((voucher) => `
    <tr>
      <td class="center mono">${toFa(voucher.voucherNumber)}</td>
      <td class="money ${amountBigInt(voucher.cashMovement) >= 0n ? 'debit' : 'credit'}">${money(voucher.cashMovement)}</td>
      <td>${esc(voucher.reason)}</td>
    </tr>`).join('');
  return `
  ${cashFlowFilterBar()}
  <div class="report-summary">
    <div class="kpi"><span>موجودی نقد ابتدای دوره</span><strong>${money(report.openingCash)}</strong></div>
    <div class="kpi" data-tone="${amountBigInt(report.netChange) >= 0n ? 'ok' : 'danger'}"><span>تغییر خالص نقد</span><strong>${money(report.netChange)}</strong></div>
    <div class="kpi"><span>موجودی نقد پایان دوره</span><strong>${money(report.closingCash)}</strong></div>
    <div class="kpi" data-tone="${variance === 0n ? 'ok' : 'danger'}"><span>مغایرت تطبیق</span><strong>${money(report.variance)}</strong></div>
    <div class="kpi" data-tone="${report.classificationComplete ? 'ok' : 'warn'}"><span>طبقه‌بندی جریان نقد</span><strong>${report.classificationComplete ? 'کامل' : 'نیازمند بررسی'}</strong></div>
    <div class="kpi" data-tone="${integrity.ok ? 'ok' : 'danger'}"><span>کنترل سلامت</span><strong>${integrity.ok ? 'بدون خطا' : `${toFa(errorCount)} خطا`}</strong></div>
  </div>
  <div class="report-layout">
    ${sections.map(cashFlowSectionCard).join('')}
  </div>
  <div class="card" style="margin-top:12px">
    <div class="section-head">
      <h2>${icon('i-bank')} تطبیق موجودی نقد</h2>
      ${report.classificationComplete ? badge('تطبیق کامل', 'ok') : badge('تطبیق ناقص', 'warn')}
    </div>
    <div class="report-total-line"><span>موجودی نقد ابتدای دوره</span><span class="money">${money(report.openingCash)}</span></div>
    <div class="report-total-line"><span>تغییر خالص دوره</span><span class="money">${money(report.netChange)}</span></div>
    <div class="report-total-line net"><span>موجودی نقد پایان دوره</span><span class="money">${money(report.closingCash)}</span></div>
    ${variance !== 0n ? `<div class="note" data-tone="danger">جمع بخش‌های جریان نقد با تغییر واقعی موجودی نقد به اندازه ${money(report.variance)} مغایرت دارد.</div>` : ''}
  </div>
  ${unresolved.length ? `<div class="card" style="margin-top:12px">
    <div class="section-head">
      <h2>${icon('i-alert')} اسناد دارای گردش نقد طبقه‌بندی‌نشده</h2>
      ${badge(`${toFa(unresolved.length)} سند`, 'warn')}
    </div>
    <div class="note" data-tone="warn">مبلغ این اسناد به هیچ فعالیتی نسبت داده نشده است؛ تا تعیین سرفصل طرف حساب، صورت جریان نقد کامل نیست.</div>
    <div class="table-wrap short">
      <table class="data-table">
        <thead><tr><th style="width:100px">شماره سند</th><th style="width:150px">گردش نقد</th><th>علت</th></tr></thead>
        <tbody>${unresolvedRows}</tbody>
      </table>
    </div>
  </div>` : ''}
  ${integrityPanel(integrity)}`;
}
