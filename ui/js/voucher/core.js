'use strict';

/* ============================================================
   Accounting voucher window
   ============================================================
   Opens in its own OS window and writes real vouchers to the
   local SQLite database. The window can load an existing
   voucher, start a new one, copy, print and delete.

   Amounts are held as whole Rials. Persian and Arabic-Indic
   digits typed or pasted are converted to ASCII on the way in;
   the FD font draws ASCII digits as Persian.
   ============================================================ */

const TAURI = window.__TAURI__ || null;
const invoke = TAURI && TAURI.core ? TAURI.core.invoke.bind(TAURI.core) : null;
const appWindow = TAURI && TAURI.window && TAURI.window.getCurrentWindow
  ? TAURI.window.getCurrentWindow()
  : null;

const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => Array.from(root.querySelectorAll(sel));

const VF_FA_DIGITS = '۰۱۲۳۴۵۶۷۸۹';
const VF_AR_DIGITS = '٠١٢٣٤٥٦٧٨٩';

const VOUCHER_TYPES = [
  'سند حسابداری عادی',
  'سند حسابداری افتتاحیه',
  'سند حسابداری اختتامیه',
  'سند بستن حساب‌های موقت',
];

const vfFa = (value) => String(value).replace(/\d/g, (d) => VF_FA_DIGITS[Number(d)]);

/* Every value kept in the form/model is ASCII; the FD font draws them
   as Persian digits. Persian and Arabic-Indic digits typed or pasted
   are converted on the way in. */
function toAsciiDigits(value) {
  return String(value)
    .replace(/[۰-۹]/g, (d) => String(VF_FA_DIGITS.indexOf(d)))
    .replace(/[٠-٩]/g, (d) => String(VF_AR_DIGITS.indexOf(d)));
}

const digitsOnly = (value) => toAsciiDigits(value).replace(/[^\d]/g, '');

/* The fiscal year chosen at the startup gate is shared through localStorage,
   the same way the main window remembers it. The voucher form uses it to pick
   a sensible default date and to refuse a new voucher in a closed year. */
const ACTIVE_FISCAL_YEAR_KEY = 'hesabdari-active-fiscal-year';

// localStorage may be disabled or full; these helpers keep the form working
// without a remembered value instead of failing at startup.
function storageGet(key) {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function storageSet(key, value) {
  try {
    localStorage.setItem(key, value);
    return true;
  } catch {
    return false;
  }
}

function normalizeYearDate(value) {
  return toAsciiDigits(String(value || '')).replace(/-/g, '/');
}

function readActiveFiscalYear() {
  try {
    const raw = storageGet(`${ACTIVE_FISCAL_YEAR_KEY}-data`);
    if (!raw) return null;
    const year = JSON.parse(raw);
    if (year && Number.isFinite(Number(year.id))) return year;
  } catch {
    // Ignore malformed persisted selection.
  }
  return null;
}

function refreshActiveFiscalYear() {
  state.activeFiscalYear = readActiveFiscalYear();
  return state.activeFiscalYear;
}

function dateInFiscalYear(date, year) {
  if (!year) return true;
  const value = normalizeYearDate(date);
  const start = normalizeYearDate(year.startDate);
  const end = normalizeYearDate(year.endDate);
  if (!value || !start || !end) return true;
  return value >= start && value <= end;
}

/* Search should not care about digit script, Arabic yeh/kaf or ZWNJ. */
function normalizeSearch(value) {
  return toAsciiDigits(String(value || ''))
    .replace(/[يى]/g, 'ی')
    .replace(/ك/g, 'ک')
    .replace(/\u200c/g, '')
    .replace(/\s+/g, ' ')
    .trim()
    .toLowerCase();
}

function errorText(error) {
  if (typeof error === 'string') return error;
  if (error && typeof error.message === 'string') return error.message;
  return String(error ?? 'خطای نامشخص');
}

