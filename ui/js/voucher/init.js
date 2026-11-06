'use strict';

/* ── Init ───────────────────────────────────────────────────── */

async function init() {
  bindVoucherGrid();
  bindAccountPicker();
  bindVoucherGridSearch();
  bindVoucherTransfer();
  bindExcelPreview();
  bindConfirm();
  bindSavedDescriptions();
  bindDocMeta();
  bindVoucherSearch();
  bindToolbar();
  bindAccountLedger();
  bindWindowKeys();
  bindCloseGuard();
  updateDescCounter();

  if (!invoke) {
    setStatus('error', 'نسخه دسکتاپ در دسترس نیست');
    toast('این فرم فقط در برنامه دسکتاپ کار می‌کند.', 'error');
    $$('[data-action]').forEach((button) => {
      if (['save', 'new', 'copy', 'print', 'delete-voucher', 'ledger', 'excel-import', 'excel-export'].includes(button.dataset.action)) {
        button.disabled = true;
      }
    });
    return;
  }

  setStatus('info', 'در حال بارگذاری…');
  state.busy = true;
  try {
    await bindVoucherRequests();
    if (typeof TAURI.event?.listen === 'function') {
      await TAURI.event.listen('accounts-changed', () => {
        loadAccounts().catch(() => {});
      });
      await TAURI.event.listen('database-restored', () => {
        handleDatabaseRestored().catch((error) => toast(errorText(error), 'error'));
      });
      await TAURI.event.listen('fiscal-year-changed', (event) => {
        handleFiscalYearChanged(event?.payload ?? null);
      });
    }
    await Promise.all([loadSettings(), loadAccounts(), refreshNextNumber()]);
    const requested = await invoke('take_voucher_request');
    state.ready = true;

    if (requested && Number(requested) > 0) {
      const voucher = await invoke('get_voucher', { number: Number(requested) });
      if (voucher) {
        loadVoucher(voucher);
      } else {
        startNew(Number(requested));
        toast(`سند شماره ${vfFa(requested)} یافت نشد؛ سند جدید با این شماره آغاز شد.`, 'info');
      }
    } else {
      startNew(state.nextNumber);
    }

    state.busy = false;
    await drainVoucherRequest();

    window.addEventListener('focus', () => {
      if (!state.ready) return;
      loadAccounts().catch(() => {});
    });
  } catch (error) {
    state.busy = false;
    setStatus('error', 'بارگذاری ناموفق');
    toast(errorText(error), 'error');
    startNew(state.nextNumber);
  }

  syncVoucherRowCount();
  refreshFormState();
}

document.addEventListener('DOMContentLoaded', init);
