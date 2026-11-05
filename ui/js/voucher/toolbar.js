'use strict';

/* ── Toolbar and keyboard ───────────────────────────────────── */

function bindToolbar() {
  $$('[data-action]').forEach((button) => {
    button.addEventListener('click', async () => {
      if (state.busy) return;
      switch (button.dataset.action) {
        case 'save':
          await saveVoucher();
          break;
        case 'new':
          await newVoucher();
          break;
        case 'copy':
          await openVoucherTransfer('copy');
          break;
        case 'transfer-document':
          await openVoucherTransfer('move');
          break;
        case 'print':
          await printVoucher();
          break;
        case 'add-row':
          addRow();
          break;
        case 'repeat-row':
          repeatRow();
          break;
        case 'delete-row':
          deleteRow();
          break;
        case 'ledger':
          await openAccountLedger();
          break;
        case 'excel-import':
          await importVoucherFromExcel();
          break;
        case 'excel-export':
          await exportVoucherToExcel();
          break;
        case 'balance':
          checkBalance();
          break;
        case 'saved-descriptions':
          openSavedDescriptions(state.activeRow);
          break;
        case 'delete-voucher':
          await deleteVoucher();
          break;
        case 'transfer-copy':
          await openVoucherTransfer('copy');
          break;
        case 'transfer-move':
          await openVoucherTransfer('move');
          break;
        case 'copy-row':
          await copyVoucherRow();
          break;
        case 'paste-row':
          await pasteVoucherRows();
          break;
        default:
          break;
      }
    });
  });
}

function bindWindowKeys() {
  document.addEventListener('keydown', (ev) => {
    if (confirmOpen()) return;
    if (ev.defaultPrevented) return;
    if (transferModalOpen() || excelPreviewOpen()) {
      if (ev.key === 'Escape') {
        ev.preventDefault();
        if (transferModalOpen()) closeVoucherTransfer();
        else closeExcelPreview();
      }
      return;
    }

    if (ev.key === 'Escape') {
      if (ledgerModalOpen()) {
        ev.preventDefault();
        closeAccountLedger();
        return;
      }
      // Escape only backs out of the account picker. The OS close button
      // closes the window; a stray Escape must not throw the entry away.
      if (accountPickerOpen()) {
        ev.preventDefault();
        closeAccountPicker(true);
      }
      return;
    }

    if (accountPickerOpen()) return;

    if (ledgerModalOpen()) {
      if (ev.ctrlKey && ev.key.toLowerCase() === 'p') {
        ev.preventDefault();
        printAccountLedger();
      }
      return;
    }

    const gridInput = ev.target?.closest?.('#voucherRows input');
    if (gridInput && ev.ctrlKey && !ev.altKey && !ev.metaKey) {
      const hasTextSelection = typeof gridInput.selectionStart === 'number' &&
        gridInput.selectionStart !== gridInput.selectionEnd;
      const key = ev.key.toLowerCase();
      if (key === 'c' && !hasTextSelection) {
        ev.preventDefault(); copyVoucherRow(); return;
      }
      if (key === 'v' && !hasTextSelection) {
        ev.preventDefault(); pasteVoucherRows(); return;
      }
    }

    if (ev.key === 'F2' && !accountPickerOpen()) {
      ev.preventDefault();
      saveVoucher();
      return;
    }

    if (ev.ctrlKey && !ev.altKey && !ev.metaKey) {
      const key = ev.key.toLowerCase();
      if (key === 's') {
        ev.preventDefault();
        saveVoucher();
        return;
      }
      if (key === 'n') {
        ev.preventDefault();
        newVoucher();
        return;
      }
      if (key === 'p') {
        ev.preventDefault();
        printVoucher();
        return;
      }
      if (key === 'f') {
        ev.preventDefault();
        const search = voucherSearchInput();
        if (search) {
          search.focus();
          search.select();
        }
        return;
      }
    }

    const reloadKey = ev.key === 'F5' || (ev.ctrlKey && (ev.key === 'r' || ev.key === 'R'));
    if (reloadKey && state.dirty) {
      ev.preventDefault();
      toast('برای جلوگیری از از دست رفتن اطلاعات، تازه‌سازی غیرفعال است.', 'info');
    }
  });

  window.addEventListener('beforeunload', (ev) => {
    if (state.allowClose || !state.dirty) return;
    ev.preventDefault();
    ev.returnValue = '';
  });
}

function bindCloseGuard() {
  if (!appWindow || typeof appWindow.onCloseRequested !== 'function') return;
  try {
    appWindow.onCloseRequested(async (event) => {
      if (state.allowClose || !state.dirty) return;
      event.preventDefault();
      const ok = await confirmAction('تغییرات ذخیره‌نشده از بین می‌رود. پنجره بسته شود؟', 'بستن بدون ذخیره');
      if (ok) {
        state.allowClose = true;
        await appWindow.close();
      }
    });
  } catch {
    // The close guard is a convenience; closing still works without it.
  }
}

