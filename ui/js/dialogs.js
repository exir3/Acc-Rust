'use strict';

/* ── Status, toast and modal ────────────────────────────────── */

let toastTimer = 0;

function toast(message, tone = 'info') {
  const el = $('#toast');
  if (!el) return;
  el.textContent = message;
  el.dataset.tone = tone;
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { el.hidden = true; }, 3400);
}

function setStatus(message, tone = 'ok') {
  const text = $('#statusText');
  const dot = $('#statusDot');
  if (text) text.textContent = message;
  if (dot) dot.dataset.tone = tone;
}

let dialogCloseHandler = null;
let dialogReturnFocus = null;
let dialogAppWasInert = false;
let dialogKeyHandler = null;
let dialogCanClose = null;

function closeDialog() {
  const modal = $('#modal');
  if (!modal || modal.hidden) return;
  if (dialogCanClose && !dialogCanClose()) return;
  modal.hidden = true;
  dialogCanClose = null;
  if (dialogKeyHandler) {
    modal.removeEventListener('keydown', dialogKeyHandler);
    dialogKeyHandler = null;
  }
  const app = $('.accounting-app');
  if (app) app.inert = dialogAppWasInert;
  const returnFocus = dialogReturnFocus;
  dialogReturnFocus = null;
  if (dialogCloseHandler) {
    const handler = dialogCloseHandler;
    dialogCloseHandler = null;
    handler();
  }
  if (returnFocus?.isConnected && !returnFocus.closest('[inert]') && !returnFocus.closest('[hidden]')) {
    returnFocus.focus();
  }
}

function openDialog(options) {
  const modal = $('#modal');
  const title = $('#dialogTitle');
  const body = $('#dialogBody');
  const foot = $('#dialogFoot');
  const dialog = modal ? modal.querySelector('.dialog') : null;
  if (!modal || !title || !body || !foot || !dialog) return null;

  const wasOpen = !modal.hidden;
  const returnFocus = wasOpen ? dialogReturnFocus : document.activeElement;
  dialogCanClose = null;
  closeDialog();
  dialogReturnFocus = returnFocus;
  dialog.classList.toggle('dialog-wide', Boolean(options.wide));
  title.textContent = options.title || '';
  body.innerHTML = options.body || '';
  foot.innerHTML = '';

  const result = {
    modal,
    body,
    foot,
    close: closeDialog,
    setActions(actions) {
      foot.innerHTML = '';
      actions.forEach((action) => {
        const button = document.createElement('button');
        button.type = 'button';
        button.className = `btn${action.primary ? ' btn-primary' : ''}${action.className ? ` ${action.className}` : ''}`;
        if (action.icon) button.innerHTML = icon(action.icon);
        const label = document.createElement('span');
        label.textContent = action.label;
        button.append(label);
        if (action.id) button.id = action.id;
        button.addEventListener('click', () => action.onClick?.(result, button));
        foot.append(button);
      });
      const closer = document.createElement('button');
      closer.type = 'button';
      closer.className = 'btn push-end';
      closer.textContent = 'بستن';
      closer.hidden = options.hideClose === true;
      closer.addEventListener('click', () => closeDialog());
      foot.append(closer);
    },
  };

  result.setActions(options.actions || []);
  dialogCloseHandler = options.onClose || null;
  dialogCanClose = typeof options.canClose === 'function' ? options.canClose : null;
  modal.hidden = false;
  const app = $('.accounting-app');
  dialogAppWasInert = Boolean(app?.inert);
  if (app) app.inert = true;
  dialogKeyHandler = (event) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      closeDialog();
      return;
    }
    if (event.key !== 'Tab') return;
    const focusable = Array.from(modal.querySelectorAll(
      'button:not(:disabled),input:not(:disabled),select:not(:disabled),textarea:not(:disabled),a[href],[tabindex]:not([tabindex="-1"])'
    )).filter((element) => !element.hidden && element.getClientRects().length > 0);
    if (!focusable.length) {
      event.preventDefault();
      dialog.focus();
      return;
    }
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && (document.activeElement === first || !modal.contains(document.activeElement))) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && (document.activeElement === last || !modal.contains(document.activeElement))) {
      event.preventDefault();
      first.focus();
    }
  };
  modal.addEventListener('keydown', dialogKeyHandler);
  if (options.actionButtons) result.setActions(options.actionButtons);
  const firstInput = body.querySelector('input, select, textarea');
  const firstFocusable = firstInput || modal.querySelector('button:not(:disabled),input:not(:disabled),select:not(:disabled),textarea:not(:disabled)');
  if (firstFocusable) setTimeout(() => { if (!modal.hidden) firstFocusable.focus(); }, 30);
  return result;
}

function confirmDialog(title, message, confirmLabel = 'حذف', danger = true) {
  return new Promise((resolve) => {
    const dialog = openDialog({
      title,
      body: `<div class="note"${danger ? ' data-tone="danger"' : ''}>${esc(message)}</div>`,
      hideClose: true,
      onClose: () => resolve(false),
      actions: [
        {
          label: confirmLabel,
          primary: danger,
          className: danger ? 'btn-danger' : '',
          onClick: () => {
            dialogCloseHandler = null;
            closeDialog();
            resolve(true);
          },
        },
        {
          label: 'انصراف',
          onClick: () => {
            dialogCloseHandler = null;
            closeDialog();
            resolve(false);
          },
        },
      ],
    });
  });
}

