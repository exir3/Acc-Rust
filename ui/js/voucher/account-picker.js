'use strict';

/* ── Account picker ─────────────────────────────────────────── */

const accountPickerCore = globalThis.AccountPickerCore;

let apTarget = null;
let apIndex = 0;
let apVisible = [];
let apMode = 'base';

const accountPickerOpen = () => {
  const modal = $('#accountPickerModal');
  return !!modal && !modal.hidden;
};

function filterAccountPicker(term) {
  const q = normalizeSearch(term);
  const row = apTarget?.closest?.('tr') || null;
  const baseCode = digitsOnly(row?.querySelector('.vf-code-input')?.value || '');
  const savedException = (account) => accountChainActive(account) ||
    (apMode === 'detail'
      ? digitsOnly(row?.dataset.savedDetail1 || '') === digitsOnly(account.code)
      : voucherAlreadyUsesAccount(account.code));
  const options = {
    mode: apMode,
    baseCode,
    path: state.accountPickerPath,
    showAll: state.accountPickerShowAll,
  };
  const source = (q
    ? accountPickerCore.searchPool(state.accounts, options)
    : accountPickerCore.visibleAccounts(state.accounts, options)
  ).filter(savedException);
  if (!q) return source.slice();
  return source.filter((account) =>
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
  const search = $('#accountPickerSearch');
  const searching = !!search && search.value.trim() !== '';
  const rootLabel = apMode === 'detail'
    ? 'تفصیلی‌های معین انتخاب‌شده'
    : state.accountPickerShowAll
      ? 'همه حساب‌های پایه (کل/معین)'
      : 'حساب‌های کل';
  const labels = searching
    ? [apMode === 'detail'
      ? 'نتایج جستجو در تفصیلی‌های معین انتخاب‌شده'
      : 'نتایج جستجو در همه حساب‌ها']
    : [rootLabel].concat(state.accountPickerPath.map((account) =>
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
    cell.textContent = term.trim()
      ? 'نتیجه‌ای برای این جستجو یافت نشد'
      : 'حسابی در این سطح تعریف نشده است';
    row.append(cell);
    fragment.append(row);
    box.append(fragment);
    return;
  }

  apVisible.forEach((account, i) => {
    const row = document.createElement('div');
    row.className = 'ap-row';
    if (i === apIndex) row.classList.add('selected');
    if (apMode !== 'detail' && !account.isPostable) row.classList.add('not-postable');
    const effectivelyActive = accountChainActive(account);
    if (!effectivelyActive) row.classList.add('inactive');
    row.setAttribute('role', 'row');
    row.tabIndex = -1;
    row.dataset.pickerIndex = String(i);
    row.setAttribute('aria-selected', i === apIndex ? 'true' : 'false');
    row.title = !effectivelyActive
      ? 'این حساب یا یکی از والدین آن غیرفعال است و فقط برای ردیف‌های ذخیره‌شدهٔ همین سند قابل استفاده است'
      : apMode === 'detail'
        ? `${account.code} — ${account.name}`
        : account.isPostable
          ? `${account.code} — ${account.name}`
          : 'این حساب پایه قابل ثبت نیست؛ یک حساب کل یا معین فعال انتخاب کنید';

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
  if (isLockedVoucher()) {
    toast('سند دائمی است و حساب‌های ردیف‌های آن قابل تغییر نیستند.', 'error');
    return;
  }
  const modal = $('#accountPickerModal');
  if (!modal) return;
  if (!state.accounts.length) {
    toast('هنوز حسابی تعریف نشده است؛ ابتدا از صفحه «حساب‌ها» حساب بسازید.', 'info');
    return;
  }

  apTarget = input || null;
  apMode = input?.classList.contains('vf-detail1-input') ? 'detail' : 'base';
  const search = $('#accountPickerSearch');
  const row = input?.closest?.('tr');
  const code = input ? digitsOnly(input.value) : '';
  if (apMode === 'detail' && !digitsOnly(row?.querySelector('.vf-code-input')?.value || '')) {
    toast('ابتدا حساب پایه (کل یا معین) را انتخاب کنید.', 'info');
    return;
  }

  // Open on the level that contains the current account so the list starts
  // as its siblings, not as a flat list of every account.
  const current = code ? accountForCode(code) : null;
  state.accountPickerPath = apMode === 'base' && current
    ? accountPickerCore.ancestors(current, state.accounts)
    : [];
  state.accountPickerShowAll = false;
  if (search) search.value = '';

  apIndex = 0;
  renderAccountPickerRows();
  const exact = apVisible.findIndex((account) =>
    digitsOnly(account.code) === digitsOnly(code));
  if (exact >= 0) {
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
  if (accountPickerCore.canDrill(state.accounts, account, apMode)) {
    state.accountPickerPath = accountPickerCore.ancestors(account, state.accounts)
      .concat(account);
    state.accountPickerShowAll = false;
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
  if (isLockedVoucher()) {
    closeAccountPicker(false);
    toast('سند دائمی است و حساب‌های ردیف‌های آن قابل تغییر نیستند.', 'error');
    return;
  }
  const account = apVisible[apIndex];
  if (!account || !apTarget) return;
  const target = apTarget;
  const row = target.closest('tr');
  const baseInput = row?.querySelector('.vf-code-input');
  const baseCode = digitsOnly(baseInput?.value || '');
  let selectedCode = account.code;

  if (apMode === 'detail') {
    if (Number(account.level) !== 3 || digitsOnly(account.parentCode) !== baseCode) {
      toast('تفصیلی انتخاب‌شده فرزند مستقیم حساب معین این ردیف نیست.', 'error');
      return;
    }
    const savedSameDetail = !!row?.dataset.lineId
      && digitsOnly(row.dataset.savedDetail1 || '') === digitsOnly(account.code);
    if (!accountChainActive(account) && !savedSameDetail) {
      toast('حساب تفصیلی یا یکی از والدین آن غیرفعال است و برای ردیف جدید قابل استفاده نیست.', 'error');
      return;
    }
  } else if (Number(account.level) === 3) {
    // Drilling into a معین shows its details; choosing one fills both the
    // base and detail columns so the saved row keeps a valid pair.
    const parent = accountForCode(account.parentCode);
    const detailInput = row?.querySelector('.vf-detail1-input');
    const savedSameDetail = !!row?.dataset.lineId
      && digitsOnly(row.dataset.savedDetail1 || '') === digitsOnly(account.code);
    if (!parent || Number(parent.level) !== 2 || !accountAllowedForNewLine(parent, row)) {
      toast('حساب معین والد این تفصیلی برای ثبت سند معتبر نیست.', 'error');
      return;
    }
    if (!accountChainActive(account) && !savedSameDetail) {
      toast('حساب تفصیلی یا یکی از والدین آن غیرفعال است و برای ردیف جدید قابل استفاده نیست.', 'error');
      return;
    }
    if (!baseInput || !detailInput) {
      toast('ردیف سند برای انتخاب تفصیلی آماده نیست.', 'error');
      return;
    }
    baseInput.value = parent.code;
    detailInput.value = account.code;
    selectedCode = parent.code;
  } else {
    if (![1, 2].includes(Number(account.level)) || !accountAllowedForNewLine(account, row)) {
      toast('برای ثبت سند، یک حساب کل یا معین فعال انتخاب کنید.', 'error');
      return;
    }
    if (baseInput && digitsOnly(baseInput.value) !== digitsOnly(account.code)) {
      baseInput.value = account.code;
      const detailInput = row?.querySelector('.vf-detail1-input');
      if (detailInput) detailInput.value = '';
    }
  }

  target.value = selectedCode;
  target.dispatchEvent(new Event('input', { bubbles: true }));
  updateRowAccount(row, true);
  updateLedger(row);
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

function escapeVoucherPickerHtml(value) {
  return String(value ?? '').replace(/[&<>"']/g, (char) => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
  })[char]);
}

function nextVoucherPickerAccountCode(level, parentCode, accounts) {
  const baseWidth = ({ 1: 4, 2: 2, 3: 3 })[Number(level)];
  if (!baseWidth || (Number(level) > 1 && !parentCode)) return '';
  const prefix = Number(level) === 1 ? '' : String(parentCode);
  const siblings = accounts.filter((account) => Number(account.level) === Number(level)
    && (Number(level) === 1 ? !account.parentCode : account.parentCode === parentCode));
  const widths = siblings.map((account) => String(account.code).length - prefix.length)
    .filter((width) => width > 0);
  const width = Math.max(baseWidth, ...widths);
  if (prefix.length + width > 50) return '';
  const minimum = Number(level) === 1 ? 10n ** BigInt(width - 1) : 1n;
  const maximum = 10n ** BigInt(width) - 1n;
  const pattern = new RegExp('^\\d{' + width + '}$');
  const used = new Set(accounts.map((account) => String(account.code)));
  const suffixes = siblings.map((account) => String(account.code).slice(prefix.length))
    .filter((suffix) => pattern.test(suffix)).map(BigInt);
  const highest = suffixes.length
    ? suffixes.reduce((max, value) => value > max ? value : max, minimum - 1n)
    : (Number(level) === 1 ? minimum : 0n);
  let next = highest + 1n;
  for (let checked = 0; checked <= accounts.length + 1; checked += 1) {
    if (next > maximum) next = minimum;
    const candidate = prefix + next.toString().padStart(width, '0');
    if (!used.has(candidate)) return candidate;
    next += 1n;
  }
  return '';
}

function closeQuickAccountCreate() {
  const modal = $('#vfQuickAccountModal');
  if (modal) modal.hidden = true;
  if (accountPickerOpen()) $('#accountPickerCreate')?.focus();
}

async function openQuickAccountCreate() {
  if (!invoke || !apTarget) return;
  const row = apTarget.closest('tr');
  const baseCode = apMode === 'detail'
    ? digitsOnly(row?.querySelector('.vf-code-input')?.value || '')
    : digitsOnly(apTarget.value || '');
  const selected = apMode === 'detail'
    ? accountForCode(baseCode)
    : (apVisible[apIndex] || accountForCode(baseCode));
  let parent = null;
  let level = 1;
  if (apMode === 'detail') {
    parent = selected;
    if (!parent || Number(parent.level) !== 2) {
      toast('برای تعریف تفصیلی ۱، ابتدا یک حساب معین معتبر انتخاب کنید.', 'info');
      return;
    }
    level = 3;
  } else {
    const pathParent = state.accountPickerPath[state.accountPickerPath.length - 1] || null;
    const anchor = selected || pathParent;
    if (anchor && Number(anchor.level) === 1) {
      parent = anchor;
      level = 2;
    } else if (anchor && Number(anchor.level) === 2) {
      parent = anchor;
      level = 3;
    } else if (anchor && Number(anchor.level) === 3) {
      parent = accountForCode(anchor.parentCode);
      if (!parent || Number(parent.level) !== 2) {
        toast('حساب معین والد این تفصیلی پیدا نشد.', 'error');
        return;
      }
      level = 3;
    }
  }

  let groups;
  try {
    const payload = await invoke('list_account_groups');
    groups = Array.isArray(payload?.groups) ? payload.groups : [];
  } catch (error) {
    toast(errorText(error), 'error');
    return;
  }
  if (!groups.length) {
    toast('ابتدا حداقل یک گروه حساب تعریف کنید.', 'error');
    return;
  }

  let modal = $('#vfQuickAccountModal');
  if (!modal) {
    modal = document.createElement('div');
    modal.id = 'vfQuickAccountModal';
    modal.className = 'modal vf-quick-account-modal';
    modal.hidden = true;
    modal.innerHTML = `
      <section class="ap-window vf-quick-account-window" role="dialog" aria-modal="true" aria-labelledby="vfQuickAccountTitle">
        <div class="ap-titlebar">
          <button class="ap-close" type="button" data-quick-close aria-label="بستن">
            <svg class="ico"><use href="#i-close" /></svg>
          </button>
          <span class="ap-title" id="vfQuickAccountTitle">تعریف حساب از داخل انتخابگر</span>
        </div>
        <form id="vfQuickAccountForm" class="vf-quick-account-form">
          <label>سطح حساب<input id="vfQuickAccountLevel" type="text" readonly /></label>
          <label>حساب بالاتر<input id="vfQuickAccountParent" type="text" readonly /></label>
          <label>گروه حساب<select id="vfQuickAccountGroup" required></select></label>
          <label>کد حساب<input id="vfQuickAccountCode" type="text" inputmode="numeric" dir="ltr" maxlength="50" required /></label>
          <label>نام حساب<input id="vfQuickAccountName" type="text" maxlength="200" required /></label>
          <label>ماهیت<select id="vfQuickAccountNature">
            <option value="debtor">بدهکار</option><option value="creditor">بستانکار</option><option value="dual">دوگانه</option>
          </select></label>
          <div class="vf-quick-account-actions">
            <button type="submit" class="ap-tool ap-tool-select">ثبت حساب</button>
            <button type="button" class="ap-tool" data-quick-close>انصراف</button>
          </div>
          <div id="vfQuickAccountError" role="status" aria-live="polite"></div>
        </form>
      </section>`;
    document.body.append(modal);
    modal.querySelectorAll('[data-quick-close]').forEach((button) => button.addEventListener('click', closeQuickAccountCreate));
    modal.addEventListener('click', (event) => {
      if (event.target === modal) closeQuickAccountCreate();
    });
    modal.querySelector('#vfQuickAccountForm').addEventListener('submit', async (event) => {
      event.preventDefault();
      const form = event.currentTarget;
      const submit = form.querySelector('[type="submit"]');
      const code = digitsOnly($('#vfQuickAccountCode').value);
      const name = $('#vfQuickAccountName').value.trim();
      const groupCode = $('#vfQuickAccountGroup').value;
      const selectedLevel = Number($('#vfQuickAccountLevel').dataset.level);
      const parentCode = $('#vfQuickAccountParent').dataset.code || '';
      const nature = $('#vfQuickAccountNature').value;
      const errorBox = $('#vfQuickAccountError');
      errorBox.textContent = '';
      if (!code || !name) {
        errorBox.textContent = 'کد و نام حساب الزامی است.';
        return;
      }
      submit.disabled = true;
      try {
        const created = await invoke('create_account', {
          code, name, level: selectedLevel, groupCode, nature,
          parentCode, isPostable: selectedLevel <= 2, isActive: true,
        });
        await loadAccounts();
        modal.hidden = true;
        const search = $('#accountPickerSearch');
        if (search) search.value = created.code;
        renderAccountPickerRows();
        const index = apVisible.findIndex((item) => digitsOnly(item.code) === digitsOnly(created.code));
        if (index >= 0) {
          apIndex = index;
          renderAccountPickerRows(false);
          chooseAccountPickerRow();
        } else {
          toast('حساب ایجاد شد؛ برای انتخاب آن، فهرست حساب‌ها را به‌روزرسانی کنید.', 'ok');
        }
      } catch (error) {
        errorBox.textContent = errorText(error);
      } finally {
        submit.disabled = false;
      }
    });
    modal.addEventListener('keydown', (event) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        closeQuickAccountCreate();
      }
    });
  }

  const groupSelect = $('#vfQuickAccountGroup');
  // eslint-disable-next-line no-unsanitized/property -- group fields are escaped in the option template
  groupSelect.innerHTML = groups.map((group) =>
    `<option value="${escapeVoucherPickerHtml(group.code)}">${escapeVoucherPickerHtml(group.code)} — ${escapeVoucherPickerHtml(group.name)}</option>`
  ).join('');
  groupSelect.value = parent?.groupCode || groups[0].code;
  const levelInput = $('#vfQuickAccountLevel');
  levelInput.value = ({ 1: 'کل', 2: 'معین', 3: 'تفصیلی ۱' })[level];
  levelInput.dataset.level = String(level);
  const parentInput = $('#vfQuickAccountParent');
  parentInput.value = parent ? `${parent.code} — ${parent.name}` : 'بدون حساب بالاتر';
  parentInput.dataset.code = parent?.code || '';
  $('#vfQuickAccountCode').value = nextVoucherPickerAccountCode(level, parent?.code || '', state.accounts);
  $('#vfQuickAccountName').value = '';
  $('#vfQuickAccountNature').value = parent?.nature || 'debtor';
  $('#vfQuickAccountError').textContent = '';
  modal.hidden = false;
  $('#vfQuickAccountName').focus();
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
    if (state.accountPickerShowAll) {
      state.accountPickerShowAll = false;
      if (search) search.value = '';
      apIndex = 0;
      renderAccountPickerRows();
      search?.focus();
    } else if (state.accountPickerPath.length) {
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
  $('#accountPickerCreate')?.addEventListener('click', openQuickAccountCreate);

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
    if (ev.key === 'Insert' || ev.key === 'F3') {
      ev.preventDefault();
      openQuickAccountCreate();
      return;
    }

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
        drillOrChooseAccountPickerRow(apVisible[apIndex]);
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
