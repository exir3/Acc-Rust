/* ============================================================
   تعریف حساب‌های کل، معین و تفصیلی — standalone window controller
   Drill-down (کل ← معین ← تفصیلی ۱) with the account-details
   form shown in its own modal, modelled on the WebApp popup.
   ============================================================ */
(() => {
  'use strict';

  const tauriCore = window.__TAURI__?.core || null;
  const invoke = typeof tauriCore?.invoke === 'function'
    ? tauriCore.invoke.bind(tauriCore)
    : null;
  const tauriWindow = window.__TAURI__?.window || null;

  const FA_DIGITS = ['۰', '۱', '۲', '۳', '۴', '۵', '۶', '۷', '۸', '۹'];
  const LEVEL_NAMES = { 1: 'کل', 2: 'معین', 3: 'تفصیلی ۱' };
  const CODE_WIDTHS = { 1: 4, 2: 2, 3: 3 };
  const MAX_LEVEL = 3;
  const MAX_ACCOUNT_NAME = 255;
  const NATURES = ['debtor', 'creditor', 'dual'];
  const CLASSIFICATION_SOURCE_LABELS = {
    seed: 'داده اولیه',
    inherit: 'ارث از حساب بالاتر',
    inherited: 'ارث از حساب بالاتر',
    auto: 'پیشنهاد گروه',
    manual: 'تعیین دستی',
  };

  const $ = (selector, root = document) => root.querySelector(selector);
  const $$ = (selector, root = document) => Array.from(root.querySelectorAll(selector));

  /* ── Helpers ─────────────────────────────────────────────────── */

  const esc = (value) => String(value ?? '').replace(/[&<>"']/g, (ch) => (
    { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[ch]
  ));

  const toFaDigits = (value) => String(value ?? '')
    .replace(/[0-9]/g, (digit) => FA_DIGITS[Number(digit)]);

  const toEnDigits = (value) => String(value ?? '')
    .replace(/[۰-۹]/g, (ch) => String(ch.charCodeAt(0) - 0x06F0))
    .replace(/[٠-٩]/g, (ch) => String(ch.charCodeAt(0) - 0x0660));

  const onlyDigits = (value) => toEnDigits(value).replace(/[^0-9]/g, '');
  const normalizeName = (value) => String(value ?? '').trim().replace(/\s+/g, ' ');
  const charCount = (value) => [...String(value ?? '')].length;

  function errorText(error) {
    if (typeof error === 'string') return error;
    if (error && typeof error.message === 'string') return error.message;
    return String(error ?? 'خطای نامشخص');
  }

  const levelName = (level) => LEVEL_NAMES[Number(level)] || `سطح ${toFaDigits(level)}`;

  function isTypingTarget(target) {
    if (!target) return false;
    const tag = target.tagName;
    return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT'
      || target.isContentEditable === true;
  }

  function isControlTarget(target) {
    return Boolean(target && target.closest
      && target.closest('button, a, summary, [contenteditable="true"]'));
  }

  /* ── State ───────────────────────────────────────────────────── */

  const state = {
    accounts: [],
    groups: [],
    fsCatalog: null,
    drillPath: [],
    selectedId: null,
    mode: 'view', // view | new | edit
    form: emptyFormShell(),
    initial: '',
    loading: false,
    loaded: false,
    fetching: false,
    saving: false,
    deleting: false,
    deletingAll: false,
    exporting: false,
    importing: false,
    error: '',
    confirm: null,
    confirmBusy: false,
    confirmReturnFocus: null,
    result: null,
    resultReturnFocus: null,
    closeApproved: false,
    refreshQueued: false,
    initialized: false,
    searchQuery: '',
    searchBasePath: null,
    searchBaseId: null,
    toastTimer: null,
    lastRowClick: { id: null, at: 0 },
  };

  function isEditing() {
    return state.mode === 'new' || state.mode === 'edit';
  }

  function isBusy() {
    return state.loading || state.saving || state.deleting
      || state.deletingAll || state.exporting || state.importing
      || state.confirmBusy;
  }

  function emptyFormShell() {
    return {
      id: null,
      code: '',
      name: '',
      level: 1,
      groupCode: '',
      parentCode: '',
      nature: 'debtor',
      isActive: true,
      fsLine: '',
      contraOf: '',
      contraKind: '',
      oppositeFsLine: '',
      splitBySign: false,
      systemRole: '',
    };
  }

  function snapshot() {
    return JSON.stringify([
      onlyDigits(state.form.code),
      normalizeName(state.form.name),
      Number(state.form.level),
      state.form.groupCode || '',
      state.form.parentCode || '',
      state.form.nature || 'debtor',
      Boolean(state.form.isActive),
      state.form.fsLine || '',
      state.form.contraOf || '',
      state.form.contraKind || '',
      state.form.oppositeFsLine || '',
      Boolean(state.form.splitBySign),
      state.form.systemRole || '',
    ]);
  }

  function isDirty() {
    return isEditing() && snapshot() !== state.initial;
  }

  /* ── Normalisation ───────────────────────────────────────────── */

  function normalizeAccount(raw) {
    return {
      id: Number(raw && raw.id) || 0,
      code: String((raw && raw.code) ?? ''),
      name: String((raw && raw.name) ?? ''),
      level: Number(raw && raw.level) || 1,
      groupCode: String((raw && raw.groupCode) ?? ''),
      groupName: String((raw && raw.groupName) ?? ''),
      nature: NATURES.includes(raw && raw.nature) ? raw.nature : 'debtor',
      parentCode: String((raw && raw.parentCode) ?? ''),
      parentName: String((raw && raw.parentName) ?? ''),
      isPostable: Boolean(raw && raw.isPostable),
      isActive: !raw || raw.isActive !== false,
      fsLine: String((raw && raw.fsLine) ?? ''),
      contraOf: String((raw && raw.contraOf) ?? ''),
      contraKind: String((raw && raw.contraKind) ?? ''),
      oppositeFsLine: String((raw && raw.oppositeFsLine) ?? ''),
      splitBySign: Boolean(raw && raw.splitBySign),
      systemRole: String((raw && raw.systemRole) ?? ''),
      effectiveFsLine: String((raw && raw.effectiveFsLine) ?? ''),
      effectiveFsLineTitle: String((raw && raw.effectiveFsLineTitle) ?? ''),
      effectiveSection: String((raw && raw.effectiveSection) ?? ''),
      classificationSource: String((raw && raw.classificationSource) ?? ''),
    };
  }

  function normalizeGroup(raw) {
    return {
      id: Number(raw && raw.id) || 0,
      code: String((raw && raw.code) ?? ''),
      name: String((raw && raw.name) ?? ''),
      type: String((raw && raw.type) ?? ''),
    };
  }

  const compareCodes = (a, b) => (
    a.code.localeCompare(b.code, 'en', { numeric: true }) || a.id - b.id
  );

  /* ── Drill-down model ────────────────────────────────────────── */

  function resolveChain(path) {
    const byCode = new Map(state.accounts.map((account) => [account.code, account]));
    const chain = [];
    for (const code of path) {
      const node = byCode.get(code);
      const parent = chain[chain.length - 1];
      if (!node) break;
      if (node.level !== chain.length + 1) break;
      // لايه‌های قابل بازکردن تا پیش از سطح تفصیلی ۱ هستند.
      if (node.level >= MAX_LEVEL) break;
      if ((node.parentCode || '') !== (parent ? parent.code : '')) break;
      chain.push(node);
    }
    return chain;
  }

  function drillChain() {
    return resolveChain(state.drillPath);
  }

  function currentParent() {
    const chain = drillChain();
    return chain.length ? chain[chain.length - 1] : null;
  }

  function currentLevel() {
    const parent = currentParent();
    return parent ? parent.level + 1 : 1;
  }

  function visibleAccounts() {
    const parent = currentParent();
    const list = parent
      ? state.accounts.filter((account) => account.parentCode === parent.code)
      : state.accounts.filter((account) => account.level === 1);
    return [...list].sort(compareCodes);
  }

  function selectedAccount() {
    return state.accounts.find((account) => account.id === state.selectedId) || null;
  }

  /* ── Search model ────────────────────────────────────────────── */

  function normalizeSearch(value) {
    return toEnDigits(String(value ?? '')).trim().toLocaleLowerCase('fa');
  }

  function searchActive() {
    return state.searchQuery.trim().length > 0;
  }

  function accountPath(account) {
    const byCode = new Map(state.accounts.map((item) => [item.code, item]));
    const chain = [];
    const seen = new Set();
    let current = account;
    while (current && !seen.has(current.code)) {
      seen.add(current.code);
      chain.unshift(current);
      current = current.parentCode ? byCode.get(current.parentCode) : null;
    }
    return chain;
  }

  function accountPathLabel(account) {
    return accountPath(account).map((item) => item.name).join(' ← ');
  }

  function searchResults() {
    const query = normalizeSearch(state.searchQuery);
    if (!query) return [];
    return state.accounts.filter((account) => {
      const code = normalizeSearch(account.code);
      const name = normalizeSearch(account.name);
      const group = normalizeSearch(account.groupName);
      const parent = normalizeSearch(account.parentName);
      return code.includes(query)
        || name.includes(query)
        || group.includes(query)
        || parent.includes(query);
    }).sort(compareCodes);
  }

  function displayedAccounts() {
    return searchActive() ? searchResults() : visibleAccounts();
  }

  function syncSearchControls() {
    const input = $('#adSearch');
    const clearButton = $('#adSearchClear');
    const status = $('#adSearchStatus');
    if (input) {
      if (document.activeElement !== input && input.value !== state.searchQuery) {
        input.value = state.searchQuery;
      }
      input.disabled = isEditing() || isBusy() || !state.loaded;
    }
    if (clearButton) clearButton.disabled = !searchActive() || isBusy();
    if (status) {
      status.classList.remove('is-empty');
      if (!searchActive()) {
        status.textContent = '';
      } else {
        const count = searchResults().length;
        status.textContent = count
          ? `${toFaDigits(count)} حساب یافت شد.`
          : 'موردی یافت نشد؛ عبارت جستجو را پاک کنید.';
        status.classList.toggle('is-empty', count === 0);
      }
    }
  }

  function resetSearch(options = {}) {
    const { restore = false } = options;
    const basePath = state.searchBasePath;
    const baseId = state.searchBaseId;
    state.searchQuery = '';
    state.searchBasePath = null;
    state.searchBaseId = null;
    const input = $('#adSearch');
    if (input) input.value = '';
    if (!restore || basePath === null) return;
    navigateTo(basePath);
    if (baseId == null) return;
    const baseAccount = state.accounts.find((account) => account.id === baseId);
    const visible = visibleAccounts();
    if (baseAccount && visible.some((account) => account.id === baseAccount.id)) {
      state.selectedId = baseAccount.id;
      state.form = formFromAccount(baseAccount);
      state.initial = snapshot();
      renderAll();
      focusSelectedRow();
    }
  }

  function activateSearchResult(account) {
    if (!account) return;
    const parentPath = accountPath(account).slice(0, -1).map((item) => item.code);
    resetSearch();
    navigateTo(parentPath, { selectCode: account.code });
  }

  function handleSearchInput(value) {
    const next = String(value ?? '');
    const wasActive = searchActive();
    if (!wasActive && next.trim()) {
      state.searchBasePath = [...state.drillPath];
      state.searchBaseId = state.selectedId;
    }
    state.searchQuery = next;
    if (!next.trim()) {
      resetSearch({ restore: true });
      renderAll();
      return;
    }
    const results = searchResults();
    const current = results.find((account) => account.id === state.selectedId);
    const nextAccount = current || results[0] || null;
    state.selectedId = nextAccount ? nextAccount.id : null;
    state.form = nextAccount ? formFromAccount(nextAccount) : emptyForm();
    state.initial = snapshot();
    clearError();
    renderAll();
  }

  function formFromAccount(account) {
    return {
      id: account.id,
      code: toFaDigits(account.code),
      name: account.name,
      level: Number(account.level) || 1,
      groupCode: account.groupCode || '',
      parentCode: account.parentCode || '',
      nature: NATURES.includes(account.nature) ? account.nature : 'debtor',
      isActive: account.isActive !== false,
      fsLine: account.fsLine || '',
      contraOf: account.contraOf || '',
      contraKind: account.contraKind || '',
      oppositeFsLine: account.oppositeFsLine || '',
      splitBySign: Boolean(account.splitBySign),
      systemRole: account.systemRole || '',
      effectiveFsLineTitle: account.effectiveFsLineTitle || '',
      effectiveSection: account.effectiveSection || '',
      classificationSource: account.classificationSource || '',
    };
  }

  function emptyForm() {
    const parent = currentParent();
    return {
      id: null,
      code: '',
      name: '',
      level: currentLevel(),
      groupCode: parent ? parent.groupCode : (state.groups[0] ? state.groups[0].code : ''),
      parentCode: parent ? parent.code : '',
      nature: parent ? parent.nature : 'debtor',
      isActive: true,
      fsLine: '',
      contraOf: '',
      contraKind: '',
      oppositeFsLine: '',
      splitBySign: false,
      systemRole: '',
      effectiveFsLineTitle: '',
      effectiveSection: '',
      classificationSource: '',
    };
  }

  /* ── Code suggestion (same policy as the WebApp) ─────────────── */

  function nextAccountCode(level, parentCode) {
    const width = CODE_WIDTHS[level];
    if (!width || (level > 1 && !parentCode)) return '';
    const usedCodes = new Set(state.accounts.map((account) => account.code));
    const min = level === 1 ? 10 ** (width - 1) : 1;
    const max = (10 ** width) - 1;
    const candidates = state.accounts
      .filter((account) => (
        account.level === level
        && (level === 1 || account.parentCode === parentCode)
      ))
      .map((account) => (
        level === 1 ? account.code : account.code.slice(parentCode.length)
      ))
      .filter((code) => new RegExp(`^\\d{${width}}$`).test(code))
      .map(Number)
      .filter((value) => value >= min && value <= max);
    const next = (candidates.length ? Math.max(...candidates) : (level === 1 ? min - 1 : 0)) + 1;
    for (const [start, end] of [
      [next, max],
      [min, Math.min(next - 1, max)],
    ]) {
      for (let suffix = start; suffix <= end; suffix += 1) {
        const candidate = parentCode + String(suffix).padStart(width, '0');
        if (!usedCodes.has(candidate)) return candidate;
      }
    }
    return '';
  }

  function isValidNewAccountCode(code, level, parentCode) {
    const width = CODE_WIDTHS[level];
    if (!width || !/^\d+$/.test(code)) return false;
    if (level === 1) return code.length === width && Number(code) >= 10 ** (width - 1);
    if (!parentCode || code.length !== parentCode.length + width) return false;
    return Number(code.slice(parentCode.length)) > 0;
  }

  /* ── Error / toast ───────────────────────────────────────────── */

  function showError(message) {
    state.error = String(message ?? '');
    const editing = isEditing();
    const bar = $('#adError');
    const text = $('#adErrorText');
    if (bar && text) {
      text.textContent = state.error;
      bar.hidden = editing || !state.error;
    }
    const formBar = $('#adFormError');
    const formText = $('#adFormErrorText');
    if (formBar && formText) {
      formText.textContent = state.error;
      formBar.hidden = !editing || !state.error;
    }
  }

  function setFieldError(control, message) {
    if (!control) return;
    control.classList.add('is-invalid');
    control.setAttribute('aria-invalid', 'true');
    const field = control.closest('.ad-field');
    if (!field) return;
    let error = field.querySelector('.ad-field-error');
    if (!error) {
      error = document.createElement('small');
      error.className = 'ad-field-error';
      error.id = control.id ? `${control.id}Error` : '';
      field.append(error);
    }
    error.textContent = message;
    if (error.id) control.setAttribute('aria-describedby', error.id);
  }

  function clearFieldError(control) {
    if (!control) return;
    control.classList.remove('is-invalid');
    control.removeAttribute('aria-invalid');
    control.removeAttribute('aria-describedby');
    control.closest('.ad-field')?.querySelector('.ad-field-error')?.remove();
  }

  function clearFieldErrors() {
    $$('.ad-input.is-invalid').forEach(clearFieldError);
  }

  function focusFieldError(control) {
    if (!control || typeof control.focus !== 'function') return;
    control.focus({ preventScroll: true });
    control.scrollIntoView({ block: 'center' });
  }

  function clearError() {
    state.error = '';
    const bar = $('#adError');
    if (bar) bar.hidden = true;
    const formBar = $('#adFormError');
    if (formBar) formBar.hidden = true;
    clearFieldErrors();
  }

  function toast(message, tone = 'ok') {
    const el = $('#adToast');
    if (!el) return;
    clearTimeout(state.toastTimer);
    el.textContent = message;
    el.dataset.tone = tone;
    el.hidden = false;
    state.toastTimer = setTimeout(() => {
      el.hidden = true;
    }, tone === 'error' ? 6000 : 3600);
  }

  /* ── Rendering ───────────────────────────────────────────────── */

  function renderAll() {
    syncSearchControls();
    renderGrid();
    renderForm();
    syncFormModal();
    renderToolbar();
  }

  function syncFormModal() {
    const modal = $('#adFormModal');
    if (!modal) return;

    const editing = isEditing();
    modal.hidden = !editing;
    if (!editing) return;

    const title = $('#adFormTitle');
    if (title) title.textContent = state.mode === 'edit' ? 'اصلاح حساب' : 'تعریف حساب جدید';

    const busy = isBusy();
    const saveBtn = $('#adSave');
    const cancelBtn = $('#adCancel');
    const closeBtn = $('#adFormClose');
    if (saveBtn) saveBtn.disabled = busy;
    if (cancelBtn) cancelBtn.disabled = busy;
    if (closeBtn) closeBtn.disabled = busy;

    const saveLabel = $('#adSaveLabel');
    if (saveLabel) saveLabel.textContent = state.saving ? 'در حال ذخیره…' : 'ذخیره (F2)';
  }

  function renderGrid() {
    const body = $('#adGridBody');
    if (!body) return;

    // Drop a stale drill path if its accounts no longer match the tree.
    const chain = drillChain();
    if (chain.length !== state.drillPath.length) {
      state.drillPath = chain.map((account) => account.code);
    }

    const visible = displayedAccounts();

    if (state.mode === 'view' && !state.loading && state.loaded) {
      const kept = visible.find((account) => account.id === state.selectedId);
      if (!kept) {
        const next = visible[0] || null;
        state.selectedId = next ? next.id : null;
        state.form = next ? formFromAccount(next) : emptyForm();
      }
    }

    if (state.loading) {
      body.innerHTML = '<div class="ad-state">در حال بارگذاری اطلاعات…</div>';
      return;
    }
    if (!state.loaded) {
      body.innerHTML = '<div class="ad-state">دریافت اطلاعات حساب‌ها انجام نشد.</div>';
      return;
    }
    if (visible.length === 0) {
      // eslint-disable-next-line no-unsanitized/property -- static empty-state markup
      body.innerHTML = searchActive()
        ? '<div class="ad-state">موردی یافت نشد؛ برای بازگشت به مسیر قبلی، جستجو را پاک کنید.</div>'
        : '<div class="ad-state">حسابی در این سطح تعریف نشده است</div>';
      return;
    }

    // eslint-disable-next-line no-unsanitized/property -- every account field below is escaped
    body.innerHTML = visible.map((account, index) => {
      const selected = account.id === state.selectedId;
      const classes = ['ad-row'];
      if (index % 2 === 1) classes.push('even');
      if (selected) classes.push('selected');
      if (!account.isActive) classes.push('is-inactive');
      if (searchActive()) classes.push('search-result');
      const tabIndex = selected && state.mode === 'view' && !isBusy() ? 0 : -1;
      const pathTitle = searchActive() ? accountPathLabel(account) : '';
      const parentLabel = searchActive()
        ? (accountPath(account).slice(0, -1).map((item) => item.name).join(' ← ')
          || (account.level === 1 ? account.groupName : '')
          || '—')
        : (account.parentName || (account.level === 1 ? account.groupName : '') || '—');
      return `
        <div class="${classes.join(' ')}" role="row" data-id="${account.id}"
             tabindex="${tabIndex}" aria-selected="${selected}"
             ${pathTitle ? `title="${esc(pathTitle)}"` : ''}>
          <div class="ad-cell ad-center ad-num" role="gridcell">${toFaDigits(index + 1)}</div>
          <div class="ad-cell ad-center ad-code" dir="ltr" role="gridcell">${esc(toFaDigits(account.code))}</div>
          <div class="ad-cell ad-start ad-name" role="gridcell" title="${esc(pathTitle || account.name)}">${esc(account.name)}</div>
          <div class="ad-cell ad-center" role="gridcell">${esc(levelName(account.level))}</div>
          <div class="ad-cell ad-start ad-path" role="gridcell" title="${esc(parentLabel)}">${esc(parentLabel)}</div>
        </div>`;
    }).join('');

    // Keep the header spacer exactly as wide as the real scrollbar.
    updateScrollbarWidth();
    requestAnimationFrame(updateScrollbarWidth);
  }

  function renderForm() {
    const codeInput = $('#adCode');
    const nameInput = $('#adName');
    const groupSelect = $('#adGroup');
    const natureSelect = $('#adNature');
    const activeCheck = $('#adActive');
    if (!codeInput || !nameInput || !groupSelect || !natureSelect || !activeCheck) return;

    const form = state.form;
    const editing = isEditing();
    const busy = isBusy();

    codeInput.value = form.code;
    codeInput.disabled = !(state.mode === 'new' && !busy);
    codeInput.title = state.mode === 'edit' ? 'کد حساب پس از ثبت قابل تغییر نیست' : '';

    nameInput.value = form.name;
    nameInput.disabled = !editing || busy;

    groupSelect.value = form.groupCode || '';
    groupSelect.disabled = !editing || busy || Number(form.level) > 1;
    groupSelect.title = Number(form.level) > 1
      ? 'گروه حساب از حساب بالادستی به ارث می‌رسد'
      : '';

    natureSelect.value = NATURES.includes(form.nature) ? form.nature : 'debtor';
    natureSelect.disabled = !editing || busy;

    activeCheck.checked = Boolean(form.isActive);
    activeCheck.disabled = !editing || busy;

    const classificationRow = $('.ad-form-row-classification');
    const hasCatalog = Boolean(state.fsCatalog && state.fsCatalog.lines);
    if (classificationRow) classificationRow.hidden = !hasCatalog;
    if (!hasCatalog) return;

    fillContraOptions();

    const fsLineSelect = $('#adFsLine');
    const roleSelect = $('#adSystemRole');
    const contraSelect = $('#adContraOf');
    const kindSelect = $('#adContraKind');
    const oppositeSelect = $('#adOppositeFsLine');
    const splitCheck = $('#adSplitBySign');
    const effective = $('#adEffective');

    if (fsLineSelect) {
      fsLineSelect.value = form.fsLine || '';
      fsLineSelect.disabled = !editing || busy;
    }
    if (roleSelect) {
      roleSelect.value = form.systemRole || '';
      roleSelect.disabled = !editing || busy;
    }
    if (contraSelect) {
      contraSelect.value = form.contraOf || '';
      contraSelect.disabled = !editing || busy;
    }
    if (kindSelect) {
      kindSelect.value = form.contraKind || '';
      kindSelect.disabled = !editing || busy;
    }
    if (oppositeSelect) {
      oppositeSelect.value = form.oppositeFsLine || '';
      oppositeSelect.disabled = !editing || busy || !form.splitBySign;
    }
    if (splitCheck) {
      splitCheck.checked = Boolean(form.splitBySign);
      splitCheck.disabled = !editing || busy;
    }
    if (effective) {
      const title = form.effectiveFsLineTitle || '';
      effective.hidden = !title;
      effective.textContent = title
        ? `سرفصل مؤثر فعلی: ${title} — ${form.effectiveSection || ''} (${CLASSIFICATION_SOURCE_LABELS[form.classificationSource] || form.classificationSource || '—'})`
        : '';
    }
  }

  function fillGroupOptions() {
    const groupSelect = $('#adGroup');
    if (!groupSelect) return;
    // eslint-disable-next-line no-unsanitized/property -- option values and labels are escaped
    groupSelect.innerHTML = state.groups.map((group) => (
      `<option value="${esc(group.code)}">${esc(toFaDigits(group.code))} - ${esc(group.name)}</option>`
    )).join('');
  }

  function fsLineOptionsHtml(catalog, selected) {
    const statements = [
      { key: 'balance_sheet', label: 'صورت وضعیت مالی' },
      { key: 'income_statement', label: 'صورت سود و زیان' },
      { key: 'memorandum', label: 'حساب‌های انتظامی' },
    ];
    return statements.map((statement) => {
      const lines = catalog.lines.filter((line) => line.statement === statement.key);
      if (!lines.length) return '';
      return `<optgroup label="${statement.label}">${lines.map((line) => (
        `<option value="${esc(line.code)}"${line.code === selected ? ' selected' : ''}>${esc(line.code)} — ${esc(line.title)}</option>`
      )).join('')}</optgroup>`;
    }).join('');
  }

  function fillFsCatalogOptions() {
    const catalog = state.fsCatalog;
    if (!catalog) return;

    const fsLineSelect = $('#adFsLine');
    if (fsLineSelect) {
      // eslint-disable-next-line no-unsanitized/property -- catalog labels are escaped
      fsLineSelect.innerHTML = '<option value="">خودکار — از حساب بالاتر یا گروه</option>'
        + fsLineOptionsHtml(catalog, '');
    }

    const oppositeSelect = $('#adOppositeFsLine');
    if (oppositeSelect) {
      // eslint-disable-next-line no-unsanitized/property -- catalog labels are escaped
      oppositeSelect.innerHTML = '<option value="">— بدون سرفصل مقابل —</option>'
        + fsLineOptionsHtml(catalog, '');
    }

    const roleSelect = $('#adSystemRole');
    if (roleSelect) {
      // eslint-disable-next-line no-unsanitized/property -- option values and labels are escaped
      roleSelect.innerHTML = '<option value="">— بدون نقش —</option>'
        + (catalog.systemRoles || []).map((option) => (
          `<option value="${esc(option.value)}">${esc(option.label)}</option>`
        )).join('');
    }

    const kindSelect = $('#adContraKind');
    if (kindSelect) {
      // eslint-disable-next-line no-unsanitized/property -- option values and labels are escaped
      kindSelect.innerHTML = '<option value="">— بدون نوع —</option>'
        + (catalog.contraKinds || []).map((option) => (
          `<option value="${esc(option.value)}">${esc(option.label)}</option>`
        )).join('');
    }

    const contraSelect = $('#adContraOf');
    if (contraSelect) {
      fillContraOptions();
    }
  }

  function fillContraOptions() {
    const contraSelect = $('#adContraOf');
    if (!contraSelect) return;
    // eslint-disable-next-line no-unsanitized/property -- option values and labels are escaped
    contraSelect.innerHTML = '<option value="">— حساب کاهنده نیست —</option>'
      + state.accounts
        .filter((account) => account.id !== state.form.id)
        .map((account) => (
          `<option value="${esc(account.code)}">${esc(toFaDigits(account.code))} — ${esc(account.name)}</option>`
        )).join('');
  }

  function renderToolbar() {
    const editing = isEditing();
    const busy = isBusy();
    const ready = state.loaded && !state.loading;
    const selected = selectedAccount();
    const parent = currentParent();

    const backBtn = $('#adBack');
    const newBtn = $('#adNew');
    const editBtn = $('#adEdit');
    const deleteBtn = $('#adDelete');
    const deleteAllBtn = $('#adDeleteAll');
    const exportBtn = $('#adExport');
    const importBtn = $('#adImport');

    if (backBtn) {
      backBtn.disabled = editing || busy || !ready || state.drillPath.length === 0;
    }
    if (newBtn) {
      newBtn.disabled = editing || busy || !ready || state.groups.length === 0
        || currentLevel() > MAX_LEVEL;
    }
    if (editBtn) {
      editBtn.disabled = !selected || editing || busy || !ready;
    }
    if (deleteBtn) {
      deleteBtn.disabled = !selected || editing || busy || !ready;
    }
    if (deleteAllBtn) {
      deleteAllBtn.disabled = editing || busy || !ready || state.accounts.length === 0;
    }
    if (exportBtn) {
      exportBtn.disabled = editing || busy || !ready || state.accounts.length === 0;
    }
    if (importBtn) {
      importBtn.disabled = editing || busy || !ready || state.groups.length === 0;
    }

    const deleteAllLabel = $('#adDeleteAllLabel');
    if (deleteAllLabel) {
      deleteAllLabel.textContent = state.deletingAll ? 'در حال حذف…' : 'حذف همه';
    }

    const exportLabel = $('#adExportLabel');
    if (exportLabel) exportLabel.textContent = state.exporting ? 'در حال تهیه…' : 'خروجی اکسل';

    const importLabel = $('#adImportLabel');
    if (importLabel) importLabel.textContent = state.importing ? 'در حال ثبت…' : 'ورودی اکسل';

    // The current parent is implicit in the drilled level; the hint is visual.
    if (newBtn && parent && currentLevel() > MAX_LEVEL) {
      newBtn.title = 'در ساختار فعلی، بعد از سطح تفصیلی ۱ امکان تعریف سطح دیگری وجود ندارد.';
    } else if (newBtn) {
      newBtn.title = 'تعریف حساب جدید (در سطح جاری)';
    }
  }

  function updateScrollbarWidth() {
    const body = $('#adGridBody');
    const head = $('#adGridHead');
    if (!body || !head) return;
    const style = getComputedStyle(body);
    const borders = (parseFloat(style.borderLeftWidth) || 0)
      + (parseFloat(style.borderRightWidth) || 0);
    const width = Math.max(0, body.offsetWidth - body.clientWidth - borders);
    head.style.setProperty('--ad-sb-w', `${width}px`);
  }

  /* ── Focus helpers ───────────────────────────────────────────── */

  function focusSelectedRow() {
    requestAnimationFrame(() => {
      if (state.mode !== 'view' || state.selectedId == null) return;
      const row = document.querySelector(`.ad-row[data-id="${state.selectedId}"]`);
      if (row) {
        row.focus({ preventScroll: true });
        row.scrollIntoView({ block: 'nearest' });
      }
    });
  }

  function focusName() {
    requestAnimationFrame(() => {
      const input = $('#adName');
      if (!input || input.disabled) return;
      input.focus();
      input.select();
    });
  }

  /* ── Data loading ────────────────────────────────────────────── */

  function maybeQueuedRefresh() {
    if (!state.refreshQueued || state.fetching || isEditing() || isBusy()) return;
    state.refreshQueued = false;
    void loadData({ quiet: true });
  }

  async function loadData(options = {}) {
    const { quiet = false, selectCode = null, keepSelection = true } = options;

    // یکی از پنجره‌های دیگر ممکن است هم‌زمان تغییر داده باشد؛ یک بار دیگر
    // پس از پایان درخواست جاری، تازه‌سازی انجام می‌شود.
    if (state.fetching) {
      state.refreshQueued = true;
      return;
    }

    state.fetching = true;
    state.refreshQueued = false;

    if (!quiet) {
      state.loading = true;
      state.loaded = false;
      renderAll();
    }

    if (!invoke) {
      state.fetching = false;
      state.loading = false;
      state.loaded = false;
      showError('حالت پیش‌نمایش — ارتباط با هسته دسکتاپ برقرار نیست.');
      renderAll();
      return;
    }

    try {
      const [accountPayload, groupPayload] = await Promise.all([
        invoke('list_accounts'),
        invoke('list_account_groups'),
      ]);

      const accountList = accountPayload && Array.isArray(accountPayload.accounts)
        ? accountPayload.accounts
        : [];
      const groupList = groupPayload && Array.isArray(groupPayload.groups)
        ? groupPayload.groups
        : [];

      state.accounts = accountList.map(normalizeAccount).sort(compareCodes);
      state.groups = groupList.map(normalizeGroup)
        .sort((a, b) => a.code.localeCompare(b.code, 'en', { numeric: true }));
      if (!state.fsCatalog) {
        try {
          const catalog = await invoke('list_fs_lines');
          if (catalog && Array.isArray(catalog.lines) && catalog.lines.length) {
            state.fsCatalog = catalog;
          }
        } catch {
          // Classification is an enhancement; the basic form still works.
        }
      }
      state.loaded = true;
      state.loading = false;
      fillGroupOptions();
      fillFsCatalogOptions();

      const chain = drillChain();
      state.drillPath = chain.map((account) => account.code);

      if (state.mode === 'view') {
        const visible = visibleAccounts();
        const wanted = selectCode
          ? visible.find((account) => account.code === selectCode)
          : null;
        const kept = keepSelection && state.selectedId != null
          ? visible.find((account) => account.id === state.selectedId)
          : null;
        const next = wanted || kept || visible[0] || null;
        state.selectedId = next ? next.id : null;
        state.form = next ? formFromAccount(next) : emptyForm();
        state.initial = snapshot();
      }

      renderAll();
      focusSelectedRow();
    } catch (error) {
      state.loading = false;
      state.loaded = false;
      showError(`خطا در بارگذاری حساب‌ها: ${errorText(error)}`);
      renderAll();
    } finally {
      state.fetching = false;
      maybeQueuedRefresh();
    }
  }

  /* ── Navigation / selection ──────────────────────────────────── */

  function selectAccount(account, options = {}) {
    if (!account || isEditing() || isBusy()) return;
    if (state.selectedId === account.id && state.mode === 'view') {
      clearError();
      return;
    }
    state.selectedId = account.id;
    state.form = formFromAccount(account);
    state.initial = snapshot();
    state.mode = 'view';
    clearError();
    renderAll();
    if (options.focus !== false) focusSelectedRow();
  }

  function navigateTo(path, options = {}) {
    if (isEditing() || isBusy()) return;
    state.lastRowClick = { id: null, at: 0 };
    if (searchActive()) {
      state.searchQuery = '';
      state.searchBasePath = null;
      state.searchBaseId = null;
      const input = $('#adSearch');
      if (input) input.value = '';
    }
    const chain = resolveChain(path);
    state.drillPath = chain.map((account) => account.code);
    state.mode = 'view';
    const visible = visibleAccounts();
    const wanted = options.selectCode
      ? visible.find((account) => account.code === options.selectCode)
      : null;
    const next = wanted || visible[0] || null;
    state.selectedId = next ? next.id : null;
    state.form = next ? formFromAccount(next) : emptyForm();
    state.initial = snapshot();
    clearError();
    renderAll();
    focusSelectedRow();
  }

  function drillInto(account) {
    if (!account || isEditing() || isBusy()) return;
    if (Number(account.level) >= MAX_LEVEL) return;
    navigateTo([...state.drillPath, account.code]);
  }

  function goBack() {
    if (state.drillPath.length === 0) return;
    navigateTo(state.drillPath.slice(0, -1));
  }

  function moveSelection(direction) {
    const visible = displayedAccounts();
    if (visible.length === 0) return;
    const index = visible.findIndex((account) => account.id === state.selectedId);
    let nextIndex;
    if (index === -1) {
      nextIndex = direction > 0 ? 0 : visible.length - 1;
    } else {
      nextIndex = Math.min(visible.length - 1, Math.max(0, index + direction));
    }
    const next = visible[nextIndex];
    if (next) selectAccount(next);
  }

  /* ── Modes ───────────────────────────────────────────────────── */

  function startNew() {
    if (isEditing() || isBusy()) return;
    if (!state.loaded) return;
    if (state.groups.length === 0) {
      showError('ابتدا یک گروه حساب تعریف کنید.');
      return;
    }
    if (searchActive()) resetSearch();
    const level = currentLevel();
    if (level > MAX_LEVEL) {
      showError('در ساختار فعلی، بعد از سطح تفصیلی ۱ امکان تعریف سطح دیگری وجود ندارد.');
      return;
    }
    const parent = currentParent();
    const parentCode = parent ? parent.code : '';
    const defaultCode = nextAccountCode(level, parentCode);
    if (!defaultCode) {
      showError('ظرفیت کدگذاری در این سطح تکمیل شده است.');
      return;
    }

    state.mode = 'new';
    state.selectedId = null;
    state.form = {
      id: null,
      code: toFaDigits(defaultCode),
      name: '',
      level,
      groupCode: parent ? parent.groupCode : (state.groups[0] ? state.groups[0].code : ''),
      parentCode,
      nature: parent ? parent.nature : 'debtor',
      isActive: true,
      fsLine: '',
      contraOf: '',
      contraKind: '',
      oppositeFsLine: '',
      splitBySign: false,
      systemRole: '',
      effectiveFsLineTitle: '',
      effectiveSection: '',
      classificationSource: '',
    };
    state.initial = snapshot();
    clearError();
    renderAll();
    focusName();
  }

  function startEdit() {
    if (searchActive()) {
      const target = selectedAccount();
      if (target) activateSearchResult(target);
      else resetSearch();
    }
    const account = selectedAccount();
    if (!account || isEditing() || isBusy() || !state.loaded) return;
    state.mode = 'edit';
    state.form = formFromAccount(account);
    state.initial = snapshot();
    clearError();
    renderAll();
    focusName();
  }

  function cancelEdit(options = {}) {
    const { force = false } = options;
    if (!isEditing() || (!force && isBusy())) return;
    state.mode = 'view';
    const account = selectedAccount();
    state.form = account ? formFromAccount(account) : emptyForm();
    state.initial = snapshot();
    clearError();
    renderAll();
    focusSelectedRow();
    maybeQueuedRefresh();
  }

  function requestCancelEdit() {
    if (!isEditing() || isBusy()) return;

    if (!isDirty()) {
      cancelEdit();
      return;
    }

    openConfirm({
      title: 'لغو ویرایش',
      message: 'تغییرات ذخیره‌نشده کنار گذاشته شوند؟',
      confirmLabel: 'لغو تغییرات',
      tone: 'danger',
      onConfirm: () => cancelEdit({ force: true }),
    });
  }

  /* ── Save ────────────────────────────────────────────────────── */

  async function save() {
    if (!isEditing() || isBusy()) return;

    const mode = state.mode;
    const editingAccount = mode === 'edit' ? selectedAccount() : null;
    const level = Number(state.form.level) || 1;
    const code = onlyDigits(state.form.code);
    const name = normalizeName(state.form.name);
    const groupCode = state.form.groupCode || '';
    const parentCode = level === 1 ? '' : (state.form.parentCode || '');
    const nature = NATURES.includes(state.form.nature) ? state.form.nature : '';
    const isActive = Boolean(state.form.isActive);
    const fsLine = state.form.fsLine || '';
    const contraOf = state.form.contraOf || '';
    const contraKind = state.form.contraKind || '';
    const splitBySign = Boolean(state.form.splitBySign);
    const oppositeFsLine = splitBySign ? (state.form.oppositeFsLine || '') : '';
    const systemRole = state.form.systemRole || '';

    clearFieldErrors();

    if (!code || !name) {
      showError('لطفاً کد و نام حساب را وارد کنید.');
      if (!code) setFieldError($('#adCode'), 'کد حساب الزامی است.');
      if (!name) setFieldError($('#adName'), 'نام حساب الزامی است.');
      focusFieldError(!code ? $('#adCode') : $('#adName'));
      return;
    }
    if (charCount(name) > MAX_ACCOUNT_NAME) {
      showError(`نام حساب نباید بیش از ${toFaDigits(MAX_ACCOUNT_NAME)} کاراکتر باشد.`);
      setFieldError($('#adName'), `نام حساب نباید بیش از ${toFaDigits(MAX_ACCOUNT_NAME)} کاراکتر باشد.`);
      focusFieldError($('#adName'));
      return;
    }
    if (!/^\d+$/.test(code)) {
      showError('کد حساب باید فقط شامل عدد باشد.');
      setFieldError($('#adCode'), 'کد حساب باید فقط شامل عدد باشد.');
      focusFieldError($('#adCode'));
      return;
    }
    if (code.length > 50) {
      showError('کد حساب نمی‌تواند بیشتر از ۵۰ رقم باشد.');
      setFieldError($('#adCode'), 'کد حساب نمی‌تواند بیشتر از ۵۰ رقم باشد.');
      focusFieldError($('#adCode'));
      return;
    }
    if (!/[1-9]/.test(code)) {
      showError('کد حساب باید بزرگ‌تر از صفر باشد.');
      setFieldError($('#adCode'), 'کد حساب باید بزرگ‌تر از صفر باشد.');
      focusFieldError($('#adCode'));
      return;
    }
    // The 4/2/3 digit prefixes are only a suggestion used by nextAccountCode().
    // The Rust and Excel import paths deliberately accept any numeric code, so
    // a different width must warn instead of blocking an otherwise valid code.
    if (mode === 'new' && !isValidNewAccountCode(code, level, parentCode)) {
      const width = CODE_WIDTHS[level];
      toast(
        level === 1
          ? `پیشنهاد برنامه آن است که کد حساب کل ${toFaDigits(width || 4)} رقم باشد؛ کد فعلی با سیاست پیشنهادی متفاوت است و ثبت می‌شود.`
          : `پیشنهاد برنامه آن است که کد ${levelName(level)} ${toFaDigits(width || 2)} رقم بیشتر از کد والد داشته باشد؛ کد فعلی ثبت می‌شود.`,
        'warn',
      );
    }
    if (mode === 'new' && state.accounts.some((account) => account.code === code)) {
      const duplicate = state.accounts.find((account) => account.code === code);
      showError(`کد «${toFaDigits(code)}» قبلاً برای حساب «${duplicate.name}» ثبت شده است.`);
      setFieldError($('#adCode'), 'این کد حساب قبلاً استفاده شده است.');
      focusFieldError($('#adCode'));
      return;
    }
    if (mode === 'edit' && !editingAccount) {
      showError('حساب انتخاب‌شده یافت نشد.');
      return;
    }
    if (level > 1) {
      const parent = state.accounts.find((account) => account.code === parentCode);
      if (!parent || parent.level !== level - 1 || parent.groupCode !== groupCode) {
        showError('حساب والد نامعتبر است.');
        return;
      }
    }
    if (level < 1 || level > MAX_LEVEL) {
      showError(`سطح حساب باید بین ۱ و ${toFaDigits(MAX_LEVEL)} باشد.`);
      return;
    }
    if (!state.groups.some((group) => group.code === groupCode)) {
      showError('گروه حساب انتخاب‌شده معتبر نیست.');
      setFieldError($('#adGroup'), 'گروه حساب انتخاب‌شده معتبر نیست.');
      focusFieldError($('#adGroup'));
      return;
    }
    if (!nature) {
      showError('ماهیت حساب باید بدهکار، بستانکار یا ماهیت دوگانه باشد.');
      setFieldError($('#adNature'), 'ماهیت حساب را انتخاب کنید.');
      focusFieldError($('#adNature'));
      return;
    }
    if (contraOf && !contraKind) {
      showError('برای حساب کاهنده، انتخاب نوع کاهنده الزامی است.');
      setFieldError($('#adContraOf'), 'حساب کاهنده انتخاب شده است.');
      setFieldError($('#adContraKind'), 'نوع کاهنده را انتخاب کنید.');
      focusFieldError($('#adContraKind'));
      return;
    }
    if (!contraOf && contraKind) {
      showError('نوع کاهنده بدون انتخاب حساب کاهنده مجاز نیست.');
      setFieldError($('#adContraKind'), 'نوع کاهنده بدون حساب کاهنده مجاز نیست.');
      setFieldError($('#adContraOf'), 'حساب کاهنده را انتخاب کنید.');
      focusFieldError($('#adContraOf'));
      return;
    }
    if (splitBySign && !oppositeFsLine) {
      showError('برای تفکیک بر اساس علامت، انتخاب سرفصل مقابل الزامی است.');
      setFieldError($('#adOppositeFsLine'), 'سرفصل مقابل را انتخاب کنید.');
      focusFieldError($('#adOppositeFsLine'));
      return;
    }
    if (oppositeFsLine && fsLine && oppositeFsLine === fsLine) {
      showError('سرفصل مقابل باید با سرفصل اصلی متفاوت باشد.');
      setFieldError($('#adFsLine'), 'سرفصل اصلی است.');
      setFieldError($('#adOppositeFsLine'), 'سرفصل مقابل باید متفاوت باشد.');
      focusFieldError($('#adOppositeFsLine'));
      return;
    }

    const classificationArgs = state.fsCatalog && state.fsCatalog.lines ? {
      fsLine,
      contraOf,
      contraKind,
      oppositeFsLine,
      splitBySign,
      systemRole,
    } : {};

    state.saving = true;
    clearError();
    renderAll();

    try {
      if (mode === 'edit') {
        await invoke('update_account', {
          id: editingAccount.id,
          name,
          groupCode,
          nature,
          parentCode,
          isActive,
          ...classificationArgs,
        });
      } else {
        await invoke('create_account', {
          code,
          name,
          level,
          groupCode,
          nature,
          parentCode,
          isPostable: level <= 2,
          isActive,
          ...classificationArgs,
        });
      }

      state.mode = 'view';
      state.saving = false;
      state.refreshQueued = false;
      await loadData({ selectCode: code, keepSelection: false });
      toast(mode === 'edit' ? 'تغییرات حساب ذخیره شد.' : 'حساب جدید ثبت شد.');
    } catch (error) {
      state.saving = false;
      showError(errorText(error));
      renderAll();
      maybeQueuedRefresh();
    }
  }

  /* ── Confirmation / result dialogs ───────────────────────────── */

  function trapFocus(event, container) {
    if (event.key !== 'Tab') return;
    const focusable = $$('button:not(:disabled), input:not(:disabled), select:not(:disabled), [tabindex="0"]', container)
      .filter((el) => el.offsetParent !== null || el === document.activeElement);
    if (!focusable.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  function openConfirm({ title, message, confirmLabel = 'بله', tone = 'warn', onConfirm }) {
    state.confirm = { title, message, confirmLabel, tone, onConfirm };
    state.confirmBusy = false;
    state.confirmReturnFocus = document.activeElement;

    const modal = $('#adConfirm');
    const box = $('#adConfirmBox');
    $('#adConfirmTitle').textContent = title;
    $('#adConfirmText').textContent = message;
    $('#adConfirmYes').textContent = confirmLabel;
    box.dataset.tone = tone;
    modal.hidden = false;
    $('#adConfirmYes').disabled = false;
    $('#adConfirmNo').disabled = false;

    requestAnimationFrame(() => {
      const no = $('#adConfirmNo');
      if (no) no.focus();
    });
  }

  function closeConfirm() {
    if (state.confirmBusy) return;
    state.confirm = null;
    const modal = $('#adConfirm');
    if (modal) modal.hidden = true;
    const returnFocus = state.confirmReturnFocus;
    state.confirmReturnFocus = null;
    if (returnFocus && returnFocus.isConnected) returnFocus.focus();
  }

  async function confirmYes() {
    if (!state.confirm || state.confirmBusy) return;
    const action = state.confirm.onConfirm;
    const returnFocus = state.confirmReturnFocus;
    state.confirm = null;
    state.confirmReturnFocus = null;
    $('#adConfirm').hidden = true;
    state.confirmBusy = true;
    renderToolbar();

    try {
      await action();
    } catch (error) {
      showError(errorText(error));
      renderAll();
    } finally {
      state.confirmBusy = false;
      renderToolbar();
      if (!state.result && returnFocus && returnFocus.isConnected) {
        returnFocus.focus();
      }
      maybeQueuedRefresh();
    }
  }

  function showResult({ title, message, list = [], tone = 'info' }) {
    state.result = { title, message, list, tone };
    state.resultReturnFocus = document.activeElement;

    const modal = $('#adResult');
    const box = $('#adResultBox');
    $('#adResultTitle').textContent = title;
    $('#adResultText').textContent = message;
    box.dataset.tone = tone;
    $('#adResultIcon').textContent = tone === 'ok' ? '✓' : tone === 'warn' ? '!' : 'i';

    const listEl = $('#adResultList');
    if (Array.isArray(list) && list.length) {
      // eslint-disable-next-line no-unsanitized/property -- each list item is escaped
      listEl.innerHTML = list.map((item) => (
        `<div class="ad-msgbox-list-item">${esc(item)}</div>`
      )).join('');
      listEl.hidden = false;
    } else {
      listEl.innerHTML = '';
      listEl.hidden = true;
    }

    modal.hidden = false;
    requestAnimationFrame(() => {
      const ok = $('#adResultOk');
      if (ok) ok.focus();
    });
  }

  function closeResult() {
    state.result = null;
    const modal = $('#adResult');
    if (modal) modal.hidden = true;
    const returnFocus = state.resultReturnFocus;
    state.resultReturnFocus = null;
    if (returnFocus && returnFocus.isConnected) returnFocus.focus();
  }

  /* ── Delete ──────────────────────────────────────────────────── */

  function deleteSelected() {
    const account = selectedAccount();
    if (!account || isEditing() || isBusy()) return;

    const children = state.accounts.filter((item) => item.parentCode === account.code);
    if (children.length > 0) {
      showError(`این حساب دارای ${toFaDigits(children.length)} زیرحساب است. ابتدا زیرحساب‌ها را حذف یا جابجا کنید.`);
      return;
    }

    openConfirm({
      title: 'حذف حساب',
      message: `آیا از حذف حساب «${account.name}» اطمینان دارید؟`,
      confirmLabel: 'حذف',
      tone: 'danger',
      onConfirm: async () => {
        state.deleting = true;
        clearError();
        renderAll();
        try {
          await invoke('delete_account', { id: account.id });
          state.mode = 'view';
          state.selectedId = null;
          state.refreshQueued = false;
          await loadData({ keepSelection: false });
          toast('حساب با موفقیت حذف شد.');
        } catch (error) {
          showError(errorText(error));
        } finally {
          state.deleting = false;
          renderAll();
          maybeQueuedRefresh();
        }
      },
    });
  }

  function deleteAll() {
    if (isEditing() || isBusy() || state.accounts.length === 0) return;
    const total = state.accounts.length;

    openConfirm({
      title: 'حذف همه حساب‌ها',
      message: `آیا از حذف کامل همه‌ی ${toFaDigits(total)} حساب تعریف‌شده (به‌همراه زیرحساب‌های آن‌ها) اطمینان دارید؟`
        + ' حساب‌هایی که در اسناد حسابداری استفاده شده باشند حذف نخواهند شد.',
      confirmLabel: 'حذف همه',
      tone: 'danger',
      onConfirm: async () => {
        state.deletingAll = true;
        clearError();
        renderAll();
        try {
          const ordered = [...state.accounts].sort((a, b) => (
            b.level - a.level || b.code.localeCompare(a.code, 'en', { numeric: true })
          ));
          const result = await invoke('delete_accounts_bulk', {
            ids: ordered.map((account) => account.id),
          });
          const deleted = Number(result && result.deletedCount) || 0;
          const blocked = result && Array.isArray(result.blocked) ? result.blocked : [];

          state.mode = 'view';
          state.selectedId = null;
          state.refreshQueued = false;
          await loadData({ keepSelection: false });

          toast(
            `${toFaDigits(deleted)} حساب حذف شد؛ ${toFaDigits(blocked.length)} حساب باقی ماند.`,
            blocked.length ? 'warn' : 'ok',
          );
          if (blocked.length > 0) {
            showResult({
              title: 'حذف همه حساب‌ها',
              tone: 'warn',
              message: `${toFaDigits(deleted)} حساب حذف شد و ${toFaDigits(blocked.length)} حساب حذف نشد.`,
              list: blocked.map((item) => (
                `${toFaDigits(item.code)} — ${item.name}: ${item.error || ''}`
              )),
            });
          }
        } catch (error) {
          showError(errorText(error));
        } finally {
          state.deletingAll = false;
          renderAll();
          maybeQueuedRefresh();
        }
      },
    });
  }

  /* ── Excel import / export ───────────────────────────────────── */

  async function exportExcel() {
    if (isEditing() || isBusy()) return;
    if (state.accounts.length === 0) {
      showError('هیچ حسابی برای خروجی گرفتن وجود ندارد.');
      return;
    }

    state.exporting = true;
    clearError();
    renderAll();
    try {
      const result = await invoke('export_account_excel');
      if (result) {
        toast(`فایل اکسل ذخیره شد: ${result.path || ''}`);
        const skipped = Array.isArray(result.skippedCodes) ? result.skippedCodes : [];
        if (skipped.length > 0) {
          showResult({
            title: 'خروجی اکسل',
            tone: 'warn',
            message: `${toFaDigits(skipped.length)} حساب در فایل خروجی درج نشد.`,
            list: skipped.map((code) => toFaDigits(code)),
          });
        }
      }
    } catch (error) {
      showError(errorText(error));
    } finally {
      state.exporting = false;
      renderAll();
    }
  }

  function importExcel() {
    if (isEditing() || isBusy()) return;
    if (state.groups.length === 0) {
      showError('ابتدا یک گروه حساب تعریف کنید.');
      return;
    }

    openConfirm({
      title: 'ورودی اکسل',
      message: 'حساب‌ها از فایل اکسل مطابق قالب WebApp وارد شوند؟ ابتدا یک پیش‌نمایش بدون تغییر داده‌ها ساخته می‌شود.',
      confirmLabel: 'انتخاب فایل',
      tone: 'info',
      onConfirm: async () => {
        clearError();
        try {
          const preview = await invoke('preview_account_excel_import');
          if (!preview) return;

          const created = Number(preview.createdCount) || 0;
          const skipped = Number(preview.skippedCount) || 0;
          const errorCount = Number(preview.errorCount) || 0;
          const summary = `${toFaDigits(created)} حساب جدید و ${toFaDigits(skipped)} حساب موجود`
            + (errorCount ? `؛ ${toFaDigits(errorCount)} خطا` : '');

          if (errorCount > 0) {
            openConfirm({
              title: 'پیش‌نمایش ورود اکسل',
              message: `${summary}. ورود یکپارچه ممکن نیست؛ فقط ردیف‌های معتبر وارد شوند؟`,
              confirmLabel: 'ورود ردیف‌های معتبر',
              tone: 'warn',
              onConfirm: () => applyAccountImport(preview.previewToken, 'partial'),
            });
            return;
          }
          if (created === 0) {
            showResult({
              title: 'پیش‌نمایش ورود اکسل',
              message: 'حساب جدیدی برای ورود وجود ندارد.',
              list: preview.skippedCodes || [],
            });
            return;
          }
          openConfirm({
            title: 'پیش‌نمایش ورود اکسل',
            message: `${summary}. همه حساب‌ها در یک تراکنش وارد شوند؟`,
            confirmLabel: 'ورود یکپارچه',
            tone: 'info',
            onConfirm: () => applyAccountImport(preview.previewToken, 'atomic'),
          });
        } catch (error) {
          showError(errorText(error));
        }
      },
    });
  }

  async function applyAccountImport(previewToken, mode) {
    state.importing = true;
    renderAll();
    try {
      const result = await invoke('import_account_excel', { previewToken, mode });
      if (!result) return;

      const created = Number(result.createdCount) || 0;
      const skipped = Number(result.skippedCount) || 0;
      const errorCount = Number(result.errorCount) || 0;
      const errors = Array.isArray(result.errors) ? result.errors : [];

      if (result.aborted) {
        showResult({
          title: 'ورود اکسل انجام نشد',
          tone: 'warn',
          message: `به دلیل وجود ${toFaDigits(errorCount)} خطا هیچ تغییری اعمال نشد.`,
          list: errors,
        });
        return;
      }

      state.mode = 'view';
      state.refreshQueued = false;
      await loadData({ keepSelection: false });

      toast(
        `${toFaDigits(created)} حساب جدید و ${toFaDigits(skipped)} حساب موجود`
          + (errorCount ? `؛ ${toFaDigits(errorCount)} خطا` : ''),
        errorCount ? 'warn' : 'ok',
      );
      if (errorCount > 0 || created > 0) {
        showResult({
          title: 'نتیجه ورود اکسل',
          tone: errorCount ? 'warn' : 'ok',
          message: `${toFaDigits(created)} حساب ایجاد شد و ${toFaDigits(skipped)} حساب موجود بود.`,
          list: accountImportDetails(result),
        });
      }
    } catch (error) {
      showError(errorText(error));
    } finally {
      state.importing = false;
      renderAll();
      maybeQueuedRefresh();
    }
  }

  function accountImportDetails(result) {
    const lines = [];
    const created = Array.isArray(result.createdCodes) ? result.createdCodes : [];
    const skipped = Array.isArray(result.skippedCodes) ? result.skippedCodes : [];
    const errors = Array.isArray(result.errors) ? result.errors : [];
    const limit = 100;
    const codes = (values) => values.slice(0, limit).map((code) => toFaDigits(code)).join('، ')
      + (values.length > limit ? ` و ${toFaDigits(values.length - limit)} مورد دیگر` : '');
    if (created.length) lines.push(`ایجادشده: ${codes(created)}`);
    if (skipped.length) lines.push(`موجود/ردشده: ${codes(skipped)}`);
    lines.push(...errors);
    return lines;
  }

  /* ── Window close guard ──────────────────────────────────────── */

  function currentWindow() {
    if (!tauriWindow || typeof tauriWindow.getCurrentWindow !== 'function') return null;
    try {
      return tauriWindow.getCurrentWindow();
    } catch (_) {
      return null;
    }
  }

  function closeWindow() {
    state.closeApproved = true;
    const win = currentWindow();
    if (win && typeof win.close === 'function') {
      win.close().catch(() => {
        state.closeApproved = false;
      });
    } else {
      window.close();
    }
  }

  function requestClose() {
    if (isBusy()) return;
    if (isEditing() && isDirty()) {
      openConfirm({
        title: 'بستن فرم',
        message: 'تغییرات ذخیره‌نشده کنار گذاشته شوند و فرم بسته شود؟',
        confirmLabel: 'بستن',
        tone: 'danger',
        onConfirm: () => closeWindow(),
      });
      return;
    }
    closeWindow();
  }

  function bindWindowClose() {
    const win = currentWindow();
    if (!win || typeof win.onCloseRequested !== 'function') return;
    const result = win.onCloseRequested((event) => {
      if (state.closeApproved) return;
      if (state.confirm || state.result || isBusy()) {
        event.preventDefault();
        return;
      }
      if (isEditing() && isDirty()) {
        event.preventDefault();
        openConfirm({
          title: 'بستن فرم',
          message: 'تغییرات ذخیره‌نشده کنار گذاشته شوند و فرم بسته شود؟',
          confirmLabel: 'بستن',
          tone: 'danger',
          onConfirm: () => closeWindow(),
        });
      }
    });
    if (result && typeof result.catch === 'function') result.catch(() => {});
  }

  /* ── Keyboard shortcuts ──────────────────────────────────────── */

  function handleKeydown(event) {
    if (!state.initialized) return;

    if (state.confirm) {
      if (event.key === 'Escape') {
        event.preventDefault();
        closeConfirm();
      } else {
        trapFocus(event, $('#adConfirm'));
      }
      return;
    }

    if (state.result) {
      if (event.key === 'Escape') {
        event.preventDefault();
        closeResult();
      } else {
        trapFocus(event, $('#adResult'));
      }
      return;
    }

    if (isEditing() && event.key === 'Tab') {
      trapFocus(event, $('#adFormModal'));
      return;
    }

    if (event.key === 'Escape') {
      event.preventDefault();
      if (searchActive()) {
        resetSearch({ restore: true });
      } else if (isEditing()) {
        requestCancelEdit();
      } else if (state.drillPath.length > 0) {
        goBack();
      } else {
        requestClose();
      }
      return;
    }

    if (event.key === 'F2') {
      event.preventDefault();
      void save();
      return;
    }

    if (event.key === 'F3') {
      event.preventDefault();
      if (!isEditing()) startEdit();
      return;
    }

    const typing = isTypingTarget(event.target);
    const control = isControlTarget(event.target);

    if (event.key === 'Delete') {
      if (typing) return;
      event.preventDefault();
      if (!isEditing()) deleteSelected();
      return;
    }

    if (event.key === 'Enter') {
      if (typing || control || isEditing()) return;
      if (searchActive()) {
        event.preventDefault();
        activateSearchResult(selectedAccount());
        return;
      }
      const account = selectedAccount();
      if (account) {
        event.preventDefault();
        event.stopPropagation();
        drillInto(account);
      }
      return;
    }

    if (event.key === 'Backspace') {
      if (typing || control || isEditing()) return;
      if (searchActive()) {
        event.preventDefault();
        resetSearch({ restore: true });
      } else if (state.drillPath.length > 0) {
        event.preventDefault();
        goBack();
      }
      return;
    }

    if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
      if (typing || control || isEditing()) return;
      if (visibleAccounts().length === 0) return;
      event.preventDefault();
      moveSelection(event.key === 'ArrowDown' ? 1 : -1);
    }
  }

  /* ── Event binding ───────────────────────────────────────────── */

  function bindEvents() {
    const gridBody = $('#adGridBody');
    gridBody?.addEventListener('click', (event) => {
      const row = event.target.closest('.ad-row');
      if (!row) return;
      const account = state.accounts.find((item) => item.id === Number(row.dataset.id));
      if (!account) return;

      // In search mode a single click resolves the result to its place in the
      // tree, so the visible list stays consistent with the drilled level.
      if (searchActive()) {
        activateSearchResult(account);
        return;
      }

      // دابل‌کلیک باید مانند Enter یک سطح پایین‌تر برود. چون رندر پس از
      // انتخاب ردیف، عنصر ردیف را جایگزین می‌کند، رویداد dblclick مرورگر
      // قابل اتکا نیست و فاصله دو کلیک را خودمان می‌سنجیم.
      const now = Date.now();
      const last = state.lastRowClick;
      if (last.id === account.id && now - last.at <= 500) {
        state.lastRowClick = { id: null, at: 0 };
        drillInto(account);
        return;
      }
      state.lastRowClick = { id: account.id, at: now };
      selectAccount(account);
    });

    $('#adForm')?.addEventListener('submit', (event) => {
      event.preventDefault();
      void save();
    });

    $('#adCode')?.addEventListener('input', (event) => {
      if (state.mode !== 'new') return;
      const digits = onlyDigits(event.target.value);
      state.form.code = toFaDigits(digits);
      event.target.value = state.form.code;
      clearFieldError(event.target);
    });
    $('#adName')?.addEventListener('input', (event) => {
      state.form.name = event.target.value;
      clearFieldError(event.target);
    });
    $('#adGroup')?.addEventListener('change', (event) => {
      state.form.groupCode = event.target.value;
      clearFieldError(event.target);
    });
    $('#adNature')?.addEventListener('change', (event) => {
      state.form.nature = event.target.value;
      clearFieldError(event.target);
    });
    $('#adActive')?.addEventListener('change', (event) => {
      state.form.isActive = event.target.checked;
    });
    $('#adFsLine')?.addEventListener('change', (event) => {
      state.form.fsLine = event.target.value;
      clearFieldError(event.target);
    });
    $('#adSystemRole')?.addEventListener('change', (event) => {
      state.form.systemRole = event.target.value;
      clearFieldError(event.target);
    });
    $('#adContraOf')?.addEventListener('change', (event) => {
      state.form.contraOf = event.target.value;
      clearFieldError(event.target);
    });
    $('#adContraKind')?.addEventListener('change', (event) => {
      state.form.contraKind = event.target.value;
      clearFieldError(event.target);
    });
    $('#adOppositeFsLine')?.addEventListener('change', (event) => {
      state.form.oppositeFsLine = event.target.value;
      clearFieldError(event.target);
    });
    $('#adSplitBySign')?.addEventListener('change', (event) => {
      state.form.splitBySign = event.target.checked;
      if (!state.form.splitBySign) state.form.oppositeFsLine = '';
      clearFieldError($('#adOppositeFsLine'));
      renderForm();
    });

    $('#adBack')?.addEventListener('click', goBack);
    $('#adNew')?.addEventListener('click', startNew);
    $('#adEdit')?.addEventListener('click', startEdit);
    $('#adDelete')?.addEventListener('click', deleteSelected);
    $('#adDeleteAll')?.addEventListener('click', deleteAll);
    $('#adSave')?.addEventListener('click', () => { void save(); });
    $('#adCancel')?.addEventListener('click', requestCancelEdit);
    $('#adFormClose')?.addEventListener('click', requestCancelEdit);
    $('#adFormModal')?.addEventListener('click', (event) => {
      if (event.target === $('#adFormModal')) requestCancelEdit();
    });
    $('#adExport')?.addEventListener('click', () => { void exportExcel(); });
    $('#adImport')?.addEventListener('click', importExcel);
    $('#adErrorClose')?.addEventListener('click', clearError);
    $('#adSearch')?.addEventListener('input', (event) => handleSearchInput(event.target.value));
    $('#adSearchClear')?.addEventListener('click', () => {
      resetSearch({ restore: true });
      if (!searchActive()) renderAll();
      $('#adSearch')?.focus();
    });

    $('#adConfirmYes')?.addEventListener('click', () => { void confirmYes(); });
    $('#adConfirmNo')?.addEventListener('click', closeConfirm);
    $('#adConfirm')?.addEventListener('click', (event) => {
      if (event.target === $('#adConfirm')) closeConfirm();
    });

    $('#adResultOk')?.addEventListener('click', closeResult);
    $('#adResult')?.addEventListener('click', (event) => {
      if (event.target === $('#adResult')) closeResult();
    });

    document.addEventListener('keydown', handleKeydown, true);
    window.addEventListener('resize', updateScrollbarWidth);
  }

  function bindDataEvents() {
    const eventApi = window.__TAURI__?.event;
    if (!eventApi || typeof eventApi.listen !== 'function') return;
    void eventApi.listen('accounts-changed', () => {
      if (!state.initialized) return;
      if (state.loading || state.fetching || isBusy() || isEditing()) {
        state.refreshQueued = true;
        return;
      }
      void loadData({ quiet: true });
    }).catch(() => {});
  }

  /* ── Boot ────────────────────────────────────────────────────── */

  function initAccountDefinitionWindow() {
    if (state.initialized) return;
    bindEvents();
    bindWindowClose();
    state.initialized = true;

    const gridBody = $('#adGridBody');
    if (gridBody && typeof ResizeObserver === 'function') {
      try {
        const observer = new ResizeObserver(() => updateScrollbarWidth());
        observer.observe(gridBody);
      } catch (_) {
        // ResizeObserver is optional; the render pass still measures.
      }
    }

    bindDataEvents();
    void loadData();
  }

  function openAccountDefinitionPopup() {
    if (!invoke) {
      window.alert('این پنجره فقط در نسخه دسکتاپ قابل استفاده است.');
      return;
    }
    window.dispatchEvent(new Event('account-definition-opened'));
    invoke('open_account_definition_window').catch((error) => {
      window.alert(error && error.message ? error.message : String(error));
    });
  }

  window.openAccountDefinitionPopup = openAccountDefinitionPopup;
  window.openAccountDefinitionForm = initAccountDefinitionWindow;

  if (document.body && document.body.dataset.accountDefinition === 'true') {
    if (document.readyState === 'loading') {
      document.addEventListener('DOMContentLoaded', initAccountDefinitionWindow, { once: true });
    } else {
      initAccountDefinitionWindow();
    }
  }
})();
