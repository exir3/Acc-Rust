'use strict';

/* ── Income statement ───────────────────────────────────────── */

PAGES.income = {
  actions: () => [
    { label: 'چاپ گزارش', icon: 'i-print', onClick: () => printWithFeedback('open_income_statement_preview', { ...state.filters.income }) },
    { label: 'به‌روزرسانی', icon: 'i-refresh', onClick: () => navigate('income', { force: true }) },
  ],
  load: async () => {
    const f = state.filters.income;
    return invoke('get_income_statement', { from: f.from, to: f.to, scope: normalizeReportScope(f.scope) });
  },
  render: renderIncomeStatement,
  bind: bindFilterOnly,
};

function incomeFilterBar() {
  const f = state.filters.income;
  return `
  <form class="filter-bar" data-filter="income">
    <div class="field">
      <label for="isFrom">از تاریخ</label>
      <input id="isFrom" class="input" name="from" data-date type="text" inputmode="numeric" value="${esc(f.from)}" placeholder="۱۴۰۵/۰۱/۰۱" />
    </div>
    <div class="field">
      <label for="isTo">تا تاریخ</label>
      <input id="isTo" class="input" name="to" data-date type="text" inputmode="numeric" value="${esc(f.to)}" placeholder="۱۴۰۵/۱۲/۲۹" />
    </div>
    ${reportScopeField('income', 'isScope')}
    <button class="btn btn-primary" type="submit">${icon('i-search')}<span>محاسبه صورت</span></button>
    <button class="btn" type="button" data-reset>${icon('i-refresh')}<span>بازنشانی</span></button>
  </form>`;
}

function renderIncomeStatement(report) {
  const net = amountBigInt(report.netProfit);
  return `
  ${incomeFilterBar()}
  <div class="report-summary">
    <div class="kpi"><span>جمع درآمدها</span><strong>${money(report.revenueTotal)}</strong></div>
    <div class="kpi"><span>بهای تمام شده</span><strong>${money(report.cogsTotal)}</strong></div>
    <div class="kpi"><span>هزینه‌های عملیاتی</span><strong>${money(report.expenseTotal)}</strong></div>
    <div class="kpi"><span>سایر درآمدهای عملیاتی</span><strong>${money(report.otherOperatingRevenueTotal)}</strong></div>
    <div class="kpi"><span>سایر هزینه‌های عملیاتی</span><strong>${money(report.otherOperatingExpenseTotal)}</strong></div>
    <div class="kpi"><span>درآمدهای غیرعملیاتی</span><strong>${money(report.nonOperatingRevenueTotal)}</strong></div>
    <div class="kpi"><span>هزینه‌های غیرعملیاتی</span><strong>${money(report.nonOperatingExpenseTotal)}</strong></div>
    <div class="kpi"><span>هزینه‌های مالی</span><strong>${money(report.financeCostTotal)}</strong></div>
    <div class="kpi"><span>مالیات بر درآمد</span><strong>${money(report.incomeTaxTotal)}</strong></div>
    <div class="kpi"><span>سود (زیان) ناخالص</span><strong>${money(report.grossProfit)}</strong></div>
    <div class="kpi" data-tone="${net >= 0n ? 'ok' : 'danger'}"><span>سود (زیان) خالص</span><strong>${money(net)}</strong></div>
  </div>
  <div class="report-layout">
    <div class="card">
      ${incomeSection('درآمدها', report.revenues, report.revenueTotal, 'i-trend')}
    </div>
    <div class="card">
      ${incomeSection('بهای تمام شده درآمدها', report.cogs, report.cogsTotal, 'i-coins')}
    </div>
  </div>
  <div class="card" style="margin-top:12px">
    <div class="report-total-line"><span>سود (زیان) ناخالص</span><span class="money">${money(report.grossProfit)}</span></div>
  </div>
  <div class="report-layout" style="margin-top:12px">
    <div class="card">
      ${incomeSection('سایر درآمدهای عملیاتی', report.otherOperatingRevenues, report.otherOperatingRevenueTotal, 'i-trend')}
    </div>
    <div class="card">
      ${incomeSection('سایر هزینه‌های عملیاتی', report.otherOperatingExpenses, report.otherOperatingExpenseTotal, 'i-bank')}
    </div>
  </div>
  <div class="card" style="margin-top:12px">
    ${incomeSection('هزینه‌های عملیاتی', report.expenses, report.expenseTotal, 'i-bank')}
  </div>
  <div class="card" style="margin-top:12px">
    <div class="report-total-line"><span>سود (زیان) عملیاتی</span><span class="money">${money(report.operatingProfit)}</span></div>
  </div>
  <div class="report-layout" style="margin-top:12px">
    <div class="card">
      ${incomeSection('درآمدهای غیرعملیاتی', report.nonOperatingRevenues, report.nonOperatingRevenueTotal, 'i-trend')}
    </div>
    <div class="card">
      ${incomeSection('هزینه‌های غیرعملیاتی', report.nonOperatingExpenses, report.nonOperatingExpenseTotal, 'i-bank')}
    </div>
  </div>
  <div class="report-layout" style="margin-top:12px">
    <div class="card">
      ${incomeSection('هزینه‌های مالی', report.financeCosts, report.financeCostTotal, 'i-bank')}
    </div>
    <div class="card">
      ${incomeSection('مالیات بر درآمد', report.incomeTaxes, report.incomeTaxTotal, 'i-bank')}
    </div>
  </div>
  <div class="card" style="margin-top:12px">
    <div class="report-total-line"><span>سود (زیان) پیش از مالیات</span><span class="money">${money(report.profitBeforeTax)}</span></div>
    <div class="report-total-line net"><span>سود (زیان) خالص دوره</span><span class="money">${money(net)}</span></div>
  </div>`;
}

function incomeSection(title, rows, total, iconName) {
  const body = rows.length ? rows.map((row) => `
    <tr>
      <td class="center mono">${toFa(row.accountCode)}</td>
      <td>${esc(row.accountName)}</td>
      <td class="money ${row.amount >= 0 ? 'debit' : 'credit'}">${money(row.amount)}</td>
    </tr>`).join('') : `<tr><td colspan="3" class="center muted">موردی ثبت نشده است.</td></tr>`;
  return `<div class="section-head">
      <h2>${icon(iconName)} ${title}</h2>
      <span class="badge" data-tone="info">${toFa(rows.length)} حساب</span>
    </div>
    <div class="table-wrap short">
      <table class="data-table">
        <thead><tr><th style="width:96px">کد حساب</th><th>عنوان</th><th style="width:140px">مبلغ</th></tr></thead>
        <tbody>${body}</tbody>
        <tfoot><tr><td colspan="2" class="bold">جمع ${title}</td><td class="num bold">${money(total)}</td></tr></tfoot>
      </table>
    </div>`;
}

