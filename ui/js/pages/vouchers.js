'use strict';

/* ── Vouchers list ──────────────────────────────────────────── */

// The filter panel is closed by default and toggled from the bottom toolbar,
// like the reference accounting-documents form (and the trial-balance popup).
let voucherListState = { filterOpen: false, keydownBound: false };

PAGES.vouchers = {
  // The form's actions live in the bottom toolbar, not in the page header.
  actions: () => [],
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

function vouchersFilterPanel() {
  const f = state.filters.vouchers;
  return `
  <form class="vlist-filter-form" data-filter="vouchers">
    <fieldset id="vouchersFilterPanel" class="tbp-filter-panel vlist-filter-panel"${voucherListState.filterOpen ? '' : ' hidden'}>
      <legend class="vlist-filter-legend">
        <span>فیلترینگ گزارش</span>
        <button type="button" class="vlist-filter-clear" data-reset>پاک کردن فیلترها</button>
      </legend>
      <div class="vlist-filter-grid">
        <div class="field tbp-filter-field vlist-filter-wide">
          <label class="tbp-filter-lbl" for="vfSearch">جست‌وجو:</label>
          <input id="vfSearch" class="tbp-filter-inp" name="search" type="search" value="${esc(f.search)}" placeholder="شماره سند، شرح یا نام حساب…" />
        </div>
        <div class="field tbp-filter-field">
          <label class="tbp-filter-lbl" for="vfFrom">از تاریخ:</label>
          <input id="vfFrom" class="tbp-filter-inp" name="from" data-date type="text" inputmode="numeric" dir="ltr" value="${esc(f.from)}" placeholder="۱۴۰۵/۰۱/۰۱" />
        </div>
        <div class="field tbp-filter-field">
          <label class="tbp-filter-lbl" for="vfTo">تا تاریخ:</label>
          <input id="vfTo" class="tbp-filter-inp" name="to" data-date type="text" inputmode="numeric" dir="ltr" value="${esc(f.to)}" placeholder="۱۴۰۵/۱۲/۲۹" />
        </div>
      </div>
    </fieldset>
  </form>`;
}

function vouchersToolbar() {
  const open = voucherListState.filterOpen;
  return `
  <div class="tbp-toolbar vlist-toolbar" role="toolbar" aria-label="عملیات لیست اسناد حسابداری">
    <div class="tbp-toolbar-group" role="group" aria-label="عملیات فرم">
      <button type="button" class="tbp-fbtn apply" data-vlist-action="apply" title="اعمال فیلتر و نمایش گزارش (PageDown)">
        <span class="tbp-tbtn-label">اعمال فیلتر</span>
      </button>
      <button type="button" class="tbp-tbtn${open ? ' pressed' : ''}" data-vlist-action="filter"
        title="فیلترینگ گزارش (Insert)" aria-pressed="${open}" aria-expanded="${open}"
        aria-controls="vouchersFilterPanel">
        <span class="tbp-tbtn-label">فیلترینگ گزارش</span>
      </button>
      <button type="button" class="tbp-tbtn" data-vlist-action="new-voucher" title="سند جدید">
        <span class="tbp-tbtn-label">سند جدید</span>
      </button>
      <button type="button" class="tbp-tbtn" data-vlist-action="refresh" title="به‌روزرسانی (F5)">
        <span class="tbp-tbtn-label">به‌روزرسانی</span>
      </button>
    </div>
    <div class="tbp-toolbar-group tbp-toolbar-close-group" role="group" aria-label="بستن فرم">
      <button type="button" class="tbp-back-btn" data-vlist-action="close" title="بستن (Escape)">
        <span class="tbp-tbtn-label">بستن</span>
      </button>
    </div>
  </div>`;
}

function renderVouchers(report) {
  const shown = report.vouchers.length;
  const matched = Number(report.matchedCount ?? shown);
  return `
  <div class="vlist-shell">
    <div class="vlist-content">
      <div class="vlist-main">
        <div class="card vlist-card">
          ${shown ? renderVoucherTable(report) : `<div class="empty-state">
            ${icon('i-doc')}
            <b>سندی در این بازه یافت نشد</b>
            <span>بازه تاریخ را تغییر دهید یا سند جدیدی ثبت کنید.</span>
            <button type="button" class="btn btn-primary" data-new-voucher>${icon('i-plus')}<span>ثبت سند جدید</span></button>
          </div>`}
        </div>
        ${loadMoreBar('vouchers', shown, matched, state.listOffsets.vouchers)}
      </div>
      ${vouchersFilterPanel()}
    </div>
    ${vouchersToolbar()}
  </div>`;
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
  root.querySelectorAll('[data-vlist-action]').forEach((button) => {
    button.addEventListener('click', () => voucherRunAction(button.dataset.vlistAction));
  });
  // #page survives re-renders, so bind the window-level shortcuts once.
  if (!voucherListState.keydownBound) {
    document.addEventListener('keydown', voucherHandleKeydown);
    voucherListState.keydownBound = true;
  }
}

function voucherHandleKeydown(event) {
  if (event.defaultPrevented) return;
  const target = event.target;
  const tag = target?.tagName;
  const inField = tag === 'INPUT' || tag === 'SELECT' || tag === 'TEXTAREA'
    || target?.isContentEditable;
  if (event.key === 'Escape') {
    event.preventDefault();
    void voucherCloseWindow();
    return;
  }
  if (event.ctrlKey || event.altKey || event.metaKey || event.shiftKey) return;
  if (event.key === 'F5') {
    event.preventDefault();
    navigate('vouchers', { force: true });
    return;
  }
  // PageDown/Insert have no native meaning in a single-line input, so they
  // keep working while the user is typing a filter value.
  const inSingleLineInput = tag === 'INPUT';
  if (event.key === 'PageDown' && (!inField || inSingleLineInput)) {
    event.preventDefault();
    voucherApplyFilters();
    return;
  }
  if (event.key === 'Insert' && (!inField || inSingleLineInput)) {
    event.preventDefault();
    voucherToggleFilterPanel();
  }
}

function voucherRunAction(action) {
  switch (action) {
    case 'apply': voucherApplyFilters(); break;
    case 'filter': voucherToggleFilterPanel(); break;
    case 'new-voucher': openVoucherWindow(); break;
    case 'refresh': navigate('vouchers', { force: true }); break;
    case 'close': void voucherCloseWindow(); break;
    default: break;
  }
}

function voucherToggleFilterPanel(open = !voucherListState.filterOpen) {
  voucherListState.filterOpen = open;
  const panel = $('#vouchersFilterPanel');
  if (panel) {
    panel.hidden = !open;
    if (open) panel.querySelector('input[name="search"]')?.focus();
  }
  $$('[data-vlist-action="filter"]').forEach((button) => {
    button.classList.toggle('pressed', open);
    button.setAttribute('aria-pressed', String(open));
    button.setAttribute('aria-expanded', String(open));
  });
}

function voucherApplyFilters() {
  // The filter panel stays mounted while hidden, so its current values are
  // submitted exactly as if the old in-page «اعمال فیلتر» button was pressed.
  const form = $('form[data-filter="vouchers"]');
  if (form) {
    form.requestSubmit();
    return;
  }
  navigate('vouchers', { force: true });
}

async function voucherCloseWindow() {
  try {
    if (currentWindow && typeof currentWindow.close === 'function') {
      await currentWindow.close();
      return;
    }
  } catch (_) { /* fall through to window.close */ }
  window.close();
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

