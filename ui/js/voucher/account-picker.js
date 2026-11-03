'use strict';

/* ── Account picker ─────────────────────────────────────────── */

let apTarget = null;
let apIndex = 0;
let apVisible = [];

const accountPickerOpen = () => {
  const modal = $('#accountPickerModal');
  return !!modal && !modal.hidden;
};

function filterAccountPicker(term) {
  const q = normalizeSearch(term);
  const parent = state.accountPickerPath[state.accountPickerPath.length - 1] || null;
  const selectable = state.accounts.filter((account) => account.isActive !== false || voucherAlreadyUsesAccount(account.code));
  let source = selectable;
  if (!q && !state.accountPickerShowAll) {
    source = parent
      ? selectable.filter((account) => digitsOnly(account.parentCode) === digitsOnly(parent.code))
      : selectable.filter((account) => Number(account.level) === 1 || !account.parentCode);
  }
  if (!q) return source.slice();
  return selectable.filter((account) =>
    normalizeSearch(account.code).includes(q) ||
    normalizeSearch(account.name).includes(q) ||
    normalizeSearch(levelLabel(account.level)).includes(q) ||
    normalizeSearch(account.parentName).includes(q) ||
    normalizeSearch(account.groupName).includes(q)
  );
}

function renderAccountPickerPath() {
  const path = $('#accountPickerPath');
  if (!path) return;
  const labels = ['همه حساب‌های کل'].concat(state.accountPickerPath.map((account) =>
    vfFa(account.code) + ' — ' + account.name
  ));
  path.textContent = labels.join('  ‹  ');
}

function apCell(text, className, dir) {
  const cell = document.createElement('div');
  cell.className = `ap-cell ${className}`;
  if (dir) cell.dir = dir;
  cell.setAttribute('role', 'gridcell');
  cell.textContent = text;
  return cell;
}

function renderAccountPickerRows(resetIndex = true) {
  const box = $('#accountPickerRows');
  if (!box) return;
  const term = $('#accountPickerSearch') ? $('#accountPickerSearch').value : '';
  apVisible = filterAccountPicker(term);
  renderAccountPickerPath();
  if (resetIndex) apIndex = 0;
  if (apIndex > apVisible.length - 1) apIndex = Math.max(0, apVisible.length - 1);

  box.textContent = '';
  const fragment = document.createDocumentFragment();

  if (!state.accounts.length) {
    box.append(apCell('هنوز حسابی تعریف نشده است؛ از صفحه «حساب‌ها» حساب ایجاد کنید.', 'ap-nostate-cell'));
    return;
  }

  if (!apVisible.length) {
    const row = document.createElement('div');
    row.className = 'ap-row ap-nostate';
    row.setAttribute('role', 'row');
    const cell = document.createElement('div');
    cell.className = 'ap-cell ap-nostate-cell';
    cell.setAttribute('role', 'gridcell');
    cell.textContent = 'نتیجه‌ای برای این جستجو یافت نشد';
    row.append(cell);
    fragment.append(row);
    box.append(fragment);
    return;
  }

  apVisible.forEach((account, i) => {
    const row = document.createElement('div');
    row.className = 'ap-row';
    if (i === apIndex) row.classList.add('selected');
    if (!account.isPostable) row.classList.add('not-postable');
    if (account.isActive === false) row.classList.add('inactive');
    row.setAttribute('role', 'row');
    row.tabIndex = -1;
    row.dataset.pickerIndex = String(i);
    row.setAttribute('aria-selected', i === apIndex ? 'true' : 'false');
    row.title = account.isActive === false
      ? 'این حساب غیرفعال است و فقط برای ردیف‌های ذخیره‌شدهٔ همین سند قابل استفاده است'
      : account.isPostable
        ? `${account.code} — ${account.name}`
        : 'این حساب قابل ثبت نیست؛ یک حساب معین یا تفصیلی ۱ قابل ثبت انتخاب کنید';

    row.append(
      apCell(vfFa(i + 1), 'rownum center'),
      apCell(vfFa(account.code), 'code center', 'ltr'),
      apCell(account.name, 'start'),
      apCell(levelLabel(account.level), 'level center'),
      apCell(account.parentName || '—', 'start')
    );
    fragment.append(row);
  });

  // Rows that keep the table pattern even when the filter is narrow.
  const fillers = Math.max(0, 8 - apVisible.length);
  for (let i = 0; i < fillers; i += 1) {
    const row = document.createElement('div');
    row.className = 'ap-row empty';
    row.setAttribute('role', 'presentation');
    row.setAttribute('aria-hidden', 'true');
    for (let c = 0; c < 5; c += 1) {
      const cell = document.createElement('div');
      cell.className = 'ap-cell';
      row.append(cell);
    }
    fragment.append(row);
  }

  box.append(fragment);

  const selected = box.querySelector('.ap-row.selected');
  if (selected) selected.scrollIntoView({ block: 'nearest' });
}

function highlightAccountPickerRow(index) {
  const box = $('#accountPickerRows');
  if (!box) return;
  const rows = $$('.ap-row[data-picker-index]', box);
  if (!rows.length) return;
  apIndex = Math.max(0, Math.min(index, rows.length - 1));
  rows.forEach((row, i) => {
    row.classList.toggle('selected', i === apIndex);
    row.setAttribute('aria-selected', i === apIndex ? 'true' : 'false');
  });
  rows[apIndex].scrollIntoView({ block: 'nearest' });
}

function openAccountPicker(input) {
  const modal = $('#accountPickerModal');
  if (!modal) return;
  if (!state.accounts.length) {
    toast('هنوز حسابی تعریف نشده است؛ ابتدا از صفحه «حساب‌ها» حساب بسازید.', 'info');
    return;
  }

  apTarget = input || null;
  state.accountPickerPath = [];
  state.accountPickerShowAll = false;
  const search = $('#accountPickerSearch');
  const code = input && input.classList.contains('vf-code-input') ? digitsOnly(input.value) : '';
  if (search) search.value = code;

  apIndex = 0;
  renderAccountPickerRows();
  const exact = apVisible.findIndex((account) => account.code === code);
  if (exact > 0) {
    apIndex = exact;
    renderAccountPickerRows(false);
  }

  modal.hidden = false;
  const main = $('.voucher-window');
  if (main) main.inert = true;

  if (search) {
    search.focus();
    search.select();
  }
  const selected = $('#accountPickerRows .ap-row.selected');
  if (selected) selected.scrollIntoView({ block: 'nearest' });
}

function drillOrChooseAccountPickerRow(account) {
  if (!account) return;
  const children = state.accounts.filter((item) =>
    digitsOnly(item.parentCode) === digitsOnly(account.code)
  );
  if (children.length) {
    state.accountPickerPath.push(account);
    const search = $('#accountPickerSearch');
    if (search) search.value = '';
    apIndex = 0;
    renderAccountPickerRows();
    search?.focus();
    return;
  }
  chooseAccountPickerRow();
}

function closeAccountPicker(refocus = false) {
  const modal = $('#accountPickerModal');
  if (!modal || modal.hidden) return;
  modal.hidden = true;
  const main = $('.voucher-window');
  if (main) main.inert = false;

  const target = apTarget;
  apTarget = null;
  if (refocus && target && target.isConnected && !target.disabled) target.focus();
}

function chooseAccountPickerRow() {
  const account = apVisible[apIndex];
  if (!account) return;

  if (!accountAllowedForNewLine(account, apTarget?.closest?.('tr') || null)) {
    const hasChildren = state.accounts.some((item) =>
      digitsOnly(item.parentCode) === digitsOnly(account.code)
    );
    if (hasChildren) {
      drillOrChooseAccountPickerRow(account);
      return;
    }
    const reason = account.isActive === false
      ? 'غیرفعال است'
      : `${levelLabel(account.level)} قابل ثبت نیست`;
    toast(`حساب «${account.name}» ${reason}؛ یک حساب معین یا تفصیلی ۱ فعال و قابل ثبت انتخاب کنید.`, 'error');
    return;
  }

  const target = apTarget;
  if (target && target.isConnected) {
    target.value = account.code; // ASCII; the FD font draws Persian digits
    target.dispatchEvent(new Event('input', { bubbles: true }));
    updateRowAccount(target.closest('tr'), true);
    updateLedger(target.closest('tr'));
  }
  closeAccountPicker(true);
}

function trapPickerFocus(ev) {
  if (ev.key !== 'Tab') return;
  const focusables = $$(
    '.ap-window button:not([disabled]), .ap-window input:not([disabled]), .ap-window [tabindex]:not([tabindex="-1"])',
    $('#accountPickerModal')
  ).filter((el) => el.offsetParent !== null);
  if (!focusables.length) return;
  const first = focusables[0];
  const last = focusables[focusables.length - 1];
  if (ev.shiftKey && document.activeElement === first) {
    ev.preventDefault();
    last.focus();
  } else if (!ev.shiftKey && document.activeElement === last) {
    ev.preventDefault();
    first.focus();
  }
}

function bindAccountPicker() {
  const modal = $('#accountPickerModal');
  const rows = $('#accountPickerRows');
  const search = $('#accountPickerSearch');
  if (!modal || !rows) return;

  const closeBtn = $('#btnAccountPickerClose');
  if (closeBtn) closeBtn.addEventListener('click', () => closeAccountPicker(true));
  const backBtn = $('#accountPickerBack');
  if (backBtn) backBtn.addEventListener('click', () => {
    if (state.accountPickerPath.length) {
      state.accountPickerPath.pop();
      state.accountPickerShowAll = false;
      if (search) search.value = '';
      apIndex = 0;
      renderAccountPickerRows();
      search?.focus();
    } else {
      closeAccountPicker(true);
    }
  });
  modal.addEventListener('click', (ev) => {
    if (ev.target.id === 'accountPickerModal') closeAccountPicker(true);
  });

  if (search) {
    let apSearchDebounceTimer = 0;
    search.addEventListener('input', () => {
      clearTimeout(apSearchDebounceTimer);
      apSearchDebounceTimer = setTimeout(() => renderAccountPickerRows(), 120);
    });
  }

  rows.addEventListener('click', (ev) => {
    const row = ev.target.closest('.ap-row[data-picker-index]');
    if (!row) return;
    apIndex = Number(row.dataset.pickerIndex);
    highlightAccountPickerRow(apIndex);
  });
  rows.addEventListener('dblclick', (ev) => {
    const row = ev.target.closest('.ap-row[data-picker-index]');
    if (!row) return;
    apIndex = Number(row.dataset.pickerIndex);
    highlightAccountPickerRow(apIndex);
    drillOrChooseAccountPickerRow(apVisible[apIndex]);
  });

  const selectBtn = $('#accountPickerSelect');
  if (selectBtn) selectBtn.addEventListener('click', chooseAccountPickerRow);

  const clearBtn = modal.querySelector('[data-picker-act="clear"]');
  const showAllAccounts = () => {
    state.accountPickerPath = [];
    state.accountPickerShowAll = true;
    if (search) search.value = '';
    renderAccountPickerRows();
    if (search) search.focus();
  };
  if (clearBtn) {
    clearBtn.addEventListener('click', showAllAccounts);
  }

  modal.addEventListener('keydown', (ev) => {
    trapPickerFocus(ev);
    if (ev.defaultPrevented || ev.ctrlKey || ev.altKey || ev.metaKey) return;

    const inField = ev.target.closest && ev.target.closest('input, textarea, select');
    const nativeControl = ev.target.closest && ev.target.closest('button, a[href], [role="button"]');
    if (nativeControl && (ev.key === 'Enter' || ev.key === ' ')) return;

    const step = Math.max(1, Math.floor(rows.clientHeight / 37) - 1);
    switch (ev.key) {
      case 'ArrowDown':
        ev.preventDefault();
        highlightAccountPickerRow(apIndex + 1);
        break;
      case 'ArrowUp':
        ev.preventDefault();
        highlightAccountPickerRow(apIndex - 1);
        break;
      case 'PageDown':
        ev.preventDefault();
        highlightAccountPickerRow(apIndex + step);
        break;
      case 'PageUp':
        ev.preventDefault();
        highlightAccountPickerRow(apIndex - step);
        break;
      case 'Home':
        if (!inField && (!search || !search.value)) {
          ev.preventDefault();
          highlightAccountPickerRow(0);
        }
        break;
      case 'End':
        if (!inField && (!search || !search.value)) {
          ev.preventDefault();
          highlightAccountPickerRow(apVisible.length - 1);
        }
        break;
      case 'Enter':
        ev.preventDefault();
        chooseAccountPickerRow();
        break;
      case 'F2':
        ev.preventDefault();
        if (apVisible[apIndex] && state.accounts.some((item) =>
          digitsOnly(item.parentCode) === digitsOnly(apVisible[apIndex].code))) {
          drillOrChooseAccountPickerRow(apVisible[apIndex]);
        } else {
          chooseAccountPickerRow();
        }
        break;
      case 'F5':
        ev.preventDefault();
        if (search) search.focus();
        break;
      case 'F6':
        ev.preventDefault();
        showAllAccounts();
        break;
      default:
        break;
    }
  });
}

