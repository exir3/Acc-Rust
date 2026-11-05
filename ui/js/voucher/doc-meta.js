'use strict';

/* ── Document meta and description ──────────────────────────── */

const VF_DATE_SEGMENT_RANGES = {
  year: [0, 4],
  month: [5, 7],
  day: [8, 10],
};

function vfDateSegmentFromCursor(position) {
  if (position <= 4) return 'year';
  if (position <= 7) return 'month';
  return 'day';
}

/* Splits a date into padded segments. Accepts Persian/Arabic digits and
   returns an ASCII "YYYY/MM/DD" string. */
function vfParseDateParts(value) {
  const english = toAsciiDigits(String(value ?? ''));
  const parts = english.split('/');
  if (parts.length === 3) {
    return {
      year: parts[0].replace(/\D/g, '').padStart(4, '0').slice(0, 4),
      month: parts[1].replace(/\D/g, '').padStart(2, '0').slice(0, 2),
      day: parts[2].replace(/\D/g, '').padStart(2, '0').slice(0, 2),
    };
  }
  const digits = english.replace(/\D/g, '').padStart(8, '0');
  digits.length = Math.min(digits.length, 8);
  return {
    year: digits.slice(0, 4),
    month: digits.slice(4, 6),
    day: digits.slice(6, 8),
  };
}

function updateDescCounter() {
  const area = $('#vfDocDesc');
  const output = $('#vfDocDescCount');
  if (!area || !output) return;
  let chars = Array.from(area.value);
  if (chars.length > MAX_DESCRIPTION) {
    area.value = chars.slice(0, MAX_DESCRIPTION).join('');
    chars = chars.slice(0, MAX_DESCRIPTION);
  }
  output.textContent = `${vfFa(chars.length)}/${vfFa(MAX_DESCRIPTION)}`;
}

/* ── Header counters ───────────────────────────────────────── */

let vfCounterSequence = 0;

async function refreshVoucherCounters(number) {
  if (!invoke) return;
  const sequence = ++vfCounterSequence;
  const target = Number(number);
  try {
    const counters = await invoke('voucher_counters', {
      number: Number.isSafeInteger(target) && target > 0 ? target : null,
    });
    if (sequence !== vfCounterSequence) return;
    const total = $('#vfTotalRecords');
    const upTo = $('#vfRecordsUpTo');
    if (total) total.textContent = vfFa(counters.totalRecords ?? 0);
    if (upTo) upTo.textContent = vfFa(counters.recordsUpToThisDoc ?? 0);
  } catch {
    // Counters are informational; a failure must not block the form.
  }
}

/* ── Permanent / temporary document ─────────────────────────── */

async function setDocumentPermanent(value) {
  if (isLockedVoucher()) {
    toast('این سند دائمی است و قابل تغییر نیست.', 'warning');
    return;
  }
  if (state.busy) return;
  if (value === Boolean(state.makePermanent)) {
    refreshFormState();
    return;
  }
  if (value) {
    const confirmed = await confirmAction(
      'سند پس از ذخیره «دائمی» می‌شود و دیگر قابل ویرایش یا حذف نخواهد بود. این عمل قابل بازگشت نیست. ادامه می‌دهید؟',
      'بله، دائمی کن'
    );
    if (!confirmed) return;
    if (isLockedVoucher()) return;
  }
  state.makePermanent = value;
  markDirty();
  refreshFormState();
}

/* ── Date segment entry ─────────────────────────────────────── */

function bindDocMeta() {
  const docNo = $('#vfDocNo');
  const date = $('#vfDocDate');
  const desc = $('#vfDocDesc');

  if (docNo) {
    docNo.addEventListener('input', () => {
      const cleaned = digitsOnly(docNo.value);
      if (docNo.value !== cleaned) docNo.value = cleaned;
    });
    docNo.addEventListener('keydown', (ev) => {
      if (ev.key === 'Enter') {
        ev.preventDefault();
        openNumber(digitsOnly(docNo.value));
      }
    });
  }

  if (date) {
    let validationSequence = 0;
    let activeSegment = 'day';
    let typedCount = 0;
    let initialSegments = { year: '', month: '', day: '' };
    let selectDayOnNextFocus = false;

    const setValue = (parts) => {
      const next = `${parts.year}/${parts.month}/${parts.day}`;
      if (date.value !== next) date.value = next;
      return next;
    };

    const selectSegment = (segment) => {
      activeSegment = segment;
      typedCount = 0;
      const [start, end] = VF_DATE_SEGMENT_RANGES[segment];
      try { date.setSelectionRange(start, end); } catch { /* ignore */ }
    };

    const updateSegmentFromCursor = () => {
      const cursor = date.selectionStart ?? 0;
      activeSegment = vfDateSegmentFromCursor(cursor);
      typedCount = 0;
    };

    // Loading a voucher with Enter in the document number selects the day
    // segment, exactly like the reference form.
    window.focusVoucherDateDay = () => {
      selectDayOnNextFocus = true;
      date.focus();
      selectSegment('day');
    };

    date.addEventListener('focus', () => {
      initialSegments = vfParseDateParts(date.value);
      typedCount = 0;
      if (selectDayOnNextFocus) {
        selectDayOnNextFocus = false;
        requestAnimationFrame(() => selectSegment('day'));
      }
    });
    date.addEventListener('click', updateSegmentFromCursor);
    date.addEventListener('input', () => {
      validationSequence += 1;
      const parts = vfParseDateParts(date.value);
      setValue(parts);
      date.classList.remove('is-invalid');
      date.removeAttribute('aria-invalid');
      date.title = '';
    });

    date.addEventListener('keydown', (ev) => {
      if (ev.key === 'Enter') {
        ev.preventDefault();
        const parts = vfParseDateParts(date.value);
        const hasChanged = parts[activeSegment] !== initialSegments[activeSegment] || typedCount > 0;
        if (!hasChanged) {
          focusFirstDocumentRow();
          return;
        }
        if (activeSegment === 'day') {
          selectSegment('month');
          initialSegments.month = parts.month;
        } else if (activeSegment === 'month') {
          selectSegment('year');
          initialSegments.year = parts.year;
        } else {
          focusFirstDocumentRow();
        }
        return;
      }

      const digitMatch = /[0-9۰-۹٠-٩]/.exec(ev.key);
      if (digitMatch && !ev.ctrlKey && !ev.metaKey && !ev.altKey) {
        ev.preventDefault();
        validationSequence += 1;
        date.classList.remove('is-invalid');
        date.removeAttribute('aria-invalid');
        const digit = toAsciiDigits(ev.key);
        const parts = vfParseDateParts(date.value);

        if (activeSegment === 'day') {
          if (typedCount === 0) {
            parts.day = `0${digit}`;
            typedCount = 1;
            setValue(parts);
            try { date.setSelectionRange(9, 9); } catch { /* ignore */ }
          } else {
            parts.day = `${parts.day.slice(1)}${digit}`;
            typedCount = 2;
            setValue(parts);
            selectSegment('month');
            initialSegments.month = parts.month;
          }
        } else if (activeSegment === 'month') {
          if (typedCount === 0) {
            parts.month = `0${digit}`;
            typedCount = 1;
            setValue(parts);
            try { date.setSelectionRange(6, 6); } catch { /* ignore */ }
          } else {
            parts.month = `${parts.month.slice(1)}${digit}`;
            typedCount = 2;
            setValue(parts);
            selectSegment('year');
            initialSegments.year = parts.year;
          }
        } else if (activeSegment === 'year') {
          if (typedCount === 0) {
            parts.year = `${digit}000`;
            typedCount = 1;
            setValue(parts);
            try { date.setSelectionRange(1, 1); } catch { /* ignore */ }
          } else if (typedCount < 4) {
            const typedSoFar = parts.year.slice(0, typedCount);
            typedCount += 1;
            parts.year = `${typedSoFar}${digit}${'0'.repeat(4 - typedCount)}`;
            setValue(parts);
            if (typedCount < 4) {
              try { date.setSelectionRange(typedCount, typedCount); } catch { /* ignore */ }
            } else {
              selectSegment('year');
            }
          }
        }
        return;
      }

      if (ev.key === 'ArrowRight' || ev.key === 'ArrowLeft') {
        setTimeout(updateSegmentFromCursor, 0);
      }
    });

    date.addEventListener('blur', async () => {
      const value = toAsciiDigits(date.value).replace(/-/g, '/').replace(/[^\d/]/g, '');
      if (!value || !invoke) return;
      const request = ++validationSequence;
      try {
        const normalizedDate = await invoke('validate_voucher_date', { date: value });
        if (request !== validationSequence || date.value !== value) return;
        if (typeof normalizedDate === 'string') date.value = normalizedDate;
        date.classList.remove('is-invalid');
        date.removeAttribute('aria-invalid');
        date.title = '';
      } catch (error) {
        if (request !== validationSequence || date.value !== value) return;
        date.classList.add('is-invalid');
        date.setAttribute('aria-invalid', 'true');
        date.title = errorText(error);
        toast(errorText(error), 'error');
      }
    });
  }

  if (desc) desc.addEventListener('input', updateDescCounter);

  $('#vfPermanentBtn')?.addEventListener('click', () => setDocumentPermanent(true));
  $('#vfChangesBtn')?.addEventListener('click', () => setDocumentPermanent(false));

  $('#vfPrevBtn')?.addEventListener('click', navigatePrev);
  $('#vfNextBtn')?.addEventListener('click', navigateNext);

  refreshVoucherCounters(currentNumber());
}
