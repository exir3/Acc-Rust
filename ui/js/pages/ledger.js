'use strict';

/* ── Ledger ─────────────────────────────────────────────────── */

PAGES.ledger = {
  actions: () => {
    const f = state.filters.ledger;
    return [
      {
        label: 'چاپ گزارش',
        icon: 'i-print',
        onClick: () => {
          if (!f.accountCode) {
            toast('ابتدا یک حساب انتخاب کنید.', 'info');
            return;
          }
          printWithFeedback('open_ledger_preview', { ...f });
        },
      },
      { label: 'به‌روزرسانی', icon: 'i-refresh', onClick: () => navigate('ledger', { force: true }) },
    ];
  },
  load: async (force) => {
    const accounts = await ensureAccounts(force);
    const f = state.filters.ledger;
    if (!f.accountCode && accounts.length) {
      const first = accounts.find((a) => a.isPostable) || accounts[0];
      f.accountCode = first.code;
    }
    if (!f.accountCode) return { accounts, report: null };
    const report = await loadPagedList(
      'ledger',
      LEDGER_PAGE_SIZE,
      (offset) => invoke('get_ledger', {
        accountCode: f.accountCode,
        from: f.from,
        to: f.to,
        scope: normalizeReportScope(f.scope),
        offset,
        limit: LEDGER_PAGE_SIZE,
      }),
      (page) => page.rowCount ?? page.rows.length,
      (page) => page.rows,
    );
    return { accounts, report };
  },
  render: renderLedger,
  bind: bindLedger,
};

function renderLedger(data) {
  const { accounts, report } = data;
  if (!accounts.length) {
    return `<div class="empty-state">
      ${icon('i-tree')}
      <b>هنوز حسابی تعریف نشده است</b>
      <span>ابتدا از بخش حساب‌ها، حساب‌های مورد نیاز را تعریف کنید.</span>
      <button type="button" class="btn btn-primary" data-go="accounts">${icon('i-plus')}<span>تعریف حساب</span></button>
    </div>`;
  }

  const f = state.filters.ledger;
  const options = accounts.map((a) => `<option value="${esc(a.code)}"${a.code === f.accountCode ? ' selected' : ''}>${toFa(a.code)} — ${esc(a.name)} (${levelLabel(a.level)})</option>`).join('');

  const filter = `
  <form class="filter-bar" data-filter="ledger">
    <div class="field grow">
      <label for="lgAccount">حساب</label>
      <select id="lgAccount" class="input wide" name="accountCode">${options}</select>
    </div>
    <div class="field">
      <label for="lgFrom">از تاریخ</label>
      <input id="lgFrom" class="input" name="from" data-date type="text" inputmode="numeric" value="${esc(f.from)}" placeholder="۱۴۰۵/۰۱/۰۱" />
    </div>
    <div class="field">
      <label for="lgTo">تا تاریخ</label>
      <input id="lgTo" class="input" name="to" data-date type="text" inputmode="numeric" value="${esc(f.to)}" placeholder="۱۴۰۵/۱۲/۲۹" />
    </div>
    ${reportScopeField('ledger', 'lgScope')}
    <button class="btn btn-primary" type="submit">${icon('i-search')}<span>نمایش گردش</span></button>
    <button class="btn" type="button" data-reset>${icon('i-refresh')}<span>بازنشانی</span></button>
  </form>`;

  if (!report) {
    return `${filter}<div class="empty-state">${icon('i-ledger')}<b>حسابی انتخاب نشده است</b><span>یک حساب را از فهرست بالا انتخاب کنید.</span></div>`;
  }

  const openingNet = amountBigInt(report.openingDebit) - amountBigInt(report.openingCredit);
  const closingNet = amountBigInt(report.closingDebit) - amountBigInt(report.closingCredit);
  // The ledger of a parent account mixes its sub-accounts, so say which
  // sub-account each row belongs to.
  const withAccount = Boolean(report.includesChildren);
  const rows = report.rows.map((row) => `
    <tr>
      <td class="nowrap">${faDate(row.date)}</td>
      <td class="center"><button type="button" class="link-btn" data-open-voucher="${row.number}">${toFa(row.number)}</button></td>
      ${withAccount ? `<td class="desc-cell" title="${esc(row.accountName)}">${toFa(row.accountCode)} — ${esc(row.accountName)}</td>` : ''}
      <td class="desc-cell">${esc(row.lineDescription || row.voucherDescription) || '—'}</td>
      <td class="money debit">${money(row.debit)}</td>
      <td class="money credit">${money(row.credit)}</td>
      <td class="money ${row.runningDebit ? 'debit' : ''}">${money(row.runningDebit)}</td>
      <td class="money ${row.runningCredit ? 'credit' : ''}">${money(row.runningCredit)}</td>
    </tr>`).join('');

  return `
  ${filter}
  <div class="report-summary">
    <div class="kpi"><span>مانده افتتاحیه</span><strong>${money(openingNet)}</strong></div>
    <div class="kpi"><span>گردش بدهکار دوره</span><strong>${money(report.totalDebit)}</strong></div>
    <div class="kpi"><span>گردش بستانکار دوره</span><strong>${money(report.totalCredit)}</strong></div>
    <div class="kpi" data-tone="${closingNet >= 0 ? 'info' : 'warn'}"><span>مانده اختتامیه</span><strong>${money(closingNet)}</strong></div>
  </div>
  <div class="card">
    <div class="info-strip">
      <span>حساب: <b>${toFa(report.accountCode)} — ${esc(report.accountName)}</b></span>
      <span class="sep"></span>
      <span>گروه: <b>${esc(report.groupName)}</b></span>
      ${withAccount ? '<span class="sep"></span><span><b>شامل گردش زیرحساب‌ها</b></span>' : ''}
      <span class="sep"></span>
      <span>دوره: <b>${report.from ? faDate(report.from) : 'ابتدا'} تا ${report.to ? faDate(report.to) : 'انتها'}</b></span>
    </div>
    <div class="table-wrap">
      <table class="data-table">
        <thead><tr>
          <th style="width:96px">تاریخ</th><th style="width:66px">شماره</th>${withAccount ? '<th style="width:200px">حساب</th>' : ''}<th>شرح</th>
          <th style="width:112px">بدهکار</th><th style="width:112px">بستانکار</th>
          <th style="width:120px">مانده بدهکار</th><th style="width:120px">مانده بستانکار</th>
        </tr></thead>
        <tbody>
          <tr>
            <td colspan="${withAccount ? 6 : 5}" class="center bold">مانده افتتاحیه</td>
            <td class="money debit bold">${money(report.openingDebit)}</td>
            <td class="money credit bold">${money(report.openingCredit)}</td>
          </tr>
          ${rows}
        </tbody>
        <tfoot><tr>
          <td colspan="${withAccount ? 4 : 3}" class="bold">گردش دوره و مانده اختتامیه</td>
          <td class="num bold">${money(report.totalDebit)}</td>
          <td class="num bold">${money(report.totalCredit)}</td>
          <td class="num bold">${money(report.closingDebit)}</td>
          <td class="num bold">${money(report.closingCredit)}</td>
        </tr></tfoot>
      </table>
    </div>
    ${report.rows.length ? '' : '<div class="note" style="margin-top:10px">در این بازه گردشی برای این حساب ثبت نشده است.</div>'}
  </div>
  ${loadMoreBar('ledger', report.rows.length, report.rowCount ?? report.rows.length, state.listOffsets.ledger)}`;
}

function bindLedger(root) {
  // [data-go] buttons are wired once by bindPageCommon.
  bindVoucherRowActions(root);
  bindListPager(root);
}

