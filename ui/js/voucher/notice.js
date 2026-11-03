'use strict';

/* ── Small in-window notice ─────────────────────────────────── */

let toastTimer = 0;

function toast(message, tone = 'info') {
  const el = $('#vfToast');
  if (!el) return;
  el.textContent = message;
  el.dataset.tone = tone;
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { el.hidden = true; }, 3600);
}

function setStatus(tone, text) {
  const box = $('#vfDocStatus');
  const label = $('#vfDocStatusText');
  if (box) box.dataset.tone = tone;
  if (label) label.textContent = text;
}

