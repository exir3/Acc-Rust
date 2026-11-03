'use strict';

/* ── Journal ────────────────────────────────────────────────── */

PAGES.journal = {
  actions: () => [
    { label: 'چاپ گزارش', icon: 'i-print', onClick: () => printWithFeedback('open_journal_preview', { ...state.filters.journal }) },
    { label: 'به‌روزرسانی', icon: 'i-refresh', onClick: () => navigate('journal', { force: true }) },
  ],
  load: async () => {
    const f = state.filters.journal;
    return loadPagedList(
      'journal',
      JOURNAL_PAGE_SIZE,
      (offset) => invoke('get_journal', {
        from: f.from,
        to: f.to,
        search: f.search,
        offset,
        limit: JOURNAL_PAGE_SIZE,
      }),
      (page) => page.rowCount ?? page.rows.length,
      (page) => page.rows,
    );
  },
  render: renderJournal,
  bind: bindJournal,
};

function journalFilterBar() {
  const f = state.filters.journal;
  return `
  <form class="filter-bar" data-filter="journal">
    <div class="field grow">
      <label for="jSearch">جست‌وجو</label>
      <input id="jSearch" class="input" name="search" type="search" value="${esc(f.search)}" placeholder="شماره سند، شرح یا نام حساب…" />
    </div>
    <div class="field">
      <label for="jFrom">از تاریخ</label>
      <input id="jFrom" class="input" name="from" data-date type="text" inputmode="numeric" value="${esc(f.from)}" placeholder="۱۴۰۵/۰۱/۰۱" />
    </div>
    <div class="field">
      <label for="jTo">تا تاریخ</label>
      <input id="jTo" class="input" name="to" data-date type="text" inputmode="numeric" value="${esc(f.to)}" placeholder="۱۴۰۵/۱۲/۲۹" />
    </div>
    <button class="btn btn-primary" type="submit">${icon('i-search')}<span>اعمال فیلتر</span></button>
    <button class="btn" type="button" data-reset>${icon('i-refresh')}<span>بازنشانی</span></button>
  </form>`;
}

function renderJournal(report) {
  const balanced = amountBigInt(report.totalDebit) === amountBigInt(report.totalCredit);
  const hasSearch = Boolean(state.filters.journal.search.trim());
  let lastVoucher = null;
  const rows = report.rows.map((row) => {
    const first = row.voucherId !== lastVoucher;
    lastVoucher = row.voucherId;
    return `<tr class="${first ? 'row-first' : ''}">
      <td class="nowrap">${first ? faDate(row.date) : ''}</td>
      <td class="center">${first ? `<button type="button" class="link-btn" data-open-voucher="${row.number}">${toFa(row.number)}</button>` : ''}</td>
      <td class="center mono">${toFa(row.accountCode)}</td>
      <td>${esc(row.accountName)}</td>
      <td class="desc-cell">${esc(row.lineDescription || row.voucherDescription) || '—'}</td>
      <td class="money debit">${money(row.debit)}</td>
      <td class="money credit">${money(row.credit)}</td>
    </tr>`;
  }).join('');

  return `
  ${journalFilterBar()}
  <div class="report-summary">
    <div class="kpi"><span>تعداد اسناد</span><strong>${toFa(report.voucherCount)}</strong></div>
    <div class="kpi"><span>تعداد ردیف</span><strong>${toFa(report.rowCount ?? report.rows.length)}</strong></div>
    <div class="kpi"><span>جمع بدهکار</span><strong>${money(report.totalDebit)}</strong></div>
    <div class="kpi"><span>جمع بستانکار</span><strong>${money(report.totalCredit)}</strong></div>
    ${hasSearch
      ? '<div class="kpi"><span>جمع ردیف‌های فیلترشده</span><strong>—</strong></div>'
      : `<div class="kpi" data-tone="${balanced ? 'ok' : 'danger'}"><span>تراز دفتر</span><strong>${balanced ? 'متوازن' : 'مغایرت'}</strong></div>`}
  </div>
  <div class="card">
    ${report.rows.length ? `
      <div class="table-wrap">
        <table class="data-table">
          <thead><tr>
            <th style="width:96px">تاریخ</th><th style="width:66px">شماره</th><th style="width:90px">کد حساب</th>
            <th style="width:210px">عنوان حساب</th><th>شرح</th>
            <th style="width:112px">بدهکار</th><th style="width:112px">بستانکار</th>
          </tr></thead>
          <tbody>${rows}</tbody>
          <tfoot><tr>
            <td colspan="5" class="center bold">جمع کل</td>
            <td class="num bold">${money(report.totalDebit)}</td>
            <td class="num bold">${money(report.totalCredit)}</td>
          </tr></tfoot>
        </table>
      </div>` : `<div class="empty-state">
        ${icon('i-journal')}
        <b>گردشی در این بازه وجود ندارد</b>
        <span>بازه تاریخ را تغییر دهید یا سند جدیدی ثبت کنید.</span>
      </div>`}
  </div>
  ${loadMoreBar('journal', report.rows.length, Number(report.rowCount ?? report.rows.length), state.listOffsets.journal)}`;
}

function bindJournal(root) {
  bindListPager(root);
  bindVoucherRowActions(root);
}

