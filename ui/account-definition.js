/* ============================================================
   تعریف حساب‌های کل، معین و تفصیلی — standalone window controller
   Drill-down (کل ← معین ← تفصیلی ۱) with an always-visible
   account-details form, modelled on the WebApp popup.
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
  const CODE_WIDTHS = { 1: 3, 2: 2, 3: 2 };
  const MAX_LEVEL = 3;
  const MAX_ACCOUNT_NAME = 255;
  const NATURES = ['debtor', 'creditor', 'dual'];

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
    };
  }

  /* ── Code suggestion (same policy as the WebApp) ─────────────── */

  function nextAccountCode(level, parentCode) {
    const width = CODE_WIDTHS[level];
    if (!width || (level > 1 && !parentCode)) return '';
    const usedCodes = new Set(state.accounts.map((account) => account.code));
    const min = level === 1 ? 100 : 1;
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
    const next = (candidates.length ? Math.max(...candidates) : (level === 1 ? 100 : 0)) + 1;
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
    if (level === 1) return code.length === width && Number(code) >= 100;
    if (!parentCode || !code.startsWith(parentCode)) return false;
    if (code.length !== parentCode.length + width) return false;
    return Number(code.slice(parentCode.length)) > 0;
  }

  /* ── Error / toast ───────────────────────────────────────────── */

  function showError(message) {
    state.error = String(message ?? '');
    const bar = $('#adError');
    const text = $('#adErrorText');
    if (!bar || !text) return;
    text.textContent = state.error;
    bar.hidden = !state.error;
  }

  function clearError() {
    state.error = '';
    const bar = $('#adError');
    if (bar) bar.hidden = true;
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
    renderBreadcrumb();
    renderGrid();
    renderForm();
    renderToolbar();
  }

  function renderBreadcrumb() {
    const nav = $('#adBreadcrumb');
    if (!nav) return;
    const chain = drillChain();
    const editing = isEditing();
    const busy = isBusy();
    const notReady = state.loading || !state.loaded;

    const parts = [];
    const rootDisabled = editing || busy || notReady || state.drillPath.length === 0;
    parts.push(
      `<button type="button" class="ad-btn" data-root`
      + `${rootDisabled ? ' disabled' : ''}>حساب‌های کل</button>`,
    );

    chain.forEach((account, index) => {
      const current = index === chain.length - 1;
      parts.push('<span class="ad-sep" aria-hidden="true">‹</span>');
      parts.push(
        `<button type="button" class="ad-btn" data-goto="${esc(account.code)}"`
        + `${editing || busy || notReady ? ' disabled' : ''}`
        + `${current ? ' aria-current="location"' : ''}>`
        + `${esc(toFaDigits(account.code))} - ${esc(account.name)}</button>`,
      );
    });

    if (state.drillPath.length > 0) {
      parts.push('<span class="ad-sep" aria-hidden="true">‹</span>');
      parts.push(
        `<button type="button" class="ad-btn" data-back`
        + `${editing || busy || notReady ? ' disabled' : ''}>بازگشت به سطح بالاتر</button>`,
      );
    }

    parts.push('<span class="ad-breadcrumb-spacer"></span>');
    parts.push(
      `<button type="button" class="ad-btn" data-reload`
      + `${editing || busy || state.loading ? ' disabled' : ''} title="بارگذاری مجدد">`
      + '<svg class="ico" aria-hidden="true"><use href="#ad-i-refresh" /></svg>'
      + '<span>بارگذاری مجدد</span></button>',
    );

    nav.innerHTML = parts.join('');
  }

  function renderGrid() {
    const body = $('#adGridBody');
    if (!body) return;

    // Drop a stale drill path if its accounts no longer match the tree.
    const chain = drillChain();
    if (chain.length !== state.drillPath.length) {
      state.drillPath = chain.map((account) => account.code);
    }

    const visible = visibleAccounts();

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
      body.innerHTML = '<div class="ad-state">حسابی در این سطح تعریف نشده است</div>';
      return;
    }

    body.innerHTML = visible.map((account, index) => {
      const selected = account.id === state.selectedId;
      const classes = ['ad-row'];
      if (index % 2 === 1) classes.push('even');
      if (selected) classes.push('selected');
      if (!account.isActive) classes.push('is-inactive');
      const tabIndex = selected && state.mode === 'view' && !isBusy() ? 0 : -1;
      return `
        <div class="${classes.join(' ')}" role="row" data-id="${account.id}"
             tabindex="${tabIndex}" aria-selected="${selected}">
          <div class="ad-cell ad-center ad-num" role="gridcell">${toFaDigits(index + 1)}</div>
          <div class="ad-cell ad-center ad-code" dir="ltr" role="gridcell">${esc(toFaDigits(account.code))}</div>
          <div class="ad-cell ad-start ad-name" role="gridcell" title="${esc(account.name)}">${esc(account.name)}</div>
          <div class="ad-cell ad-center" role="gridcell">${esc(levelName(account.level))}</div>
          <div class="ad-cell ad-start" role="gridcell" title="${esc(account.parentName || (account.level === 1 ? account.groupName : ''))}">${esc(account.parentName || (account.level === 1 ? account.groupName : '') || '—')}</div>
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
  }

  function fillGroupOptions() {
    const groupSelect = $('#adGroup');
    if (!groupSelect) return;
    groupSelect.innerHTML = state.groups.map((group) => (
      `<option value="${esc(group.code)}">${esc(toFaDigits(group.code))} - ${esc(group.name)}</option>`
    )).join('');
  }

  function renderToolbar() {
    const editing = isEditing();
    const busy = isBusy();
    const ready = state.loaded && !state.loading;
    const selected = selectedAccount();
    const parent = currentParent();

    const newBtn = $('#adNew');
    const editBtn = $('#adEdit');
    const deleteBtn = $('#adDelete');
    const deleteAllBtn = $('#adDeleteAll');
    const exportBtn = $('#adExport');
    const importBtn = $('#adImport');
    const editActions = $('#adEditActions');
    const saveBtn = $('#adSave');
    const cancelBtn = $('#adCancel');

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
    if (editActions) editActions.hidden = !editing;
    if (saveBtn) saveBtn.disabled = busy;
    if (cancelBtn) cancelBtn.disabled = busy;

    const saveLabel = $('#adSaveLabel');
    if (saveLabel) saveLabel.textContent = state.saving ? 'در حال ذخیره…' : 'ذخیره (F2)';

    const deleteAllLabel = $('#adDeleteAllLabel');
    if (deleteAllLabel) {
      deleteAllLabel.textContent = state.deletingAll ? 'در حال حذف…' : 'حذف همه';
    }

    const exportLabel = $('#adExportLabel');
    if (exportLabel) exportLabel.textContent = state.exporting ? 'در حال تهیه…' : 'خروجی اکسل';

    const importLabel = $('#adImportLabel');
    if (importLabel) importLabel.textContent = state.importing ? 'در حال ثبت…' : 'ورودی اکسل';

    // The parent is implicit in the breadcrumb; the hint is only visual.
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
      state.loaded = true;
      state.loading = false;
      fillGroupOptions();

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

  function navigateTo(path) {
    if (isEditing() || isBusy()) return;
    state.lastRowClick = { id: null, at: 0 };
    const chain = resolveChain(path);
    state.drillPath = chain.map((account) => account.code);
    state.mode = 'view';
    const visible = visibleAccounts();
    const next = visible[0] || null;
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
    const visible = visibleAccounts();
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
    };
    state.initial = snapshot();
    clearError();
    renderAll();
    focusName();
  }

  function startEdit() {
    const account = selectedAccount();
    if (!account || isEditing() || isBusy() || !state.loaded) return;
    state.mode = 'edit';
    state.form = formFromAccount(account);
    state.initial = snapshot();
    clearError();
    renderAll();
    focusName();
  }

  function cancelEdit() {
    if (!isEditing() || isBusy()) return;
    state.mode = 'view';
    const account = selectedAccount();
    state.form = account ? formFromAccount(account) : emptyForm();
    state.initial = snapshot();
    clearError();
    renderAll();
    focusSelectedRow();
    maybeQueuedRefresh();
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

    if (!code || !name) {
      showError('لطفاً کد و نام حساب را وارد کنید.');
      return;
    }
    if (charCount(name) > MAX_ACCOUNT_NAME) {
      showError(`نام حساب نباید بیش از ${toFaDigits(MAX_ACCOUNT_NAME)} کاراکتر باشد.`);
      return;
    }
    if (!/^\d+$/.test(code)) {
      showError('کد حساب باید فقط شامل عدد باشد.');
      return;
    }
    if (mode === 'new' && !isValidNewAccountCode(code, level, parentCode)) {
      const width = CODE_WIDTHS[level];
      showError(level === 1
        ? `کد حساب کل باید دقیقاً ${toFaDigits(width || 3)} رقم باشد و از ۱۰۰ شروع شود.`
        : `کد حساب ${levelName(level)} باید با کد والد شروع شود و ${toFaDigits(width || 2)} رقم به آن اضافه کند.`);
      return;
    }
    if (mode === 'new' && state.accounts.some((account) => account.code === code)) {
      const duplicate = state.accounts.find((account) => account.code === code);
      showError(`کد «${toFaDigits(code)}» قبلاً برای حساب «${duplicate.name}» ثبت شده است.`);
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
      return;
    }
    if (!nature) {
      showError('ماهیت حساب باید بدهکار، بستانکار یا ماهیت دوگانه باشد.');
      return;
    }

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
      message: 'حساب‌ها از فایل اکسل مطابق قالب WebApp وارد شوند؟',
      confirmLabel: 'انتخاب فایل',
      tone: 'info',
      onConfirm: async () => {
        state.importing = true;
        clearError();
        renderAll();
        try {
          const result = await invoke('import_account_excel');
          if (!result) return;

          state.mode = 'view';
          state.refreshQueued = false;
          await loadData({ keepSelection: false });

          const created = Number(result.createdCount) || 0;
          const skipped = Number(result.skippedCount) || 0;
          const errors = Array.isArray(result.errors) ? result.errors : [];

          toast(
            `${toFaDigits(created)} حساب جدید و ${toFaDigits(skipped)} حساب موجود`
              + (errors.length ? `؛ ${toFaDigits(errors.length)} خطا` : ''),
            errors.length ? 'warn' : 'ok',
          );
          if (errors.length > 0) {
            showResult({
              title: 'نتیجه ورودی اکسل',
              tone: 'warn',
              message: `${toFaDigits(created)} حساب ایجاد شد و ${toFaDigits(skipped)} حساب موجود بود.`,
              list: errors,
            });
          }
        } catch (error) {
          showError(errorText(error));
        } finally {
          state.importing = false;
          renderAll();
          maybeQueuedRefresh();
        }
      },
    });
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
    if (isEditing()) {
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
      if (isEditing()) {
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

    if (event.key === 'Escape') {
      event.preventDefault();
      if (isEditing()) {
        cancelEdit();
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
      if (state.drillPath.length > 0) {
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
    const nav = $('#adBreadcrumb');
    nav?.addEventListener('click', (event) => {
      const button = event.target.closest('button');
      if (!button || button.disabled) return;
      if (button.hasAttribute('data-root')) {
        navigateTo([]);
        return;
      }
      if (button.hasAttribute('data-back')) {
        goBack();
        return;
      }
      if (button.hasAttribute('data-reload')) {
        void loadData();
        return;
      }
      if (button.hasAttribute('data-goto')) {
        const code = button.dataset.goto;
        const chain = drillChain();
        const index = chain.findIndex((account) => account.code === code);
        if (index >= 0) navigateTo(chain.slice(0, index + 1).map((account) => account.code));
      }
    });

    const gridBody = $('#adGridBody');
    gridBody?.addEventListener('click', (event) => {
      const row = event.target.closest('.ad-row');
      if (!row) return;
      const account = state.accounts.find((item) => item.id === Number(row.dataset.id));
      if (!account) return;

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
    });
    $('#adName')?.addEventListener('input', (event) => {
      state.form.name = event.target.value;
    });
    $('#adGroup')?.addEventListener('change', (event) => {
      state.form.groupCode = event.target.value;
    });
    $('#adNature')?.addEventListener('change', (event) => {
      state.form.nature = event.target.value;
    });
    $('#adActive')?.addEventListener('change', (event) => {
      state.form.isActive = event.target.checked;
    });

    $('#adNew')?.addEventListener('click', startNew);
    $('#adEdit')?.addEventListener('click', startEdit);
    $('#adDelete')?.addEventListener('click', deleteSelected);
    $('#adDeleteAll')?.addEventListener('click', deleteAll);
    $('#adSave')?.addEventListener('click', () => { void save(); });
    $('#adCancel')?.addEventListener('click', cancelEdit);
    $('#adExport')?.addEventListener('click', () => { void exportExcel(); });
    $('#adImport')?.addEventListener('click', importExcel);
    $('#adErrorClose')?.addEventListener('click', clearError);

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
