'use strict';

/* ============================================================
   نرم‌افزار حسابداری — main window controller
   ============================================================ */

const tauriCore = window.__TAURI__?.core || null;
const invoke = typeof tauriCore?.invoke === 'function'
  ? tauriCore.invoke.bind(tauriCore)
  : null;

const currentWindow = (window.__TAURI__ && window.__TAURI__.window
  && typeof window.__TAURI__.window.getCurrentWindow === 'function')
  ? window.__TAURI__.window.getCurrentWindow()
  : null;
// The trial balance is shown in its own OS window, but it reuses this
// controller and hides the main-window chrome (see accounting.css).
const IS_TRIAL_POPUP = window.__IS_TRIAL_BALANCE_POPUP__ === true
  || currentWindow?.label === 'trial-balance';
// The voucher list lives in its own popup window, like the voucher form.
const IS_VOUCHER_LIST_POPUP = window.__IS_VOUCHER_LIST_POPUP__ === true
  || currentWindow?.label === 'voucher-list';
if (IS_TRIAL_POPUP && document.body) document.body.classList.add('is-trial-popup');
if (IS_VOUCHER_LIST_POPUP && document.body) document.body.classList.add('is-voucher-list-popup');

const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => Array.from(root.querySelectorAll(sel));

const FA_DIGITS = '۰۱۲۳۴۵۶۷۸۹';
const AR_DIGITS = '٠١٢٣٤٥٦٧٨٩';
const APP_VERSION = '۵.۰';
// Fallbacks; `loadSettings` replaces them with the backend limits.
let VOUCHER_PAGE_SIZE = 200;
let JOURNAL_PAGE_SIZE = 500;

const state = {
  page: 'workspace',
  settings: null,
  accounts: [],
  groups: [],
  accountsLoaded: false,
  groupsLoaded: false,
  databasePath: '',
  filters: {},
  listOffsets: { vouchers: 0, journal: 0 },
  accountFilters: { search: '', group: '', level: '' },
  accountDrillPath: [],
  voucherCount: 0,
  refreshOnFocus: false,
  ready: false,
};

/*
 * ساختار منوبار اصلی — برگرفته از رابط سوم برنامه‌ی WebApp.
 * آیتم‌هایی که `page` دارند به صفحه‌ی موجود در همین برنامه می‌روند،
 * آیتم‌های `command` به پنجره/عمل موجود وصل می‌شوند و بقیه فعلاً
 * «در دست ساخت» هستند تا بعداً فرم مربوطه برایشان ساخته شود.
 */
const MENU_BAR = [
  {
    label: 'اطلاعات پایه',
    items: [
      { label: 'فضای کار', icon: '🖼️', page: 'workspace' },
      { label: 'تعریف گروه حسابها', icon: '📂', command: 'groups-window' },
      { label: 'تعریف حسابهای کل، معین و تفصیلی ۱', icon: '📁', command: 'account-definition' },
      { label: 'مرور حسابها', icon: '🔍', page: 'accounts' },
      { label: 'لیست اسناد حسابداری', icon: '📜', command: 'voucher-list' },
      { label: 'مرتب‌سازی و ادغام اسناد', icon: '🧮', command: 'reorganize-vouchers' },
      { label: 'ورود دسته‌جمعی اسناد از اکسل', icon: '📥', command: 'bulk-voucher-import' },
      { label: 'خروجی دسته‌جمعی اسناد به اکسل', icon: '📤' },
      { label: 'حذف دسته‌جمعی اسناد', icon: '🗑️', command: 'bulk-voucher-delete' },
      { label: 'تغییر وضعیت اسناد (موقت/دائمی)', icon: '🔁' },
    ],
  },
  {
    label: 'عملیات مالی',
    items: [
      { label: 'صدور سند حسابداری', icon: '📝', command: 'new-voucher' },
    ],
  },
  {
    label: 'گزارشات حسابداری',
    items: [
      { label: 'گزارش تراز حساب', icon: '📊', page: 'trial' },
      { label: 'صورت سود و زیان', icon: '📈', page: 'income' },
      { label: 'ترازنامه', icon: '⚖️', page: 'balance' },
      { label: 'کدینگ / چارت حساب‌ها', icon: '🗂️', page: 'accounts' },
      { label: 'اسناد به تفکیک وضعیت', icon: '🏷️' },
      { label: 'مرور پیشرفته حساب', icon: '🔎' },
      { label: 'تحلیل حساب‌ها', icon: '🧠' },
    ],
  },
  {
    label: 'صورت‌های مالی',
    items: [
      { label: 'صورت سود و زیان', icon: '📈', page: 'income' },
      { label: 'ترازنامه (صورت وضعیت مالی)', icon: '⚖️', page: 'balance' },
      { label: 'صورت تغییرات حقوق مالکانه', icon: '🏛️' },
      { label: 'صورت جریان‌های نقدی (استاندارد ۲)', icon: '💵' },
      { label: 'صورت بهای تمام‌شده کالای فروش‌رفته', icon: '🏭' },
      { label: 'تخصیص سود و اندوخه', icon: '🪙' },
      { label: 'گزارش مقایسه‌ای دو دوره', icon: '🔀' },
    ],
  },
  {
    label: 'دفاتر قانونی',
    items: [
      { label: 'دفتر کل / معین / تفصیلی', icon: '📚', page: 'ledger' },
      { label: 'دفتر روزنامه', icon: '📖', page: 'journal' },
      { label: 'دفتر معین به تفکیک تاریخ', icon: '🗓️' },
    ],
  },
  {
    label: 'مدیریت دوره مالی',
    items: [
      { label: 'بستن حساب‌ها (ویزارد پایان سال)', icon: '🔒' },
      { label: 'صدور سند افتتاحیه', icon: '📄' },
      { label: 'تعریف سال مالی جدید', icon: '📅' },
    ],
  },
  {
    label: 'حقوق و دستمزد',
    items: [],
    empty: 'برای این بخش هنوز گزینه‌ای تعریف نشده است',
  },
  {
    label: 'تنظیمات',
    size: 'compact',
    items: [
      { label: 'تنظیمات برنامه', icon: '⚙️', page: 'settings' },
      { label: 'تنظیمات گزارش‌ها', icon: '🖨️' },
      { label: 'تغییر تصویر پس‌زمینه', icon: '🖼️', command: 'choose-wallpaper' },
      { label: 'تنظیمات فونت', icon: '🔤' },
      { label: 'تنظیمات زوم (بزرگ‌نمایی)', icon: '🔍' },
      { label: 'مدیریت کاربران', icon: '👥' },
      { label: 'تعریف سال مالی جدید', icon: '📅' },
    ],
  },
  {
    label: 'راهنما',
    size: 'small',
    items: [
      { label: 'راهنمای نرم‌افزار', icon: '📖', command: 'help' },
      { label: 'درباره', icon: 'ℹ️', command: 'about' },
      { label: 'میانبرهای صفحه‌کلید', icon: '⌨️', command: 'shortcuts' },
    ],
  },
];

/* ── Text and number helpers ────────────────────────────────── */

function toFa(value) {
  return String(value ?? '').replace(/[0-9]/g, (d) => FA_DIGITS[Number(d)]);
}

function toEn(value) {
  return String(value ?? '')
    .replace(/[۰-۹]/g, (ch) => String(ch.charCodeAt(0) - 0x06F0))
    .replace(/[٠-٩]/g, (ch) => String(ch.charCodeAt(0) - 0x0660));
}

function digitsOnly(value) {
  return toEn(value).replace(/[^0-9]/g, '');
}

function amountBigInt(value) {
  const text = String(value ?? 0).replace(/,/g, '').trim();
  if (/^-?\d+$/.test(text)) return BigInt(text);
  const numeric = Number(value);
  return BigInt(Number.isFinite(numeric) ? Math.trunc(numeric) : 0);
}

function normalizeSearch(value) {
  return toEn(String(value ?? ''))
    .replace(/[يى]/g, 'ی')
    .replace(/ك/g, 'ک')
    .replace(/\u200c/g, '')
    .replace(/\s+/g, ' ')
    .trim()
    .toLowerCase();
}

function esc(value) {
  return String(value ?? '').replace(/[&<>"']/g, (ch) => (
    { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[ch]
  ));
}

function money(value) {
  const amount = amountBigInt(value);
  const negative = amount < 0n;
  const text = (negative ? -amount : amount).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return `${negative ? '-' : ''}${toFa(text)}`;
}

function faDate(value) {
  if (!value) return '—';
  return toFa(value);
}

function errorText(error) {
  if (typeof error === 'string') return error;
  if (error && typeof error.message === 'string') return error.message;
  return String(error ?? 'خطای نامشخص');
}

function icon(name) {
  return `<svg class="ico" aria-hidden="true"><use href="#${name}" /></svg>`;
}

function levelLabel(level) {
  return ({ 1: 'کل', 2: 'معین', 3: 'تفصیلی ۱', 4: 'تفصیلی ۲ (قدیمی)' })[Number(level)] || '—';
}

function accountPathTo(accounts, account) {
  const path = [];
  const visited = new Set();
  let current = account;
  while (current && !visited.has(current.code)) {
    visited.add(current.code);
    path.unshift(current.code);
    current = current.parentCode
      ? accounts.find((item) => item.code === current.parentCode)
      : null;
  }
  return path;
}

function nextAccountCode(accounts, level, parentCode = '') {
  const sameLevel = accounts.filter((account) => Number(account.level) === level);
  const siblings = sameLevel.filter((account) =>
    level === 1 ? !account.parentCode : account.parentCode === parentCode
  );
  const width = level === 1
    ? Math.max(4, ...sameLevel.map((account) => String(account.code).length))
    : Math.max(
      2,
      ...sameLevel.map((account) => String(account.code).length - String(account.parentCode || '').length)
    );
  const prefix = level === 1 ? '' : parentCode;
  const used = new Set(accounts.map((account) => String(account.code)));
  const suffixes = siblings
    .map((account) => String(account.code).slice(prefix.length))
    .filter((suffix) => /^\d+$/.test(suffix))
      .map(BigInt);
    const minimum = level === 1 ? 10n ** BigInt(width - 1) : 1n;
    const maximum = 10n ** BigInt(width) - 1n;
    const next = (suffixes.length ? suffixes.reduce((max, value) => value > max ? value : max, minimum - 1n) : minimum - 1n) + 1n;
    const ranges = [
      [next, maximum],
      [minimum, next - 1n < maximum ? next - 1n : maximum],
    ];
    for (const [start, end] of ranges) {
      let suffix = start;
      let checked = 0;
      while (suffix <= end && checked <= accounts.length) {
        const code = `${prefix}${suffix.toString().padStart(width, '0')}`;
        if (!used.has(code)) return code;
        suffix += 1n;
        checked += 1;
      }
  }
    return '';
}

function natureLabel(nature) {
  return ({ debtor: 'بدهکار', creditor: 'بستانکار', dual: 'دوگانه' })[nature] || nature || '—';
}

function typeTone(type) {
  return ({
    'دارایی': 'info',
    'بدهی': 'warn',
    'سرمایه': 'ok',
    'درآمد': 'ok',
    'هزینه': 'danger',
  })[type] || 'neutral';
}

function levelClass(level) {
  const n = Number(level);
  return n >= 1 && n <= 4 ? `l${n}` : '';
}

function badge(value, tone) {
  return `<span class="badge" data-tone="${esc(tone || 'neutral')}">${esc(value)}</span>`;
}

/* ── Status, toast and modal ────────────────────────────────── */

let toastTimer = 0;

function toast(message, tone = 'info') {
  const el = $('#toast');
  if (!el) return;
  el.textContent = message;
  el.dataset.tone = tone;
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { el.hidden = true; }, 3400);
}

function setStatus(message, tone = 'ok') {
  const text = $('#statusText');
  const dot = $('#statusDot');
  if (text) text.textContent = message;
  if (dot) dot.dataset.tone = tone;
}

let dialogCloseHandler = null;
let dialogReturnFocus = null;
let dialogAppWasInert = false;
let dialogKeyHandler = null;
let dialogCanClose = null;

function closeDialog() {
  const modal = $('#modal');
  if (!modal || modal.hidden) return;
  if (dialogCanClose && !dialogCanClose()) return;
  modal.hidden = true;
  dialogCanClose = null;
  if (dialogKeyHandler) {
    modal.removeEventListener('keydown', dialogKeyHandler);
    dialogKeyHandler = null;
  }
  const app = $('.accounting-app');
  if (app) app.inert = dialogAppWasInert;
  const returnFocus = dialogReturnFocus;
  dialogReturnFocus = null;
  if (dialogCloseHandler) {
    const handler = dialogCloseHandler;
    dialogCloseHandler = null;
    handler();
  }
  if (returnFocus?.isConnected && !returnFocus.closest('[inert]') && !returnFocus.closest('[hidden]')) {
    returnFocus.focus();
  }
}

function openDialog(options) {
  const modal = $('#modal');
  const title = $('#dialogTitle');
  const body = $('#dialogBody');
  const foot = $('#dialogFoot');
  const dialog = modal ? modal.querySelector('.dialog') : null;
  if (!modal || !title || !body || !foot || !dialog) return null;

  const wasOpen = !modal.hidden;
  const returnFocus = wasOpen ? dialogReturnFocus : document.activeElement;
  dialogCanClose = null;
  closeDialog();
  dialogReturnFocus = returnFocus;
  dialog.classList.toggle('dialog-wide', Boolean(options.wide));
  title.textContent = options.title || '';
  body.innerHTML = options.body || '';
  foot.innerHTML = '';

  const result = {
    modal,
    body,
    foot,
    close: closeDialog,
    setActions(actions) {
      foot.innerHTML = '';
      actions.forEach((action) => {
        const button = document.createElement('button');
        button.type = 'button';
        button.className = `btn${action.primary ? ' btn-primary' : ''}${action.className ? ` ${action.className}` : ''}`;
        if (action.icon) button.innerHTML = icon(action.icon);
        const label = document.createElement('span');
        label.textContent = action.label;
        button.append(label);
        if (action.id) button.id = action.id;
        button.addEventListener('click', () => action.onClick?.(result, button));
        foot.append(button);
      });
      const closer = document.createElement('button');
      closer.type = 'button';
      closer.className = 'btn push-end';
      closer.textContent = 'بستن';
      closer.hidden = options.hideClose === true;
      closer.addEventListener('click', () => closeDialog());
      foot.append(closer);
    },
  };

  result.setActions(options.actions || []);
  dialogCloseHandler = options.onClose || null;
  dialogCanClose = typeof options.canClose === 'function' ? options.canClose : null;
  modal.hidden = false;
  const app = $('.accounting-app');
  dialogAppWasInert = Boolean(app?.inert);
  if (app) app.inert = true;
  dialogKeyHandler = (event) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      closeDialog();
      return;
    }
    if (event.key !== 'Tab') return;
    const focusable = Array.from(modal.querySelectorAll(
      'button:not(:disabled),input:not(:disabled),select:not(:disabled),textarea:not(:disabled),a[href],[tabindex]:not([tabindex="-1"])'
    )).filter((element) => !element.hidden && element.getClientRects().length > 0);
    if (!focusable.length) {
      event.preventDefault();
      dialog.focus();
      return;
    }
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && (document.activeElement === first || !modal.contains(document.activeElement))) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && (document.activeElement === last || !modal.contains(document.activeElement))) {
      event.preventDefault();
      first.focus();
    }
  };
  modal.addEventListener('keydown', dialogKeyHandler);
  if (options.actionButtons) result.setActions(options.actionButtons);
  const firstInput = body.querySelector('input, select, textarea');
  const firstFocusable = firstInput || modal.querySelector('button:not(:disabled),input:not(:disabled),select:not(:disabled),textarea:not(:disabled)');
  if (firstFocusable) setTimeout(() => { if (!modal.hidden) firstFocusable.focus(); }, 30);
  return result;
}

function confirmDialog(title, message, confirmLabel = 'حذف', danger = true) {
  return new Promise((resolve) => {
    const dialog = openDialog({
      title,
      body: `<div class="note"${danger ? ' data-tone="danger"' : ''}>${esc(message)}</div>`,
      hideClose: true,
      onClose: () => resolve(false),
      actions: [
        {
          label: confirmLabel,
          primary: danger,
          className: danger ? 'btn-danger' : '',
          onClick: () => {
            dialogCloseHandler = null;
            closeDialog();
            resolve(true);
          },
        },
        {
          label: 'انصراف',
          onClick: () => {
            dialogCloseHandler = null;
            closeDialog();
            resolve(false);
          },
        },
      ],
    });
  });
}

/* ── Date and filter defaults ───────────────────────────────── */

function currentYear() {
  const today = state.settings?.today || '';
  const year = toEn(today).slice(0, 4);
  return /^\d{4}$/.test(year) ? year : '';
}

function defaultFilters() {
  const today = state.settings?.today || '';
  const year = currentYear();
  const from = year ? `${year}/01/01` : '';
  return {
    vouchers: { from, to: today, search: '' },
    journal: { from, to: today, search: '' },
    ledger: { accountCode: '', from, to: today },
    trial: {
      from,
      to: today,
      accountFrom: '',
      accountTo: '',
      showZero: true,
      multiLevel: false,
      balanceType: 'شش ستونی',
      showAccountCode: true,
      showAccountName: true,
      showPrintDateTime: true,
    },
    income: { from, to: today },
    balance: { asOf: today },
  };
}

function filterValue(key, name) {
  const group = state.filters[key] || {};
  return group[name] ?? '';
}

/* ── Data loading ───────────────────────────────────────────── */

async function ensureAccounts(force = false) {
  if (!state.accountsLoaded || force) {
    const payload = await invoke('list_accounts');
    state.accounts = Array.isArray(payload?.accounts) ? payload.accounts : [];
    state.databasePath = payload?.databasePath || state.databasePath;
    state.accountsLoaded = true;
  }
  return state.accounts;
}

async function ensureGroups(force = false) {
  if (!state.groupsLoaded || force) {
    const payload = await invoke('list_account_groups');
    state.groups = Array.isArray(payload?.groups) ? payload.groups : [];
    state.databasePath = payload?.databasePath || state.databasePath;
    state.groupsLoaded = true;
  }
  return state.groups;
}

async function loadSettings() {
  const settings = await invoke('get_settings');
  state.settings = settings;
  applyLimits(settings?.limits);
  updateBranding(settings);
  return settings;
}

function applyLimits(limits) {
  if (!limits) return;
  const voucherPage = Number(limits.voucherPageSize);
  if (Number.isSafeInteger(voucherPage) && voucherPage > 0) VOUCHER_PAGE_SIZE = voucherPage;
  const journalPage = Number(limits.journalPageSize);
  if (Number.isSafeInteger(journalPage) && journalPage > 0) JOURNAL_PAGE_SIZE = journalPage;
}

// Loads a page and, when the current offset no longer exists (for example
// after deleting the last voucher of the last page), retries the last page
// instead of showing an empty list.
async function loadPagedList(kind, step, fetchPage, totalOf, rowsOf) {
  let report = await fetchPage(state.listOffsets[kind] || 0);
  const total = Number(totalOf(report) || 0);
  const shown = rowsOf(report).length;
  if (!total) {
    state.listOffsets[kind] = 0;
    return report;
  }
  if ((state.listOffsets[kind] || 0) > 0 && shown === 0) {
    state.listOffsets[kind] = Math.max(0, Math.floor((total - 1) / step) * step);
    report = await fetchPage(state.listOffsets[kind]);
  }
  return report;
}

function updateBranding(settings) {
  if (!settings) return;
  const company = settings.companyName || 'واحد اقتصادی';
  const fiscal = settings.fiscalYear || '—';
  const today = faDate(settings.today || '');
  const statusCompany = $('#statusCompany');
  const statusFiscal = $('#statusFiscal');
  const statusDate = $('#statusDate');
  if (statusCompany) statusCompany.textContent = `شرکت: ${company}`;
  if (statusFiscal) statusFiscal.textContent = `سال مالی: ${toFa(fiscal)}`;
  if (statusDate) statusDate.textContent = `تاریخ: ${today}`;
}

/* ── Page actions ───────────────────────────────────────────── */

function renderActions(actions) {
  const bar = $('#pageActions');
  if (!bar) return;
  bar.innerHTML = '';
  (actions || []).forEach((action) => {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = `btn${action.primary ? ' btn-primary' : ''}`;
    if (action.title) button.title = action.title;
    if (action.icon) button.innerHTML = icon(action.icon);
    const label = document.createElement('span');
    label.textContent = action.label;
    button.append(label);
    button.addEventListener('click', () => action.onClick?.());
    bar.append(button);
  });
}

/* ── Page definitions ───────────────────────────────────────── */

const PAGES = {};

function pageMeta(page) {
  const titles = {
    vouchers: ['لیست اسناد حسابداری', 'عملیات'],
    journal: ['دفتر روزنامه', 'عملیات'],
    ledger: ['دفتر کل', 'گزارش‌ها'],
    trial: ['تراز آزمایشی', 'گزارش‌ها'],
    income: ['صورت سود و زیان', 'گزارش‌ها'],
    balance: ['ترازنامه', 'گزارش‌ها'],
    accounts: ['حساب‌ها', 'تعریف‌ها'],
    groups: ['گروه‌های حساب', 'تعریف‌ها'],
    settings: ['تنظیمات', 'سیستم'],
  };
  return titles[page] || ['فضای کار', ''];
}

function applyWallpaper(dataUrl) {
  const app = $('.accounting-app');
  if (!app) return;
  if (dataUrl) app.style.setProperty('--workspace-wallpaper', `url("${dataUrl}")`);
  else app.style.removeProperty('--workspace-wallpaper');
}

function chooseWallpaper(file) {
  if (!file) return;
  createImageBitmap(file).then((bitmap) => {
    const scale = Math.min(1, 2400 / bitmap.width, 1600 / bitmap.height);
    const canvas = document.createElement('canvas');
    canvas.width = Math.round(bitmap.width * scale);
    canvas.height = Math.round(bitmap.height * scale);
    canvas.getContext('2d').drawImage(bitmap, 0, 0, canvas.width, canvas.height);
    bitmap.close();
    const dataUrl = canvas.toDataURL('image/webp', 0.82);
    localStorage.setItem('hesabdari-workspace-wallpaper', dataUrl);
    applyWallpaper(dataUrl);
    setStatus('تصویر پس‌زمینه ذخیره شد.', 'ok');
  }).catch((error) => {
    toast(`بارگذاری تصویر ناموفق بود: ${errorText(error)}`, 'error');
  });
}

const printAction = (command, argsBuilder) => ({
  label: 'چاپ گزارش',
  icon: 'i-print',
  onClick: () => printWithFeedback(command, argsBuilder()),
});

async function printWithFeedback(command, args) {
  setStatus('در حال آماده‌سازی پیش‌نمایش چاپ…', 'busy');
  try {
    await invoke(command, args);
    setStatus('پیش‌نمایش چاپ باز شد.', 'ok');
  } catch (error) {
    setStatus('آماده‌سازی گزارش ناموفق بود.', 'error');
    toast(errorText(error), 'error');
  }
}

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

function loadMoreBar(kind, shown, total, offset) {
  if (!total) return '';
  const step = kind === 'journal' ? JOURNAL_PAGE_SIZE : VOUCHER_PAGE_SIZE;
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
      <td class="center"><span class="voucher-tag">${esc(v.type)}</span></td>
      <td class="desc-cell">${esc(v.description) || '—'}</td>
      <td class="center">${toFa(v.lineCount)}</td>
      <td class="money debit">${money(v.totalDebit)}</td>
      <td class="money credit">${money(v.totalCredit)}</td>
      <td class="center nowrap">
        <button type="button" class="btn btn-sm" data-open-voucher="${v.number}" title="ویرایش سند">${icon('i-edit')}</button>
        <button type="button" class="btn btn-sm" data-print-voucher="${v.number}" title="چاپ سند">${icon('i-print')}</button>
        <button type="button" class="btn btn-sm" data-delete-voucher="${v.id}" data-voucher-updated="${esc(v.updatedAt || '')}" data-voucher-label="${esc(v.description || `سند ${v.number}`)}" title="حذف سند">${icon('i-trash')}</button>
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
      const step = kind === 'journal' ? JOURNAL_PAGE_SIZE : VOUCHER_PAGE_SIZE;
      state.listOffsets[kind] = Math.max(0, (state.listOffsets[kind] || 0) - step);
      navigate(state.page, { force: true });
    });
  });
  root.querySelectorAll('[data-page-next]').forEach((button) => {
    button.addEventListener('click', () => {
      const kind = button.dataset.pageNext;
      const step = kind === 'journal' ? JOURNAL_PAGE_SIZE : VOUCHER_PAGE_SIZE;
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

/* ── Ledger ─────────────────────────────────────────────────── */

PAGES.ledger = {
  actions: () => {
    const f = state.filters.ledger;
    return [
      {
        label: 'چاپ گزارش',
        icon: 'i-print',
        onClick: () => {
          if (!f.accountCode) {
            toast('ابتدا یک حساب انتخاب کنید.', 'info');
            return;
          }
          printWithFeedback('open_ledger_preview', { ...f });
        },
      },
      { label: 'به‌روزرسانی', icon: 'i-refresh', onClick: () => navigate('ledger', { force: true }) },
    ];
  },
  load: async (force) => {
    const accounts = await ensureAccounts(force);
    const f = state.filters.ledger;
    if (!f.accountCode && accounts.length) {
      const first = accounts.find((a) => a.isPostable) || accounts[0];
      f.accountCode = first.code;
    }
    if (!f.accountCode) return { accounts, report: null };
    const report = await invoke('get_ledger', {
      accountCode: f.accountCode,
      from: f.from,
      to: f.to,
    });
    return { accounts, report };
  },
  render: renderLedger,
  bind: bindLedger,
};

function renderLedger(data) {
  const { accounts, report } = data;
  if (!accounts.length) {
    return `<div class="empty-state">
      ${icon('i-tree')}
      <b>هنوز حسابی تعریف نشده است</b>
      <span>ابتدا از بخش حساب‌ها، حساب‌های مورد نیاز را تعریف کنید.</span>
      <button type="button" class="btn btn-primary" data-go="accounts">${icon('i-plus')}<span>تعریف حساب</span></button>
    </div>`;
  }

  const f = state.filters.ledger;
  const options = accounts.map((a) => `<option value="${esc(a.code)}"${a.code === f.accountCode ? ' selected' : ''}>${toFa(a.code)} — ${esc(a.name)} (${levelLabel(a.level)})</option>`).join('');

  const filter = `
  <form class="filter-bar" data-filter="ledger">
    <div class="field grow">
      <label for="lgAccount">حساب</label>
      <select id="lgAccount" class="input wide" name="accountCode">${options}</select>
    </div>
    <div class="field">
      <label for="lgFrom">از تاریخ</label>
      <input id="lgFrom" class="input" name="from" data-date type="text" inputmode="numeric" value="${esc(f.from)}" placeholder="۱۴۰۵/۰۱/۰۱" />
    </div>
    <div class="field">
      <label for="lgTo">تا تاریخ</label>
      <input id="lgTo" class="input" name="to" data-date type="text" inputmode="numeric" value="${esc(f.to)}" placeholder="۱۴۰۵/۱۲/۲۹" />
    </div>
    <button class="btn btn-primary" type="submit">${icon('i-search')}<span>نمایش گردش</span></button>
    <button class="btn" type="button" data-reset>${icon('i-refresh')}<span>بازنشانی</span></button>
  </form>`;

  if (!report) {
    return `${filter}<div class="empty-state">${icon('i-ledger')}<b>حسابی انتخاب نشده است</b><span>یک حساب را از فهرست بالا انتخاب کنید.</span></div>`;
  }

  const openingNet = amountBigInt(report.openingDebit) - amountBigInt(report.openingCredit);
  const closingNet = amountBigInt(report.closingDebit) - amountBigInt(report.closingCredit);
  // The ledger of a parent account mixes its sub-accounts, so say which
  // sub-account each row belongs to.
  const withAccount = Boolean(report.includesChildren);
  const rows = report.rows.map((row) => `
    <tr>
      <td class="nowrap">${faDate(row.date)}</td>
      <td class="center"><button type="button" class="link-btn" data-open-voucher="${row.number}">${toFa(row.number)}</button></td>
      ${withAccount ? `<td class="desc-cell" title="${esc(row.accountName)}">${toFa(row.accountCode)} — ${esc(row.accountName)}</td>` : ''}
      <td class="desc-cell">${esc(row.lineDescription || row.voucherDescription) || '—'}</td>
      <td class="money debit">${money(row.debit)}</td>
      <td class="money credit">${money(row.credit)}</td>
      <td class="money ${row.runningDebit ? 'debit' : ''}">${money(row.runningDebit)}</td>
      <td class="money ${row.runningCredit ? 'credit' : ''}">${money(row.runningCredit)}</td>
    </tr>`).join('');

  return `
  ${filter}
  <div class="report-summary">
    <div class="kpi"><span>مانده افتتاحیه</span><strong>${money(openingNet)}</strong></div>
    <div class="kpi"><span>گردش بدهکار دوره</span><strong>${money(report.totalDebit)}</strong></div>
    <div class="kpi"><span>گردش بستانکار دوره</span><strong>${money(report.totalCredit)}</strong></div>
    <div class="kpi" data-tone="${closingNet >= 0 ? 'info' : 'warn'}"><span>مانده اختتامیه</span><strong>${money(closingNet)}</strong></div>
  </div>
  <div class="card">
    <div class="info-strip">
      <span>حساب: <b>${toFa(report.accountCode)} — ${esc(report.accountName)}</b></span>
      <span class="sep"></span>
      <span>گروه: <b>${esc(report.groupName)}</b></span>
      ${withAccount ? '<span class="sep"></span><span><b>شامل گردش زیرحساب‌ها</b></span>' : ''}
      <span class="sep"></span>
      <span>دوره: <b>${report.from ? faDate(report.from) : 'ابتدا'} تا ${report.to ? faDate(report.to) : 'انتها'}</b></span>
    </div>
    <div class="table-wrap">
      <table class="data-table">
        <thead><tr>
          <th style="width:96px">تاریخ</th><th style="width:66px">شماره</th>${withAccount ? '<th style="width:200px">حساب</th>' : ''}<th>شرح</th>
          <th style="width:112px">بدهکار</th><th style="width:112px">بستانکار</th>
          <th style="width:120px">مانده بدهکار</th><th style="width:120px">مانده بستانکار</th>
        </tr></thead>
        <tbody>
          <tr>
            <td colspan="${withAccount ? 6 : 5}" class="center bold">مانده افتتاحیه</td>
            <td class="money debit bold">${money(report.openingDebit)}</td>
            <td class="money credit bold">${money(report.openingCredit)}</td>
          </tr>
          ${rows}
        </tbody>
        <tfoot><tr>
          <td colspan="${withAccount ? 4 : 3}" class="bold">گردش دوره و مانده اختتامیه</td>
          <td class="num bold">${money(report.totalDebit)}</td>
          <td class="num bold">${money(report.totalCredit)}</td>
          <td class="num bold">${money(report.closingDebit)}</td>
          <td class="num bold">${money(report.closingCredit)}</td>
        </tr></tfoot>
      </table>
    </div>
    ${report.rows.length ? '' : '<div class="note" style="margin-top:10px">در این بازه گردشی برای این حساب ثبت نشده است.</div>'}
  </div>`;
}

function bindLedger(root) {
  // [data-go] buttons are wired once by bindPageCommon.
  bindVoucherRowActions(root);
}

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

  return `
  ${trialFilterTrigger()}
  <div class="report-summary">
    <div class="kpi"><span>تعداد ردیف گزارش</span><strong>${toFa(report.rows.length)}</strong></div>
    <div class="kpi"><span>گردش بدهکار دوره</span><strong>${money(report.periodDebit)}</strong></div>
    <div class="kpi"><span>گردش بستانکار دوره</span><strong>${money(report.periodCredit)}</strong></div>
    <div class="kpi" data-tone="${balanced ? 'ok' : 'danger'}"><span>وضعیت تراز</span><strong>${balanced ? 'متوازن' : 'مغایرت'}</strong></div>
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

/* ── Income statement ───────────────────────────────────────── */

PAGES.income = {
  actions: () => [
    { label: 'چاپ گزارش', icon: 'i-print', onClick: () => printWithFeedback('open_income_statement_preview', { ...state.filters.income }) },
    { label: 'به‌روزرسانی', icon: 'i-refresh', onClick: () => navigate('income', { force: true }) },
  ],
  load: async () => {
    const f = state.filters.income;
    return invoke('get_income_statement', { from: f.from, to: f.to });
  },
  render: renderIncomeStatement,
  bind: bindFilterOnly,
};

function incomeFilterBar() {
  const f = state.filters.income;
  return `
  <form class="filter-bar" data-filter="income">
    <div class="field">
      <label for="isFrom">از تاریخ</label>
      <input id="isFrom" class="input" name="from" data-date type="text" inputmode="numeric" value="${esc(f.from)}" placeholder="۱۴۰۵/۰۱/۰۱" />
    </div>
    <div class="field">
      <label for="isTo">تا تاریخ</label>
      <input id="isTo" class="input" name="to" data-date type="text" inputmode="numeric" value="${esc(f.to)}" placeholder="۱۴۰۵/۱۲/۲۹" />
    </div>
    <button class="btn btn-primary" type="submit">${icon('i-search')}<span>محاسبه صورت</span></button>
    <button class="btn" type="button" data-reset>${icon('i-refresh')}<span>بازنشانی</span></button>
  </form>`;
}

function renderIncomeStatement(report) {
  const net = amountBigInt(report.netProfit);
  return `
  ${incomeFilterBar()}
  <div class="report-summary">
    <div class="kpi"><span>جمع درآمدها</span><strong>${money(report.revenueTotal)}</strong></div>
    <div class="kpi"><span>بهای تمام شده</span><strong>${money(report.cogsTotal)}</strong></div>
    <div class="kpi"><span>هزینه‌های عملیاتی</span><strong>${money(report.expenseTotal)}</strong></div>
    <div class="kpi"><span>سایر درآمدهای عملیاتی</span><strong>${money(report.otherOperatingRevenueTotal)}</strong></div>
    <div class="kpi"><span>سایر هزینه‌های عملیاتی</span><strong>${money(report.otherOperatingExpenseTotal)}</strong></div>
    <div class="kpi"><span>درآمدهای غیرعملیاتی</span><strong>${money(report.nonOperatingRevenueTotal)}</strong></div>
    <div class="kpi"><span>هزینه‌های غیرعملیاتی</span><strong>${money(report.nonOperatingExpenseTotal)}</strong></div>
    <div class="kpi"><span>سود (زیان) ناخالص</span><strong>${money(report.grossProfit)}</strong></div>
    <div class="kpi" data-tone="${net >= 0n ? 'ok' : 'danger'}"><span>سود (زیان) خالص</span><strong>${money(net)}</strong></div>
  </div>
  <div class="report-layout">
    <div class="card">
      ${incomeSection('درآمدها', report.revenues, report.revenueTotal, 'i-trend')}
    </div>
    <div class="card">
      ${incomeSection('بهای تمام شده درآمدها', report.cogs, report.cogsTotal, 'i-coins')}
    </div>
  </div>
  <div class="card" style="margin-top:12px">
    <div class="report-total-line"><span>سود (زیان) ناخالص</span><span class="money">${money(report.grossProfit)}</span></div>
  </div>
  <div class="report-layout" style="margin-top:12px">
    <div class="card">
      ${incomeSection('سایر درآمدهای عملیاتی', report.otherOperatingRevenues, report.otherOperatingRevenueTotal, 'i-trend')}
    </div>
    <div class="card">
      ${incomeSection('سایر هزینه‌های عملیاتی', report.otherOperatingExpenses, report.otherOperatingExpenseTotal, 'i-bank')}
    </div>
  </div>
  <div class="card" style="margin-top:12px">
    ${incomeSection('هزینه‌های عملیاتی', report.expenses, report.expenseTotal, 'i-bank')}
  </div>
  <div class="report-layout" style="margin-top:12px">
    <div class="card">
      ${incomeSection('درآمدهای غیرعملیاتی', report.nonOperatingRevenues, report.nonOperatingRevenueTotal, 'i-trend')}
    </div>
    <div class="card">
      ${incomeSection('هزینه‌های غیرعملیاتی', report.nonOperatingExpenses, report.nonOperatingExpenseTotal, 'i-bank')}
    </div>
  </div>
  <div class="card" style="margin-top:12px">
    <div class="report-total-line"><span>سود (زیان) عملیاتی</span><span class="money">${money(report.operatingProfit)}</span></div>
    <div class="report-total-line net"><span>سود (زیان) خالص دوره</span><span class="money">${money(net)}</span></div>
  </div>`;
}

function incomeSection(title, rows, total, iconName) {
  const body = rows.length ? rows.map((row) => `
    <tr>
      <td class="center mono">${toFa(row.accountCode)}</td>
      <td>${esc(row.accountName)}</td>
      <td class="money ${row.amount >= 0 ? 'debit' : 'credit'}">${money(row.amount)}</td>
    </tr>`).join('') : `<tr><td colspan="3" class="center muted">موردی ثبت نشده است.</td></tr>`;
  return `<div class="section-head">
      <h2>${icon(iconName)} ${title}</h2>
      <span class="badge" data-tone="info">${toFa(rows.length)} حساب</span>
    </div>
    <div class="table-wrap short">
      <table class="data-table">
        <thead><tr><th style="width:96px">کد حساب</th><th>عنوان</th><th style="width:140px">مبلغ</th></tr></thead>
        <tbody>${body}</tbody>
        <tfoot><tr><td colspan="2" class="bold">جمع ${title}</td><td class="num bold">${money(total)}</td></tr></tfoot>
      </table>
    </div>`;
}

/* ── Balance sheet ──────────────────────────────────────────── */

PAGES.balance = {
  actions: () => [
    { label: 'چاپ گزارش', icon: 'i-print', onClick: () => printWithFeedback('open_balance_sheet_preview', { asOf: state.filters.balance.asOf }) },
    { label: 'به‌روزرسانی', icon: 'i-refresh', onClick: () => navigate('balance', { force: true }) },
  ],
  load: async () => {
    const f = state.filters.balance;
    return invoke('get_balance_sheet', { asOf: f.asOf });
  },
  render: renderBalanceSheet,
  bind: bindFilterOnly,
};

function balanceFilterBar() {
  const f = state.filters.balance;
  return `
  <form class="filter-bar" data-filter="balance">
    <div class="field">
      <label for="bsDate">تا تاریخ</label>
      <input id="bsDate" class="input" name="asOf" data-date type="text" inputmode="numeric" value="${esc(f.asOf)}" placeholder="۱۴۰۵/۱۲/۲۹" />
    </div>
    <button class="btn btn-primary" type="submit">${icon('i-search')}<span>محاسبه ترازنامه</span></button>
    <button class="btn" type="button" data-reset>${icon('i-refresh')}<span>بازنشانی</span></button>
  </form>`;
}

function renderBalanceSheet(report) {
  const equityRows = [...(report.equity || [])];
  equityRows.push({
    accountCode: '',
    accountName: 'سود (زیان) انباشته دوره',
    amount: report.profit,
  });
  const difference = amountBigInt(report.assetsTotal) - amountBigInt(report.totalLiabilitiesEquity);
  return `
  ${balanceFilterBar()}
  <div class="report-summary">
    <div class="kpi"><span>جمع دارایی‌ها</span><strong>${money(report.assetsTotal)}</strong></div>
    <div class="kpi"><span>جمع بدهی‌ها</span><strong>${money(report.liabilitiesTotal)}</strong></div>
    <div class="kpi"><span>حقوق مالکانه</span><strong>${money(amountBigInt(report.equityTotal) + amountBigInt(report.profit))}</strong></div>
    <div class="kpi" data-tone="${report.balanced ? 'ok' : 'danger'}"><span>وضعیت ترازنامه</span><strong>${report.balanced ? 'متوازن' : `مغایرت ${money(difference)}`}</strong></div>
  </div>
  <div class="report-layout">
    <div class="card">
      <div class="section-head">
        <h2>${icon('i-balance')} دارایی‌ها</h2>
        <span class="badge" data-tone="info">${toFa((report.assets || []).length)} حساب</span>
      </div>
      ${balanceTable(report.assets, report.assetsTotal, 'جمع دارایی‌ها')}
    </div>
    <div class="card">
      <div class="section-head">
        <h2>${icon('i-bank')} بدهی‌ها و حقوق مالکانه</h2>
        ${report.balanced ? badge('تراز است', 'ok') : badge('مغایرت دارد', 'danger')}
      </div>
      <h3 class="muted small" style="margin:2px 0 5px">بدهی‌ها</h3>
      ${balanceTable(report.liabilities, report.liabilitiesTotal, 'جمع بدهی‌ها')}
      <h3 class="muted small" style="margin:14px 0 5px">حقوق مالکانه</h3>
      ${balanceTable(equityRows, amountBigInt(report.equityTotal) + amountBigInt(report.profit), 'جمع حقوق مالکانه')}
      <div class="report-total-line net" style="margin-top:12px">
        <span>جمع بدهی‌ها و حقوق مالکانه</span>
        <span class="money">${money(report.totalLiabilitiesEquity)}</span>
      </div>
    </div>
  </div>`;
}

function balanceTable(rows, total, totalLabel) {
  const body = (rows || []).length ? rows.map((row) => `
    <tr>
      <td class="center mono">${row.accountCode ? toFa(row.accountCode) : '—'}</td>
      <td>${esc(row.accountName)}</td>
      <td class="money ${row.amount >= 0 ? 'debit' : 'credit'}">${money(row.amount)}</td>
    </tr>`).join('') : `<tr><td colspan="3" class="center muted">موردی ثبت نشده است.</td></tr>`;
  return `<div class="table-wrap short">
    <table class="data-table">
      <thead><tr><th style="width:92px">کد</th><th>عنوان</th><th style="width:132px">مبلغ</th></tr></thead>
      <tbody>${body}</tbody>
      <tfoot><tr><td colspan="2" class="bold">${totalLabel}</td><td class="num bold">${money(total)}</td></tr></tfoot>
    </table>
  </div>`;
}

/* ── Chart of accounts ─────────────────────────────────────── */

PAGES.accounts = {
  actions: () => [
    { label: 'تعریف و اصلاح حساب‌ها', icon: 'i-tree', primary: true, onClick: () => window.openAccountDefinitionPopup?.() },
    { label: 'حساب جدید سریع', icon: 'i-plus', onClick: () => openAccountDialog(null) },
    { label: 'گروه‌ها', icon: 'i-folder', onClick: () => openGroupsWindow() },
    { label: 'به‌روزرسانی', icon: 'i-refresh', onClick: () => navigate('accounts', { force: true }) },
  ],
  load: async (force) => {
    const [accounts, groups] = await Promise.all([ensureAccounts(force), ensureGroups(force)]);
    return { accounts, groups };
  },
  render: renderAccounts,
  bind: bindAccounts,
};

function renderAccounts(data) {
  const { accounts, groups } = data;
  const filters = state.accountFilters;
  const accountByCode = new Map(accounts.map((account) => [account.code, account]));
  const parentCodes = new Set(accounts.map((account) => account.parentCode).filter(Boolean));
  const groupOptions = groups.map((g) => `<option value="${esc(g.code)}"${g.code === filters.group ? ' selected' : ''}>${toFa(g.code)} — ${esc(g.name)}</option>`).join('');
  const drillChain = [];
  for (const code of state.accountDrillPath) {
    const expectedLevel = drillChain.length + 1;
    const parent = drillChain[drillChain.length - 1];
    const node = accountByCode.get(code);
    if (!node || Number(node.level) !== expectedLevel || (node.parentCode || '') !== (parent?.code || '')) break;
    drillChain.push(node);
  }
  if (drillChain.length !== state.accountDrillPath.length) {
    state.accountDrillPath = drillChain.map((account) => account.code);
  }
  const currentParent = drillChain[drillChain.length - 1] || null;
  const search = normalizeSearch(filters.search);
  const hasGlobalFilter = Boolean(search || filters.group || filters.level);
  const filtered = accounts.filter((account) => {
    if (!hasGlobalFilter && (currentParent
      ? account.parentCode !== currentParent.code
      : Number(account.level) !== 1)) return false;
    if (search || filters.group || filters.level) {
      if (filters.group && account.groupCode !== filters.group) return false;
      if (filters.level && Number(account.level) !== Number(filters.level)) return false;
    }
    const haystack = normalizeSearch(`${account.code} ${account.name} ${account.groupName} ${account.parentName}`);
    if (search && !haystack.includes(search)) return false;
    return true;
  });

  const rows = filtered.map((account, index) => {
    const hasChildren = parentCodes.has(account.code);
    return `
    <tr>
      <td class="center row-index">${toFa(index + 1)}</td>
      <td class="center mono">${toFa(account.code)}</td>
      <td class="desc-cell"><b>${esc(account.name)}</b></td>
      <td class="center"><span class="level-pill ${levelClass(account.level)}">${levelLabel(account.level)}</span></td>
      <td>${esc(account.groupName)}</td>
      <td class="center">${badge(natureLabel(account.nature), account.nature === 'creditor' ? 'warn' : account.nature === 'debtor' ? 'info' : 'neutral')}</td>
      <td class="muted">${esc(account.parentName) || '—'}</td>
      <td class="center">${account.isPostable ? badge('قابل ثبت', 'ok') : badge('غیرقابل ثبت', 'neutral')}</td>
      <td class="center nowrap">
        ${hasChildren ? `<button type="button" class="btn btn-sm account-drill-btn" data-drill-account="${esc(account.code)}" title="نمایش زیرحساب‌ها">${icon('i-chevron')}</button>` : ''}
        <button type="button" class="btn btn-sm" data-edit-account="${account.id}" title="ویرایش حساب">${icon('i-edit')}</button>
        <button type="button" class="btn btn-sm" data-delete-account="${account.id}" data-account-label="${esc(account.name)}" data-account-code="${esc(account.code)}" title="حذف حساب">${icon('i-trash')}</button>
      </td>
    </tr>`;
  }).join('');
  const pathButtons = [
    `<button type="button" class="account-path-btn" data-account-path="" ${drillChain.length === 0 ? 'aria-current="location" disabled' : ''}>حساب‌های کل</button>`,
    ...drillChain.map((account, index) => {
      const path = drillChain.slice(0, index + 1).map((item) => item.code).join(',');
      return `<span aria-hidden="true">‹</span><button type="button" class="account-path-btn" data-account-path="${esc(path)}" ${index === drillChain.length - 1 ? 'aria-current="location" disabled' : ''}>${toFa(account.code)} - ${esc(account.name)}</button>`;
    }),
  ].join('');

  return `
  <form class="filter-bar" data-filter="accountFilters">
    <div class="field grow">
      <label for="acSearch">جست‌وجو</label>
      <input id="acSearch" class="input" name="search" type="search" value="${esc(filters.search)}" placeholder="کد، نام حساب، گروه یا حساب بالاتر…" />
    </div>
    <div class="field">
      <label for="acGroup">گروه حساب</label>
      <select id="acGroup" class="input wide" name="group">
        <option value="">همه گروه‌ها</option>
        ${groupOptions}
      </select>
    </div>
    <div class="field">
      <label for="acLevel">سطح</label>
      <select id="acLevel" class="input" name="level">
        <option value="">همه سطوح</option>
        ${[1, 2, 3].map((level) => `<option value="${level}"${String(filters.level) === String(level) ? ' selected' : ''}>${levelLabel(level)}</option>`).join('')}
      </select>
    </div>
    <button class="btn btn-primary" type="submit">${icon('i-search')}<span>فیلتر</span></button>
    <button class="btn" type="button" data-reset>${icon('i-refresh')}<span>بازنشانی</span></button>
  </form>

  <nav class="account-browser-path" aria-label="مسیر حساب‌ها">
    <div class="account-path-items">${pathButtons}</div>
    <div class="account-path-meta">${hasGlobalFilter ? 'نتایج فیلتر در همه سطح‌ها' : `سطح ${levelLabel(currentParent ? Number(currentParent.level) + 1 : 1)}`} · ${toFa(filtered.length)} حساب</div>
  </nav>

  <div class="card">
    <div class="section-head">
      <h2>${icon('i-tree')} نمودار حساب‌ها</h2>
      <span class="badge" data-tone="info">${toFa(filtered.length)} حساب در این سطح</span>
    </div>
    ${filtered.length ? `
      <div class="table-wrap">
        <table class="data-table">
          <thead><tr>
            <th style="width:52px">ردیف</th><th style="width:96px">کد حساب</th><th>نام حساب</th><th style="width:92px">سطح</th>
            <th style="width:180px">گروه</th><th style="width:92px">ماهیت</th>
            <th style="width:180px">حساب بالاتر</th><th style="width:96px">ثبت سند</th><th style="width:104px">عملیات</th>
          </tr></thead>
          <tbody>${rows}</tbody>
        </table>
      </div>` : `<div class="empty-state">
        ${icon('i-tree')}
        <b>${hasGlobalFilter ? 'حسابی با این فیلتر یافت نشد' : currentParent ? 'زیرحسابی در این سطح تعریف نشده است' : 'حساب کلی تعریف نشده است'}</b>
        <span>${hasGlobalFilter ? 'فیلترها را تغییر دهید یا بازنشانی کنید.' : 'برای ایجاد حساب در این سطح، حساب جدید را تعریف کنید.'}</span>
        <button type="button" class="btn btn-primary" data-new-account>${icon('i-plus')}<span>حساب جدید</span></button>
      </div>`}
  </div>`;
}

function bindAccounts(root) {
  root.querySelectorAll('[data-account-path]').forEach((button) => {
    button.addEventListener('click', () => {
      state.accountDrillPath = button.dataset.accountPath ? button.dataset.accountPath.split(',') : [];
      navigate('accounts', { force: true });
    });
  });
  root.querySelectorAll('[data-drill-account]').forEach((button) => {
    button.addEventListener('click', () => {
      const account = state.accounts.find((item) => item.code === button.dataset.drillAccount);
      state.accountDrillPath = account ? accountPathTo(state.accounts, account) : [];
      state.accountFilters = { ...state.accountFilters, search: '', level: '' };
      navigate('accounts', { force: true });
    });
  });
  root.querySelectorAll('tbody tr').forEach((row) => {
    row.addEventListener('dblclick', (event) => {
      if (event.target.closest('button')) return;
      const codeCell = row.querySelector('.mono');
      const code = codeCell ? toEn(codeCell.textContent) : '';
      const hasChildren = state.accounts.some((account) => account.parentCode === code);
      if (!hasChildren) return;
      const account = state.accounts.find((item) => item.code === code);
      state.accountDrillPath = account ? accountPathTo(state.accounts, account) : [];
      state.accountFilters = { ...state.accountFilters, search: '', level: '' };
      navigate('accounts', { force: true });
    });
  });
  // «حساب جدید» buttons are wired once by bindPageCommon.
  root.querySelectorAll('[data-edit-account]').forEach((button) => {
    const id = Number(button.dataset.editAccount);
    const account = state.accounts.find((a) => Number(a.id) === id);
    button.addEventListener('click', () => openAccountDialog(account || null));
  });
  root.querySelectorAll('[data-delete-account]').forEach((button) => {
    button.addEventListener('click', async () => {
      const id = Number(button.dataset.deleteAccount);
      const label = button.dataset.accountLabel || 'این حساب';
      const code = button.dataset.accountCode || '';
      const confirmed = await confirmDialog(
        'حذف حساب',
        `آیا از حذف حساب «${toFa(code)} — ${label}» مطمئن هستید؟ حسابی که در اسناد استفاده شده باشد قابل حذف نیست.`
      );
      if (!confirmed) return;
      try {
        await invoke('delete_account', { id });
        toast('حساب حذف شد.', 'ok');
        state.accountsLoaded = false;
        await navigate('accounts', { force: true });
      } catch (error) {
        toast(errorText(error), 'error');
      }
    });
  });
}

/* ── Account form ───────────────────────────────────────────── */

function openAccountDialog(account) {
  const isEdit = Boolean(account);
  const groups = state.groups;
  if (!groups.length) {
    toast('ابتدا حداقل یک گروه حساب تعریف کنید.', 'info');
    return;
  }

  const hierarchy = state.accountDrillPath
    .map((code) => state.accounts.find((item) => item.code === code))
    .filter(Boolean);
  const requestedLevel = Number(state.accountFilters.level);
  const level = account
    ? Number(account.level)
    : requestedLevel || Math.min(3, hierarchy.length + 1);
  const initialParentCode = account?.parentCode || (level > 1 ? hierarchy[level - 2]?.code || '' : '');
  const initialGroupCode = account?.groupCode || state.accountFilters.group || (level > 1
    ? hierarchy[level - 2]?.groupCode || groups[0]?.code
    : groups[0]?.code);
  const groupOptions = groups.map((g) => `<option value="${esc(g.code)}"${initialGroupCode === g.code ? ' selected' : ''}>${toFa(g.code)} — ${esc(g.name)} (${esc(g.type)})</option>`).join('');
  const nature = account ? account.nature : 'debtor';
  const suggestedCode = account?.code || nextAccountCode(state.accounts, level, initialParentCode);

  const dialog = openDialog({
    title: isEdit ? 'ویرایش حساب' : 'تعریف حساب جدید',
    wide: true,
    body: `
      <form id="acctForm" class="acct-form" autocomplete="off">
        <div class="field">
          <label for="acctCode">کد حساب <span class="req">*</span></label>
          <input id="acctCode" class="input mono" type="text" inputmode="numeric" maxlength="20" value="${esc(toFa(suggestedCode))}" ${isEdit ? 'readonly' : ''} placeholder="کد پیشنهادی بر اساس حساب بالاتر" />
        </div>
        <div class="field wide">
          <label for="acctName">نام حساب <span class="req">*</span></label>
          <input id="acctName" class="input" type="text" value="${esc(account?.name || '')}" placeholder="نام حساب" />
        </div>
        <div class="field">
          <label for="acctLevel">سطح حساب</label>
          <select id="acctLevel" class="input" ${isEdit ? 'disabled' : ''}>
            ${(isEdit ? [Number(account.level)] : [1, 2, 3]).map((value) => `<option value="${value}"${Number(level) === value ? ' selected' : ''}>${levelLabel(value)}</option>`).join('')}
          </select>
        </div>
        <div class="field wide">
          <label for="acctGroup">گروه حساب <span class="req">*</span></label>
          <select id="acctGroup" class="input">${groupOptions}</select>
        </div>
        <div class="field">
          <label for="acctNature">ماهیت حساب</label>
          <select id="acctNature" class="input">
            <option value="debtor"${nature === 'debtor' ? ' selected' : ''}>بدهکار</option>
            <option value="creditor"${nature === 'creditor' ? ' selected' : ''}>بستانکار</option>
            <option value="dual"${nature === 'dual' ? ' selected' : ''}>دوگانه</option>
          </select>
        </div>
        <div class="field wide">
          <label for="acctParent">حساب بالاتر</label>
          <select id="acctParent" class="input"></select>
        </div>
        <div class="field">
          <label>&nbsp;</label>
          <label class="check-field" style="padding-top:0">
            <input id="acctPostable" type="checkbox" ${account?.isPostable ? 'checked' : ''} ${Number(level) === 1 ? 'disabled' : ''} />
            <span>قابل ثبت در سند</span>
          </label>
        </div>
        <div class="form-error" id="acctError" hidden></div>
      </form>
      <div class="form-note">
        کد پیشنهادی از روی آخرین هم‌سطح تولید می‌شود و برای حساب‌های پایین‌تر با کد حساب بالاتر آغاز می‌شود. کد حساب پس از ثبت قابل تغییر نیست؛ گروه حساب سطوح پایین‌تر از حساب بالاتر گرفته می‌شود.
      </div>`,
    actions: [
      {
        label: isEdit ? 'ذخیره تغییرات' : 'ثبت حساب',
        icon: 'i-save',
        primary: true,
        onClick: async (ref, button) => {
          await submitAccountForm(ref, button, account);
        },
      },
      { label: 'انصراف', onClick: () => closeDialog() },
    ],
  });

  if (!dialog) return;
  const codeInput = $('#acctCode', dialog.body);
  const levelSelect = $('#acctLevel', dialog.body);
  const groupSelect = $('#acctGroup', dialog.body);
  const parentSelect = $('#acctParent', dialog.body);
  const postable = $('#acctPostable', dialog.body);

  if (codeInput) {
    codeInput.addEventListener('input', () => {
      const cleaned = digitsOnly(codeInput.value);
      if (codeInput.value !== cleaned) codeInput.value = cleaned;
    });
  }

  const refreshParents = () => {
    if (!parentSelect) return;
    const currentLevel = Number(levelSelect?.value || level);
    const groupCode = groupSelect?.value || '';
    if (currentLevel <= 1) {
      parentSelect.innerHTML = '<option value="">— حساب سطح کل است —</option>';
      parentSelect.disabled = true;
      if (groupSelect) groupSelect.disabled = false;
      if (postable) postable.checked = false;
      if (postable) postable.disabled = true;
      if (!isEdit && codeInput) codeInput.value = toFa(nextAccountCode(state.accounts, currentLevel));
      return;
    }
    parentSelect.disabled = false;
    if (groupSelect) groupSelect.disabled = !isEdit;
    if (postable) postable.disabled = false;
    const candidates = state.accounts.filter((a) => Number(a.level) === currentLevel - 1 && a.groupCode === groupCode);
    const previous = parentSelect.value || (account && Number(account.level) === currentLevel
      ? account.parentCode
      : initialParentCode);
    parentSelect.innerHTML = candidates.length
      ? `<option value="">— انتخاب حساب بالاتر —</option>${candidates.map((a) => `<option value="${esc(a.code)}"${a.code === previous ? ' selected' : ''}>${toFa(a.code)} — ${esc(a.name)}</option>`).join('')}`
      : '<option value="">— حسابی در سطح بالاتر وجود ندارد —</option>';
    if (candidates.length && !parentSelect.value) parentSelect.value = candidates[0].code;
    if (!isEdit && codeInput) codeInput.value = toFa(nextAccountCode(state.accounts, currentLevel, parentSelect.value));
  };

  levelSelect?.addEventListener('change', refreshParents);
  groupSelect?.addEventListener('change', refreshParents);
  parentSelect?.addEventListener('change', () => {
    if (!isEdit && codeInput) codeInput.value = toFa(nextAccountCode(state.accounts, Number(levelSelect?.value || level), parentSelect.value));
  });
  refreshParents();
  if (account?.parentCode && parentSelect) parentSelect.value = account.parentCode;

  setTimeout(() => (isEdit ? $('#acctName', dialog.body) : codeInput)?.focus(), 40);
}

async function submitAccountForm(ref, button, account) {
  const errorBox = $('#acctError', ref.body);
  const codeInput = $('#acctCode', ref.body);
  const nameInput = $('#acctName', ref.body);
  const levelSelect = $('#acctLevel', ref.body);
  const groupSelect = $('#acctGroup', ref.body);
  const natureSelect = $('#acctNature', ref.body);
  const parentSelect = $('#acctParent', ref.body);
  const postable = $('#acctPostable', ref.body);

  const showError = (message) => {
    if (!errorBox) return;
    errorBox.textContent = message;
    errorBox.hidden = false;
  };

  const code = digitsOnly(codeInput?.value || '');
  const name = (nameInput?.value || '').trim().replace(/\s+/g, ' ');
  const level = Number(levelSelect?.value || 1);
  const groupCode = groupSelect?.value || '';
  const nature = natureSelect?.value || 'debtor';
  const parentCode = level <= 1 ? '' : (parentSelect?.value || '');
  const isPostable = level > 1 && Boolean(postable?.checked);

  if (!account && !code) return showError('کد حساب الزامی است.');
  if (!name) return showError('نام حساب الزامی است.');
  if (!groupCode) return showError('گروه حساب را انتخاب کنید.');
  if (level > 1 && !parentCode) return showError('برای این سطح، انتخاب حساب بالاتر الزامی است.');

  if (errorBox) errorBox.hidden = true;
  if (button) button.disabled = true;

  try {
    if (account) {
      await invoke('update_account', {
        id: account.id,
        name,
        groupCode,
        nature,
        parentCode,
        isPostable,
      });
      toast('حساب با موفقیت ویرایش شد.', 'ok');
    } else {
      await invoke('create_account', {
        code,
        name,
        level,
        groupCode,
        nature,
        parentCode,
        isPostable,
      });
      toast('حساب جدید ثبت شد.', 'ok');
    }
    closeDialog();
    state.accountsLoaded = false;
    await navigate('accounts', { force: true });
  } catch (error) {
    showError(errorText(error));
    if (button) button.disabled = false;
  }
}

/* ── Groups overview ────────────────────────────────────────── */

PAGES.groups = {
  actions: () => [
    { label: 'مدیریت گروه‌ها', icon: 'i-folder', primary: true, onClick: () => openGroupsWindow() },
    { label: 'به‌روزرسانی', icon: 'i-refresh', onClick: () => navigate('groups', { force: true }) },
  ],
  load: async (force) => {
    const groups = await ensureGroups(force);
    return groups;
  },
  render: renderGroups,
  bind: bindFilterOnly,
};

function renderGroups(groups) {
  const rows = groups.map((group) => `
    <tr>
      <td class="center mono">${toFa(group.code)}</td>
      <td><b>${esc(group.name)}</b></td>
      <td class="center">${badge(group.type, typeTone(group.type))}</td>
      <td class="center">${toFa(group.accountCount)}</td>
      <td class="center muted">${faDate(group.updatedAt)}</td>
      <td class="center">
        <button type="button" class="btn btn-sm" data-open-groups title="مدیریت گروه‌ها">${icon('i-edit')}</button>
      </td>
    </tr>`).join('');

  return `
  <div class="info-strip">
    <span>گروه‌های حساب، دسته‌بندی اصلی نمودار حساب‌ها هستند و در گزارش‌های مالی تعیین‌کننده طبقه هر حساب‌اند.</span>
    <span class="sep"></span>
    <b>${toFa(groups.length)} گروه</b>
  </div>
  <div class="card">
    <div class="section-head">
      <h2>${icon('i-folder')} گروه‌های حساب</h2>
      <span class="badge" data-tone="info">${toFa(groups.reduce((sum, g) => sum + Number(g.accountCount || 0), 0))} حساب</span>
    </div>
    ${groups.length ? `
      <div class="table-wrap">
        <table class="data-table">
          <thead><tr>
            <th style="width:96px">کد گروه</th><th>نام گروه</th><th style="width:140px">نوع</th>
            <th style="width:110px">تعداد حساب</th><th style="width:140px">آخرین تغییر</th><th style="width:80px">عملیات</th>
          </tr></thead>
          <tbody>${rows}</tbody>
        </table>
      </div>` : `<div class="empty-state">
        ${icon('i-folder')}
        <b>گروه حسابی وجود ندارد</b>
        <span>برای شروع، حداقل یک گروه حساب تعریف کنید.</span>
        <button type="button" class="btn btn-primary" data-open-groups>${icon('i-plus')}<span>مدیریت گروه‌ها</span></button>
      </div>`}
  </div>`;
}

/* ── Settings ───────────────────────────────────────────────── */

PAGES.settings = {
  actions: () => [
    { label: 'ویرایش مشخصات', icon: 'i-edit', primary: true, onClick: () => openSettingsDialog() },
  ],
  load: async () => {
    const settings = await invoke('get_settings');
    state.settings = settings;
    updateBranding(settings);
    return settings;
  },
  render: renderSettings,
  bind: bindSettings,
};

function renderSettings(settings) {
  const today = settings.today || '';
  return `
  <div class="settings-grid">
    <div class="card">
      <div class="section-head">
        <h2>${icon('i-settings')} مشخصات واحد اقتصادی</h2>
      </div>
      <dl class="info-list">
        <div><dt>نام شرکت / واحد اقتصادی</dt><dd>${esc(settings.companyName || '—')}</dd></div>
        <div><dt>سال مالی</dt><dd>${esc(settings.fiscalYear || '—')}</dd></div>
      </dl>
      <div class="form-actions">
        <button type="button" class="btn btn-primary" data-edit-settings>${icon('i-edit')}<span>ویرایش مشخصات</span></button>
      </div>
      <div class="form-note">
        نام شرکت در سرصفحه همه گزارش‌ها و در نوار وضعیت نمایش داده می‌شود.
        سال مالی به گزارش‌های چاپی اضافه می‌شود.
      </div>
    </div>

    <div class="card">
      <div class="section-head">
        <h2>${icon('i-info')} اطلاعات برنامه</h2>
      </div>
      <dl class="info-list">
        <div><dt>نام برنامه</dt><dd>نرم‌افزار حسابداری</dd></div>
        <div><dt>نسخه</dt><dd class="ltr">${APP_VERSION}</dd></div>
        <div><dt>تاریخ امروز</dt><dd class="ltr">${faDate(today)}</dd></div>
        <div><dt>تعداد حساب‌ها</dt><dd>${toFa(state.accountsLoaded ? state.accounts.length : '—')}</dd></div>
        <div><dt>تعداد اسناد</dt><dd>${toFa(state.voucherCount || 0)}</dd></div>
        <div><dt>مسیر پایگاه داده</dt><dd class="ltr" style="font-size:10px;max-width:260px;word-break:break-all">${esc(state.databasePath || '—')}</dd></div>
      </dl>
      <div class="form-note" style="margin-top:14px">
        همه اطلاعات به‌صورت محلی و آفلاین در پایگاه داده SQLite ذخیره می‌شود.
      </div>
      <div class="section-head">
        <h2>${icon('i-save')} پشتیبان‌گیری</h2>
      </div>
      <p class="form-note">بازیابی، اطلاعات فعلی را جایگزین می‌کند و پیش از آن یک نسخهٔ ایمنی نگه می‌دارد.</p>
      <div class="form-actions">
        <button type="button" class="btn" data-backup-database>${icon('i-save')}<span>تهیه پشتیبان</span></button>
        <button type="button" class="btn" data-restore-database>${icon('i-refresh')}<span>بازیابی پشتیبان</span></button>
      </div>
    </div>
  </div>`;
}

function bindSettings(root) {
  root.querySelector('[data-edit-settings]')?.addEventListener('click', () => openSettingsDialog());
  root.querySelector('[data-backup-database]')?.addEventListener('click', async (event) => {
    const button = event.currentTarget;
    button.disabled = true;
    try {
      const path = await invoke('backup_database');
      if (path) toast(`پشتیبان ذخیره شد: ${path}`, 'ok');
    } catch (error) {
      toast(errorText(error), 'error');
    } finally {
      button.disabled = false;
    }
  });
  root.querySelector('[data-restore-database]')?.addEventListener('click', async (event) => {
    // `currentTarget` is cleared once this handler awaits, so keep the
    // button reference before opening the confirmation dialog.
    const button = event.currentTarget;
    const confirmed = await confirmDialog(
      'بازیابی پشتیبان',
      'اطلاعات فعلی با فایل پشتیبان جایگزین می‌شود. پیش از بازیابی یک نسخهٔ ایمنی خودکار ساخته خواهد شد. ادامه می‌دهید؟'
    );
    if (!confirmed) return;
    button.disabled = true;
    try {
      const safetyPath = await invoke('restore_database');
      if (!safetyPath) return;
      await Promise.all([ensureAccounts(true), ensureGroups(true)]);
      state.voucherCount = await invoke('get_voucher_count');
      await navigate('settings', { force: true });
      toast(`بازیابی انجام شد؛ نسخهٔ قبلی در ${safetyPath} نگهداری شد.`, 'ok');
    } catch (error) {
      toast(errorText(error), 'error');
    } finally {
      button.disabled = false;
    }
  });
}

function openSettingsDialog() {
  const settings = state.settings || {};
  const dialog = openDialog({
    title: 'ویرایش مشخصات واحد اقتصادی',
    body: `
      <form id="settingsForm" class="settings-form" autocomplete="off">
        <div class="field">
          <label for="setCompany">نام شرکت / واحد اقتصادی <span class="req">*</span></label>
          <input id="setCompany" class="input" name="companyName" type="text" value="${esc(settings.companyName)}" placeholder="نام شرکت" />
        </div>
        <div class="field">
          <label for="setFiscal">سال مالی</label>
          <input id="setFiscal" class="input" name="fiscalYear" type="text" value="${esc(settings.fiscalYear)}" placeholder="۱۴۰۵" />
        </div>
        <div class="form-error" id="settingsError" hidden></div>
        <div class="form-actions">
          <button class="btn btn-primary" type="submit">${icon('i-save')}<span>ذخیره تنظیمات</span></button>
        </div>
      </form>`,
  });
  const form = dialog?.body.querySelector('#settingsForm');
  if (!form) return;
  form.addEventListener('submit', async (event) => {
    event.preventDefault();
    const errorBox = $('#settingsError', form);
    const companyName = (form.elements.companyName?.value || '').trim();
    const fiscalYear = (form.elements.fiscalYear?.value || '').trim();
    if (!companyName) {
      if (errorBox) {
        errorBox.textContent = 'نام شرکت الزامی است.';
        errorBox.hidden = false;
      }
      return;
    }
    const submit = form.querySelector('[type="submit"]');
    if (submit) submit.disabled = true;
    if (errorBox) errorBox.hidden = true;
    try {
      const saved = await invoke('save_settings', { companyName, fiscalYear });
      state.settings = saved;
      updateBranding(saved);
      closeDialog();
      toast('تنظیمات ذخیره شد.', 'ok');
      await navigate('settings', { force: true });
    } catch (error) {
      if (errorBox) {
        errorBox.textContent = errorText(error);
        errorBox.hidden = false;
      }
      if (submit) submit.disabled = false;
    }
  });
}

/* ── Shared event binding ───────────────────────────────────── */

function bindFilterOnly(root) {
  // Filter forms and generic navigation are handled by bindPageCommon.
}

function bindPageCommon(root) {
  root.querySelectorAll('[data-go]').forEach((button) => {
    button.addEventListener('click', () => navigate(button.dataset.go));
  });
  root.querySelectorAll('[data-open-groups]').forEach((button) => {
    button.addEventListener('click', () => openGroupsWindow());
  });
  root.querySelectorAll('[data-new-voucher]').forEach((button) => {
    button.addEventListener('click', () => openVoucherWindow());
  });
  root.querySelectorAll('[data-open-voucher-list]').forEach((button) => {
    button.addEventListener('click', () => openVoucherListWindow());
  });
  root.querySelectorAll('[data-open-voucher]').forEach((button) => {
    button.addEventListener('click', () => openVoucherWindow(Number(button.dataset.openVoucher)));
  });
  root.querySelectorAll('[data-new-account]').forEach((button) => {
    button.addEventListener('click', () => openAccountDialog(null));
  });

  // Digits and date input hygiene.
  root.querySelectorAll('input[data-date]').forEach((input) => {
    input.addEventListener('input', () => {
      const cleaned = toEn(input.value).replace(/[^\d/]/g, '');
      if (input.value !== cleaned) input.value = cleaned;
    });
  });

  // Filter forms.
  root.querySelectorAll('form[data-filter]').forEach((form) => {
    const key = form.dataset.filter;
    form.addEventListener('submit', (event) => {
      event.preventDefault();
      const data = new FormData(form);
      if (key === 'accountFilters') {
        state.accountFilters = {
          search: String(data.get('search') || ''),
          group: String(data.get('group') || ''),
          level: String(data.get('level') || ''),
        };
        navigate('accounts', { force: true });
        return;
      }
      const target = state.filters[key];
      if (!target) return;
      if (state.listOffsets[key] !== undefined) state.listOffsets[key] = 0;
      for (const [name, value] of data.entries()) {
        target[name] = String(value);
      }
      navigate(state.page, { force: true });
    });

    const reset = form.querySelector('[data-reset]');
    if (reset) {
      reset.addEventListener('click', () => {
        if (key === 'accountFilters') {
          state.accountFilters = { search: '', group: '', level: '' };
          navigate('accounts', { force: true });
          return;
        }
        const defaults = defaultFilters()[key];
        if (defaults) Object.assign(state.filters[key], defaults);
        if (state.listOffsets[key] !== undefined) state.listOffsets[key] = 0;
        navigate(state.page, { force: true });
      });
    }
  });
}

/* ── Main menu bar ─────────────────────────────────────────── */

let openMenuIndex = null;
let menuOpenedByHover = false;

function buildMenubar() {
  const nav = $('#menubarNav');
  if (!nav) return;

  nav.innerHTML = MENU_BAR.map((menu, menuIndex) => {
    const menuItems = menu.items.map((item, itemIndex) => {
      const pending = !item.page && !item.command;
      return `
        <button type="button" class="menu-item" role="menuitem"
          data-item-index="${itemIndex}"
          ${pending ? 'title="این فرم هنوز ساخته نشده است"' : ''}>
          <span class="menu-item-icon" aria-hidden="true">${esc(item.icon || '•')}</span>
          <span class="menu-item-label">${esc(item.label)}</span>
        </button>`;
    }).join('');

    const body = menuItems || `
      <div class="menu-empty" role="status">
        ${esc(menu.empty || 'برای این بخش هنوز گزینه‌ای تعریف نشده است')}
      </div>`;

    return `
      <div class="menu-wrap" data-menu-index="${menuIndex}">
        <button type="button" class="menu-root" aria-haspopup="menu" aria-expanded="false"
          data-menu-index="${menuIndex}">
          ${esc(menu.label)}
        </button>
        <div class="menu-dropdown" role="menu" aria-label="${esc(menu.label)}"
          data-size="${esc(menu.size || 'wide')}" hidden>
          <div class="menu-list">${body}</div>
        </div>
      </div>`;
  }).join('');
}

function openMenubarMenu(index, { fromHover = false } = {}) {
  $$('#menubarNav .menu-wrap').forEach((wrap, wrapIndex) => {
    const isOpen = wrapIndex === index;
    wrap.classList.toggle('is-open', isOpen);
    const trigger = $('.menu-root', wrap);
    if (trigger) trigger.setAttribute('aria-expanded', isOpen ? 'true' : 'false');
    const dropdown = $('.menu-dropdown', wrap);
    if (dropdown) dropdown.hidden = !isOpen;
  });
  openMenuIndex = index;
  menuOpenedByHover = fromHover;
}

function closeMenubarMenus() {
  $$('#menubarNav .menu-wrap').forEach((wrap) => {
    wrap.classList.remove('is-open');
    const trigger = $('.menu-root', wrap);
    if (trigger) trigger.setAttribute('aria-expanded', 'false');
    const dropdown = $('.menu-dropdown', wrap);
    if (dropdown) dropdown.hidden = true;
  });
  openMenuIndex = null;
  menuOpenedByHover = false;
}

function notifyPendingMenu(label) {
  toast(`«${label}» هنوز آماده نیست و در نسخه‌های بعدی اضافه می‌شود.`, 'info');
  setStatus('این بخش در دست ساخت است.', 'idle');
}

function openReorganizeVouchers() {
  if (!invoke) {
    toast('مرتب‌سازی و ادغام اسناد فقط در نسخه دسکتاپ ممکن است.', 'error');
    return;
  }

  const operations = {
    'merge-then-sort': 'ابتدا ادغام، سپس مرتب‌سازی',
    merge: 'فقط ادغام اسناد عادی هم‌تاریخ',
    sort: 'فقط مرتب‌سازی بر اساس تاریخ',
  };
  const dialog = openDialog({
    title: 'مرتب‌سازی و ادغام اسناد',
    wide: true,
    body: `
      <div class="form-grid">
        <label class="field">
          <span>عملیات</span>
          <select class="input" id="reorganizeOperation">
            ${Object.entries(operations).map(([value, label]) => `<option value="${value}"${value === 'merge-then-sort' ? ' selected' : ''}>${label}</option>`).join('')}
          </select>
        </label>
        <div class="note" id="reorganizeDescription">اسناد عادی هم‌تاریخ ادغام می‌شوند و سپس شماره‌ها بر اساس تاریخ بازچینی می‌شوند.</div>
      </div>
      <div id="reorganizePreview" class="reorganize-preview"></div>`,
    actions: [],
  });
  if (!dialog) return;

  let preview = null;
  let busy = false;
  const operationInput = dialog.body.querySelector('#reorganizeOperation');
  const description = dialog.body.querySelector('#reorganizeDescription');
  const previewArea = dialog.body.querySelector('#reorganizePreview');

  const setInitialActions = () => dialog.setActions([
    {
      label: 'پیش‌نمایش تغییرات',
      primary: true,
      icon: 'i-search',
      onClick: runPreview,
    },
  ]);

  const runPreview = async () => {
    if (busy) return;
    busy = true;
    const operation = operationInput.value;
    preview = null;
    operationInput.disabled = true;
    previewArea.innerHTML = '<div class="note">در حال تحلیل اسناد…</div>';
    dialog.setActions([]);
    try {
      preview = await invoke('preview_voucher_reorganization', { operation });
      const mergeGroups = preview.mergeGroups || [];
      const unbalancedGroups = preview.unbalancedGroups || [];
      const changes = preview.numberChanges || [];
      const noChanges = mergeGroups.length === 0 && changes.length === 0;
      const mergeRows = mergeGroups.slice(0, 20).map((group) => `
        <tr>
          <td>${esc(group.date)}</td>
          <td>${group.sourceVoucherNumbers.map(toFa).join('، ')}</td>
          <td>${toFa(group.keepVoucherNumber)}</td>
          <td>${toFa(group.lineCount)}</td>
        </tr>`).join('');
      const changeRows = changes.slice(0, 20).map((change) => `
        <tr>
          <td>${esc(change.date)}</td>
          <td>${toFa(change.oldNumber)}</td>
          <td>${toFa(change.newNumber)}</td>
        </tr>`).join('');
      const unbalancedRows = unbalancedGroups.slice(0, 10).map((group) => `
        <li>${esc(group.date)}: ${group.sourceVoucherNumbers.map(toFa).join('، ')} — ${esc(group.reason || 'گروه قابل ادغام نیست.')}</li>`).join('');
      previewArea.innerHTML = `
        <div class="info-list">
          <div><dt>تعداد اسناد</dt><dd>${toFa(preview.totalVouchers)}</dd></div>
          <div><dt>پس از ادغام</dt><dd>${toFa(preview.totalVouchersAfter)}</dd></div>
          <div><dt>گروه‌های قابل ادغام</dt><dd>${toFa(mergeGroups.length)}</dd></div>
          <div><dt>اسناد حذف‌شونده در ادغام</dt><dd>${toFa(preview.removedVoucherCount)}</dd></div>
          <div><dt>تغییر شماره</dt><dd>${toFa(changes.length)}</dd></div>
        </div>
        ${noChanges ? '<div class="note">تغییری برای این عملیات پیدا نشد.</div>' : ''}
        ${unbalancedGroups.length ? `<div class="note" data-tone="danger">${toFa(unbalancedGroups.length)} گروه هم‌تاریخ به دلیل نامتوازن‌بودن یا محدودیت داده ادغام نمی‌شوند.<ul>${unbalancedRows}</ul></div>` : ''}
        ${mergeGroups.length ? `
          <h3 class="reorganize-section-title">گروه‌های ادغام (${toFa(mergeGroups.length)})</h3>
          <div class="reorganize-table-wrap"><table><thead><tr><th>تاریخ</th><th>شماره اسناد</th><th>شماره باقی‌مانده</th><th>ردیف</th></tr></thead><tbody>${mergeRows}</tbody></table></div>
          ${mergeGroups.length > 20 ? `<div class="form-note">${toFa(mergeGroups.length - 20)} گروه دیگر در فهرست نمایش داده نشده است.</div>` : ''}` : ''}
        ${changes.length ? `
          <h3 class="reorganize-section-title">نمونه تغییر شماره‌ها (${toFa(changes.length)})</h3>
          <div class="reorganize-table-wrap"><table><thead><tr><th>تاریخ</th><th>شماره فعلی</th><th>شماره جدید</th></tr></thead><tbody>${changeRows}</tbody></table></div>
          ${changes.length > 20 ? `<div class="form-note">${toFa(changes.length - 20)} تغییر دیگر در فهرست نمایش داده نشده است.</div>` : ''}` : ''}
        ${mergeGroups.length ? '<label class="reorganize-confirm"><input id="reorganizeConfirmMerge" type="checkbox"><span>ادغام اسناد و حذف سرآیندهای تکراری را تأیید می‌کنم.</span></label>' : ''}
        <div class="note" data-reorganize-error hidden></div>`;

      dialog.setActions([
        {
          label: 'تأیید و اجرا',
          primary: true,
          icon: 'i-check',
          id: 'reorganizeApply',
          onClick: async (currentDialog, button) => {
            if (!preview || busy) return;
            if (preview.mergeGroups.length && !currentDialog.body.querySelector('#reorganizeConfirmMerge')?.checked) return;
            busy = true;
            button.disabled = true;
            try {
              const result = await invoke('apply_voucher_reorganization', {
                operation: preview.operation,
                planToken: preview.planToken,
              });
              currentDialog.close();
              const parts = [];
              if (result.mergedGroupCount) parts.push(`${toFa(result.mergedGroupCount)} گروه ادغام شد`);
              if (result.removedVoucherCount) parts.push(`${toFa(result.removedVoucherCount)} سند حذف شد`);
              if (result.renumberedCount) parts.push(`${toFa(result.renumberedCount)} شماره بازچینی شد`);
              toast(parts.length ? `${parts.join('، ')}.` : 'تغییری لازم نبود.', 'ok');
              await navigate('vouchers', { force: true });
            } catch (error) {
              const message = error instanceof Error ? error.message : String(error);
              const errorBox = currentDialog.body.querySelector('[data-reorganize-error]');
              if (errorBox) {
                errorBox.textContent = message;
                errorBox.hidden = false;
                errorBox.dataset.tone = 'danger';
              }
              const requiresMergeConfirmation = preview.mergeGroups.length > 0;
              const mergeConfirmed = Boolean(currentDialog.body.querySelector('#reorganizeConfirmMerge')?.checked);
              button.disabled = requiresMergeConfirmation && !mergeConfirmed;
            } finally {
              busy = false;
            }
          },
        },
        { label: 'پیش‌نمایش مجدد', icon: 'i-refresh', onClick: runPreview },
      ]);

      const applyButton = dialog.foot.querySelector('#reorganizeApply');
      const confirmMerge = dialog.body.querySelector('#reorganizeConfirmMerge');
      if (applyButton) applyButton.disabled = noChanges || mergeGroups.length > 0;
      confirmMerge?.addEventListener('change', () => {
        if (applyButton) applyButton.disabled = noChanges || !confirmMerge.checked;
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      previewArea.innerHTML = `<div class="note" data-tone="danger">${esc(message)}</div>`;
      setInitialActions();
    } finally {
      operationInput.disabled = false;
      busy = false;
    }
  };

  operationInput.addEventListener('change', () => {
    preview = null;
    previewArea.innerHTML = '';
    const descriptions = {
      'merge-then-sort': 'اسناد عادی هم‌تاریخ ادغام می‌شوند و سپس شماره‌ها بر اساس تاریخ بازچینی می‌شوند.',
      merge: 'فقط اسناد عادی با تاریخ یکسان ادغام می‌شوند؛ شماره‌ی سند باقی‌مانده تغییر نمی‌کند.',
      sort: 'شماره‌ی تمام اسناد بر اساس تاریخ، به‌صورت متوالی بازچینی می‌شود.',
    };
    description.textContent = descriptions[operationInput.value];
    setInitialActions();
  });
  setInitialActions();
}

function openBulkVoucherImport() {
  if (!invoke) {
    toast('ورود دسته‌جمعی اکسل فقط در نسخه دسکتاپ ممکن است.', 'error');
    return;
  }

  const dialog = openDialog({
    title: 'ورود دسته‌جمعی اسناد از اکسل',
    wide: true,
    body: '<div class="note">فایل اکسل با ستون‌های سند، تاریخ، کد کل، کد معین، شرح، بدهکار و بستانکار را انتخاب کنید.</div>',
    actions: [],
  });
  if (!dialog) return;

  let selecting = false;
  const selectFile = async () => {
    if (selecting) return;
    selecting = true;
    dialog.body.innerHTML = '<div class="note">فایل در حال بررسی و اسناد در حال اعتبارسنجی هستند…</div>';
    dialog.setActions([]);
    try {
      const preview = await invoke('analyze_bulk_voucher_excel');
      if (!preview) {
        dialog.body.innerHTML = '<div class="note">انتخاب فایل لغو شد.</div>';
        dialog.setActions([{ label: 'انتخاب فایل اکسل', primary: true, onClick: selectFile }]);
        return;
      }

      const conflicts = Number(preview.numberConflictCount) || 0;
      dialog.body.innerHTML = `
        <div class="note">فایل «${esc(preview.fileName)}» آماده‌ی ورود است. شماره اسناد حفظ می‌شود.</div>
        <dl class="info-list">
          <div><dt>تعداد سند</dt><dd>${toFa(preview.voucherCount)}</dd></div>
          <div><dt>تعداد ردیف</dt><dd>${toFa(preview.lineCount)}</dd></div>
          <div><dt>حساب‌های جدید</dt><dd>${toFa(preview.newAccountCount)}</dd></div>
          <div><dt>شماره‌های تکراری</dt><dd>${toFa(conflicts)}</dd></div>
        </dl>
        ${conflicts ? `<label class="bulk-import-renumber"><input id="bulkImportRenumber" type="checkbox"><span>برای ${toFa(conflicts)} سند دارای شماره‌ی تکراری، شماره‌ی آزاد جدید اختصاص بده</span></label>` : ''}
        <div class="note" data-import-error hidden></div>`;

      dialog.setActions([
        {
          id: 'bulkImportCommit',
          label: 'تأیید و ثبت اسناد',
          primary: true,
          icon: 'i-check',
          onClick: async (currentDialog, button) => {
            const renumberConflicts = Boolean(currentDialog.body.querySelector('#bulkImportRenumber')?.checked);
            button.disabled = true;
            try {
              const count = await invoke('commit_bulk_voucher_excel', { renumberConflicts });
              currentDialog.close();
              toast(`${toFa(count)} سند با موفقیت وارد شد.`, 'ok');
              await navigate('vouchers', { force: true });
            } catch (error) {
              const message = error instanceof Error ? error.message : String(error);
              const errorBox = currentDialog.body.querySelector('[data-import-error]');
              if (errorBox) {
                errorBox.textContent = message;
                errorBox.hidden = false;
                errorBox.dataset.tone = 'danger';
              }
              button.disabled = false;
            }
          },
        },
        { id: 'bulkImportReselect', label: 'انتخاب فایل دیگر', icon: 'i-refresh', onClick: selectFile },
      ]);

      const commitButton = dialog.foot.querySelector('#bulkImportCommit');
      const renumberCheckbox = dialog.body.querySelector('#bulkImportRenumber');
      if (commitButton && conflicts > 0) commitButton.disabled = true;
      renumberCheckbox?.addEventListener('change', () => {
        if (commitButton) commitButton.disabled = conflicts > 0 && !renumberCheckbox.checked;
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      dialog.body.innerHTML = `<div class="note" data-tone="danger">${esc(message)}</div>`;
      dialog.setActions([{ label: 'انتخاب فایل اکسل', primary: true, onClick: selectFile }]);
    } finally {
      selecting = false;
    }
  };

  dialog.setActions([{ label: 'انتخاب فایل اکسل', primary: true, icon: 'i-doc', onClick: selectFile }]);
}

async function openBulkVoucherDeleteWindow() {
  if (!invoke) {
    toast('حذف دسته‌جمعی اسناد فقط در نسخه دسکتاپ ممکن است.', 'error');
    return;
  }
  try {
    await invoke('open_bulk_voucher_delete_window');
    state.refreshOnFocus = true;
  } catch (error) {
    toast(errorText(error), 'error');
  }
}

function runMenuItem(item) {
  if (item.page) {
    navigate(item.page);
    return;
  }
  switch (item.command) {
    case 'groups-window':
      openGroupsWindow();
      return;
    case 'account-definition':
      window.openAccountDefinitionPopup?.();
      return;
    case 'new-voucher':
      openVoucherWindow();
      return;
    case 'voucher-list':
      openVoucherListWindow();
      return;
    case 'bulk-voucher-import':
      openBulkVoucherImport();
      return;
    case 'reorganize-vouchers':
      openReorganizeVouchers();
      return;
    case 'bulk-voucher-delete':
      openBulkVoucherDeleteWindow();
      return;
    case 'choose-wallpaper':
      $('#wallpaperPicker')?.click();
      return;
    case 'help':
      showHelpDialog();
      return;
    case 'about':
      showAboutDialog();
      return;
    case 'shortcuts':
      showShortcutsDialog();
      return;
    default:
      notifyPendingMenu(item.label);
  }
}

function bindMenubar() {
  const nav = $('#menubarNav');
  if (!nav) return;

  nav.addEventListener('click', (event) => {
    const trigger = event.target.closest('.menu-root');
    if (trigger) {
      const index = Number(trigger.dataset.menuIndex);
      if (openMenuIndex === index) {
        // اگر منو با عبور ماوس باز شده، کلیک روی همان عنوان آن را نبندد.
        if (menuOpenedByHover) menuOpenedByHover = false;
        else closeMenubarMenus();
      } else {
        openMenubarMenu(index);
      }
      return;
    }

    const itemButton = event.target.closest('.menu-item');
    if (!itemButton) return;
    const wrap = itemButton.closest('.menu-wrap');
    const menuIndex = Number(wrap?.dataset.menuIndex);
    const itemIndex = Number(itemButton.dataset.itemIndex);
    const entry = MENU_BAR[menuIndex]?.items?.[itemIndex];
    closeMenubarMenus();
    if (entry) runMenuItem(entry);
  });

  // مثل رابط سوم: وقتی یک منو باز است، عبور ماوس روی منوی دیگر آن را باز می‌کند.
  nav.addEventListener('mouseover', (event) => {
    if (openMenuIndex === null) return;
    const wrap = event.target.closest('.menu-wrap');
    if (!wrap) return;
    const index = Number(wrap.dataset.menuIndex);
    if (Number.isInteger(index) && index !== openMenuIndex) {
      openMenubarMenu(index, { fromHover: true });
    }
  });

  // Keyboard support for the menu bar: arrows move between menus and items.
  nav.addEventListener('keydown', (event) => {
    const item = event.target.closest('.menu-item');
    const trigger = event.target.closest('.menu-root');
    if (!item && !trigger) return;

    const triggers = $$('#menubarNav .menu-root');
    const activeTrigger = trigger || item.closest('.menu-wrap')?.querySelector('.menu-root');
    const menuIndex = triggers.indexOf(activeTrigger);
    if (menuIndex < 0) return;

    const focusItems = (index, last = false) => {
      const wrap = $$('#menubarNav .menu-wrap')[index];
      const items = wrap ? $$('.menu-item', wrap) : [];
      const target = last ? items[items.length - 1] : items[0];
      if (target) target.focus();
    };
    const moveMenu = (offset, focusFirst = false) => {
      const nextIndex = (menuIndex + offset + triggers.length) % triggers.length;
      if (item) {
        closeMenubarMenus();
        openMenubarMenu(nextIndex);
        if (focusFirst) focusItems(nextIndex);
        else triggers[nextIndex].focus();
      } else {
        triggers[nextIndex].focus();
        if (openMenuIndex !== null) {
          openMenubarMenu(nextIndex);
          focusItems(nextIndex);
        }
      }
    };

    if (item) {
      const items = $$('.menu-item', item.closest('.menu-wrap'));
      const itemIndex = items.indexOf(item);
      if (event.key === 'ArrowDown') {
        event.preventDefault();
        items[(itemIndex + 1) % items.length]?.focus();
      } else if (event.key === 'ArrowUp') {
        event.preventDefault();
        items[(itemIndex - 1 + items.length) % items.length]?.focus();
      } else if (event.key === 'Home') {
        event.preventDefault();
        items[0]?.focus();
      } else if (event.key === 'End') {
        event.preventDefault();
        items[items.length - 1]?.focus();
      } else if (event.key === 'ArrowRight') {
        event.preventDefault();
        moveMenu(1, true);
      } else if (event.key === 'ArrowLeft') {
        event.preventDefault();
        moveMenu(-1, true);
      } else if (event.key === 'Escape') {
        event.preventDefault();
        closeMenubarMenus();
        triggers[menuIndex]?.focus();
      } else if (event.key === 'Tab') {
        closeMenubarMenus();
      }
      return;
    }

    if (event.key === 'ArrowRight') {
      event.preventDefault();
      moveMenu(1);
    } else if (event.key === 'ArrowLeft') {
      event.preventDefault();
      moveMenu(-1);
    } else if (event.key === 'ArrowDown') {
      event.preventDefault();
      openMenubarMenu(menuIndex);
      focusItems(menuIndex);
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      openMenubarMenu(menuIndex);
      focusItems(menuIndex, true);
    } else if (event.key === 'Escape' && openMenuIndex !== null) {
      event.preventDefault();
      closeMenubarMenus();
    }
  });

  document.addEventListener('pointerdown', (event) => {
    const bar = $('.menubar');
    if (bar && !bar.contains(event.target)) closeMenubarMenus();
  });

  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && openMenuIndex !== null) closeMenubarMenus();
  });
}

function showHelpDialog() {
  openDialog({
    title: 'راهنمای نرم‌افزار',
    body: `
      <div class="note">
        <b>دسترسی به بخش‌ها</b><br>
        از منوبار بالای پنجره، بخش مورد نظر را انتخاب کنید. بخش‌های «اطلاعات پایه»،
        «گزارشات حسابداری»، «صورت‌های مالی» و «دفاتر قانونی» صفحات موجود را باز می‌کنند
        و بقیه گزینه‌ها به‌تدریج تکمیل می‌شوند.
      </div>
      <div class="note">
        <b>ثبت سند جدید</b><br>
        از «عملیات مالی ← صدور سند حسابداری» یا کلید <span dir="ltr">Ctrl + N</span> استفاده کنید.
      </div>`,
  });
}

function showAboutDialog() {
  openDialog({
    title: 'درباره',
    body: `
      <div class="note">
        <b>نرم‌افزار حسابداری</b><br>
        نسخه ${esc(APP_VERSION)}<br>
        نرم‌افزار حسابداری دسکتاپ با پایگاه داده محلی SQLite.
      </div>`,
  });
}

function showShortcutsDialog() {
  openDialog({
    title: 'میانبرهای صفحه‌کلید',
    body: `
      <div class="note">
        <b><span dir="ltr">Ctrl + N</span></b> — صدور سند جدید<br>
        <b><span dir="ltr">Esc</span></b> — بستن منوها و پنجره‌ها
      </div>`,
  });
}

async function exitApplication() {
  const confirmed = await confirmDialog(
    'خروج از برنامه',
    'آیا از بستن نرم‌افزار مطمئن هستید؟',
    'خروج',
    true
  );
  if (!confirmed) return;

  try {
    const tauriWindow = window.__TAURI__ && window.__TAURI__.window;
    if (tauriWindow && typeof tauriWindow.getCurrentWindow === 'function') {
      await tauriWindow.getCurrentWindow().close();
    } else {
      window.close();
    }
  } catch (error) {
    toast(errorText(error), 'error');
  }
}

/* ── Navigation ─────────────────────────────────────────────── */

let navSeq = 0;

async function navigate(page, { force = false } = {}) {
  // Outside the dedicated trial-balance window, this report opens as a popup
  // instead of replacing the page inside the main window.
  if (page === 'trial' && !IS_TRIAL_POPUP) {
    openTrialBalanceWindow();
    return;
  }
  if (page === 'workspace') {
    navSeq += 1;
    state.page = page;
    $('.accounting-app')?.classList.add('is-workspace-home');
    renderActions([]);
    const workspaceRoot = $('#page');
    if (workspaceRoot) workspaceRoot.replaceChildren();
    document.title = 'نرم‌افزار حسابداری';
    setStatus('فضای کار آماده است.', 'ok');
    return;
  }
  // The voucher list is a popup window too; only its own window renders it.
  if (page === 'vouchers' && !IS_VOUCHER_LIST_POPUP) {
    openVoucherListWindow();
    return;
  }
  const def = PAGES[page];
  if (!def) return;
  $('.accounting-app')?.classList.remove('is-workspace-home');
  if (state.page !== page) {
    state.listOffsets.vouchers = 0;
    state.listOffsets.journal = 0;
  }
  state.page = page;

  const [title, section] = pageMeta(page);
  const titleEl = $('#pageTitle');
  const crumbs = $('#crumbs');
  if (titleEl) titleEl.textContent = title;
  if (crumbs) crumbs.innerHTML = `حسابداری / ${esc(section)} / <b>${esc(title)}</b>`;
  document.title = `${title} — نرم‌افزار حسابداری`;

  renderActions(def.actions ? def.actions() : []);
  const root = $('#page');
  if (!root) return;
  const seq = ++navSeq;
  root.innerHTML = `<div class="empty-state">
    <div class="spinner" aria-hidden="true" style="width:26px;height:26px"></div>
    <b>در حال بارگذاری…</b>
    <span>${esc(title)}</span>
  </div>`;

  try {
    const data = await def.load(force);
    if (seq !== navSeq || state.page !== page) return;
    root.innerHTML = def.render(data);
    bindPageCommon(root);
    def.bind?.(root, data);
    setStatus(`صفحه «${title}» آماده است.`, 'ok');
  } catch (error) {
    if (seq !== navSeq || state.page !== page) return;
    root.innerHTML = `<div class="card">
      <div class="empty-state">
        ${icon('i-warning')}
        <b>بارگذاری «${esc(title)}» ناموفق بود</b>
        <span>${esc(errorText(error))}</span>
        <button type="button" class="btn btn-primary" data-retry>${icon('i-refresh')}<span>تلاش دوباره</span></button>
      </div>
    </div>`;
    root.querySelector('[data-retry]')?.addEventListener('click', () => navigate(page, { force: true }));
    setStatus('خطا در بارگذاری اطلاعات.', 'error');
  }
}

/* ── Windows and chrome ─────────────────────────────────────── */

async function openVoucherWindow(number = null) {
  if (!invoke) return;
  try {
    await invoke('open_voucher_window', { number: number || null });
    state.refreshOnFocus = true;
  } catch (error) {
    toast(errorText(error), 'error');
  }
}

async function openVoucherListWindow() {
  if (!invoke) return;
  try {
    await invoke('open_voucher_list_window');
    state.refreshOnFocus = true;
  } catch (error) {
    toast(errorText(error), 'error');
  }
}

async function openTrialBalanceWindow() {
  if (!invoke) return;
  try {
    await invoke('open_trial_balance_window');
    state.refreshOnFocus = true;
  } catch (error) {
    toast(errorText(error), 'error');
  }
}

async function openGroupsWindow() {
  if (!invoke) return;
  try {
    await invoke('open_account_groups_window');
    state.refreshOnFocus = true;
  } catch (error) {
    toast(errorText(error), 'error');
  }
}

function bindChrome() {
  if (!IS_VOUCHER_LIST_POPUP) {
    const homeButton = $('[data-workspace-home]');
    homeButton?.addEventListener('click', () => void navigate('workspace'));
    homeButton?.addEventListener('keydown', (event) => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        void navigate('workspace');
      }
    });
  }
  $('#wallpaperPicker')?.addEventListener('change', (event) => {
    chooseWallpaper(event.target.files?.[0]);
    event.target.value = '';
  });
  window.addEventListener('account-definition-opened', () => {
    state.refreshOnFocus = true;
  });

  window.addEventListener('accounts-changed', () => {
    state.accountsLoaded = false;
    state.groupsLoaded = false;
    if (state.page === 'accounts') void navigate('accounts', { force: true });
  });
  const eventApi = window.__TAURI__?.event;
  if (typeof eventApi?.listen === 'function') {
    void eventApi.listen('accounts-changed', () => {
      window.dispatchEvent(new Event('accounts-changed'));
    }).catch(() => {});
  }

  // The bulk-delete form runs in its own window; when it finishes, any open
  // voucher list or voucher-derived report should reload its data.
  const voucherPages = new Set(['vouchers', 'journal', 'ledger', 'trial', 'income', 'balance']);
  window.addEventListener('vouchers-changed', () => {
    state.listOffsets.vouchers = 0;
    state.listOffsets.journal = 0;
    if (voucherPages.has(state.page)) void navigate(state.page, { force: true });
  });
  if (typeof eventApi?.listen === 'function') {
    void eventApi.listen('vouchers-changed', () => {
      window.dispatchEvent(new Event('vouchers-changed'));
    }).catch(() => {});
  }
  $('#btnExitApp')?.addEventListener('click', () => {
    void exitApplication();
  });
  $$('[data-quick]').forEach((button) => {
    button.addEventListener('click', () => {
      const quick = button.dataset.quick;
      if (quick === 'new-voucher') {
        openVoucherWindow();
      } else {
        navigate(quick);
      }
    });
  });

  $('#dialogClose')?.addEventListener('click', closeDialog);
  $('#modal')?.addEventListener('click', (event) => {
    if (event.target.id === 'modal') closeDialog();
  });
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && !$('#modal')?.hidden) {
      event.preventDefault();
      closeDialog();
      return;
    }
    // Ctrl+N opens a new voucher, as described in the README.
    if (!IS_TRIAL_POPUP && event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey
        && event.key.toLowerCase() === 'n' && $('#modal')?.hidden !== false) {
      event.preventDefault();
      openVoucherWindow();
    }
  });

  let focusTimer = 0;
  window.addEventListener('focus', () => {
    if (!state.refreshOnFocus || !state.ready) return;
    clearTimeout(focusTimer);
    focusTimer = setTimeout(async () => {
      state.refreshOnFocus = false;
      state.accountsLoaded = false;
      state.groupsLoaded = false;
      try {
        await loadSettings();
        await navigate(state.page, { force: true });
      } catch (_) { /* a later explicit refresh will report the error */ }
    }, 420);
  });
}

/* ── Init ───────────────────────────────────────────────────── */

async function init() {
  if (!IS_TRIAL_POPUP && !IS_VOUCHER_LIST_POPUP) {
    buildMenubar();
    bindMenubar();
  }
  bindChrome();

  if (!invoke) {
    renderFatal('این صفحه فقط داخل برنامه دسکتاپ نرم‌افزار حسابداری کار می‌کند.');
    return;
  }

  try {
    setStatus('در حال آماده‌سازی پایگاه داده…', 'busy');
    await loadSettings();
    state.filters = defaultFilters();
    state.ready = true;
    applyWallpaper(localStorage.getItem('hesabdari-workspace-wallpaper'));
    await navigate(IS_TRIAL_POPUP ? 'trial' : IS_VOUCHER_LIST_POPUP ? 'vouchers' : 'workspace');
    setStatus('آماده', 'ok');
  } catch (error) {
    renderFatal(errorText(error));
  }
}

function renderFatal(message) {
  const root = $('#page');
  if (!root) return;
  root.innerHTML = `<div class="card">
    <div class="empty-state">
      ${icon('i-warning')}
      <b>برنامه آماده نیست</b>
      <span>${esc(message)}</span>
      <button type="button" class="btn btn-primary" data-retry>${icon('i-refresh')}<span>تلاش دوباره</span></button>
    </div>
  </div>`;
  root.querySelector('[data-retry]')?.addEventListener('click', () => window.location.reload());
  setStatus('خطا در راه‌اندازی برنامه.', 'error');
}

document.addEventListener('DOMContentLoaded', init);
