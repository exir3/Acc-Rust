'use strict';

/* ── Confirmation dialog ────────────────────────────────────── */

let confirmResolve = null;
let confirmReturnFocus = null;
let confirmUnderlyingModal = null;
let confirmMainWasInert = false;
let excelReturnFocus = null;
let excelMainWasInert = false;
let transferReturnFocus = null;
let transferMainWasInert = false;

const confirmOpen = () => {
  const modal = $('#vfConfirmModal');
  return !!modal && !modal.hidden;
};

function confirmAction(message, confirmLabel = 'تأیید') {
  const modal = $('#vfConfirmModal');
  const text = $('#vfConfirmText');
  const okButton = $('#vfConfirmOk');
  if (!modal || !text || !okButton) {
    return Promise.resolve(window.confirm(message));
  }

  return new Promise((resolve) => {
    if (confirmResolve) {
      const previous = confirmResolve;
      confirmResolve = null;
      previous(false);
    }
    confirmResolve = resolve;
    confirmReturnFocus = document.activeElement;
    text.textContent = message;
    okButton.textContent = confirmLabel;
    modal.hidden = false;
    const main = $('.voucher-window');
    confirmMainWasInert = Boolean(main?.inert);
    if (main) main.inert = true;
    confirmUnderlyingModal = transferModalOpen()
      ? $('#vfTransferModal')
      : (excelPreviewOpen() ? $('#vfExcelPreviewModal') : null);
    if (confirmUnderlyingModal) confirmUnderlyingModal.inert = true;
    okButton.focus();
  });
}

function settleConfirm(value) {
  const modal = $('#vfConfirmModal');
  if (!modal || modal.hidden) return;
  modal.hidden = true;
  const main = $('.voucher-window');
  if (main) main.inert = confirmMainWasInert;
  if (confirmUnderlyingModal) confirmUnderlyingModal.inert = false;
  confirmUnderlyingModal = null;
  const resolve = confirmResolve;
  confirmResolve = null;
  if (resolve) resolve(value);
  const returnFocus = confirmReturnFocus;
  confirmReturnFocus = null;
  if (returnFocus?.isConnected && !returnFocus.closest('[inert]')) returnFocus.focus();
}

function bindConfirm() {
  const modal = $('#vfConfirmModal');
  if (!modal) return;
  $('#vfConfirmOk')?.addEventListener('click', () => settleConfirm(true));
  $('#vfConfirmCancel')?.addEventListener('click', () => settleConfirm(false));
  modal.addEventListener('click', (ev) => {
    if (ev.target.id === 'vfConfirmModal') settleConfirm(false);
  });
  modal.addEventListener('keydown', (ev) => {
    if (ev.key === 'Escape') {
      ev.preventDefault();
      settleConfirm(false);
    } else if (ev.key === 'Enter' && ev.target.tagName !== 'BUTTON') {
      ev.preventDefault();
      settleConfirm(true);
    } else if (ev.key === 'Tab') {
      const focusable = $$('#vfConfirmModal button:not([disabled])', modal);
      if (!focusable.length) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (ev.shiftKey && document.activeElement === first) {
        ev.preventDefault();
        last.focus();
      } else if (!ev.shiftKey && document.activeElement === last) {
        ev.preventDefault();
        first.focus();
      }
    }
  });
}

async function guardUnsaved() {
  if (!state.dirty) return true;
  return confirmAction('تغییرات ذخیره‌نشده این سند از بین می‌رود. ادامه می‌دهید؟', 'ادامه بدون ذخیره');
}

