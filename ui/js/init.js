'use strict';

/* ── Init ───────────────────────────────────────────────────── */

async function init() {
  if (!IS_TRIAL_POPUP && !IS_VOUCHER_LIST_POPUP) {
    buildMenubar();
    bindMenubar();
  }
  bindChrome();

  if (!invoke) {
    renderFatal('این صفحه فقط داخل برنامه دسکتاپ نرم‌افزار حسابداری کار می‌کند.');
    return;
  }

  try {
    setStatus('در حال آماده‌سازی پایگاه داده…', 'busy');
    await loadSettings();
    state.filters = defaultFilters();
    state.ready = true;
    applyWallpaper(localStorage.getItem('hesabdari-workspace-wallpaper'));
    await navigate(IS_TRIAL_POPUP ? 'trial' : IS_VOUCHER_LIST_POPUP ? 'vouchers' : 'workspace');
    setStatus('آماده', 'ok');
  } catch (error) {
    renderFatal(errorText(error));
  }
}

function renderFatal(message) {
  const root = $('#page');
  if (!root) return;
  root.innerHTML = `<div class="card">
    <div class="empty-state">
      ${icon('i-warning')}
      <b>برنامه آماده نیست</b>
      <span>${esc(message)}</span>
      <button type="button" class="btn btn-primary" data-retry>${icon('i-refresh')}<span>تلاش دوباره</span></button>
    </div>
  </div>`;
  root.querySelector('[data-retry]')?.addEventListener('click', () => window.location.reload());
  setStatus('خطا در راه‌اندازی برنامه.', 'error');
}

document.addEventListener('DOMContentLoaded', init);
