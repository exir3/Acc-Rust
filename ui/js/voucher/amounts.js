'use strict';

/* ── Amounts ────────────────────────────────────────────────── */

const groupDigits = (digits) => String(digits).replace(/\B(?=(\d{3})+(?!\d))/g, ',');

function formatAmount(value) {
  const big = typeof value === 'bigint' ? value : BigInt(digitsOnly(value) || '0');
  const negative = big < 0n;
  const digits = (negative ? -big : big).toString();
  return `${negative ? '-' : ''}${groupDigits(digits)}`;
}

function parseAmountInput(value) {
  const normalized = toAsciiDigits(String(value ?? '')).trim();
  if (!normalized) return 0n;
  if (!/^(?:\d+|\d{1,3}(?:,\d{3})+)$/.test(normalized)) return null;
  return BigInt(normalized.replace(/,/g, ''));
}

// Fallbacks mirror the backend. `loadSettings` replaces them with the values
// returned by `get_settings`, so a limit change needs no front-end edit.
let MAX_AMOUNT = 999_999_999_999_999n;
let MAX_VOUCHER_LINES = 1000;
let MAX_VOUCHER_NUMBER = Number.MAX_SAFE_INTEGER;
let MAX_DESCRIPTION = 1000;
let MAX_LINE_DESCRIPTION = 500;

function applyLimits(limits) {
  if (!limits) return;
  const lines = Number(limits.maxVoucherLines);
  if (Number.isSafeInteger(lines) && lines > 0) MAX_VOUCHER_LINES = lines;
  const maxAmount = Number(limits.maxAmount);
  if (Number.isSafeInteger(maxAmount) && maxAmount > 0) MAX_AMOUNT = BigInt(maxAmount);
  const maxNumber = Number(limits.maxVoucherNumber);
  if (Number.isSafeInteger(maxNumber) && maxNumber > 0) MAX_VOUCHER_NUMBER = maxNumber;

  const maxDescription = Number(limits.maxDescription);
  if (Number.isSafeInteger(maxDescription) && maxDescription > 0) {
    MAX_DESCRIPTION = maxDescription;
    const area = $('#vfDocDesc');
    if (area) area.maxLength = maxDescription;
    updateDescCounter();
  }
  const maxLineDescription = Number(limits.maxLineDescription);
  if (Number.isSafeInteger(maxLineDescription) && maxLineDescription > 0) {
    MAX_LINE_DESCRIPTION = maxLineDescription;
    $$('.vf-desc-input').forEach((input) => { input.maxLength = maxLineDescription; });
  }
}

// Mirrors jalali_calendar's 33-year leap rule so the form can reject impossible
// dates (e.g. 31 Mehr or 30 Esfand in a common year) before a network round-trip.
function isJalaliLeapYear(year) {
  const countLeapDays = (value) => Math.trunc(value / 33) * 8
    + Math.trunc((((value % 33) + 3) / 4));
  const shifted = year + 1595;
  return countLeapDays(shifted + 1) - countLeapDays(shifted) === 1;
}

function jalaliMonthLength(year, month) {
  if (month >= 1 && month <= 6) return 31;
  if (month >= 7 && month <= 11) return 30;
  if (month === 12) return isJalaliLeapYear(year) ? 30 : 29;
  return 0;
}

