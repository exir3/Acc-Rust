'use strict';

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

// Mirrors jalali_calendar's 33-year leap rule so the report filters can reject
// impossible dates (e.g. 31 Mehr or 30 Esfand in a common year) before the
// whole page is replaced by a backend error.
function isJalaliLeapYear(year) {
  const countLeapDays = (value) => Math.trunc(value / 33) * 8
    + Math.trunc((((value % 33) + 3) / 4));
  const shifted = year + 1595;
  return countLeapDays(shifted + 1) - countLeapDays(shifted) === 1;
}

function isValidJalaliDate(value) {
  const text = toEn(value).trim().replace(/-/g, '/');
  const parts = text.split('/');
  if (parts.length !== 3) return false;
  if (!/^\d{4}$/.test(parts[0]) || !/^\d{1,2}$/.test(parts[1]) || !/^\d{1,2}$/.test(parts[2])) {
    return false;
  }
  const year = Number(parts[0]);
  const month = Number(parts[1]);
  const day = Number(parts[2]);
  if (year < 1 || year > 3177 || month < 1 || month > 12) return false;
  const maxDay = month <= 6 ? 31 : month <= 11 ? 30 : isJalaliLeapYear(year) ? 30 : 29;
  return day >= 1 && day <= maxDay;
}

function padJalaliDate(value) {
  const parts = toEn(value).trim().replace(/-/g, '/').split('/');
  if (parts.length !== 3) return toEn(value).trim();
  return `${parts[0].padStart(4, '0')}/${parts[1].padStart(2, '0')}/${parts[2].padStart(2, '0')}`;
}

function setFieldError(input, message) {
  input.classList.add('is-invalid');
  input.setAttribute('aria-invalid', 'true');
  const field = input.closest('.field');
  if (!field) return;
  let error = field.querySelector('.field-error');
  if (!error) {
    error = document.createElement('small');
    error.className = 'field-error';
    field.append(error);
  }
  error.textContent = message;
}

function clearFieldError(input) {
  input.classList.remove('is-invalid');
  input.removeAttribute('aria-invalid');
  input.closest('.field')?.querySelector('.field-error')?.remove();
}

function validateDateFields(form) {
  let firstInvalid = null;
  form.querySelectorAll('input[data-date]').forEach((input) => {
    clearFieldError(input);
    const value = input.value.trim();
    if (!value) {
      if (input.name === 'asOf') {
        setFieldError(input, 'تاریخ ترازنامه را وارد کنید.');
        firstInvalid = firstInvalid || input;
      }
      return;
    }
    if (!isValidJalaliDate(value)) {
      setFieldError(input, 'تاریخ شمسی معتبر نیست؛ نمونه: ۱۴۰۵/۱۲/۲۹.');
      firstInvalid = firstInvalid || input;
    }
  });
  const from = form.querySelector('[name="from"]');
  const to = form.querySelector('[name="to"]');
  if (from?.value.trim() && to?.value.trim()) {
    if (padJalaliDate(from.value) > padJalaliDate(to.value)) {
      setFieldError(to, 'تاریخ پایان نباید پیش از تاریخ شروع باشد.');
      firstInvalid = firstInvalid || to;
    }
  }
  if (firstInvalid) {
    firstInvalid.focus();
    setStatus('تاریخ واردشده معتبر نیست.', 'error');
    return false;
  }
  return true;
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
  const baseWidth = ({ 1: 4, 2: 2, 3: 3 })[Number(level)];
  if (!baseWidth || (Number(level) > 1 && !parentCode)) return '';

  const prefix = Number(level) === 1 ? '' : String(parentCode);
  const siblings = accounts.filter((account) =>
    Number(account.level) === Number(level)
    && (Number(level) === 1
      ? !account.parentCode
      : account.parentCode === parentCode)
  );
  const suffixWidths = siblings
    .map((account) => String(account.code).length - prefix.length)
    .filter((width) => Number.isInteger(width) && width > 0);
  const width = Math.max(baseWidth, ...suffixWidths);
  if (prefix.length + width > 50) return '';

  const minimum = Number(level) === 1 ? 10n ** BigInt(width - 1) : 1n;
  const maximum = 10n ** BigInt(width) - 1n;
  const suffixPattern = new RegExp(`^\\d{${width}}$`);
  const suffixes = siblings
    .map((account) => String(account.code).slice(prefix.length))
    .filter((suffix) => suffixPattern.test(suffix))
    .map(BigInt);
  const maximumUsed = suffixes.length
    ? suffixes.reduce((max, value) => (value > max ? value : max), minimum - 1n)
    : (Number(level) === 1 ? minimum : 0n);
  const next = maximumUsed + 1n;
  const ranges = [
    [next, maximum],
    [minimum, next - 1n < maximum ? next - 1n : maximum],
  ];
  const used = new Set(accounts.map((account) => String(account.code)));

  for (const [start, end] of ranges) {
    let suffix = start;
    let checked = 0;
    while (suffix <= end && checked <= accounts.length) {
      const candidate = `${prefix}${suffix.toString().padStart(width, '0')}`;
      if (!used.has(candidate)) return candidate;
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

function integrityPanel(integrity) {
  if (!integrity) return '';
  const counts = integrity.counts || {};
  const issues = Array.isArray(integrity.issues) ? integrity.issues : [];
  const ok = integrity.ok !== false;
  const tone = ok ? 'ok' : 'danger';
  const severityTone = { error: 'danger', warning: 'warn', info: 'info' };
  const severityLabel = { error: 'خطا', warning: 'هشدار', info: 'اطلاع' };
  const rows = issues.map((issue) => {
    const issueTone = severityTone[issue.severity] || 'neutral';
    const label = severityLabel[issue.severity] || issue.severity;
    const accounts = Array.isArray(issue.accounts) && issue.accounts.length
      ? `<div class="muted small">نمونه حساب‌ها: ${issue.accounts.map((code) => `<span class="mono">${esc(toFa(code))}</span>`).join('، ')}</div>`
      : '';
    return `<div class="integrity-issue" data-tone="${issueTone}">
      <div class="integrity-issue-head">${badge(label, issueTone)}<span>${esc(issue.message)}</span></div>
      ${accounts}
    </div>`;
  }).join('');
  return `<div class="integrity-panel" data-tone="${tone}">
    <div class="integrity-head">
      <div><strong>کنترل سلامت صورت‌های مالی</strong> ${badge(ok ? 'بدون خطا' : 'نیازمند بررسی', tone)}</div>
      <div class="muted small">${toFa(counts.error || 0)} خطا · ${toFa(counts.warning || 0)} هشدار · ${toFa(counts.info || 0)} اطلاع</div>
    </div>
    ${rows || '<div class="muted small">هیچ موردی یافت نشد.</div>'}
  </div>`;
}

