'use strict';

/* ── Balance sheet ──────────────────────────────────────────── */

PAGES.balance = {
  actions: () => [
    {
      label: 'چاپ گزارش',
      icon: 'i-print',
      onClick: () => {
        const asOf = state.filters.balance.asOf;
        if (!asOf || !isValidJalaliDate(asOf)) {
          toast('تاریخ ترازنامه معتبر نیست؛ آن را اصلاح کنید.', 'error');
          return;
        }
        printWithFeedback('open_balance_sheet_preview', { asOf, scope: normalizeReportScope(state.filters.balance.scope) });
      },
    },
    { label: 'به‌روزرسانی', icon: 'i-refresh', onClick: () => navigate('balance', { force: true }) },
  ],
  load: async () => {
    const f = state.filters.balance;
    return invoke('get_balance_sheet', { asOf: f.asOf, scope: normalizeReportScope(f.scope) });
  },
  render: renderBalanceSheet,
  bind: bindFilterOnly,
};

function balanceFilterBar() {
  const f = state.filters.balance;
  return `
  <form class="filter-bar" data-filter="balance">
    <div class="field">
      <label for="bsDate">تا تاریخ</label>
      <input id="bsDate" class="input" name="asOf" data-date type="text" inputmode="numeric" value="${esc(f.asOf)}" placeholder="۱۴۰۵/۱۲/۲۹" />
    </div>
    ${reportScopeField('balance', 'bsScope')}
    <button class="btn btn-primary" type="submit">${icon('i-search')}<span>محاسبه ترازنامه</span></button>
    <button class="btn" type="button" data-reset>${icon('i-refresh')}<span>بازنشانی</span></button>
  </form>`;
}

function renderBalanceSheet(report) {
  const equityRows = [...(report.equity || [])];
  equityRows.push({
    accountCode: '',
    accountName: 'سود (زیان) انباشته دوره',
    amount: report.profit,
  });
  const difference = amountBigInt(report.assetsTotal) - amountBigInt(report.totalLiabilitiesEquity);
  const unclassifiedDebit = report.unclassifiedDebit || [];
  const unclassifiedCredit = report.unclassifiedCredit || [];
  const integrity = report.integrity || { ok: true, counts: { error: 0, warning: 0, info: 0 }, issues: [] };
  const errorCount = Number(integrity.counts?.error || 0);
  const classificationNote = (unclassifiedDebit.length || unclassifiedCredit.length)
    ? `<div class="note" data-tone="warn">مانده‌های طبقه‌بندی‌نشده در ردیف جداگانه نمایش داده و در جمع دارایی‌ها/بدهی‌ها لحاظ شده‌اند؛ تا تعیین سرفصل، ترازنامه را تأییدشده تلقی نکنید.</div>`
    : '';
  return `
  ${balanceFilterBar()}
  ${classificationNote}
  <div class="report-summary">
    <div class="kpi"><span>جمع دارایی‌ها</span><strong>${money(report.assetsTotal)}</strong></div>
    <div class="kpi"><span>جمع بدهی‌ها</span><strong>${money(report.liabilitiesTotal)}</strong></div>
    <div class="kpi"><span>حقوق مالکانه</span><strong>${money(amountBigInt(report.equityTotal) + amountBigInt(report.profit))}</strong></div>
    <div class="kpi" data-tone="${report.balanced ? 'ok' : 'danger'}"><span>وضعیت ترازنامه</span><strong>${report.balanced ? 'متوازن' : `مغایرت ${money(difference)}`}</strong></div>
    <div class="kpi" data-tone="${integrity.ok ? 'ok' : 'danger'}"><span>کنترل سلامت</span><strong>${integrity.ok ? 'بدون خطا' : `${toFa(errorCount)} خطا`}</strong></div>
  </div>
  <div class="report-layout">
    <div class="card">
      <div class="section-head">
        <h2>${icon('i-balance')} دارایی‌ها</h2>
        <span class="badge" data-tone="info">${toFa((report.assets || []).length)} حساب</span>
        ${unclassifiedDebit.length ? badge(`${toFa(unclassifiedDebit.length)} طبقه‌بندی‌نشده`, 'warn') : ''}
      </div>
      ${balanceTable(report.assets, report.assetsTotal, 'جمع دارایی‌ها', unclassifiedDebit, 'طبقه‌بندی‌نشده (مانده بدهکار)')}
    </div>
    <div class="card">
      <div class="section-head">
        <h2>${icon('i-bank')} بدهی‌ها و حقوق مالکانه</h2>
        ${report.balanced ? badge('تراز است', 'ok') : badge('مغایرت دارد', 'danger')}
        ${unclassifiedCredit.length ? badge(`${toFa(unclassifiedCredit.length)} طبقه‌بندی‌نشده`, 'warn') : ''}
      </div>
      <h3 class="muted small" style="margin:2px 0 5px">بدهی‌ها</h3>
      ${balanceTable(report.liabilities, report.liabilitiesTotal, 'جمع بدهی‌ها', unclassifiedCredit, 'طبقه‌بندی‌نشده (مانده بستانکار)')}
      <h3 class="muted small" style="margin:14px 0 5px">حقوق مالکانه</h3>
      ${balanceTable(equityRows, amountBigInt(report.equityTotal) + amountBigInt(report.profit), 'جمع حقوق مالکانه')}
      <div class="report-total-line net" style="margin-top:12px">
        <span>جمع بدهی‌ها و حقوق مالکانه</span>
        <span class="money">${money(report.totalLiabilitiesEquity)}</span>
      </div>
    </div>
  </div>
  ${integrityPanel(integrity)}`;
}

function balanceTable(rows, total, totalLabel, unclassified, unclassifiedTitle) {
  const classified = rows || [];
  const extra = unclassified || [];
  const body = classified.length || extra.length ? [
    ...classified.map((row) => `
    <tr>
      <td class="center mono">${row.accountCode ? esc(toFa(row.accountCode)) : '—'}</td>
      <td>${esc(row.accountName)}</td>
      <td class="money ${row.amount >= 0 ? 'debit' : 'credit'}">${money(row.amount)}</td>
    </tr>`),
    ...(extra.length ? [
      `<tr class="unclassified-head"><td colspan="3">${esc(unclassifiedTitle || 'طبقه‌بندی‌نشده')}</td></tr>`,
      ...extra.map((row) => `
    <tr class="unclassified">
      <td class="center mono">${esc(toFa(row.accountCode))}</td>
      <td>${esc(row.accountName)}</td>
      <td class="money ${row.amount >= 0 ? 'debit' : 'credit'}">${money(row.amount)}</td>
    </tr>`),
    ] : []),
  ].join('') : `<tr><td colspan="3" class="center muted">موردی ثبت نشده است.</td></tr>`;
  return `<div class="table-wrap short">
    <table class="data-table">
      <thead><tr><th style="width:92px">کد</th><th>عنوان</th><th style="width:132px">مبلغ</th></tr></thead>
      <tbody>${body}</tbody>
      <tfoot><tr><td colspan="2" class="bold">${totalLabel}</td><td class="num bold">${money(total)}</td></tr></tfoot>
    </table>
  </div>`;
}

