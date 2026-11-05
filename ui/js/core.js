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
let LEDGER_PAGE_SIZE = 500;

const state = {
  page: 'workspace',
  settings: null,
  accounts: [],
  groups: [],
  accountsLoaded: false,
  groupsLoaded: false,
  databasePath: '',
  fiscalYears: [],
  legacyLevel4: null,
  filters: {},
  listOffsets: { vouchers: 0, journal: 0, ledger: 0 },
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

