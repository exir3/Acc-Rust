'use strict';

/* ── Navigation ─────────────────────────────────────────────── */

let navSeq = 0;

async function navigate(page, { force = false } = {}) {
  // Outside the dedicated trial-balance window, this report opens as a popup
  // instead of replacing the page inside the main window.
  if (page === 'trial' && !IS_TRIAL_POPUP) {
    openTrialBalanceWindow();
    return;
  }
  if (page === 'workspace') {
    navSeq += 1;
    state.page = page;
    $('.accounting-app')?.classList.add('is-workspace-home');
    renderActions([]);
    const workspaceRoot = $('#page');
    if (workspaceRoot) workspaceRoot.replaceChildren();
    document.title = 'نرم‌افزار حسابداری';
    setStatus('آماده', 'ok');
    return;
  }
  // The voucher list is a popup window too; only its own window renders it.
  if (page === 'vouchers' && !IS_VOUCHER_LIST_POPUP) {
    openVoucherListWindow();
    return;
  }
  const def = PAGES[page];
  if (!def) return;
  $('.accounting-app')?.classList.remove('is-workspace-home');
  if (state.page !== page) {
    state.listOffsets.vouchers = 0;
    state.listOffsets.journal = 0;
    state.listOffsets.ledger = 0;
  }
  state.page = page;

  const [title, section] = pageMeta(page);
  const titleEl = $('#pageTitle');
  const crumbs = $('#crumbs');
  if (titleEl) titleEl.textContent = title;
  if (crumbs) crumbs.innerHTML = `حسابداری / ${esc(section)} / <b>${esc(title)}</b>`;
  document.title = `${title} — نرم‌افزار حسابداری`;

  renderActions(def.actions ? def.actions() : []);
  const root = $('#page');
  if (!root) return;
  const seq = ++navSeq;
  root.innerHTML = `<div class="empty-state">
    <div class="spinner" aria-hidden="true" style="width:26px;height:26px"></div>
    <b>در حال بارگذاری…</b>
    <span>${esc(title)}</span>
  </div>`;

  try {
    const data = await def.load(force);
    if (seq !== navSeq || state.page !== page) return;
    // eslint-disable-next-line no-unsanitized/property -- page renderers escape their data
    root.innerHTML = def.render(data);
    bindPageCommon(root);
    def.bind?.(root, data);
    setStatus(`صفحه «${title}» آماده است.`, 'ok');
    // Callers that report success to the user (for example the trial balance
    // «generate» button) must wait for this: it is true only after the data
    // really loaded and rendered, never when the load failed or went stale.
    return true;
  } catch (error) {
    if (seq !== navSeq || state.page !== page) return;
    root.innerHTML = `<div class="card">
      <div class="empty-state">
        ${icon('i-warning')}
        <b>بارگذاری «${esc(title)}» ناموفق بود</b>
        <span>${esc(errorText(error))}</span>
        <button type="button" class="btn btn-primary" data-retry>${icon('i-refresh')}<span>تلاش دوباره</span></button>
      </div>
    </div>`;
    root.querySelector('[data-retry]')?.addEventListener('click', () => navigate(page, { force: true }));
    setStatus('خطا در بارگذاری اطلاعات.', 'error');
  }
}

