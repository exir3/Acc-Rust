'use strict';

/* ── Trial balance (reference-style desktop form) ───────────── */

const TRIAL_BALANCE_TYPES = ['چهار ستونی', 'شش ستونی', 'هشت ستونی'];

let trialView = null;
let trialAbort = null;
let trialSearchTimer = 0;
let trialScrollbarFrame = 0;
let trialState = {
  search: '',
  drillPath: [],
  selectedCode: '',
  filterOpen: false,
};

PAGES.trial = {
  // The report has its own toolbar inside the popup window.
  actions: () => [],
  load: async () => invoke('get_trial_balance', trialRequest()),
  render: renderTrialBalance,
  bind: bindTrialBalance,
};

function trialRequest() {
  const f = state.filters.trial;
  // The form works like the reference app: fetch the complete tree once and
  // apply the view filters (level, zero balances, search, drill-down) locally.
  return {
    from: f.from || '',
    to: f.to || '',
    scope: normalizeReportScope(f.scope),
    accountFrom: f.accountFrom || '',
    accountTo: f.accountTo || '',
    showZero: true,
    multiLevel: true,
  };
}

function trialPrintRequest() {
  const f = state.filters.trial;
  return {
    from: f.from || '',
    to: f.to || '',
    scope: normalizeReportScope(f.scope),
    accountFrom: f.accountFrom || '',
    accountTo: f.accountTo || '',
    showZero: Boolean(f.showZero),
    multiLevel: Boolean(f.multiLevel),
    balanceType: TRIAL_BALANCE_TYPES.includes(f.balanceType) ? f.balanceType : 'شش ستونی',
    showAccountCode: Boolean(f.showAccountCode),
    showAccountName: Boolean(f.showAccountName),
    showPrintDateTime: Boolean(f.showPrintDateTime),
  };
}

/* ── Formatting and hierarchical helpers ─────────────────────── */

function trialFormatAmount(value) {
  const amount = amountBigInt(value);
  const negative = amount < 0n;
  const text = (negative ? -amount : amount)
    .toString()
    .replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return negative ? `(${toFa(text)})` : toFa(text);
}

function trialCleanCode(code) {
  const digits = toEn(String(code ?? '')).replace(/[^\d]/g, '');
  return digits || String(code ?? '');
}

function trialCodeSegments(code, width) {
  const normalized = toEn(String(code ?? '')).trim().replace(/[^\d.]/g, '');
  if (!normalized) return [];
  if (normalized.includes('.')) return normalized.split('.').filter(Boolean);
  const segmentWidth = width > 0 ? width : normalized.length;
  const parts = [];
  for (let index = 0; index < normalized.length; index += segmentWidth) {
    parts.push(normalized.slice(index, index + segmentWidth));
  }
  return parts;
}

function trialCompareSegments(left, right) {
  const length = Math.min(left.length, right.length);
  for (let index = 0; index < length; index += 1) {
    const a = Number(left[index]);
    const b = Number(right[index]);
    if (a !== b) return a < b ? -1 : 1;
  }
  return 0;
}

function trialRowNumbers(row) {
  return {
    od: amountBigInt(row.openingDebit),
    oc: amountBigInt(row.openingCredit),
    pd: amountBigInt(row.periodDebit),
    pc: amountBigInt(row.periodCredit),
    cd: amountBigInt(row.closingDebit),
    cc: amountBigInt(row.closingCredit),
  };
}

function createTrialView(report) {
  const rows = (report?.rows || []).map((row) => {
    const code = String(row.accountCode ?? '');
    const name = row.accountName || '';
    return {
      code,
      name,
      groupName: row.groupName || '',
      parentCode: String(row.parentCode ?? ''),
      level: Number(row.level) || 1,
      numbers: trialRowNumbers(row),
      // Normalized once here; the search box filters on every keystroke.
      searchText: normalizeSearch(`${code} ${name}`),
    };
  });

  const byCode = new Map(rows.map((row) => [row.code, row]));
  const byCleanCode = new Map(rows.map((row) => [trialCleanCode(row.code), row]));
  const parentOf = new Map();
  const childrenOf = new Map();

  for (const row of rows) {
    let parent = '';
    // The backend sends the explicit parent; the prefix search only serves
    // older payloads that predate the field.
    const explicitParent = row.parentCode;
    if (explicitParent && explicitParent !== row.code && byCode.has(explicitParent)) {
      parent = explicitParent;
    } else {
      const clean = trialCleanCode(row.code);
      for (let length = clean.length - 1; length > 0; length -= 1) {
        const candidate = byCleanCode.get(clean.slice(0, length));
        if (candidate && candidate.code !== row.code && candidate.level < row.level) {
          parent = candidate.code;
          break;
        }
      }
    }
    parentOf.set(row.code, parent);
    if (parent) {
      if (!childrenOf.has(parent)) childrenOf.set(parent, []);
      childrenOf.get(parent).push(row.code);
    }
  }

  const roots = rows.filter((row) => !parentOf.get(row.code));
  const firstRoot = roots[0] || rows[0];
  const glWidth = firstRoot ? trialCleanCode(firstRoot.code).length : 3;

  const rootTotals = roots.reduce(
    (total, row) => ({
      od: total.od + row.numbers.od,
      oc: total.oc + row.numbers.oc,
      pd: total.pd + row.numbers.pd,
      pc: total.pc + row.numbers.pc,
      cd: total.cd + row.numbers.cd,
      cc: total.cc + row.numbers.cc,
    }),
    { od: 0n, oc: 0n, pd: 0n, pc: 0n, cd: 0n, cc: 0n },
  );

  return {
    rows,
    byCode,
    parentOf,
    childrenOf,
    roots,
    glWidth,
    rootTotals,
    visibleRows: [],
    visibleTotals: null,
  };
}

function trialValidateDrillPath(view) {
  while (trialState.drillPath.length) {
    const last = trialState.drillPath[trialState.drillPath.length - 1];
    if (view.byCode.has(last.code)) break;
    trialState.drillPath.pop();
  }
}

function trialCurrentParent() {
  return trialState.drillPath.length
    ? trialState.drillPath[trialState.drillPath.length - 1].code
    : '';
}

function trialIsDescendant(view, code, ancestorCode) {
  let current = view.parentOf.get(code) || '';
  const guard = new Set();
  while (current && !guard.has(current)) {
    if (current === ancestorCode) return true;
    guard.add(current);
    current = view.parentOf.get(current) || '';
  }
  return false;
}

function trialFilterRows(view) {
  const f = state.filters.trial;
  const query = normalizeSearch(trialState.search);
  const hasSearch = Boolean(query);
  const fromCode = digitsOnly(f.accountFrom || '');
  const toCode = digitsOnly(f.accountTo || '');
  const hasCodeRange = Boolean(fromCode || toCode);
  const currentParent = trialCurrentParent();
  const multiLevel = Boolean(f.multiLevel);

  let rows = view.rows;

  if (!hasSearch && !hasCodeRange) {
    if (multiLevel) {
      if (currentParent) {
        rows = rows.filter(
          (row) => row.code === currentParent || trialIsDescendant(view, row.code, currentParent),
        );
      }
    } else {
      rows = currentParent
        ? rows.filter((row) => view.parentOf.get(row.code) === currentParent)
        : view.roots;
    }
  }

  if (hasSearch) {
    rows = rows.filter((row) => row.searchText.includes(query));
  }

  if (!f.showZero) {
    rows = rows.filter((row) => row.numbers.cd !== 0n || row.numbers.cc !== 0n);
  }

  if (hasCodeRange) {
    const fromSegments = fromCode ? trialCodeSegments(fromCode, view.glWidth) : null;
    const toSegments = toCode ? trialCodeSegments(toCode, view.glWidth) : null;
    if (fromSegments) {
      rows = rows.filter(
        (row) => trialCompareSegments(trialCodeSegments(row.code, view.glWidth), fromSegments) >= 0,
      );
    }
    if (toSegments) {
      rows = rows.filter(
        (row) => trialCompareSegments(trialCodeSegments(row.code, view.glWidth), toSegments) <= 0,
      );
    }
  }

  return rows;
}

function trialTotals(view, rows) {
  const visibleCodes = new Set(rows.map((row) => row.code));
  const topRows = rows.filter((row) => {
    let parent = view.parentOf.get(row.code) || '';
    const guard = new Set();
    while (parent && !guard.has(parent)) {
      if (visibleCodes.has(parent)) return false;
      guard.add(parent);
      parent = view.parentOf.get(parent) || '';
    }
    return true;
  });

  return topRows.reduce(
    (total, row) => ({
      od: total.od + row.numbers.od,
      oc: total.oc + row.numbers.oc,
      pd: total.pd + row.numbers.pd,
      pc: total.pc + row.numbers.pc,
      cd: total.cd + row.numbers.cd,
      cc: total.cc + row.numbers.cc,
    }),
    { od: 0n, oc: 0n, pd: 0n, pc: 0n, cd: 0n, cc: 0n },
  );
}

function trialBalanceInfo(view) {
  const f = state.filters.trial;
  if (!view.rows.length) return { tone: 'neutral', label: '—' };
  if (digitsOnly(f.accountFrom || '') || digitsOnly(f.accountTo || '')) {
    return { tone: 'neutral', label: 'محدوده انتخاب‌شده' };
  }
  const totals = view.rootTotals;
  const balanced = totals.od === totals.oc && totals.pd === totals.pc && totals.cd === totals.cc;
  return balanced
    ? { tone: 'ok', label: '✓ تراز' }
    : { tone: 'not-ok', label: '⚠ نامتوازن' };
}

function trialHasActiveFilters() {
  const f = state.filters.trial;
  const defaults = defaultFilters().trial;
  return String(f.from || '') !== String(defaults.from || '')
    || String(f.to || '') !== String(defaults.to || '')
    || digitsOnly(f.accountFrom || '') !== ''
    || digitsOnly(f.accountTo || '') !== ''
    || (f.balanceType || 'شش ستونی') !== defaults.balanceType
    || Boolean(f.showZero) !== Boolean(defaults.showZero)
    || Boolean(f.multiLevel) !== Boolean(defaults.multiLevel)
    || Boolean(f.showAccountCode) !== Boolean(defaults.showAccountCode)
    || Boolean(f.showAccountName) !== Boolean(defaults.showAccountName)
    || Boolean(f.showPrintDateTime) !== Boolean(defaults.showPrintDateTime);
}

/* ── Columns and cells ──────────────────────────────────────── */

function trialColumns() {
  const f = state.filters.trial;
  const balanceType = TRIAL_BALANCE_TYPES.includes(f.balanceType) ? f.balanceType : 'شش ستونی';
  const isFourColumn = balanceType === 'چهار ستونی';
  const showOpening = balanceType === 'شش ستونی' || balanceType === 'هشت ستونی';
  const showAccumulated = balanceType === 'هشت ستونی';

  const columns = [{ key: 'row', title: 'ردیف', width: '48px', align: 'center' }];
  if (f.showAccountCode !== false) {
    columns.push({ key: 'code', title: 'کد حساب', width: '121px', align: 'center' });
  }
  if (f.showAccountName !== false) {
    columns.push({ key: 'name', title: 'نام حساب', align: 'start' });
  }
  if (showOpening) {
    columns.push({ key: 'openDebit', title: 'افتتاحیه بدهکار', align: 'amount' });
    columns.push({ key: 'openCredit', title: 'افتتاحیه بستانکار', align: 'amount' });
  }
  columns.push({
    key: 'debtTurnover',
    title: isFourColumn ? 'جمع بدهکار' : 'گردش بدهکار',
    align: 'amount',
  });
  columns.push({
    key: 'creditTurnover',
    title: isFourColumn ? 'جمع بستانکار' : 'گردش بستانکار',
    align: 'amount',
  });
  if (showAccumulated) {
    columns.push({ key: 'accumDebit', title: 'جمع دوره بدهکار', align: 'amount' });
    columns.push({ key: 'accumCredit', title: 'جمع دوره بستانکار', align: 'amount' });
  }
  columns.push({ key: 'debtBalance', title: 'مانده بدهکار', align: 'amount' });
  columns.push({ key: 'creditBalance', title: 'مانده بستانکار', align: 'amount' });
  return columns;
}

function trialDataTemplate(columns) {
  const parts = columns.map((column) => {
    if (column.width) return column.width;
    if (column.key === 'name') return 'minmax(150px, 2fr)';
    return 'minmax(64px, 1fr)';
  });
  return parts.join(' ');
}

function trialGridTemplate(columns) {
  return `${trialDataTemplate(columns)} var(--tbp-sb, 12px)`;
}

function trialIsFourColumn() {
  return state.filters.trial.balanceType === 'چهار ستونی';
}

function trialCellNumber(row, key) {
  const numbers = row.numbers;
  switch (key) {
    case 'openDebit': return numbers.od;
    case 'openCredit': return numbers.oc;
    case 'debtTurnover': return trialIsFourColumn() ? numbers.od + numbers.pd : numbers.pd;
    case 'creditTurnover': return trialIsFourColumn() ? numbers.oc + numbers.pc : numbers.pc;
    case 'accumDebit': return numbers.od + numbers.pd;
    case 'accumCredit': return numbers.oc + numbers.pc;
    case 'debtBalance': return numbers.cd;
    case 'creditBalance': return numbers.cc;
    default: return 0n;
  }
}

function trialDebitColumn(key) {
  return ['openDebit', 'debtTurnover', 'accumDebit', 'debtBalance'].includes(key);
}

function trialAmountClass(value, key) {
  if (value < 0n) return 'amount-negative';
  if (value === 0n) return 'amount-zero';
  return trialDebitColumn(key) ? 'amount-debit' : 'amount-credit';
}

/* ── Markup ─────────────────────────────────────────────────── */

function trialSearchBarMarkup() {
  const active = trialHasActiveFilters();
  const balance = trialView ? trialBalanceInfo(trialView) : { tone: 'neutral', label: '—' };
  return `
  <div class="tbp-search-bar" role="search">
    <label class="tbp-search-label" for="tbpSearch">جستجو:</label>
    <div class="tbp-search-wrap">
      <input id="tbpSearch" class="tbp-search-input" type="text" value="${esc(trialState.search)}"
        placeholder="نام حساب یا کد حساب..." autocomplete="off" spellcheck="false"
        aria-label="جستجوی حساب" />
    </div>
    ${active ? `<button type="button" class="tbp-filter-indicator" data-tbp-action="filter"
      title="مشاهده/ویرایش فیلترها" aria-controls="tbpFilterPanel" aria-expanded="${trialState.filterOpen}">فیلتر فعال</button>` : ''}
    <div class="tbp-balance-status ${balance.tone}" role="status" aria-live="polite">${esc(balance.label)}</div>
  </div>`;
}

function trialFilterPanelMarkup() {
  const f = state.filters.trial;
  const balanceType = TRIAL_BALANCE_TYPES.includes(f.balanceType) ? f.balanceType : 'شش ستونی';
  return `
  <fieldset id="tbpFilterPanel" class="tbp-filter-panel">
    <legend>تنظیمات گزارش و فیلتر</legend>
    <form class="tbp-filter-form" data-tbp-filter-form>
      <div class="tbp-filter-field">
        <label class="tbp-filter-lbl" for="tbpBalanceType">نوع تراز:</label>
        <select id="tbpBalanceType" name="balanceType" class="tbp-filter-sel">
          ${TRIAL_BALANCE_TYPES.map((type) => `<option value="${type}"${type === balanceType ? ' selected' : ''}>${type}</option>`).join('')}
        </select>
      </div>
      <div class="tbp-filter-field">
        <label class="tbp-filter-lbl" for="tbpFromDate">از تاریخ:</label>
        <input id="tbpFromDate" name="from" data-date class="tbp-filter-inp" type="text"
          inputmode="numeric" dir="ltr" value="${esc(f.from || '')}" placeholder="۱۴۰۵/۰۱/۰۱" />
      </div>
      <div class="tbp-filter-field">
        <label class="tbp-filter-lbl" for="tbpToDate">تا تاریخ:</label>
        <input id="tbpToDate" name="to" data-date class="tbp-filter-inp" type="text"
          inputmode="numeric" dir="ltr" value="${esc(f.to || '')}" placeholder="۱۴۰۵/۱۲/۲۹" />
      </div>
      <div class="tbp-filter-field">
        <label class="tbp-filter-lbl" for="tbpScope">محدوده اسناد:</label>
        <select id="tbpScope" name="scope" class="tbp-filter-sel">
          ${REPORT_SCOPES.map((scope) => `<option value="${scope.value}"${scope.value === normalizeReportScope(f.scope) ? ' selected' : ''}>${esc(scope.label)}</option>`).join('')}
        </select>
        <small class="tbp-filter-help">رسمی: فقط دائمی • کاری: دائمی و موقت • فقط موقت: فقط اسناد موقت</small>
      </div>
      <div class="tbp-filter-field">
        <label class="tbp-filter-lbl" for="tbpAccountFrom">از کد:</label>
        <input id="tbpAccountFrom" name="accountFrom" class="tbp-filter-inp" type="text"
          inputmode="numeric" dir="ltr" value="${esc(f.accountFrom || '')}" placeholder="ابتدای بازه" />
      </div>
      <div class="tbp-filter-field">
        <label class="tbp-filter-lbl" for="tbpAccountTo">تا کد:</label>
        <input id="tbpAccountTo" name="accountTo" class="tbp-filter-inp" type="text"
          inputmode="numeric" dir="ltr" value="${esc(f.accountTo || '')}" placeholder="انتهای بازه" />
      </div>
      <div class="tbp-filter-options" aria-label="گزینه‌های نمایش">
        <label class="tbp-filter-check">
          <input type="checkbox" name="showZero"${f.showZero ? ' checked' : ''} />
          نمایش اقلام با مانده صفر
        </label>
        <label class="tbp-filter-check">
          <input type="checkbox" name="showPrintDateTime"${f.showPrintDateTime ? ' checked' : ''} />
          نمایش ساعت و تاریخ چاپ
        </label>
        <label class="tbp-filter-check">
          <input type="checkbox" name="multiLevel"${f.multiLevel ? ' checked' : ''} />
          نمایش تراز به‌صورت چندسطحی
        </label>
      </div>
      <div class="tbp-filter-button-row">
        <button type="button" class="tbp-fbtn apply" data-tbp-action="apply-filter">اعمال فیلتر</button>
        <button type="button" class="tbp-fbtn cancel" data-tbp-action="cancel-filter">انصراف</button>
        <button type="button" class="tbp-fbtn clear" data-tbp-action="clear-filter">پاک کردن فیلتر</button>
      </div>
    </form>
  </fieldset>`;
}

function trialToolbarMarkup() {
  return `
  <div class="tbp-toolbar" role="toolbar" aria-label="عملیات گزارش تراز">
    <div class="tbp-toolbar-group" role="group" aria-label="عملیات گزارش">
      <button type="button" class="tbp-tbtn" data-tbp-action="refresh" title="به‌روزرسانی (F5)">
        <span class="tbp-tbtn-label">به‌روزرسانی</span>
      </button>
      <button type="button" class="tbp-tbtn" data-tbp-action="generate" title="تولید گزارش (PgDn)">
        <span class="tbp-tbtn-label">تولید گزارش</span>
      </button>
      <button type="button" class="tbp-tbtn${trialState.filterOpen ? ' pressed' : ''}" data-tbp-action="filter"
        title="فیلتر (Ins)" aria-pressed="${trialState.filterOpen}" aria-controls="tbpFilterPanel"
        aria-expanded="${trialState.filterOpen}">
        <span class="tbp-tbtn-label">فیلتر</span>
      </button>
      <button type="button" class="tbp-tbtn" data-tbp-action="print" title="چاپ (F7)">
        <span class="tbp-tbtn-label">چاپ</span>
      </button>
    </div>
    <div class="tbp-toolbar-group tbp-toolbar-close-group" role="group" aria-label="بستن فرم">
      <button type="button" class="tbp-back-btn" data-tbp-action="close" title="بازگشت (Escape)">
        <span class="tbp-tbtn-label">بازگشت</span>
      </button>
    </div>
  </div>`;
}

function trialFillerRows(columns, startIndex, count) {
  const template = trialDataTemplate(columns);
  let html = '';
  for (let index = 0; index < count; index += 1) {
    const even = (startIndex + index) % 2 === 1 ? ' even' : '';
    const cells = columns.map((column) => {
      if (column.key === 'row') return '<div class="tbp-dc col-center row-num" role="cell"></div>';
      if (column.key === 'code') return '<div class="tbp-dc col-center code-cell" role="cell"></div>';
      if (column.key === 'name') return '<div class="tbp-dc col-start" role="cell"></div>';
      return '<div class="tbp-dc col-amount col-mono amount-zero" role="cell"></div>';
    }).join('');
    html += `<div class="tbp-data-row${even}" role="row" aria-hidden="true" style="grid-template-columns:${template}">${cells}</div>`;
  }
  return html;
}

function trialGridMarkup(columns, rows, totals) {
  const dataTemplate = trialDataTemplate(columns);
  const gridTemplate = trialGridTemplate(columns);
  const header = columns.map((column) => {
    const nameClass = column.align === 'start' ? ' name-col' : '';
    return `<div class="tbp-gh-cell${nameClass}" role="columnheader" title="${esc(column.title)}">${esc(column.title)}</div>`;
  }).join('');

  const bodyRows = rows.map((row, index) => {
    const even = index % 2 === 1 ? ' even' : '';
    const selected = row.code === trialState.selectedCode ? ' selected' : '';
    const drillable = trialView.childrenOf.has(row.code) ? ' drillable' : '';
    const cells = columns.map((column) => {
      if (column.key === 'row') {
        return `<div class="tbp-dc col-center row-num" role="cell">${toFa(index + 1)}</div>`;
      }
      if (column.key === 'code') {
        return `<div class="tbp-dc col-center code-cell" dir="ltr" role="cell">${toFa(row.code)}</div>`;
      }
      if (column.key === 'name') {
        return `<div class="tbp-dc col-start tbp-name-cell" role="cell"><span>${esc(row.name)}</span></div>`;
      }
      const value = trialCellNumber(row, column.key);
      return `<div class="tbp-dc col-amount col-mono ${trialAmountClass(value, column.key)}" role="cell">${trialFormatAmount(value)}</div>`;
    }).join('');
    return `
    <div class="tbp-data-row${even}${selected}${drillable}" role="row" data-code="${esc(row.code)}"
      tabindex="${selected ? '0' : '-1'}" aria-selected="${selected ? 'true' : 'false'}"
      style="grid-template-columns:${dataTemplate}">${cells}</div>`;
  }).join('');

  const fillerCount = Math.max(0, 30 - rows.length);
  const filler = fillerCount ? trialFillerRows(columns, rows.length, fillerCount) : '';
  const body = rows.length
    ? `${bodyRows}${filler}`
    : '<div class="tbp-state-row"><div class="tbp-state-text">داده‌ای یافت نشد</div></div>';

  let totalsHtml = '';
  if (rows.length) {
    const labelSpan = columns.filter((column) => column.key === 'code' || column.key === 'name').length;
    let labelWritten = false;
    const totalCells = columns.map((column) => {
      if (column.key === 'row') return '<div class="tbp-tc total-label"></div>';
      if (column.key === 'code' || column.key === 'name') {
        if (labelWritten) return '';
        labelWritten = true;
        return `<div class="tbp-tc total-label" style="grid-column:span ${Math.max(labelSpan, 1)}"></div>`;
      }
      const value = trialTotalValue(column.key, totals);
      return `<div class="tbp-tc total-amount">${trialFormatAmount(value)}</div>`;
    }).join('');
    totalsHtml = `
    <div class="tbp-totals-row" role="row" style="grid-template-columns:${gridTemplate}">
      ${totalCells}<div class="tbp-sb-spacer" role="presentation"></div>
    </div>`;
  }

  return `
  <div class="tbp-grid-header-row" role="row" style="grid-template-columns:${gridTemplate}">
    ${header}<div class="tbp-sb-spacer" role="presentation"></div>
  </div>
  <div class="tbp-grid-body" role="rowgroup">${body}</div>
  ${totalsHtml}`;
}

function trialTotalValue(key, totals) {
  const four = trialIsFourColumn();
  switch (key) {
    case 'openDebit': return totals.od;
    case 'openCredit': return totals.oc;
    case 'debtTurnover': return four ? totals.od + totals.pd : totals.pd;
    case 'creditTurnover': return four ? totals.oc + totals.pc : totals.pc;
    case 'accumDebit': return totals.od + totals.pd;
    case 'accumCredit': return totals.oc + totals.pc;
    case 'debtBalance': return totals.cd;
    case 'creditBalance': return totals.cc;
    default: return 0n;
  }
}

/* ── Rendering and interaction ──────────────────────────────── */

function renderTrialBalance(report) {
  trialView = createTrialView(report);
  trialValidateDrillPath(trialView);
  return `
  <div class="tbp-shell">
    ${trialSearchBarMarkup()}
    <div class="tbp-content">
      <div id="tbpGrid" class="tbp-grid-container" role="grid" aria-label="جدول تراز آزمایشی حساب‌ها"></div>
    </div>
    ${trialToolbarMarkup()}
  </div>`;
}

function trialGridParts() {
  if (!trialView) return null;
  if (!document.getElementById('tbpGrid')) return null;
  return document.getElementById('tbpGrid');
}

function trialSyncScrollbarWidth() {
  // Resize events fire in bursts; measure at most once per frame.
  if (trialScrollbarFrame) return;
  trialScrollbarFrame = requestAnimationFrame(() => {
    trialScrollbarFrame = 0;
    const grid = document.getElementById('tbpGrid');
    const body = grid?.querySelector('.tbp-grid-body');
    if (!grid || !body) return;
    grid.style.setProperty('--tbp-sb', `${Math.max(0, body.offsetWidth - body.clientWidth)}px`);
  });
}

function trialEnsureSelection(rows) {
  if (!rows.length) {
    trialState.selectedCode = '';
    return;
  }
  if (!rows.some((row) => row.code === trialState.selectedCode)) {
    trialState.selectedCode = rows[0].code;
  }
}

function trialRefreshGrid({ resetScroll = false, focusSelected = false } = {}) {
  const grid = trialGridParts();
  if (!grid || !trialView) return;

  const columns = trialColumns();
  const rows = trialFilterRows(trialView);
  trialEnsureSelection(rows);
  trialView.visibleRows = rows;
  trialView.visibleTotals = trialTotals(trialView, rows);
  grid.innerHTML = trialGridMarkup(columns, rows, trialView.visibleTotals);

  const body = grid.querySelector('.tbp-grid-body');
  if (resetScroll && body) body.scrollTop = 0;
  trialSyncScrollbarWidth();
  if (focusSelected) trialFocusSelectedRow();
}

function trialSelectedElement() {
  const grid = document.getElementById('tbpGrid');
  if (!grid) return null;
  return Array.from(grid.querySelectorAll('.tbp-data-row[data-code]'))
    .find((element) => element.dataset.code === trialState.selectedCode) || null;
}

function trialFocusSelectedRow() {
  const element = trialSelectedElement();
  if (!element) return;
  element.focus({ preventScroll: true });
  element.scrollIntoView({ block: 'nearest' });
}

function trialSelectCode(code, focus = false) {
  if (!trialView?.visibleRows.some((row) => row.code === code)) return;
  trialState.selectedCode = code;
  const grid = document.getElementById('tbpGrid');
  if (!grid) return;
  for (const element of grid.querySelectorAll('.tbp-data-row[data-code]')) {
    const selected = element.dataset.code === code;
    element.classList.toggle('selected', selected);
    element.setAttribute('aria-selected', selected ? 'true' : 'false');
    element.tabIndex = selected ? 0 : -1;
  }
  if (focus) trialFocusSelectedRow();
}

function trialMoveSelection(step) {
  const rows = trialView?.visibleRows || [];
  if (!rows.length) return;
  const currentIndex = rows.findIndex((row) => row.code === trialState.selectedCode);
  let nextIndex;
  if (currentIndex === -1) nextIndex = step > 0 ? 0 : rows.length - 1;
  else nextIndex = Math.max(0, Math.min(rows.length - 1, currentIndex + step));
  trialSelectCode(rows[nextIndex].code, true);
}

function trialDrillInto(code) {
  if (!trialView || !trialView.childrenOf.has(code)) return;
  const path = [];
  const guard = new Set();
  let current = code;
  while (current && !guard.has(current)) {
    guard.add(current);
    const row = trialView.byCode.get(current);
    if (!row) break;
    path.unshift({ code: row.code, name: row.name });
    current = trialView.parentOf.get(current) || '';
  }
  trialState.drillPath = path;
  trialState.selectedCode = '';
  trialRefreshGrid({ resetScroll: true });
}

function trialDrillUp() {
  if (!trialState.drillPath.length) return;
  trialState.drillPath.pop();
  trialState.selectedCode = '';
  trialRefreshGrid({ resetScroll: true });
}

function trialDrillSelected() {
  const row = trialView?.visibleRows.find((item) => item.code === trialState.selectedCode);
  if (row) trialDrillInto(row.code);
}

function trialSetFilterPanel(open) {
  const content = document.querySelector('.tbp-content');
  if (!content) return;
  const existing = content.querySelector('#tbpFilterPanel');
  if (open && !existing) content.insertAdjacentHTML('beforeend', trialFilterPanelMarkup());
  if (!open && existing) existing.remove();
  trialState.filterOpen = open;

  for (const button of document.querySelectorAll('[data-tbp-action="filter"]')) {
    button.classList.toggle('pressed', open);
    button.setAttribute('aria-expanded', open ? 'true' : 'false');
    if (button.classList.contains('tbp-tbtn')) {
      button.setAttribute('aria-pressed', open ? 'true' : 'false');
    }
  }

  if (open) {
    content.querySelector('#tbpFilterPanel [name="from"]')?.focus();
  } else {
    document.querySelector('.tbp-tbtn[data-tbp-action="filter"]')?.focus();
  }
}

function trialApplyFilter() {
  const form = document.querySelector('#tbpFilterPanel [data-tbp-filter-form]');
  if (!form) return;
  if (!form.reportValidity()) return;
  if (!validateDateFields(form)) {
    toast('تاریخ واردشده معتبر نیست.', 'error');
    return;
  }
  const data = new FormData(form);
  const f = state.filters.trial;
  f.from = padJalaliDate(String(data.get('from') || ''));
  f.to = padJalaliDate(String(data.get('to') || ''));
  f.accountFrom = digitsOnly(String(data.get('accountFrom') || ''));
  f.accountTo = digitsOnly(String(data.get('accountTo') || ''));
  f.scope = normalizeReportScope(data.get('scope'));
  f.balanceType = TRIAL_BALANCE_TYPES.includes(String(data.get('balanceType')))
    ? String(data.get('balanceType'))
    : 'شش ستونی';
  f.showZero = form.querySelector('[name="showZero"]').checked;
  f.showPrintDateTime = form.querySelector('[name="showPrintDateTime"]').checked;
  f.multiLevel = form.querySelector('[name="multiLevel"]').checked;
  trialState.filterOpen = false;
  trialState.selectedCode = '';
  navigate('trial', { force: true });
}

function trialClearFilter() {
  Object.assign(state.filters.trial, defaultFilters().trial);
  trialState.filterOpen = false;
  trialState.drillPath = [];
  trialState.selectedCode = '';
  navigate('trial', { force: true });
}

async function trialGenerateReport() {
  if (trialState.filterOpen) {
    trialApplyFilter();
    return;
  }
  // The message must follow the load: a slow or failed report used to say
  // «generated» immediately and then show an error (or nothing) later.
  const loaded = await navigate('trial', { force: true });
  if (loaded) toast('گزارش تراز آزمایشی تولید شد.', 'ok');
}

function trialRefreshReport() {
  navigate('trial', { force: true });
}

function trialPrintReport() {
  printWithFeedback('open_trial_balance_preview', trialPrintRequest());
}

async function trialCloseWindow() {
  try {
    if (currentWindow && typeof currentWindow.close === 'function') {
      await currentWindow.close();
      return;
    }
  } catch (_) { /* fall through to window.close */ }
  window.close();
}

function trialRunAction(action) {
  switch (action) {
    case 'refresh': trialRefreshReport(); break;
    case 'generate': void trialGenerateReport(); break;
    case 'filter': trialSetFilterPanel(!trialState.filterOpen); break;
    case 'print': trialPrintReport(); break;
    case 'close': void trialCloseWindow(); break;
    case 'apply-filter': trialApplyFilter(); break;
    case 'cancel-filter': trialSetFilterPanel(false); break;
    case 'clear-filter': trialClearFilter(); break;
    default: break;
  }
}

function trialHandleKeydown(event) {
  if (!trialView || !document.getElementById('tbpGrid')) return;
  const tag = event.target?.tagName;
  const isInput = tag === 'INPUT' || tag === 'SELECT' || tag === 'TEXTAREA' || event.target?.isContentEditable;

  switch (event.key) {
    case 'Escape':
      event.preventDefault();
      if (trialState.filterOpen) {
        trialSetFilterPanel(false);
      } else if (trialState.drillPath.length) {
        trialDrillUp();
      } else {
        void trialCloseWindow();
      }
      break;
    case 'F5':
      event.preventDefault();
      trialRefreshReport();
      break;
    case 'PageDown':
      if (!isInput) {
        event.preventDefault();
        void trialGenerateReport();
      }
      break;
    case 'Insert':
      if (!isInput) {
        event.preventDefault();
        trialSetFilterPanel(!trialState.filterOpen);
      }
      break;
    case 'F7':
      if (!isInput) {
        event.preventDefault();
        trialPrintReport();
      }
      break;
    case 'ArrowUp':
    case 'ArrowDown':
      // Text fields keep their native caret movement; row navigation only
      // applies when focus is on the report grid or the page itself.
      if (isInput) break;
      event.preventDefault();
      trialMoveSelection(event.key === 'ArrowDown' ? 1 : -1);
      break;
    case 'Enter':
      if (isInput) break;
      event.preventDefault();
      trialDrillSelected();
      break;
    case 'Backspace':
      if (isInput) break;
      if (trialState.drillPath.length) {
        event.preventDefault();
        trialDrillUp();
      }
      break;
    default:
      break;
  }
}

function bindTrialBalance(root) {
  trialAbort?.abort();
  trialAbort = new AbortController();
  const { signal } = trialAbort;

  const search = root.querySelector('#tbpSearch');
  search?.addEventListener('input', () => {
    trialState.search = search.value;
    clearTimeout(trialSearchTimer);
    trialSearchTimer = setTimeout(() => {
      trialRefreshGrid({ resetScroll: true });
    }, 300);
  }, { signal });

  root.addEventListener('click', (event) => {
    const action = event.target.closest('[data-tbp-action]');
    if (action) {
      event.preventDefault();
      trialRunAction(action.dataset.tbpAction);
      return;
    }
    const rowElement = event.target.closest('.tbp-data-row[data-code]');
    if (rowElement) trialSelectCode(rowElement.dataset.code, true);
  }, { signal });

  root.addEventListener('dblclick', (event) => {
    const rowElement = event.target.closest('.tbp-data-row[data-code]');
    if (!rowElement) return;
    trialDrillInto(rowElement.dataset.code);
  }, { signal });

  // Input hygiene for the inline filter panel.
  root.addEventListener('input', (event) => {
    const input = event.target;
    if (!(input instanceof HTMLInputElement)) return;
    if (input.matches('#tbpFilterPanel input[data-date]')) {
      clearFieldError(input);
      const cleaned = toEn(input.value).replace(/[^\d/]/g, '');
      if (input.value !== cleaned) input.value = cleaned;
    } else if (input.matches('#tbpFilterPanel [name="accountFrom"], #tbpFilterPanel [name="accountTo"]')) {
      const cleaned = digitsOnly(input.value);
      if (input.value !== cleaned) input.value = cleaned;
    }
  }, { signal });

  root.addEventListener('submit', (event) => {
    if (!event.target.matches('#tbpFilterPanel [data-tbp-filter-form]')) return;
    event.preventDefault();
    trialApplyFilter();
  }, { signal });

  document.addEventListener('keydown', trialHandleKeydown, { signal });
  window.addEventListener('resize', trialSyncScrollbarWidth, { signal });

  trialRefreshGrid({ resetScroll: true });
  search?.focus();
}
