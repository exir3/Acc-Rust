'use strict';

/* ── Data loading ───────────────────────────────────────────── */

async function loadAccounts() {
  if (!invoke) return;
  const payload = await invoke('list_accounts');
  state.accounts = Array.isArray(payload?.accounts) ? payload.accounts : [];
  state.accountByCode = new Map(state.accounts.map((account) => [String(account.code), account]));
  if (state.ready) gridRows().forEach((row) => updateRowAccount(row));
}

async function loadSettings() {
  if (!invoke) return;
  const settings = await invoke('get_settings');
  if (settings) {
    state.settings = settings;
    applyLimits(settings.limits);
  }
}

async function refreshNextNumber() {
  if (!invoke) return;
  const value = await invoke('next_voucher_number');
  state.nextNumber = Number(value) > 0 ? Number(value) : 1;
}

let queuedVoucherRequest;
let hasQueuedVoucherRequest = false;
let handlingVoucherRequest = false;

async function drainVoucherRequest() {
  if (!hasQueuedVoucherRequest || handlingVoucherRequest || state.busy || !state.ready) return;
  const number = queuedVoucherRequest;
  queuedVoucherRequest = undefined;
  hasQueuedVoucherRequest = false;
  await handleVoucherRequest(number);
}

async function handleVoucherRequest(number) {
  if (handlingVoucherRequest || state.busy) {
    queuedVoucherRequest = number;
    hasQueuedVoucherRequest = true;
    return;
  }
  handlingVoucherRequest = true;
  try {
    if (!(await guardUnsaved())) return;
    const draftSnapshot = JSON.stringify(collectDraft());
    state.busy = true;

    if (number && Number(number) > 0) {
      const voucher = await invoke('get_voucher', { number: Number(number) });
      if (hasQueuedVoucherRequest) return;
      if (JSON.stringify(collectDraft()) !== draftSnapshot) {
        state.dirty = true;
        toast('فرم هنگام بازکردن سند تغییر کرد؛ اطلاعات حفظ شد.', 'warning');
        return;
      }
      if (voucher) loadVoucher(voucher);
      else {
        await refreshNextNumber();
        if (hasQueuedVoucherRequest) return;
        if (JSON.stringify(collectDraft()) !== draftSnapshot) {
          state.dirty = true;
          toast('فرم هنگام بازکردن سند تغییر کرد؛ اطلاعات حفظ شد.', 'warning');
          return;
        }
        startNew(Number(number));
      }
    } else {
      await refreshNextNumber();
      if (hasQueuedVoucherRequest) return;
      if (JSON.stringify(collectDraft()) !== draftSnapshot) {
        state.dirty = true;
        toast('فرم هنگام بازکردن سند تغییر کرد؛ اطلاعات حفظ شد.', 'warning');
        return;
      }
      startNew(state.nextNumber);
    }

    if (appWindow) {
      try {
        await appWindow.setFocus();
      } catch {
        // Focus is best-effort.
      }
    }
  } catch (error) {
    toast(errorText(error), 'error');
  } finally {
    state.busy = false;
    handlingVoucherRequest = false;
    // loadVoucher refreshed the form while busy was still true, which disabled
    // the mutating controls. Refresh once more now that the request is done.
    refreshFormState();
    if (hasQueuedVoucherRequest) void drainVoucherRequest();
  }
}

async function bindVoucherRequests() {
  if (!TAURI || !TAURI.event || typeof TAURI.event.listen !== 'function') return;
  try {
    await TAURI.event.listen('voucher-request', (event) => {
      const number = event && event.payload;
      if (state.busy || handlingVoucherRequest || !state.ready) {
        queuedVoucherRequest = number;
        hasQueuedVoucherRequest = true;
        return;
      }
      void handleVoucherRequest(number);
    });
  } catch {
    // Events are an optimisation; opening the window still loads a voucher.
  }
}

async function handleDatabaseRestored() {
  await loadAccounts();
  if (!state.voucher) return;
  if (state.dirty) {
    state.restoredConflict = true;
    setStatus('error', 'پایگاه داده بازیابی شده؛ ذخیره مسدود است');
    toast('پایگاه داده هنگام ویرایش این سند بازیابی شد. برای جلوگیری از بازنویسی داده‌ها، ذخیره مسدود است؛ پنجره را ببندید و سند را دوباره باز کنید.', 'error');
    return;
  }
  const voucher = await invoke('get_voucher', { number: state.voucher.number });
  if (voucher) {
    loadVoucher(voucher);
  } else {
    state.restoredConflict = true;
    setStatus('error', 'سند در پایگاه بازیابی‌شده موجود نیست');
    toast('این سند در پایگاه بازیابی‌شده وجود ندارد؛ پنجره را ببندید و سند را دوباره انتخاب کنید.', 'error');
  }
}

