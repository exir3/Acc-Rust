'use strict';

/* ── Vouchers list ──────────────────────────────────────────── */

PAGES.vouchers = {
  actions: () => [
    { label: 'سند جدید', icon: 'i-plus', primary: true, onClick: () => openVoucherWindow() },
    { label: 'به‌روزرسانی', icon: 'i-refresh', onClick: () => navigate('vouchers', { force: true }) },
  ],
  load: async () => {
    const f = state.filters.vouchers;
    const report = await loadPagedList(
      'vouchers',
      VOUCHER_PAGE_SIZE,
      (offset) => invoke('list_vouchers', {
        from: f.from,
        to: f.to,
        search: f.search,
        offset,
        limit: VOUCHER_PAGE_SIZE,
      }),
      (page) => page.matchedCount ?? page.totalCount ?? page.vouchers.length,
      (page) => page.vouchers,
    );
    state.voucherCount = report.totalCount ?? report.vouchers.length;
    return report;
  },
  render: renderVouchers,
  bind: bindVouchers,
};

function vouchersFilterBar() {
  const f = state.filters.vouchers;
  return `
  <form class="filter-bar" data-filter="vouchers">
    <div class="field grow">
      <label for="vfSearch">جست‌وجو</label>
      <input id="vfSearch" class="input" name="search" type="search" value="${esc(f.search)}" placeholder="شماره سند، شرح یا نام حساب…" />
    </div>
    <div class="field">
      <label for="vfFrom">از تاریخ</label>
      <input id="vfFrom" class="input" name="from" data-date type="text" inputmode="numeric" value="${esc(f.from)}" placeholder="۱۴۰۵/۰۱/۰۱" />
    </div>
    <div class="field">
      <label for="vfTo">تا تاریخ</label>
      <input id="vfTo" class="input" name="to" data-date type="text" inputmode="numeric" value="${esc(f.to)}" placeholder="۱۴۰۵/۱۲/۲۹" />
    </div>
    <button class="btn btn-primary" type="submit">${icon('i-search')}<span>اعمال فیلتر</span></button>
    <button class="btn" type="button" data-reset>${icon('i-refresh')}<span>بازنشانی</span></button>
  </form>`;
}

function renderVouchers(report) {
  const shown = report.vouchers.length;
  const matched = Number(report.matchedCount ?? shown);
  return `
  ${vouchersFilterBar()}
  <div class="card">
    ${shown ? renderVoucherTable(report) : `<div class="empty-state">
      ${icon('i-doc')}
      <b>سندی در این بازه یافت نشد</b>
      <span>بازه تاریخ را تغییر دهید یا سند جدیدی ثبت کنید.</span>
      <button type="button" class="btn btn-primary" data-new-voucher>${icon('i-plus')}<span>ثبت سند جدید</span></button>
    </div>`}
  </div>
  ${loadMoreBar('vouchers', shown, matched, state.listOffsets.vouchers)}`;
}

function listPageStep(kind) {
  if (kind === 'journal') return JOURNAL_PAGE_SIZE;
  if (kind === 'ledger') return LEDGER_PAGE_SIZE;
  return VOUCHER_PAGE_SIZE;
}

function loadMoreBar(kind, shown, total, offset) {
  if (!total) return '';
  const hasPrev = offset > 0;
  const hasNext = offset + shown < total;
  if (!hasPrev && !hasNext) return '';
  return `<div class="load-more">
    <span>نمایش ${toFa(offset + (shown ? 1 : 0))} تا ${toFa(offset + shown)} از ${toFa(total)}</span>
    <div class="load-more-actions">
      <button type="button" class="btn" data-page-prev="${kind}" ${hasPrev ? '' : 'disabled'}><span>صفحه قبل</span></button>
      <button type="button" class="btn" data-page-next="${kind}" ${hasNext ? '' : 'disabled'}><span>صفحه بعد</span></button>
    </div>
  </div>`;
}

function renderVoucherTable(report) {
  const vouchers = report.vouchers;
  const rows = vouchers.map((v) => `
    <tr>
      <td class="center"><button type="button" class="link-btn" data-open-voucher="${v.number}" title="باز کردن سند">${toFa(v.number)}</button></td>
      <td class="nowrap">${faDate(v.date)}</td>
      <td class="center"><span class="voucher-tag">${esc(v.type)} · ${v.isTemporary === false ? 'دائمی' : 'موقت'}</span></td>
      <td class="desc-cell">${esc(v.description) || '—'}</td>
      <td class="center">${toFa(v.lineCount)}</td>
      <td class="money debit">${money(v.totalDebit)}</td>
      <td class="money credit">${money(v.totalCredit)}</td>
      <td class="center nowrap">
        <button type="button" class="btn btn-sm" data-open-voucher="${v.number}" title="ویرایش سند">${icon('i-edit')}</button>
        <button type="button" class="btn btn-sm" data-print-voucher="${v.number}" title="چاپ سند">${icon('i-print')}</button>
${v.isTemporary === false ? '' : `<button type="button" class="btn btn-sm" data-delete-voucher="${v.id}" data-voucher-updated="${esc(v.updatedAt || '')}" data-voucher-label="${esc(v.description || `سند ${v.number}`)}" title="حذف سند">${icon('i-trash')}</button>`}
      </td>
    </tr>`).join('');

  // Totals come from the backend so they still describe the whole filter
  // when only the first page is displayed.
  const totalDebit = amountBigInt(report.totalDebit);
  const totalCredit = amountBigInt(report.totalCredit);
  const balanced = totalDebit === totalCredit;
  const footTone = balanced ? 'ok' : 'danger';

  return `<div class="table-wrap">
    <table class="data-table">
      <thead><tr>
        <th style="width:66px">شماره</th><th style="width:96px">تاریخ</th><th style="width:150px">نوع سند</th>
        <th>شرح</th><th style="width:58px">ردیف</th><th style="width:112px">بدهکار</th>
        <th style="width:112px">بستانکار</th><th style="width:128px">عملیات</th>
      </tr></thead>
      <tbody>${rows}</tbody>
      <tfoot><tr>
        <td colspan="4" class="center bold" style="font-size:.85em;opacity:.75">${toFa(report.matchedCount ?? vouchers.length)} سند</td>
        <td></td>
        <td class="num bold" data-tone="${footTone}">${money(totalDebit)}</td>
        <td class="num bold" data-tone="${footTone}">${money(totalCredit)}</td>
        <td class="center" style="font-size:.8em;opacity:.75">${balanced ? '✔ متوازن' : '✗ مغایرت'}</td>
      </tr></tfoot>
    </table>
  </div>`;
}

function bindVouchers(root) {
  // «سند جدید» buttons are wired once by bindPageCommon.
  bindListPager(root);
  bindVoucherRowActions(root);
}

function bindListPager(root) {
  root.querySelectorAll('[data-page-prev]').forEach((button) => {
    button.addEventListener('click', () => {
      const kind = button.dataset.pagePrev;
      const step = listPageStep(kind);
      state.listOffsets[kind] = Math.max(0, (state.listOffsets[kind] || 0) - step);
      navigate(state.page, { force: true });
    });
  });
  root.querySelectorAll('[data-page-next]').forEach((button) => {
    button.addEventListener('click', () => {
      const kind = button.dataset.pageNext;
      const step = listPageStep(kind);
      state.listOffsets[kind] = (state.listOffsets[kind] || 0) + step;
      navigate(state.page, { force: true });
    });
  });
}

function bindVoucherRowActions(root) {
  // Opening a voucher is wired once by bindPageCommon.
  root.querySelectorAll('[data-print-voucher]').forEach((button) => {
    button.addEventListener('click', () => {
      printWithFeedback('open_voucher_preview', { number: Number(button.dataset.printVoucher) });
    });
  });
  root.querySelectorAll('[data-delete-voucher]').forEach((button) => {
    button.addEventListener('click', async () => {
      const id = Number(button.dataset.deleteVoucher);
      const label = button.dataset.voucherLabel || 'این سند';
      const confirmed = await confirmDialog(
        'حذف سند حسابداری',
        `آیا از حذف «${label}» مطمئن هستید؟ ردیف‌های سند نیز حذف می‌شوند و این عملیات قابل بازگشت نیست.`
      );
      if (!confirmed) return;
      try {
        await invoke('delete_voucher', {
          id,
          expectedUpdatedAt: button.dataset.voucherUpdated || null,
        });
        toast('سند حسابداری حذف شد.', 'ok');
        await navigate(state.page, { force: true });
      } catch (error) {
        toast(errorText(error), 'error');
      }
    });
  });
}

