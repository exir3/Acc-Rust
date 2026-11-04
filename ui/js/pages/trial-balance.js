'use strict';

/* ── Trial balance ──────────────────────────────────────────── */

PAGES.trial = {
  actions: () => [
    { label: 'بازه گزارش', icon: 'i-search', onClick: openTrialFilterDialog },
    { label: 'چاپ گزارش', icon: 'i-print', onClick: () => printWithFeedback('open_trial_balance_preview', trialPrintRequest()) },
    { label: 'به‌روزرسانی', icon: 'i-refresh', onClick: () => navigate('trial', { force: true }) },
  ],
  load: async () => {
    return invoke('get_trial_balance', trialRequest());
  },
  render: renderTrialBalance,
  bind: (root) => {
    bindVoucherRowActions(root);
    root.querySelector('[data-trial-filter]')?.addEventListener('click', openTrialFilterDialog);
  },
};

const TRIAL_BALANCE_TYPES = ['چهار ستونی', 'شش ستونی', 'هشت ستونی'];

function trialRequest() {
  const f = state.filters.trial;
  return {
    from: f.from || '',
    to: f.to || '',
    accountFrom: f.accountFrom || '',
    accountTo: f.accountTo || '',
    showZero: Boolean(f.showZero),
    multiLevel: Boolean(f.multiLevel),
  };
}

function trialPrintRequest() {
  return {
    ...trialRequest(),
    balanceType: TRIAL_BALANCE_TYPES.includes(state.filters.trial.balanceType)
      ? state.filters.trial.balanceType
      : 'شش ستونی',
    showAccountCode: Boolean(state.filters.trial.showAccountCode),
    showAccountName: Boolean(state.filters.trial.showAccountName),
    showPrintDateTime: Boolean(state.filters.trial.showPrintDateTime),
  };
}

function trialFilterTrigger() {
  const f = state.filters.trial;
  const range = f.accountFrom || f.accountTo
    ? `${f.accountFrom ? toFa(f.accountFrom) : '…'} تا ${f.accountTo ? toFa(f.accountTo) : '…'}`
    : 'همه کدها';
  return `
  <div class="filter-bar trial-filter-trigger">
    <span>بازه گزارش: <b>${faDate(f.from)} تا ${faDate(f.to)}</b></span>
    <span>کد حساب: <b>${range}</b></span>
    <span>ستون‌ها: <b>${esc(f.balanceType || 'شش ستونی')}</b>${f.multiLevel ? ' · چندسطحی' : ''}</span>
    <button class="btn btn-primary" type="button" data-trial-filter>${icon('i-search')}<span>تغییر فیلترها</span></button>
  </div>`;
}

function openTrialFilterDialog() {
  const f = state.filters.trial;
  const balanceType = TRIAL_BALANCE_TYPES.includes(f.balanceType) ? f.balanceType : 'شش ستونی';
  const dialog = openDialog({
    title: 'فیلترهای صورت تراز آزمایشی',
    body: `
      <form class="filter-bar trial-dialog-form" data-trial-filter-form>
        <div class="field">
          <label for="tbDialogFrom">از تاریخ</label>
          <input id="tbDialogFrom" class="input" name="from" data-date type="text" inputmode="numeric" value="${esc(f.from)}" placeholder="۱۴۰۵/۰۱/۰۱" />
        </div>
        <div class="field">
          <label for="tbDialogTo">تا تاریخ</label>
          <input id="tbDialogTo" class="input" name="to" data-date type="text" inputmode="numeric" value="${esc(f.to)}" placeholder="۱۴۰۵/۱۲/۲۹" />
        </div>
        <div class="field">
          <label for="tbDialogAccountFrom">از کد حساب</label>
          <input id="tbDialogAccountFrom" class="input" name="accountFrom" type="text" dir="ltr" inputmode="numeric" value="${esc(f.accountFrom || '')}" placeholder="مثلاً ۱۱۰۱" />
        </div>
        <div class="field">
          <label for="tbDialogAccountTo">تا کد حساب</label>
          <input id="tbDialogAccountTo" class="input" name="accountTo" type="text" dir="ltr" inputmode="numeric" value="${esc(f.accountTo || '')}" placeholder="مثلاً ۱۱۹۹" />
        </div>
        <div class="field">
          <label for="tbDialogBalanceType">نوع تراز</label>
          <select id="tbDialogBalanceType" class="input" name="balanceType">
            ${TRIAL_BALANCE_TYPES.map((type) => `<option value="${type}"${type === balanceType ? ' selected' : ''}>${type}</option>`).join('')}
          </select>
        </div>
        <label class="field trial-check">
          <input type="checkbox" name="showZero"${f.showZero ? ' checked' : ''} />
          <span>نمایش اقلام با مانده صفر</span>
        </label>
        <label class="field trial-check">
          <input type="checkbox" name="multiLevel"${f.multiLevel ? ' checked' : ''} />
          <span>نمایش چندسطحی (کل، معین و تفصیلی)</span>
        </label>
        <label class="field trial-check">
          <input type="checkbox" name="showAccountCode"${f.showAccountCode ? ' checked' : ''} />
          <span>نمایش ستون کد حساب</span>
        </label>
        <label class="field trial-check">
          <input type="checkbox" name="showAccountName"${f.showAccountName ? ' checked' : ''} />
          <span>نمایش ستون نام حساب</span>
        </label>
        <label class="field trial-check">
          <input type="checkbox" name="showPrintDateTime"${f.showPrintDateTime ? ' checked' : ''} />
          <span>نمایش تاریخ چاپ در گزارش</span>
        </label>
      </form>`,
    actions: [
      {
        label: 'محاسبه تراز',
        icon: 'i-search',
        primary: true,
        onClick: (result) => {
          const form = result.body.querySelector('[data-trial-filter-form]');
          if (!form?.reportValidity()) return;
          if (!validateDateFields(form)) return;
          const data = new FormData(form);
          state.filters.trial.from = String(data.get('from') || '');
          state.filters.trial.to = String(data.get('to') || '');
          state.filters.trial.accountFrom = toEn(String(data.get('accountFrom') || '')).replace(/[^\d]/g, '');
          state.filters.trial.accountTo = toEn(String(data.get('accountTo') || '')).replace(/[^\d]/g, '');
          state.filters.trial.balanceType = TRIAL_BALANCE_TYPES.includes(String(data.get('balanceType')))
            ? String(data.get('balanceType'))
            : 'شش ستونی';
          state.filters.trial.showZero = form.querySelector('[name="showZero"]').checked;
          state.filters.trial.multiLevel = form.querySelector('[name="multiLevel"]').checked;
          state.filters.trial.showAccountCode = form.querySelector('[name="showAccountCode"]').checked;
          state.filters.trial.showAccountName = form.querySelector('[name="showAccountName"]').checked;
          state.filters.trial.showPrintDateTime = form.querySelector('[name="showPrintDateTime"]').checked;
          result.close();
          navigate('trial', { force: true });
        },
      },
      {
        label: 'بازنشانی',
        icon: 'i-refresh',
        onClick: (result) => {
          const defaults = defaultFilters().trial;
          result.body.querySelector('[name="from"]').value = defaults.from;
          result.body.querySelector('[name="to"]').value = defaults.to;
          result.body.querySelector('[name="accountFrom"]').value = '';
          result.body.querySelector('[name="accountTo"]').value = '';
          result.body.querySelector('[name="balanceType"]').value = defaults.balanceType;
          result.body.querySelector('[name="showZero"]').checked = defaults.showZero;
          result.body.querySelector('[name="multiLevel"]').checked = defaults.multiLevel;
          result.body.querySelector('[name="showAccountCode"]').checked = defaults.showAccountCode;
          result.body.querySelector('[name="showAccountName"]').checked = defaults.showAccountName;
          result.body.querySelector('[name="showPrintDateTime"]').checked = defaults.showPrintDateTime;
        },
      },
    ],
  });
  if (!dialog) return;

  const form = dialog.body.querySelector('[data-trial-filter-form]');
  form?.addEventListener('submit', (event) => {
    event.preventDefault();
    dialog.foot.querySelector('.btn-primary')?.click();
  });
  form?.querySelectorAll('input[data-date]').forEach((input) => {
    input.addEventListener('input', () => {
      clearFieldError(input);
      const cleaned = toEn(input.value).replace(/[^\d/]/g, '');
      if (input.value !== cleaned) input.value = cleaned;
    });
  });
  form?.querySelectorAll('[name="accountFrom"], [name="accountTo"]').forEach((input) => {
    input.addEventListener('input', () => {
      const cleaned = toEn(input.value).replace(/[^\d]/g, '');
      if (input.value !== cleaned) input.value = cleaned;
    });
  });
}

function renderTrialBalance(report) {
  const balanceType = TRIAL_BALANCE_TYPES.includes(state.filters.trial.balanceType)
    ? state.filters.trial.balanceType
    : 'شش ستونی';
  const isFourColumn = balanceType === 'چهار ستونی';
  const showOpening = balanceType === 'شش ستونی' || balanceType === 'هشت ستونی';
  const showAccumulated = balanceType === 'هشت ستونی';
  const multiLevel = Boolean(state.filters.trial.multiLevel);
  const showAccountCode = state.filters.trial.showAccountCode !== false;
  const showAccountName = state.filters.trial.showAccountName !== false;
  const identityColumns = 1 + (showAccountCode ? 1 : 0) + (showAccountName ? 1 : 0);

  const rows = report.rows.map((row) => {
    const indent = multiLevel && showAccountName ? Math.max(0, Number(row.level) - 1) * 16 : 0;
    const turnoverDebit = isFourColumn
      ? amountBigInt(row.openingDebit) + amountBigInt(row.periodDebit)
      : row.periodDebit;
    const turnoverCredit = isFourColumn
      ? amountBigInt(row.openingCredit) + amountBigInt(row.periodCredit)
      : row.periodCredit;
    const opening = showOpening
      ? `<td class="money debit">${money(row.openingDebit)}</td>
         <td class="money credit">${money(row.openingCredit)}</td>`
      : '';
    const accumulated = showAccumulated
      ? `<td class="money debit">${money(amountBigInt(row.openingDebit) + amountBigInt(row.periodDebit))}</td>
         <td class="money credit">${money(amountBigInt(row.openingCredit) + amountBigInt(row.periodCredit))}</td>`
      : '';
    const codeCell = showAccountCode
      ? `<td class="center mono">${toFa(row.accountCode)}</td>`
      : '';
    const nameCell = showAccountName
      ? `<td style="${indent ? `padding-right:${indent}px` : ''}">${esc(row.accountName)}</td>`
      : '';
    return `
    <tr>
      ${codeCell}${nameCell}
      <td class="desc-cell">${esc(row.groupName)}</td>
      ${opening}
      <td class="money debit">${money(turnoverDebit)}</td>
      <td class="money credit">${money(turnoverCredit)}</td>
      ${accumulated}
      <td class="money debit bold">${money(row.closingDebit)}</td>
      <td class="money credit bold">${money(row.closingCredit)}</td>
    </tr>`;
  }).join('');

  const codeHeader = showAccountCode
    ? `<th rowspan="2" style="width:100px">کد حساب</th>`
    : '';
  const nameHeader = showAccountName ? `<th rowspan="2">عنوان حساب</th>` : '';
  const openingHeader = showOpening
    ? `<th colspan="2">مانده افتتاحیه</th>`
    : '';
  const openingCells = showOpening
    ? `<th style="width:110px">بدهکار</th><th style="width:110px">بستانکار</th>`
    : '';
  const accumulatedHeader = showAccumulated ? `<th colspan="2">جمع دوره</th>` : '';
  const accumulatedCells = showAccumulated
    ? `<th style="width:110px">بدهکار</th><th style="width:110px">بستانکار</th>`
    : '';
  const openingTotal = showOpening
    ? `<td class="num bold">${money(report.openingDebit)}</td><td class="num bold">${money(report.openingCredit)}</td>`
    : '';
  const accumulatedTotal = showAccumulated
    ? `<td class="num bold">${money(amountBigInt(report.openingDebit) + amountBigInt(report.periodDebit))}</td>
       <td class="num bold">${money(amountBigInt(report.openingCredit) + amountBigInt(report.periodCredit))}</td>`
    : '';
  const totalTurnoverDebit = isFourColumn
    ? amountBigInt(report.openingDebit) + amountBigInt(report.periodDebit)
    : report.periodDebit;
  const totalTurnoverCredit = isFourColumn
    ? amountBigInt(report.openingCredit) + amountBigInt(report.periodCredit)
    : report.periodCredit;

  const balanced = amountBigInt(report.closingDebit) === amountBigInt(report.closingCredit)
    && amountBigInt(report.periodDebit) === amountBigInt(report.periodCredit);
  const hasCodeFilter = Boolean(state.filters.trial.accountFrom || state.filters.trial.accountTo);
  const statusLabel = hasCodeFilter ? 'وضعیت محدوده' : 'وضعیت تراز';
  const statusText = hasCodeFilter ? 'جمع محدودهٔ انتخاب‌شده' : balanced ? 'متوازن' : 'مغایرت';
  const statusTone = hasCodeFilter ? '' : ` data-tone="${balanced ? 'ok' : 'danger'}"`;

  return `
  ${trialFilterTrigger()}
  <div class="report-summary">
    <div class="kpi"><span>تعداد ردیف گزارش</span><strong>${toFa(report.rows.length)}</strong></div>
    <div class="kpi"><span>گردش بدهکار دوره</span><strong>${money(report.periodDebit)}</strong></div>
    <div class="kpi"><span>گردش بستانکار دوره</span><strong>${money(report.periodCredit)}</strong></div>
    <div class="kpi"${statusTone}><span>${statusLabel}</span><strong>${statusText}</strong></div>
  </div>
  <div class="card">
    ${report.rows.length ? `
      <div class="table-wrap">
        <table class="data-table">
          <thead>
            <tr>
              ${codeHeader}
              ${nameHeader}
              <th rowspan="2" style="width:180px">گروه</th>
              ${openingHeader}
              <th colspan="2">${isFourColumn ? 'جمع دوره' : 'گردش دوره'}</th>
              ${accumulatedHeader}
              <th colspan="2">مانده اختتامیه</th>
            </tr>
            <tr>
              ${openingCells}
              <th style="width:110px">${isFourColumn ? 'جمع بدهکار' : 'بدهکار'}</th>
              <th style="width:110px">${isFourColumn ? 'جمع بستانکار' : 'بستانکار'}</th>
              ${accumulatedCells}
              <th style="width:110px">بدهکار</th><th style="width:110px">بستانکار</th>
            </tr>
          </thead>
          <tbody>${rows}</tbody>
          <tfoot><tr>
            <td colspan="${identityColumns}" class="center bold">جمع کل</td>
            ${openingTotal}
            <td class="num bold">${money(totalTurnoverDebit)}</td>
            <td class="num bold">${money(totalTurnoverCredit)}</td>
            ${accumulatedTotal}
            <td class="num bold">${money(report.closingDebit)}</td>
            <td class="num bold">${money(report.closingCredit)}</td>
          </tr></tfoot>
        </table>
      </div>` : `<div class="empty-state">
        ${icon('i-scale')}
        <b>حسابی با گردش یا مانده یافت نشد</b>
        <span>در این بازه هیچ سندی ثبت نشده، حساب‌ها مانده‌ای ندارند یا فیلتر کد حساب را محدود کرده است.</span>
      </div>`}
  </div>`;
}

