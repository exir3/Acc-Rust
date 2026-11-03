'use strict';

/* ============================================================
   In-app print preview window controller.
   Receives the report HTML from the Rust backend, renders it on
   an A4 sheet and offers print / save-HTML / zoom.
   ============================================================ */

const tauriCore = window.__TAURI__?.core || null;
const invoke = typeof tauriCore?.invoke === 'function'
  ? tauriCore.invoke.bind(tauriCore)
  : null;
const tauriWindow = window.__TAURI__?.window || null;

const PX_PER_MM = 96 / 25.4;
const MIN_ZOOM = 0.25;
const MAX_ZOOM = 2;

const $ = (sel) => document.querySelector(sel);

const state = {
  doc: null,
  zoom: 1,
  fit: true,
};

const faNum = (value) => String(value).replace(/[0-9]/g, (d) => '۰۱۲۳۴۵۶۷۸۹'[Number(d)]);

function setStatus(text, tone) {
  const el = $('#ppStatusText');
  if (!el) return;
  el.textContent = text;
  const dot = document.querySelector('#ppStatus .pp-status-dot');
  if (dot) dot.style.background = tone === 'error' ? '#e05a5a' : tone === 'ok' ? '#37b98a' : '#7fa8d8';
}

function showError(message) {
  $('#ppLoading').hidden = true;
  const box = $('#ppError');
  if (box) {
    $('#ppErrorText').textContent = message;
    box.hidden = false;
  }
}

function paperWidthPx() {
  return state.doc ? state.doc.paperWidthMm * PX_PER_MM : 794;
}

function fitZoom() {
  const available = Math.max(320, window.innerWidth - 56);
  return Math.max(MIN_ZOOM, Math.min(1, available / paperWidthPx()));
}

function applyZoom() {
  const sheet = $('#ppSheet');
  if (!sheet) return;
  sheet.style.setProperty('--pp-zoom', String(state.zoom));
  const label = $('#ppZoomLabel');
  if (label) label.textContent = `${faNum(Math.round(state.zoom * 100))}٪`;
}

function updatePageCount() {
  const sheet = $('#ppSheet');
  const chip = $('#ppPages');
  if (!sheet || !chip || !state.doc) return;
  const zoom = state.zoom || 1;
  const sheetH = sheet.getBoundingClientRect().height / zoom;
  const marginTop = state.doc.marginTopMm * PX_PER_MM;
  const marginBottom = state.doc.marginBottomMm * PX_PER_MM;
  const pageH = state.doc.paperHeightMm * PX_PER_MM;
  const printable = Math.max(1, pageH - marginTop - marginBottom);
  const contentH = Math.max(0, sheetH - marginTop - marginBottom);
  const pages = Math.max(1, Math.ceil(contentH / printable));
  chip.textContent = `حدود ${faNum(pages)} صفحه`;
  chip.hidden = false;
}

function renderDoc(doc) {
  state.doc = doc;

  // Report styles (page size, tables, fonts) go into the preview document so
  // the screen view and the printed output share exactly the same rules.
  document.querySelectorAll('[data-pp-report-style]').forEach((node) => node.remove());
  let parsed;
  try {
    parsed = new DOMParser().parseFromString(doc.html, 'text/html');
  } catch (_) {
    parsed = null;
  }

  if (parsed) {
    parsed.head.querySelectorAll('style').forEach((style) => {
      const clone = document.createElement('style');
      clone.setAttribute('data-pp-report-style', '1');
      clone.textContent = style.textContent;
      document.head.appendChild(clone);
    });
    parsed.head.querySelectorAll('link[rel="stylesheet"]').forEach((link) => {
      const clone = document.createElement('link');
      clone.rel = 'stylesheet';
      clone.href = link.getAttribute('href') || '';
      clone.setAttribute('data-pp-report-style', '1');
      document.head.appendChild(clone);
    });
  }

  const oldSheet = $('#ppSheet');
  if (oldSheet) oldSheet.remove();

  const sheet = document.createElement('div');
  sheet.id = 'ppSheet';
  sheet.dir = 'rtl';
  sheet.style.setProperty('--pp-w', `${doc.paperWidthMm}mm`);
  sheet.style.setProperty('--pp-h', `${doc.paperHeightMm}mm`);
  sheet.style.setProperty(
    '--pp-pad',
    `${doc.marginTopMm}mm ${doc.marginRightMm}mm ${doc.marginBottomMm}mm ${doc.marginLeftMm}mm`
  );

  const reportDoc = parsed ? parsed.body.querySelector('.report-doc') : null;
  if (reportDoc) {
    sheet.appendChild(document.importNode(reportDoc, true));
  } else if (parsed) {
    Array.from(parsed.body.childNodes).forEach((node) => sheet.appendChild(document.importNode(node, true)));
  } else {
    const fallback = document.createElement('div');
    fallback.className = 'report-doc';
    fallback.innerHTML = doc.html;
    sheet.appendChild(fallback);
    setStatus('نمایش گزارش در حالت ساده انجام شد.', 'error');
  }

  const toolbar = $('#ppToolbar');
  document.body.insertBefore(sheet, toolbar);

  document.title = `${doc.title} — پیش‌نمایش چاپ`;
  $('#ppTitle').textContent = doc.title || 'گزارش';
  $('#ppSubtitle').textContent = doc.subtitle || '';
  const orientation = doc.paperWidthMm > doc.paperHeightMm ? 'افقی' : 'عمودی';
  $('#ppPaper').textContent = `A4 ${orientation}`;

  const printBtn = $('#ppPrint');
  const saveBtn = $('#ppSave');
  if (printBtn) printBtn.disabled = false;
  if (saveBtn) saveBtn.disabled = false;

  state.fit = true;
  state.zoom = fitZoom();
  applyZoom();

  // Let the browser lay the report out before measuring page count.
  requestAnimationFrame(() => {
    updatePageCount();
    $('#ppLoading').hidden = true;
    setStatus('پیش‌نمایش آماده است. برای چاپ از دکمه «چاپ / PDF» استفاده کنید.', 'ok');
  });
}

function zoomBy(delta) {
  state.fit = false;
  state.zoom = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, Math.round((state.zoom + delta) * 100) / 100));
  applyZoom();
  requestAnimationFrame(updatePageCount);
}

function zoomFit() {
  state.fit = true;
  state.zoom = fitZoom();
  applyZoom();
  requestAnimationFrame(updatePageCount);
}

async function printReport() {
  if (!state.doc) return;
  setStatus('در حال باز کردن پنجره چاپ…');
  try {
    window.focus();
    window.print();
  } catch (e) {
    setStatus(`باز کردن پنجره چاپ ناموفق بود: ${e}`, 'error');
  }
}

async function saveHtml() {
  if (!state.doc) return;
  if (!invoke) {
    setStatus('ذخیره فایل فقط در نسخه دسکتاپ امکان‌پذیر است.', 'error');
    return;
  }
  const btn = $('#ppSave');
  if (btn) btn.disabled = true;
  setStatus('در حال ذخیره فایل HTML…');
  try {
    const saved = await invoke('save_report_html', {
      defaultName: state.doc.fileName,
      html: state.doc.html,
    });
    if (saved) {
      setStatus(`گزارش ذخیره شد: ${saved}`, 'ok');
    } else {
      setStatus('ذخیره فایل لغو شد.');
    }
  } catch (e) {
    setStatus(String(e), 'error');
  } finally {
    if (btn) btn.disabled = false;
  }
}

async function closeWindow() {
  try {
    if (tauriWindow && tauriWindow.getCurrentWindow) {
      await tauriWindow.getCurrentWindow().close();
      return;
    }
  } catch (_) { /* fall through */ }
  window.close();
}

function bindEvents() {
  $('#ppPrint').addEventListener('click', printReport);
  $('#ppSave').addEventListener('click', saveHtml);
  $('#ppClose').addEventListener('click', closeWindow);
  $('#ppErrorClose').addEventListener('click', closeWindow);
  $('#ppZoomIn').addEventListener('click', () => zoomBy(0.1));
  $('#ppZoomOut').addEventListener('click', () => zoomBy(-0.1));
  $('#ppZoomFit').addEventListener('click', zoomFit);

  window.addEventListener('resize', () => {
    if (state.fit) {
      state.zoom = fitZoom();
      applyZoom();
    }
    updatePageCount();
  });

  window.addEventListener('beforeprint', () => setStatus('در حال آماده‌سازی خروجی چاپ…'));
  window.addEventListener('afterprint', () => setStatus('پنجره چاپ بسته شد.', 'ok'));

  document.addEventListener('keydown', (ev) => {
    if (ev.ctrlKey || ev.metaKey) {
      if (ev.key === 'p' || ev.key === 'P') {
        ev.preventDefault();
        printReport();
      } else if (ev.key === '+' || ev.key === '=') {
        ev.preventDefault();
        zoomBy(0.1);
      } else if (ev.key === '-') {
        ev.preventDefault();
        zoomBy(-0.1);
      } else if (ev.key === '0') {
        ev.preventDefault();
        zoomFit();
      }
      return;
    }
    if (ev.key === 'Escape') closeWindow();
  });
}

async function init() {
  bindEvents();
  $('#ppPrint').disabled = true;
  $('#ppSave').disabled = true;

  if (!invoke) {
    showError('پیش‌نمایش چاپ فقط در نسخه دسکتاپ برنامه قابل استفاده است.');
    return;
  }

  try {
    const doc = await invoke('get_print_doc');
    renderDoc(doc);
  } catch (e) {
    showError(String(e));
  }
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', init);
} else {
  init();
}
