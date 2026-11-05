'use strict';

/* ── Draft / persistence ────────────────────────────────────── */

function setDocType(value) {
  const select = $('#vfDocType');
  if (!select) return;
  const wanted = value || VOUCHER_TYPES[0];
  let option = Array.from(select.options).find((item) => item.value === wanted || item.textContent === wanted);
  if (!option) {
    option = document.createElement('option');
    option.value = wanted;
    option.textContent = wanted;
    select.append(option);
  }
  select.value = option.value;
}

function currentNumber() {
  const typed = Number(digitsOnly($('#vfDocNo')?.value || ''));
  return typed > 0 ? typed : (state.voucher?.number || state.nextNumber);
}

function fillGrid(lines) {
  const tbody = $('#voucherRows');
  if (!tbody) return;
  state.overflowVoucherLines = Math.max(0, (lines || []).length - MAX_VOUCHER_LINES);
  if (state.overflowVoucherLines) {
    toast(`این سند ${vfFa(lines.length)} ردیف دارد و ظرفیت فرم ${vfFa(MAX_VOUCHER_LINES)} ردیف است؛ سند فقط‌خواندنی باز شد.`, 'error');
  }
  tbody.textContent = '';
  const rendered = (lines || []).slice(0, MAX_VOUCHER_LINES);
  if (rendered.length) {
    // Build all rows in one HTML insertion. Calling appendGridRow per line made
    // every call rescan the existing rows, turning a 1000-line voucher into a
    // quadratic amount of DOM work.
    tbody.insertAdjacentHTML('beforeend', rendered.map(() => voucherRowTemplate()).join(''));
    const rows = Array.from(tbody.children);
    rendered.forEach((line, index) => {
      const row = rows[index];
      if (!row) return;
      // Keep the stable id of the saved line so the backend can tell an edited
      // saved row from a newly added row using the same (possibly inactive or
      // legacy) account.
      if (line.id != null) {
        row.dataset.lineId = String(line.id);
        row.dataset.savedAccount = digitsOnly(line.accountCode || '');
        row.dataset.savedDetail1 = digitsOnly(line.detail1Code || '');
      }
      row.querySelector('.vf-code-input').value = line.accountCode || '';
      row.querySelector('.vf-detail1-input').value = line.detail1Code || '';
      row.querySelector('.vf-desc-input').value = line.description || '';
      if (Number(line.debit) > 0) {
        row.querySelector('.vf-debit-input').value = formatAmount(line.debit);
      }
      if (Number(line.credit) > 0) {
        row.querySelector('.vf-credit-input').value = formatAmount(line.credit);
      }
      updateRowAccount(row);
    });
  }
  ensureTrailingBlank();
  syncVoucherRowCount();
  refreshFormState();
}

function startNew(number) {
  state.voucher = null;
  state.makePermanent = false;
  state.restoredConflict = false;
  state.overflowVoucherLines = 0;
  state.dirty = false;
  state.activeRow = null;
  $('#vfDocNo').value = number > 0 ? String(number) : '';
  $('#vfDocDate').value = state.settings.today || '';
  setDocType(VOUCHER_TYPES[0]);
  $('#vfDocDesc').value = '';
  updateDescCounter();
  fillGrid([]);
  setStatus('info', 'سند جدید — ذخیره نشده');
  requestAnimationFrame(() => $('#voucherRows .vf-code-input')?.focus());
}

function loadVoucher(voucher) {
  state.voucher = voucher;
  state.makePermanent = voucher.isTemporary === false;
  state.restoredConflict = false;
  state.dirty = false;
  state.activeRow = null;
  $('#vfDocNo').value = String(voucher.number);
  $('#vfDocDate').value = voucher.date || state.settings.today || '';
  setDocType(voucher.type);
  $('#vfDocDesc').value = voucher.description || '';
  updateDescCounter();
  fillGrid(voucher.lines || []);
  setStatus(
    voucher.isTemporary === false ? 'warn' : 'ok',
    voucher.isTemporary === false
      ? `سند شماره ${vfFa(voucher.number)} دائمی — فقط‌خواندنی`
      : `سند شماره ${vfFa(voucher.number)} ذخیره‌شده — موقت`
  );
  refreshFormState();
}

function collectDraft() {
  const lines = [];
  for (const row of gridRows()) {
    const accountCode = digitsOnly(row.querySelector('.vf-code-input')?.value || '');
    const detail1Code = digitsOnly(row.querySelector('.vf-detail1-input')?.value || '');
    const description = (row.querySelector('.vf-desc-input')?.value || '').trim();
    const debitValue = parseAmountInput(row.querySelector('.vf-debit-input')?.value || '');
    const creditValue = parseAmountInput(row.querySelector('.vf-credit-input')?.value || '');
    const debit = debitValue === null ? Number.NaN : Number(debitValue);
    const credit = creditValue === null ? Number.NaN : Number(creditValue);
    if (!accountCode && !detail1Code && !description && !debit && !credit) continue;
    const lineId = Number(row.dataset.lineId || 0);
    lines.push({ id: lineId > 0 ? lineId : null, accountCode, detail1Code: detail1Code || null, description, debit, credit });
  }

  const numberValue = Number(digitsOnly($('#vfDocNo')?.value || ''));
  return {
    id: state.voucher ? state.voucher.id : null,
    number: numberValue > 0 ? numberValue : null,
    date: toAsciiDigits($('#vfDocDate')?.value || '').replace(/-/g, '/').replace(/[^\d/]/g, ''),
    voucherType: $('#vfDocType')?.value || VOUCHER_TYPES[0],
    description: $('#vfDocDesc')?.value || '',
    isPermanent: Boolean(state.makePermanent),
    lines,
  };
}

function validateDraft(draft) {
  if (state.overflowVoucherLines) {
    return `این سند ${vfFa(state.overflowVoucherLines + MAX_VOUCHER_LINES)} ردیف دارد و به‌دلیل ظرفیت فرم قابل ذخیره نیست.`;
  }
  if (draft.number != null && (!Number.isSafeInteger(draft.number) || draft.number > MAX_VOUCHER_NUMBER)) {
    return `شماره سند باید عدد صحیح و حداکثر ${vfFa(MAX_VOUCHER_NUMBER)} باشد.`;
  }
  if (!draft.date) return 'تاریخ سند را وارد کنید.';
  if (draft.lines.length > MAX_VOUCHER_LINES) {
    return `تعداد ردیف‌های سند نمی‌تواند بیشتر از ${vfFa(MAX_VOUCHER_LINES)} باشد.`;
  }
  const dateParts = /^(\d{4})\/(\d{1,2})\/(\d{1,2})$/.exec(draft.date);
  if (!dateParts) return 'تاریخ سند باید به شکل ۱۴۰۵/۰۱/۰۱ وارد شود.';
  const year = Number(dateParts[1]);
  const month = Number(dateParts[2]);
  const day = Number(dateParts[3]);
  if (year < 1 || year > 3177 || month < 1 || month > 12 || day < 1
      || day > jalaliMonthLength(year, month)) {
    return 'سال، ماه یا روز تاریخ سند معتبر نیست.';
  }
  // The Rust/Jalali calendar remains authoritative and is still called before
  // saving; this check only avoids a round-trip for impossible dates.
  if (draft.lines.length < 2) return 'سند حسابداری باید حداقل دو ردیف داشته باشد.';

  let debit = 0n;
  let credit = 0n;
  for (let index = 0; index < draft.lines.length; index += 1) {
    const line = draft.lines[index];
    const rowNo = vfFa(index + 1);
    if (!line.accountCode) return `کد حساب ردیف ${rowNo} را وارد کنید.`;
    const account = accountForCode(line.accountCode);
    if (!account) return `حساب با کد ${vfFa(line.accountCode)} یافت نشد.`;
    const savedLine = line.id == null
      ? null
      : (state.voucher?.lines || []).find((saved) => saved.id === line.id) || null;
    const savedAccountMatches = !!savedLine
      && digitsOnly(savedLine.accountCode) === line.accountCode
      && digitsOnly(savedLine.detail1Code || '') === digitsOnly(line.detail1Code || '');
    if (!savedAccountMatches && account.isActive === false) {
      return `حساب «${account.name}» غیرفعال است و برای ردیف جدید سند قابل انتخاب نیست.`;
    }
    if (!savedAccountMatches && (![1, 2].includes(Number(account.level)) || !account.isPostable)) {
      return `حساب پایه «${account.name}» باید از سطح کل یا معینِ قابل ثبت باشد.`;
    }
    if (line.detail1Code) {
      const detail = accountForCode(line.detail1Code);
      if (!detail || Number(account.level) !== 2 || Number(detail.level) !== 3
          || digitsOnly(detail.parentCode) !== line.accountCode
          || String(detail.groupCode) !== String(account.groupCode)) {
        return `تفصیلی ردیف ${rowNo} باید فرزند مستقیم همان حساب معین باشد.`;
      }
      if (detail.isActive === false && !savedAccountMatches) {
        return `حساب تفصیلی «${detail.name}» غیرفعال است و برای ردیف جدید قابل استفاده نیست.`;
      }
    }
    if (!line.debit && !line.credit) return `مبلغ ردیف ${rowNo} را وارد کنید.`;
    if (line.debit && line.credit) return `در ردیف ${rowNo} فقط یکی از بدهکار یا بستانکار را وارد کنید.`;
    if (line.debit < 0 || line.credit < 0) return `مبلغ ردیف ${rowNo} نمی‌تواند منفی باشد.`;
    if (!Number.isSafeInteger(line.debit) || !Number.isSafeInteger(line.credit)
      || BigInt(line.debit) > MAX_AMOUNT || BigInt(line.credit) > MAX_AMOUNT) {
      return `مبلغ ردیف ${rowNo} باید عدد صحیح و حداکثر ${vfFa(formatAmount(MAX_AMOUNT))} ریال باشد.`;
    }
    debit += BigInt(line.debit);
    credit += BigInt(line.credit);
  }

  if (debit === 0n) return 'مبلغ سند باید بزرگ‌تر از صفر باشد.';
  if (debit !== credit) {
    return `سند تراز نیست: جمع بدهکار ${vfFa(formatAmount(debit))} و جمع بستانکار ${vfFa(formatAmount(credit))} است.`;
  }
  return '';
}

let saveIdempotencySignature = null;
let saveIdempotencyKey = null;

function getSaveIdempotencyKey(signature) {
  if (saveIdempotencySignature !== signature) {
    saveIdempotencySignature = signature;
    saveIdempotencyKey = null;
  }
  if (!saveIdempotencyKey) {
    const random = globalThis.crypto;
    if (random && typeof random.randomUUID === 'function') {
      saveIdempotencyKey = random.randomUUID();
    } else if (random && typeof random.getRandomValues === 'function') {
      saveIdempotencyKey = Array.from(random.getRandomValues(new Uint32Array(4)), (n) => n.toString(36)).join('');
    } else {
      saveIdempotencyKey = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
    }
  }
  return saveIdempotencyKey;
}

async function saveVoucher(options = {}) {
  if (isLockedVoucher()) {
    toast('این سند دائمی است و قابل ویرایش یا ذخیره مجدد نیست.', 'error');
    return null;
  }
  if (state.restoredConflict) {
    toast('پایگاه داده بازیابی شده است؛ برای حفظ تغییرات فرم، آن را ذخیره نکنید. پنجره را ببندید و سند را دوباره باز کنید.', 'error');
    return null;
  }
  if (state.busy) return null;
  if (!invoke) {
    toast('ذخیره‌سازی فقط در نسخه دسکتاپ ممکن است.', 'error');
    return null;
  }

  const draft = collectDraft();
  const draftSnapshot = JSON.stringify(draft);
  const problem = validateDraft(draft);
  if (problem) {
    markAllCodesFinal();
    toast(problem, 'error');
    return null;
  }

  state.busy = true;
  setStatus('info', 'در حال اعتبارسنجی و ذخیره…');
  try {
    // The Rust/Jalali calendar implementation is authoritative, including
    // leap-year validation. This command gives the form early feedback.
    const normalizedDate = await invoke('validate_voucher_date', { date: draft.date });
    // Check the original form before writing the normalized date back. Doing
    // this in the opposite order could overwrite a date typed during the
    // asynchronous validation and accidentally save the stale draft.
    if (JSON.stringify(collectDraft()) !== draftSnapshot) {
      state.busy = false;
      state.dirty = true;
      setStatus('warn', 'فرم تغییر کرده است');
      toast('اطلاعات فرم هنگام اعتبارسنجی تغییر کرد؛ لطفاً دوباره ذخیره کنید.', 'warning');
      return null;
    }
    const dateInput = $('#vfDocDate');
    if (dateInput && typeof normalizedDate === 'string') dateInput.value = normalizedDate;
    draft.date = normalizedDate;
    dateInput?.classList.remove('is-invalid');
    dateInput?.removeAttribute('aria-invalid');
    if (dateInput) dateInput.title = '';
  } catch (error) {
    state.busy = false;
    if (JSON.stringify(collectDraft()) !== draftSnapshot) state.dirty = true;
    const dateInput = $('#vfDocDate');
    dateInput?.classList.add('is-invalid');
    dateInput?.setAttribute('aria-invalid', 'true');
    if (dateInput) dateInput.title = errorText(error);
    setStatus('error', 'تاریخ نامعتبر');
    toast(errorText(error), 'error');
    return null;
  }

  let saveSnapshot = '';
  try {
    saveSnapshot = JSON.stringify(collectDraft());
    const expectedUpdatedAt = state.voucher?.updatedAt || null;
    const idempotencySignature = JSON.stringify({ draft, expectedUpdatedAt });
    const idempotencyKey = getSaveIdempotencyKey(idempotencySignature);
    const saved = await invoke('save_voucher', {
      input: draft,
      expectedUpdatedAt,
      idempotencyKey,
    });
    let committedVoucher = saved;
    const currentDraft = collectDraft();
    if (JSON.stringify(currentDraft) !== saveSnapshot) {
      state.busy = false;
      state.voucher = saved;
      if (currentDraft.number === draft.number) $('#vfDocNo').value = String(saved.number);
      state.dirty = true;
      setStatus('warn', 'تغییرات ذخیره‌نشده');
      toast('نسخه اولیه سند ذخیره شد؛ ویرایش‌های جدید حفظ شده‌اند. برای دائمی‌سازی، تغییرات باقی‌مانده را ذخیره کنید.', 'warning');
    } else {
      let finalVoucher = saved;
      if (draft.isPermanent && saved.isTemporary !== false) {
        try {
          setStatus('info', 'در حال دائمی‌کردن سند…');
          finalVoucher = await invoke('set_voucher_permanent', {
            id: saved.id,
            expectedUpdatedAt: saved.updatedAt,
          });
        } catch (finalizeError) {
          state.busy = false;
          loadVoucher(saved);
          toast(`سند ذخیره شد، اما دائمی‌سازی انجام نشد: ${errorText(finalizeError)}`, 'warning');
          return saved;
        }
      }
      state.busy = false;
      loadVoucher(finalVoucher);
    }

    if (!options.silent) {
      toast(draft.isPermanent && !state.dirty
        ? `سند شماره ${vfFa(saved.number)} ذخیره و دائمی شد.`
        : `سند شماره ${vfFa(saved.number)} ذخیره شد.`, 'ok');
    }
    try {
      await refreshNextNumber();
    } catch (refreshError) {
      state.nextNumber = Math.max(state.nextNumber || 1, Number(saved.number) + 1);
      toast(`سند ذخیره شد؛ دریافت شماره بعدی ناموفق بود: ${errorText(refreshError)}`, 'warning');
    }
    return committedVoucher;
  } catch (error) {
    state.busy = false;
    if (saveSnapshot && JSON.stringify(collectDraft()) !== saveSnapshot) state.dirty = true;
    setStatus('error', 'ذخیره ناموفق');
    toast(errorText(error), 'error');
    return null;
  }
}

function resetNumberInput() {
  const value = state.voucher ? state.voucher.number : state.nextNumber;
  const input = $('#vfDocNo');
  if (input) input.value = String(value);
}

function draftChangedSince(snapshot) {
  return JSON.stringify(collectDraft()) !== snapshot;
}

function warnDraftChangedDuringLoad() {
  state.dirty = true;
  setStatus('warn', 'تغییرات ذخیره‌نشده');
  toast('فرم هنگام بارگذاری سند تغییر کرد؛ اطلاعات حفظ شد و جابه‌جایی انجام نشد.', 'warning');
}

async function openNumber(number, options = {}) {
  if (state.busy || !invoke) return;
  const numberText = toAsciiDigits(String(number ?? '')).trim();
  const target = /^\d+$/.test(numberText) ? Number(numberText) : Number.NaN;
  if (!Number.isSafeInteger(target) || target <= 0 || target > MAX_VOUCHER_NUMBER) {
    toast('شماره سند معتبر نیست.', 'error');
    resetNumberInput();
    return;
  }

  if (state.voucher && !state.dirty && state.voucher.number === target) return;
  if (!(await guardUnsaved())) {
    resetNumberInput();
    return;
  }

  const snapshot = JSON.stringify(collectDraft());
  state.busy = true;
  try {
    const voucher = await invoke('get_voucher', { number: target });
    state.busy = false;
    if (draftChangedSince(snapshot)) {
      warnDraftChangedDuringLoad();
      return;
    }
    if (voucher) {
      loadVoucher(voucher);
      globalThis.focusVoucherDateDay?.();
      return;
    }
    await refreshNextNumber();
    if (draftChangedSince(snapshot)) {
      warnDraftChangedDuringLoad();
      return;
    }
    startNew(target);
    globalThis.focusVoucherDateDay?.();
    if (!options.quiet) {
      toast(`سند شماره ${vfFa(target)} یافت نشد؛ سند جدید با این شماره آغاز شد.`, 'info');
    }
  } catch (error) {
    state.busy = false;
    resetNumberInput();
    toast(errorText(error), 'error');
  }
}

async function navigatePrev() {
  if (state.busy || !invoke) return;
  const base = currentNumber();
  if (base <= 1) {
    toast('سند قبلی وجود ندارد.', 'info');
    return;
  }
  if (!(await guardUnsaved())) {
    resetNumberInput();
    return;
  }

  const snapshot = JSON.stringify(collectDraft());
  state.busy = true;
  try {
    const target = await invoke('adjacent_voucher_number', { number: base, direction: 'previous' });
    if (target == null) {
      state.busy = false;
      toast('سند قبلی وجود ندارد.', 'info');
      return;
    }
    const voucher = await invoke('get_voucher', { number: target });
    state.busy = false;
    if (draftChangedSince(snapshot)) {
      warnDraftChangedDuringLoad();
      return;
    }
    if (voucher) loadVoucher(voucher);
    else toast(`سند شماره ${vfFa(target)} یافت نشد.`, 'info');
  } catch (error) {
    state.busy = false;
    toast(errorText(error), 'error');
  }
}

async function navigateNext() {
  if (state.busy || !invoke) return;
  if (!(await guardUnsaved())) {
    resetNumberInput();
    return;
  }

  const snapshot = JSON.stringify(collectDraft());
  const base = currentNumber();
  state.busy = true;
  try {
    const target = await invoke('adjacent_voucher_number', { number: base, direction: 'next' });
    if (draftChangedSince(snapshot)) {
      state.busy = false;
      warnDraftChangedDuringLoad();
      return;
    }
    if (target == null) {
      await refreshNextNumber();
      if (draftChangedSince(snapshot)) {
        state.busy = false;
        warnDraftChangedDuringLoad();
        return;
      }
      state.busy = false;
      startNew(state.nextNumber);
      toast('سند بعدی وجود ندارد؛ سند جدید آغاز شد.', 'info');
      return;
    }
    const voucher = await invoke('get_voucher', { number: target });
    if (draftChangedSince(snapshot)) {
      state.busy = false;
      warnDraftChangedDuringLoad();
      return;
    }
    if (voucher) {
      state.busy = false;
      loadVoucher(voucher);
      return;
    }
    await refreshNextNumber();
    if (draftChangedSince(snapshot)) {
      state.busy = false;
      warnDraftChangedDuringLoad();
      return;
    }
    state.busy = false;
    if (target >= state.nextNumber) {
      startNew(state.nextNumber);
      toast('سند بعدی وجود ندارد؛ سند جدید آغاز شد.', 'info');
    } else {
      toast(`سند شماره ${vfFa(target)} یافت نشد.`, 'info');
    }
  } catch (error) {
    state.busy = false;
    toast(errorText(error), 'error');
  }
}

async function newVoucher() {
  if (state.busy) return;
  if (!(await guardUnsaved())) return;
  const draftSnapshot = JSON.stringify(collectDraft());
  state.busy = true;
  try {
    await refreshNextNumber();
    if (JSON.stringify(collectDraft()) !== draftSnapshot) {
      state.dirty = true;
      toast('فرم هنگام دریافت شماره تغییر کرد؛ اطلاعات حفظ شد و سند جدید باز نشد.', 'warning');
      return;
    }
    startNew(state.nextNumber);
    toast('سند جدید آماده است.', 'info');
  } catch (error) {
    toast(`دریافت شماره سند جدید ناموفق بود: ${errorText(error)}`, 'error');
  } finally {
    state.busy = false;
  }
}

async function copyVoucher() {
  if (state.busy) return;
  if (state.overflowVoucherLines) {
    toast('کپی این سند از فرم محدودشده ممکن نیست.', 'error');
    return;
  }
  const draft = collectDraft();
  const draftSnapshot = JSON.stringify(draft);
  if (!draft.lines.length) {
    toast('سندی برای کپی وجود ندارد.', 'info');
    return;
  }

  state.busy = true;
  try {
    await refreshNextNumber();
  } catch (error) {
    toast(errorText(error), 'error');
    return;
  } finally {
    state.busy = false;
  }
  if (JSON.stringify(collectDraft()) !== draftSnapshot) {
    state.dirty = true;
    toast('فرم هنگام دریافت شماره تغییر کرد؛ اطلاعات حفظ شد و کپی ساخته نشد.', 'warning');
    return;
  }

  const copy = {
    date: draft.date || state.settings.today || '',
    voucherType: draft.voucherType,
    description: draft.description,
    lines: draft.lines.map(({ id, ...line }) => ({ ...line, id: null })),
  };

  state.voucher = null;
  state.makePermanent = false;
  state.dirty = false;
  state.activeRow = null;
  $('#vfDocNo').value = String(state.nextNumber);
  $('#vfDocDate').value = copy.date;
  setDocType(copy.voucherType);
  $('#vfDocDesc').value = copy.description;
  updateDescCounter();
  fillGrid(copy.lines);
  markDirty();
  toast(`سند جدید از روی سند جاری ساخته شد (شماره ${vfFa(state.nextNumber)}).`, 'info');
  requestAnimationFrame(() => $('#voucherRows .vf-code-input')?.focus());
}

async function deleteVoucher() {
  if (isLockedVoucher()) {
    toast('سند دائمی است و قابل حذف نیست.', 'error');
    return;
  }
  if (state.busy) return;
  if (state.overflowVoucherLines) {
    toast('حذف این سند از فرم محدودشده ممکن نیست.', 'error');
    return;
  }
  const voucher = state.voucher;
  if (!voucher) {
    toast('ابتدا یک سند ذخیره‌شده را باز کنید.', 'info');
    return;
  }

  const ok = await confirmAction(
    state.dirty
      ? `سند شماره ${vfFa(voucher.number)} برای همیشه حذف شود؟ تغییرات ذخیره‌نشدهٔ فرم نیز از بین می‌رود و این کار قابل بازگشت نیست.`
      : `سند شماره ${vfFa(voucher.number)} برای همیشه حذف شود؟ این کار قابل بازگشت نیست.`,
    'حذف سند'
  );
  if (!ok) return;

  state.busy = true;
  try {
    await invoke('delete_voucher', {
      id: voucher.id,
      expectedUpdatedAt: voucher.updatedAt || null,
    });
    state.busy = false;
    // The deletion has committed. Do not report it as failed just because
    // refreshing the suggested next number has a transient error.
    try {
      await refreshNextNumber();
    } catch (refreshError) {
      state.nextNumber = Math.max(1, state.nextNumber || Number(voucher.number));
      startNew(state.nextNumber);
      toast(`سند شماره ${vfFa(voucher.number)} حذف شد. دریافت شماره بعدی ناموفق بود: ${errorText(refreshError)}`, 'warning');
      return;
    }
    startNew(state.nextNumber);
    toast(`سند شماره ${vfFa(voucher.number)} حذف شد.`, 'ok');
  } catch (error) {
    state.busy = false;
    toast(errorText(error), 'error');
  }
}

async function printVoucher() {
  if (state.busy) return;
  if (!invoke) {
    toast('چاپ فقط در نسخه دسکتاپ ممکن است.', 'error');
    return;
  }

  let voucher = state.voucher;
  if (!voucher || state.dirty) {
    voucher = await saveVoucher({ silent: true });
    if (!voucher) return;
  }

  try {
    await invoke('open_voucher_preview', { number: voucher.number });
  } catch (error) {
    toast(errorText(error), 'error');
  }
}

function checkBalance() {
  const draft = collectDraft();
  if (draft.lines.length === 0) {
    toast('سند هنوز ردیفی ندارد.', 'info');
    return;
  }
  const problem = validateDraft(draft);
  if (problem) {
    markAllCodesFinal();
    toast(problem, 'error');
    return;
  }
  toast(`سند تراز است؛ جمع بدهکار و بستانکار ${vfFa(formatAmount(totalsFor(null).debit))} ریال.`, 'ok');
}

