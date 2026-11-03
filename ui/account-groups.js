'use strict';

/* ============================================================
   تعریف گروه حساب‌ها — standalone window controller
   ============================================================ */

const tauriCore = window.__TAURI__?.core || null;
const invoke = typeof tauriCore?.invoke === 'function'
  ? tauriCore.invoke.bind(tauriCore)
  : null;
const tauriWindow = (window.__TAURI__ && window.__TAURI__.window)
  ? window.__TAURI__.window
  : null;

const TYPES = ['دارایی', 'بدهی', 'سرمایه', 'درآمد', 'هزینه', 'سایر'];
const SECTIONS = [
  'non_current_asset',
  'current_asset',
  'equity',
  'non_current_liability',
  'current_liability',
  'revenue',
  'cogs',
  'selling_expense',
  'admin_expense',
  'other_operating',
  'finance_cost',
  'other_non_operating',
  'income_tax',
  'memorandum',
];
const FA_DIGITS = ['۰', '۱', '۲', '۳', '۴', '۵', '۶', '۷', '۸', '۹'];
const charCount = (value) => [...String(value ?? '')].length;

const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => Array.from(root.querySelectorAll(sel));

const state = {
  groups: [],
  selectedId: null,
  mode: 'view', // view | new | edit
  form: { code: '', name: '', type: TYPES[0], defaultSection: '' },
  initialCode: '',
  loading: true,
  saving: false,
  deleting: false,
  dbPath: '',
  confirm: null,
  confirmReturnFocus: null,
  confirmBusy: false,
  closeApproved: false,
  errorTimer: null,
  toastTimer: null,
  toastHideTimer: null,
  refreshQueued: false,
  initialized: false,
};

/* ── Helpers ─────────────────────────────────────────────────── */

const esc = (value) => String(value ?? '').replace(/[&<>"']/g, (ch) => (
  { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[ch]
));

function toFaDigits(value) {
  return String(value ?? '').replace(/[0-9]/g, (d) => FA_DIGITS[Number(d)]);
}

function toEnDigits(value) {
  return String(value ?? '')
    .replace(/[۰-۹]/g, (ch) => String(ch.charCodeAt(0) - 0x06F0))
    .replace(/[٠-٩]/g, (ch) => String(ch.charCodeAt(0) - 0x0660));
}

function onlyDigits(value) {
  return toEnDigits(value).replace(/[^0-9]/g, '');
}

function normalizeName(value) {
  return String(value ?? '').trim().replace(/\s+/g, ' ');
}

function errorText(error) {
  if (typeof error === 'string') return error;
  if (error && typeof error.message === 'string') return error.message;
  return String(error ?? 'خطای نامشخص');
}

function normalizeGroup(raw) {
  return {
    id: Number(raw && raw.id),
    code: String((raw && raw.code) ?? ''),
    name: String((raw && raw.name) ?? ''),
    type: TYPES.includes(raw && raw.type) ? raw.type : String((raw && raw.type) ?? ''),
    defaultSection: SECTIONS.includes(raw && raw.defaultSection) ? raw.defaultSection : '',
    accountCount: Math.max(0, Number(raw && raw.accountCount) || 0),
  };
}

function sortGroups(list) {
  list.sort((a, b) => a.code.localeCompare(b.code, 'en', { numeric: true }) || a.id - b.id);
  return list;
}

function selectedGroup() {
  return state.groups.find((group) => group.id === state.selectedId) || null;
}

function setFormFromGroup(group) {
  state.form = group
    ? {
      code: toFaDigits(group.code),
      name: group.name,
      type: group.type || TYPES[0],
      defaultSection: group.defaultSection || '',
    }
    : { code: '', name: '', type: TYPES[0], defaultSection: '' };
}

function isTypingTarget(target) {
  if (!target) return false;
  const tag = target.tagName;
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || target.isContentEditable;
}

/* ── Error / toast ───────────────────────────────────────────── */

function showError(message, severity = 'validation') {
  clearTimeout(state.errorTimer);
  const bar = $('#agError');
  const text = $('#agErrorText');
  if (!bar || !text) return;
  text.textContent = message;
  bar.dataset.severity = severity;
  bar.hidden = false;

  // Transport/database errors clear themselves; validation messages stay.
  if (severity === 'network') {
    state.errorTimer = setTimeout(() => {
      bar.hidden = true;
    }, 6000);
  }
}

function clearError() {
  clearTimeout(state.errorTimer);
  const bar = $('#agError');
  if (bar) bar.hidden = true;
}

function toast(message, tone = 'ok') {
  const el = $('#agToast');
  if (!el) return;
  clearTimeout(state.toastTimer);
  clearTimeout(state.toastHideTimer);
  el.textContent = message;
  el.dataset.tone = tone;
  el.hidden = false;
  requestAnimationFrame(() => el.classList.add('show'));
  state.toastTimer = setTimeout(() => {
    el.classList.remove('show');
    state.toastHideTimer = setTimeout(() => {
      el.hidden = true;
    }, 220);
  }, 3200);
}

/* ── Rendering ───────────────────────────────────────────────── */

function render() {
  renderGrid();
  renderForm();
  renderToolbar();
  renderStatus();
}

function renderGrid() {
  const body = $('#agGridBody');
  const count = $('#agGroupCount');
  if (!body || !count) return;

  count.textContent = state.loading ? '…' : `${toFaDigits(state.groups.length)} گروه`;

  if (state.loading) {
    body.innerHTML = '<div class="ag-empty"><span>در حال بارگذاری اطلاعات…</span></div>';
    return;
  }

  if (state.groups.length === 0) {
    body.innerHTML = `
      <div class="ag-empty">
        <svg class="ico" aria-hidden="true"><use href="#i-folder" /></svg>
        <b>گروهی تعریف نشده است</b>
        <span>برای افزودن نخستین گروه حساب، دکمه «جدید» را بزنید.</span>
      </div>`;
    return;
  }

  body.innerHTML = state.groups.map((group, index) => {
    const selected = group.id === state.selectedId;
    const classes = ['ag-row'];
    if (index % 2 === 1) classes.push('even');
    if (selected) classes.push('selected');
    const tabIndex = state.mode === 'view' ? 0 : -1;
    return `
      <div class="${classes.join(' ')}" role="row" data-id="${group.id}"
           tabindex="${tabIndex}" aria-selected="${selected}">
        <div class="ag-cell" role="gridcell">${toFaDigits(index + 1)}</div>
        <div class="ag-cell ag-cell-code" role="gridcell" dir="ltr">${esc(toFaDigits(group.code))}</div>
        <div class="ag-cell" role="gridcell" title="${esc(group.name)}">${esc(group.name)}</div>
        <div class="ag-cell" role="gridcell">${esc(group.type)}</div>
      </div>`;
  }).join('');
}

function renderForm() {
  const editing = state.mode !== 'view';
  const group = selectedGroup();

  const codeInput = $('#agCode');
  const nameInput = $('#agName');
  const typeSelect = $('#agType');
  const sectionSelect = $('#agSection');

  if (codeInput) {
    codeInput.value = state.form.code;
    codeInput.disabled = state.mode !== 'new' || state.saving || state.deleting;
    codeInput.title = state.mode === 'edit' ? 'کد گروه پس از ثبت قابل تغییر نیست' : '';
  }
  if (nameInput) {
    nameInput.value = state.form.name;
    nameInput.disabled = !editing || state.saving || state.deleting;
  }
  if (typeSelect) {
    typeSelect.value = state.form.type;
    typeSelect.disabled = !editing || state.saving || state.deleting;
  }
  if (sectionSelect) {
    sectionSelect.value = state.form.defaultSection || '';
    sectionSelect.disabled = !editing || state.saving || state.deleting;
  }

  let hint = 'برای ویرایش، دکمه اصلاح را بزنید.';
  if (state.mode === 'new') {
    hint = 'کد بعدی پیشنهاد شده است؛ می‌توانید آن را تغییر دهید.';
  } else if (state.mode === 'edit') {
    hint = 'کد گروه پس از ثبت قابل تغییر نیست.';
  } else if (!group) {
    hint = 'گروهی انتخاب نشده است. برای شروع، دکمه «جدید» را بزنید.';
  }
  const hintEl = $('#agHint');
  if (hintEl) hintEl.textContent = hint;

  const chip = $('#agAccountChip');
  if (chip) {
    if (group && state.mode !== 'new') {
      chip.hidden = false;
      chip.dataset.tone = group.accountCount > 0 ? 'warn' : '';
      chip.textContent = group.accountCount > 0
        ? `${toFaDigits(group.accountCount)} حساب وابسته`
        : 'بدون حساب وابسته';
    } else {
      chip.hidden = true;
    }
  }
}

function renderToolbar() {
  const editing = state.mode !== 'view';
  const group = selectedGroup();
  const busy = state.saving || state.deleting;

  const newBtn = $('#agNew');
  const editBtn = $('#agEdit');
  const deleteBtn = $('#agDelete');
  const saveBtn = $('#agSave');
  const cancelBtn = $('#agCancel');

  if (newBtn) {
    newBtn.hidden = editing;
    newBtn.disabled = busy || state.loading;
  }
  if (editBtn) {
    editBtn.hidden = editing;
    editBtn.disabled = !group || busy || state.loading;
  }
  if (deleteBtn) {
    deleteBtn.hidden = editing;
    deleteBtn.disabled = !group
      || state.groups.length <= 1
      || group.accountCount > 0
      || busy
      || state.loading;
  }
  if (saveBtn) {
    saveBtn.hidden = !editing;
    saveBtn.disabled = busy;
  }
  if (cancelBtn) {
    cancelBtn.hidden = !editing;
    cancelBtn.disabled = busy;
  }

  const saveLabel = $('#agSaveLabel');
  if (saveLabel) saveLabel.textContent = state.saving ? 'در حال ذخیره…' : 'ذخیره (F2)';
}

function renderStatus() {
  const text = $('#agDbText');
  const status = $('#agDbStatus');
  if (!text || !status) return;

  if (state.dbPath) {
    const file = state.dbPath.split(/[\\/]/).pop() || state.dbPath;
    text.textContent = `SQLite · ${file}`;
    status.title = state.dbPath;
  } else {
    text.textContent = 'SQLite';
    status.title = '';
  }
}

function focusRow(id) {
  requestAnimationFrame(() => {
    const row = document.querySelector(`.ag-row[data-id="${id}"]`);
    if (row) row.focus({ preventScroll: true });
  });
}

function focusSelectedRow() {
  if (state.selectedId != null) focusRow(state.selectedId);
}

/* ── Data loading ────────────────────────────────────────────── */

async function loadGroups() {
  state.loading = true;
  render();

  if (!invoke) {
    state.loading = false;
    showError('حالت پیش‌نمایش — ارتباط با هسته دسکتاپ برقرار نیست.', 'network');
    render();
    return;
  }

  try {
    const payload = await invoke('list_account_groups');
    const list = payload && Array.isArray(payload.groups) ? payload.groups : [];
    state.groups = sortGroups(list.map(normalizeGroup));
    state.dbPath = String((payload && payload.databasePath) || '');

    const kept = state.selectedId;
    const next = state.groups.find((group) => group.id === kept) || state.groups[0] || null;
    state.selectedId = next ? next.id : null;
    if (state.mode === 'view') setFormFromGroup(next);

    state.loading = false;
    render();
    if (state.mode === 'view' && !state.saving) focusSelectedRow();
  } catch (error) {
    state.loading = false;
    showError(`خطا در بارگذاری گروه‌های حساب: ${errorText(error)}`, 'network');
    render();
  }

  // Changes made while a local save/delete or another reload was running are
  // picked up once, without interrupting an active edit.
  if (state.refreshQueued) {
    state.refreshQueued = false;
    void loadGroups();
  }
}

/* ── Selection / modes ───────────────────────────────────────── */

function selectGroup(id) {
  if (state.mode !== 'view') return;
  const group = state.groups.find((item) => item.id === id);
  if (!group) return;
  state.selectedId = group.id;
  setFormFromGroup(group);
  clearError();
  render();
  focusRow(group.id);
}

function startNew() {
  if (state.saving || state.deleting || state.loading) return;

  let maxCode = 0n;
  for (const group of state.groups) {
    const code = onlyDigits(group.code);
    if (!code) continue;
    try {
      const value = BigInt(code);
      if (value > maxCode) maxCode = value;
    } catch (_) {
      // A malformed legacy code should not block adding a new group.
    }
  }

  state.mode = 'new';
  state.selectedId = null;
  state.form = {
    code: toFaDigits((maxCode + 1n).toString()),
    name: '',
    type: TYPES[0],
    defaultSection: '',
  };
  state.initialCode = onlyDigits(state.form.code);
  clearError();
  render();

  requestAnimationFrame(() => {
    const nameInput = $('#agName');
    if (nameInput) {
      nameInput.focus();
      nameInput.select();
    }
  });
}

function startEdit() {
  if (state.mode !== 'view' || !selectedGroup() || state.saving || state.deleting) return;
  state.mode = 'edit';
  clearError();
  render();

  requestAnimationFrame(() => {
    const nameInput = $('#agName');
    if (nameInput) {
      nameInput.focus();
      nameInput.select();
    }
  });
}

function cancelEdit() {
  if (state.mode === 'view') return;
  state.mode = 'view';
  setFormFromGroup(selectedGroup());
  clearError();
  render();
  focusSelectedRow();
}

function isDirty() {
  const group = selectedGroup();
  if (state.mode === 'new') {
    return normalizeName(state.form.name) !== ''
      || onlyDigits(state.form.code) !== state.initialCode
      || state.form.type !== TYPES[0]
      || state.form.defaultSection !== '';
  }
  if (state.mode === 'edit' && group) {
    return normalizeName(state.form.name) !== group.name
      || state.form.type !== group.type
      || state.form.defaultSection !== group.defaultSection;
  }
  return false;
}

/* ── Save / delete ───────────────────────────────────────────── */

async function save() {
  if (state.mode === 'view' || state.saving || state.deleting) return;

  const wasNew = state.mode === 'new';
  const code = onlyDigits(state.form.code);
  const name = normalizeName(state.form.name);
  const type = state.form.type;
  const defaultSection = state.form.defaultSection || '';

  if (!code || !name) {
    showError('لطفاً کد و نام گروه را وارد کنید.', 'validation');
    return;
  }
  if (charCount(code) > 50) {
    showError('کد گروه نمی‌تواند بیشتر از ۵۰ کاراکتر باشد.', 'validation');
    return;
  }
  if (!/^0*[1-9][0-9]*$/.test(code)) {
    showError('کد گروه باید فقط شامل ارقام انگلیسی و بزرگ‌تر از صفر باشد.', 'validation');
    return;
  }
  if (charCount(name) > 255) {
    showError('نام گروه نمی‌تواند بیشتر از ۲۵۵ کاراکتر باشد.', 'validation');
    return;
  }
  if (!TYPES.includes(type)) {
    showError('نوع حساب انتخاب‌شده معتبر نیست.', 'validation');
    return;
  }
  if (defaultSection && !SECTIONS.includes(defaultSection)) {
    showError('بخش پیش‌فرض صورت مالی انتخاب‌شده معتبر نیست.', 'validation');
    return;
  }
  if (wasNew && state.groups.some((group) => group.code === code)) {
    showError('کد گروه تکراری است.', 'validation');
    return;
  }
  if (!invoke) {
    showError('حالت پیش‌نمایش — ارتباط با هسته دسکتاپ برقرار نیست.', 'network');
    return;
  }

  const editingGroup = wasNew ? null : selectedGroup();
  if (!wasNew && !editingGroup) {
    showError('گروه حساب انتخاب‌شده یافت نشد.', 'validation');
    return;
  }

  state.saving = true;
  clearError();
  render();

  try {
    let saved;
    if (wasNew) {
      saved = normalizeGroup(await invoke('create_account_group', {
        code,
        name,
        groupType: type,
        defaultSection,
      }));
      if (!Number.isFinite(saved.id) || saved.id <= 0) {
        throw new Error('پاسخ نامعتبر از پایگاه داده دریافت شد.');
      }
      state.groups.push(saved);
    } else {
      saved = normalizeGroup(await invoke('update_account_group', {
        id: editingGroup.id,
        code: editingGroup.code,
        name,
        groupType: type,
        defaultSection,
      }));
      if (!Number.isFinite(saved.id) || saved.id <= 0) {
        throw new Error('پاسخ نامعتبر از پایگاه داده دریافت شد.');
      }
      state.groups = state.groups.map((group) => (group.id === saved.id ? saved : group));
    }

    sortGroups(state.groups);
    state.selectedId = saved.id;
    state.mode = 'view';
    setFormFromGroup(saved);
    state.saving = false;
    render();
    toast(wasNew ? 'گروه حساب با موفقیت ایجاد شد.' : 'گروه حساب با موفقیت به‌روزرسانی شد.');
    focusSelectedRow();
    flushQueuedGroupRefresh();
  } catch (error) {
    state.saving = false;
    render();
    showError(errorText(error), 'validation');
  }
}

function deleteSelected() {
  if (state.mode !== 'view' || state.saving || state.deleting) return;

  const group = selectedGroup();
  if (!group) return;

  if (state.groups.length <= 1) {
    showError('حداقل یک گروه باید وجود داشته باشد؛ حذف آخرین گروه مجاز نیست.', 'validation');
    return;
  }
  if (group.accountCount > 0) {
    showError(
      `این گروه حساب دارای ${toFaDigits(group.accountCount)} حساب (کل/معین/تفصیل) است. ابتدا تمام حساب‌های زیرمجموعه این گروه را حذف کنید.`,
      'validation'
    );
    return;
  }
  if (!invoke) {
    showError('حالت پیش‌نمایش — ارتباط با هسته دسکتاپ برقرار نیست.', 'network');
    return;
  }

  openConfirm({
    title: 'حذف گروه حساب',
    message: `آیا از حذف گروه «${group.name}» اطمینان دارید؟`,
    confirmLabel: 'حذف',
    tone: 'danger',
    onConfirm: async () => {
      state.deleting = true;
      clearError();
      render();
      try {
        await invoke('delete_account_group', { id: group.id });
        state.groups = state.groups.filter((item) => item.id !== group.id);
        const next = state.groups.find((item) => item.id === state.selectedId)
          || state.groups[0]
          || null;
        state.selectedId = next ? next.id : null;
        state.mode = 'view';
        setFormFromGroup(next);
        toast('گروه حساب با موفقیت حذف شد.');
        focusSelectedRow();
      } catch (error) {
        showError(errorText(error), 'validation');
      } finally {
        state.deleting = false;
        render();
        flushQueuedGroupRefresh();
      }
    },
  });
}

function flushQueuedGroupRefresh() {
  if (!state.refreshQueued) return;
  state.refreshQueued = false;
  void loadGroups();
}

/* ── Confirmation dialog ─────────────────────────────────────── */

function openConfirm({ title, message, confirmLabel = 'بله', tone = 'warn', onConfirm }) {
  state.confirm = { title, message, confirmLabel, tone, onConfirm };
  state.confirmBusy = false;
  state.confirmReturnFocus = document.activeElement;

  $('#agConfirmTitle').textContent = title;
  $('#agConfirmText').textContent = message;
  $('#agConfirmYes').textContent = confirmLabel;
  $('#agConfirmBox').dataset.tone = tone;
  $('#agConfirm').hidden = false;
  $('#agConfirmYes').disabled = false;
  $('#agConfirmNo').disabled = false;

  requestAnimationFrame(() => {
    const noButton = $('#agConfirmNo');
    if (noButton) noButton.focus();
  });
}

function closeConfirm() {
  if (state.confirmBusy) return;
  state.confirm = null;
  $('#agConfirm').hidden = true;
  const returnFocus = state.confirmReturnFocus;
  state.confirmReturnFocus = null;
  if (returnFocus?.isConnected && !returnFocus.closest('[inert]')) returnFocus.focus();
}

async function confirmYes() {
  if (!state.confirm || state.confirmBusy) return;
  state.confirmBusy = true;
  $('#agConfirmYes').disabled = true;
  $('#agConfirmNo').disabled = true;

  const action = state.confirm.onConfirm;
  try {
    await action();
  } catch (error) {
    showError(errorText(error), 'validation');
  } finally {
    state.confirmBusy = false;
    state.confirm = null;
    $('#agConfirm').hidden = true;
    state.confirmReturnFocus = null;
  }
}

/* ── Window close guard ──────────────────────────────────────── */

function closeWindow() {
  state.closeApproved = true;
  if (tauriWindow && tauriWindow.getCurrentWindow) {
    tauriWindow.getCurrentWindow().close().catch(() => {
      state.closeApproved = false;
    });
  } else {
    window.close();
  }
}

function requestClose() {
  if (state.saving || state.deleting) return;

  if (isDirty()) {
    openConfirm({
      title: 'بستن فرم',
      message: 'تغییرات ذخیره‌نشده از بین می‌رود. بستن فرم را تأیید می‌کنید؟',
      confirmLabel: 'بستن',
      tone: 'danger',
      onConfirm: () => closeWindow(),
    });
    return;
  }

  closeWindow();
}

function bindWindowClose() {
  if (!tauriWindow || !tauriWindow.getCurrentWindow) return;
  const win = tauriWindow.getCurrentWindow();
  const result = win.onCloseRequested((event) => {
    if (state.closeApproved) return;
    if (state.confirm) {
      event.preventDefault();
      return;
    }
    if (state.saving || state.deleting) {
      event.preventDefault();
      return;
    }
    if (isDirty()) {
      event.preventDefault();
      requestClose();
    }
  });
  if (result && typeof result.catch === 'function') result.catch(() => {});
}

/* ── Keyboard / navigation ───────────────────────────────────── */

function moveSelection(direction) {
  if (state.groups.length === 0) return;
  const current = state.groups.findIndex((group) => group.id === state.selectedId);
  let nextIndex;
  if (current === -1) {
    nextIndex = direction > 0 ? 0 : state.groups.length - 1;
  } else if (direction > 0) {
    nextIndex = Math.min(current + 1, state.groups.length - 1);
  } else {
    nextIndex = Math.max(current - 1, 0);
  }
  const next = state.groups[nextIndex];
  if (next) selectGroup(next.id);
}

function handleKeydown(event) {
  if (!state.initialized) return;

  if (state.confirm) {
    if (event.key === 'Escape') {
      event.preventDefault();
      closeConfirm();
    }
    return;
  }

  if (event.key === 'Escape') {
    event.preventDefault();
    if (state.mode === 'view') {
      requestClose();
    } else if (isDirty()) {
      // Escape must not throw away typed changes silently; it follows the
      // same confirmation path as the cancel and close buttons.
      openConfirm({
        title: 'لغو ویرایش',
        message: 'تغییرات ذخیره‌نشده از بین می‌رود. لغو ویرایش را تأیید می‌کنید؟',
        confirmLabel: 'لغو تغییرات',
        tone: 'danger',
        onConfirm: () => cancelEdit(),
      });
    } else {
      cancelEdit();
    }
    return;
  }

  if (event.key === 'F2') {
    event.preventDefault();
    save();
    return;
  }

  if (event.key === 'F3') {
    event.preventDefault();
    startEdit();
    return;
  }

  if (isTypingTarget(event.target)) return;

  if (event.key === 'Delete' && state.mode === 'view') {
    event.preventDefault();
    deleteSelected();
    return;
  }

  if (state.mode === 'view' && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) {
    if (state.groups.length === 0) return;
    event.preventDefault();
    moveSelection(event.key === 'ArrowDown' ? 1 : -1);
  }
}

/* ── Event binding / init ────────────────────────────────────── */

function bindEvents() {
  $('#agForm').addEventListener('submit', (event) => {
    event.preventDefault();
    save();
  });

  $('#agCode').addEventListener('input', (event) => {
    const digits = onlyDigits(event.target.value);
    state.form.code = toFaDigits(digits);
    event.target.value = state.form.code;
  });

  $('#agName').addEventListener('input', (event) => {
    state.form.name = event.target.value;
  });

  $('#agType').addEventListener('change', (event) => {
    state.form.type = event.target.value;
  });

  $('#agSection').addEventListener('change', (event) => {
    state.form.defaultSection = event.target.value;
  });

  $('#agGridBody').addEventListener('click', (event) => {
    const row = event.target.closest('.ag-row');
    if (row) selectGroup(Number(row.dataset.id));
  });

  $('#agGridBody').addEventListener('dblclick', (event) => {
    const row = event.target.closest('.ag-row');
    if (!row || state.mode !== 'view') return;
    selectGroup(Number(row.dataset.id));
    startEdit();
  });

  $('#agGridBody').addEventListener('keydown', (event) => {
    if (event.key !== 'Enter' && event.key !== ' ') return;
    const row = event.target.closest('.ag-row');
    if (!row) return;
    event.preventDefault();
    selectGroup(Number(row.dataset.id));
  });

  $('#agNew').addEventListener('click', startNew);
  $('#agEdit').addEventListener('click', startEdit);
  $('#agDelete').addEventListener('click', deleteSelected);
  $('#agSave').addEventListener('click', save);
  $('#agCancel').addEventListener('click', cancelEdit);
  $('#agErrorClose').addEventListener('click', clearError);

  $('#agConfirmYes').addEventListener('click', confirmYes);
  $('#agConfirmNo').addEventListener('click', closeConfirm);
  $('#agConfirm').addEventListener('click', (event) => {
    if (event.target === $('#agConfirm')) closeConfirm();
  });
  $('#agConfirm').addEventListener('keydown', (event) => {
    if (event.key !== 'Tab') return;
    const focusable = [$('#agConfirmYes'), $('#agConfirmNo')]
      .filter((button) => button && !button.disabled);
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
  });

  document.addEventListener('keydown', handleKeydown);
}

function init() {
  bindEvents();
  bindWindowClose();
  state.initialized = true;
  const eventApi = window.__TAURI__?.event;
  if (typeof eventApi?.listen === 'function') {
    void eventApi.listen('accounts-changed', () => {
      if (!state.initialized) return;
      if (state.loading || state.saving || state.deleting) {
        state.refreshQueued = true;
        return;
      }
      void loadGroups();
    }).catch(() => {});
  }
  loadGroups();
}

document.addEventListener('DOMContentLoaded', init);
