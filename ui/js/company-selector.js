'use strict';

/* ============================================================
   انتخاب سال مالی — دروازهٔ ورود برنامه
   ============================================================ */

const CSF_STATUS_META = {
  open: { label: 'باز', tone: 'ok' },
  closed: { label: 'بسته', tone: 'warn' },
  final: { label: 'نهایی', tone: 'info' },
};

const csfState = {
  years: [],
  selectedId: null,
  query: '',
  loading: false,
  busy: false,
  bound: false,
  emptyMode: 'none',
  previousTitle: '',
  returnPage: 'workspace',
};

function csfEl(id) {
  return document.getElementById(id);
}

function csfDelay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function csfStatusMeta(status) {
  return CSF_STATUS_META[status] || CSF_STATUS_META.open;
}

function csfSelectedYear() {
  if (csfState.selectedId == null) return null;
  return csfState.years.find((year) => Number(year.id) === Number(csfState.selectedId)) || null;
}

function csfById(id) {
  return csfState.years.find((year) => Number(year.id) === Number(id)) || null;
}

function csfDateInRange(date, start, end) {
  const value = toEn(date || '');
  if (!value) return false;
  return toEn(start || '') <= value && value <= toEn(end || '');
}

function csfApplyWallpaper() {
  const selector = csfEl('companySelector');
  if (!selector) return;
  const wallpaper = storageGet('hesabdari-workspace-wallpaper');
  if (wallpaper) {
    selector.style.backgroundImage = `linear-gradient(rgba(2, 6, 23, .42), rgba(2, 6, 23, .42)), url("${wallpaper}")`;
  } else {
    selector.style.removeProperty('background-image');
  }
}

function csfSetNotice(message, tone = 'info') {
  const notice = csfEl('csfNotice');
  if (!notice) return;
  notice.textContent = message || '';
  notice.dataset.tone = tone;
  notice.hidden = !message;
}

function csfClearNotice() {
  csfSetNotice('', 'info');
}

/* ── Selection ──────────────────────────────────────────────── */

function csfResolveSelection(preserve) {
  const years = csfState.years;
  if (!years.length) return null;

  if (preserve) {
    const kept = csfById(csfState.selectedId) || csfById(state.activeFiscalYear?.id);
    if (kept) return Number(kept.id);
  }

  const stored = Number(storageGet(ACTIVE_FISCAL_YEAR_KEY) || 0);
  if (stored && csfById(stored)) return stored;

  const active = csfById(state.activeFiscalYear?.id);
  if (active) return Number(active.id);

  const today = toEn(state.settings?.today || '');
  if (today) {
    const containing = years.find((year) => csfDateInRange(today, year.startDate, year.endDate));
    if (containing) return Number(containing.id);
  }

  const open = [...years].reverse().find((year) => year.status === 'open');
  if (open) return Number(open.id);

  return Number(years[years.length - 1].id);
}

function csfSelectYear(id, { focus = false } = {}) {
  const year = csfById(id);
  if (!year) return;
  csfState.selectedId = Number(year.id);
  const list = csfEl('csfYearList');
  if (list) {
    list.querySelectorAll('[data-csf-year]').forEach((row) => {
      const selected = Number(row.dataset.csfYear) === Number(year.id);
      row.classList.toggle('selected', selected);
      row.setAttribute('aria-selected', selected ? 'true' : 'false');
      if (selected && focus) row.focus();
    });
  }
  csfUpdateActions();
  csfUpdateSelectionNote();
}

function csfUpdateSelectionNote() {
  const note = csfEl('csfSelectionNote');
  if (!note) return;
  const year = csfSelectedYear();
  if (!year) {
    note.dataset.tone = 'neutral';
    note.textContent = 'برای ورود به برنامه، یک سال مالی را انتخاب کنید. فقط سال مالی «باز» اجازه ثبت سند دارد.';
    return;
  }
  const meta = csfStatusMeta(year.status);
  const suffix = year.status === 'open'
    ? 'ثبت و ویرایش سند در این سال مجاز است.'
    : year.status === 'closed'
      ? 'این سال بسته است؛ ثبت و تغییر سند در آن مجاز نیست.'
      : 'این سال نهایی شده است؛ تغییر اسناد فقط با بازگشایی ثبت‌شده امکان‌پذیر است.';
  note.dataset.tone = year.status === 'open' ? 'ok' : 'warn';
  note.textContent = `سال مالی «${year.title}» — ${meta.label}. ${suffix}`;
}

/* ── Actions state ──────────────────────────────────────────── */

function csfUpdateActions() {
  const busy = csfState.busy || csfState.loading;
  const year = csfSelectedYear();
  const hasYear = Boolean(year);

  const set = (id, disabled, title = '') => {
    const button = csfEl(id);
    if (!button) return;
    button.disabled = disabled;
    if (title) button.title = title;
    else button.removeAttribute('title');
  };

  set('csfEnter', busy || !hasYear);
  set('csfNewYear', busy);
  set('csfEditYear', busy || !hasYear);
  set('csfDeleteYear', busy || !hasYear || Number(year?.voucherCount || 0) > 0,
    hasYear && Number(year.voucherCount || 0) > 0 ? 'سال مالی دارای سند قابل حذف نیست.' : '');
  set('csfBackup', busy);
  set('csfRestore', busy);
  set('csfRefresh', busy);
}

async function csfRunBusy(button, busyLabel, task) {
  if (csfState.busy) return;
  const label = button?.querySelector('.csf-tbtn-label');
  const original = label?.textContent || '';
  csfState.busy = true;
  button?.classList.add('is-busy');
  if (label) label.textContent = busyLabel;
  csfUpdateActions();
  try {
    await task();
  } catch (error) {
    const message = errorText(error);
    csfSetNotice(message, 'error');
    toast(message, 'error');
  } finally {
    csfState.busy = false;
    button?.classList.remove('is-busy');
    if (label) label.textContent = original;
    csfUpdateActions();
  }
}

/* ── Rendering ──────────────────────────────────────────────── */

function csfRenderRows() {
  const list = csfEl('csfYearList');
  if (!list) return;
  const today = toEn(state.settings?.today || '');

  list.innerHTML = csfState.years.map((year) => {
    const selected = Number(year.id) === Number(csfState.selectedId);
    const meta = csfStatusMeta(year.status);
    const isCurrent = today && csfDateInRange(today, year.startDate, year.endDate);
    const search = normalizeSearch([
      year.title,
      year.startDate,
      year.endDate,
      meta.label,
      `${toFa(year.voucherCount || 0)} سند`,
    ].join(' '));

    return `<li role="none">
      <button type="button" class="csf-row${selected ? ' selected' : ''}" role="option"
        aria-selected="${selected ? 'true' : 'false'}" data-csf-year="${Number(year.id)}"
        data-search="${esc(search)}"
        title="${esc(`${year.title} — ${faDate(year.startDate)} تا ${faDate(year.endDate)}`)}">
        <span class="csf-row-title">
          <span class="csf-row-avatar"><svg class="ico" aria-hidden="true"><use href="#i-calendar" /></svg></span>
          <span class="csf-row-name">${esc(year.title)}${isCurrent ? '<span class="csf-row-current">جاری</span>' : ''}</span>
        </span>
        <span class="csf-row-range">
          <span dir="ltr">${faDate(year.startDate)}</span>
          <span class="csf-range-sep">تا</span>
          <span class="csf-range-end" dir="ltr">${faDate(year.endDate)}</span>
        </span>
        <span><span class="csf-badge" data-tone="${meta.tone}">${meta.label}</span></span>
        <span class="csf-row-count">${toFa(year.voucherCount || 0)}</span>
        <span class="csf-row-check"><svg class="ico" aria-hidden="true"><use href="#i-check" /></svg></span>
      </button>
    </li>`;
  }).join('');
}

function csfSetEmptyState(mode, { title, text, action } = {}) {
  const empty = csfEl('csfEmpty');
  const table = csfEl('csfTable');
  const titleEl = csfEl('csfEmptyTitle');
  const textEl = csfEl('csfEmptyText');
  const actionEl = csfEl('csfEmptyAction');
  if (!empty || !table) return;

  csfState.emptyMode = mode;
  const show = mode !== 'none';
  empty.hidden = !show;
  table.hidden = show;
  if (!show) return;

  if (titleEl) titleEl.textContent = title || '';
  if (textEl) textEl.textContent = text || '';
  if (actionEl) {
    actionEl.hidden = !action;
    if (action) {
      actionEl.querySelector('span').textContent = action;
      actionEl.disabled = csfState.loading || csfState.busy;
    }
  }
}

function csfApplyFilter() {
  const input = csfEl('csfSearch');
  const clear = csfEl('csfSearchClear');
  const count = csfEl('csfCount');
  const list = csfEl('csfYearList');
  const query = normalizeSearch(input?.value || '');
  csfState.query = input?.value || '';

  if (clear) clear.hidden = !input?.value;
  if (count) {
    count.textContent = csfState.years.length
      ? query
        ? `${toFa(0)} مورد`
        : `${toFa(csfState.years.length)} سال مالی`
      : '';
  }

  let visible = 0;
  list?.querySelectorAll('[data-csf-year]').forEach((row) => {
    const match = !query || String(row.dataset.search || '').includes(query);
    const item = row.closest('li');
    if (item) item.hidden = !match;
    if (match) visible += 1;
  });

  if (count) {
    count.textContent = csfState.years.length
      ? query
        ? `${toFa(visible)} از ${toFa(csfState.years.length)} سال مالی`
        : `${toFa(csfState.years.length)} سال مالی`
      : '';
  }

  if (csfState.loading && !csfState.years.length) {
    csfSetEmptyState('loading', {
      title: 'در حال دریافت سال‌های مالی…',
      text: 'اطلاعات شرکت و سال‌های مالی از پایگاه داده خوانده می‌شود.',
    });
    return;
  }

  if (!csfState.years.length) {
    csfSetEmptyState('none-years', {
      title: 'هنوز سال مالی تعریف نشده است',
      text: 'برای شروع، نخستین سال مالی را تعریف کنید.',
      action: 'تعریف سال مالی',
    });
    return;
  }

  if (!visible) {
    csfSetEmptyState('no-results', {
      title: 'نتیجه‌ای برای جست‌وجو پیدا نشد',
      text: 'عبارت جست‌وجو را تغییر دهید یا جست‌وجو را پاک کنید.',
      action: 'پاک کردن جست‌وجو',
    });
    return;
  }

  csfSetEmptyState('none');
}

function csfRender() {
  const settings = state.settings || {};
  const nameEl = csfEl('csfCompanyName');
  const metaEl = csfEl('csfCompanyMeta');

  if (nameEl) nameEl.textContent = settings.companyName || 'واحد اقتصادی';
  if (metaEl) {
    const defaultYear = String(settings.fiscalYear || '').trim();
    const parts = [];
    if (defaultYear) parts.push(`سال مالی پیش‌فرض: ${toFa(defaultYear)}`);
    parts.push(csfState.years.length
      ? `${toFa(csfState.years.length)} سال مالی تعریف‌شده`
      : 'بدون سال مالی تعریف‌شده');
    metaEl.textContent = parts.join(' · ');
  }

  csfRenderRows();
  csfUpdateActions();
  csfUpdateSelectionNote();
  csfApplyFilter();
}

/* ── Loading ────────────────────────────────────────────────── */

async function refreshCompanySelector({ preserveSelection = true, focus = false } = {}) {
  if (!invoke || csfState.loading) return;
  csfState.loading = true;
  csfUpdateActions();
  if (!csfState.years.length) csfApplyFilter();

  try {
    const [settings, report] = await Promise.all([
      invoke('get_settings'),
      invoke('list_fiscal_years'),
    ]);
    state.settings = settings;
    applyLimits(settings?.limits);
    state.fiscalYears = Array.isArray(report?.years) ? report.years : [];
    csfState.years = state.fiscalYears;

    if (state.activeFiscalYear && !csfById(state.activeFiscalYear.id)) {
      setActiveFiscalYear(null);
    }
    csfState.selectedId = csfResolveSelection(preserveSelection);
    updateBranding(state.settings);
    csfClearNotice();
    csfState.loading = false;
    csfRender();

    if (focus) {
      const row = csfEl('csfYearList')?.querySelector('.csf-row.selected');
      (row || csfEl('csfSearch'))?.focus();
    }
  } catch (error) {
    csfState.loading = false;
    csfRender();
    csfSetNotice(`دریافت اطلاعات شرکت و سال مالی ناموفق بود: ${errorText(error)}`, 'error');
  } finally {
    csfState.loading = false;
    csfUpdateActions();
  }
}

/* ── Entry ──────────────────────────────────────────────────── */

async function closeCompanySelector({ animate = true } = {}) {
  const selector = csfEl('companySelector');
  if (!selector || selector.hidden) return;

  if (animate && !window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
    selector.classList.add('is-leaving');
    await csfDelay(200);
  }

  selector.hidden = true;
  selector.classList.remove('is-open', 'is-leaving');
  csfClearNotice();
  const app = $('.accounting-app');
  if (app) app.inert = false;
  document.title = csfState.previousTitle || 'نرم‌افزار حسابداری';
}

async function enterCsfProgram() {
  const year = csfSelectedYear();
  if (!year || csfState.busy) return;

  await csfRunBusy(csfEl('csfEnter'), 'در حال ورود…', async () => {
    setActiveFiscalYear(year);
    updateBranding(state.settings);
    state.filters = defaultFilters();
    await closeCompanySelector({ animate: true });
    await navigate(IS_TRIAL_POPUP ? 'trial' : IS_VOUCHER_LIST_POPUP ? 'vouchers' : (csfState.returnPage || 'workspace'), { force: true });
    setStatus(`سال مالی «${year.title}» انتخاب شد.`, 'ok');
  });
}

async function openCompanySelector({ startup = false } = {}) {
  const selector = csfEl('companySelector');
  if (!selector) return;

  if (!selector.hidden) {
    await refreshCompanySelector({ preserveSelection: true, focus: true });
    return;
  }

  csfState.previousTitle = document.title;
  csfState.startup = startup;
  csfState.returnPage = startup ? 'workspace' : (state.page || 'workspace');
  document.body.classList.remove('is-booting');
  csfApplyWallpaper();
  selector.hidden = false;
  selector.classList.remove('is-leaving');
  // Restart the entrance animation on every open.
  void selector.offsetWidth;
  selector.classList.add('is-open');

  const app = $('.accounting-app');
  if (app) app.inert = true;
  document.title = 'انتخاب سال مالی شرکت جاری — نرم‌افزار حسابداری';

  bindCompanySelector();
  await refreshCompanySelector({ preserveSelection: false, focus: true });
}

/* ── Fiscal-year actions ────────────────────────────────────── */

async function csfCreateYear() {
  if (csfState.busy) return;
  openFiscalYearDialog(null, {
    onSaved: async (saved) => {
      await refreshCompanySelector({ preserveSelection: false });
      if (saved?.id) {
        csfSelectYear(saved.id, { focus: true });
        csfSetNotice(`سال مالی «${saved.title}» ایجاد شد.`, 'ok');
      }
    },
  });
}

function csfEditYear() {
  if (csfState.busy) return;
  const year = csfSelectedYear();
  if (!year) return;
  openFiscalYearDialog(year, {
    onSaved: async (saved) => {
      await refreshCompanySelector({ preserveSelection: false });
      if (saved?.id) {
        csfSelectYear(saved.id, { focus: true });
        csfSetNotice(`سال مالی «${saved.title}» ذخیره شد.`, 'ok');
      }
    },
  });
}

function csfDeleteYear() {
  const year = csfSelectedYear();
  if (!year || csfState.busy) return;
  void (async () => {
    const confirmed = await confirmDialog(
      'حذف سال مالی',
      `سال مالی «${year.title}» حذف می‌شود. فقط سال‌های بدون سند قابل حذف هستند. ادامه می‌دهید؟`,
    );
    if (!confirmed) return;

    await csfRunBusy(csfEl('csfDeleteYear'), 'در حال حذف…', async () => {
      await invoke('delete_fiscal_year', { id: year.id });
      csfState.selectedId = null;
      await refreshCompanySelector({ preserveSelection: false });
      csfSetNotice(`سال مالی «${year.title}» حذف شد.`, 'ok');
      toast('سال مالی حذف شد.', 'ok');
    });
  })();
}

function csfBackup() {
  void csfRunBusy(csfEl('csfBackup'), 'در حال تهیه…', async () => {
    const path = await invoke('backup_database');
    if (!path) return;
    csfSetNotice(`نسخهٔ پشتیبان ذخیره شد: ${path}`, 'ok');
    toast('نسخهٔ پشتیبان با موفقیت تهیه شد.', 'ok');
  });
}

function csfRestore() {
  void (async () => {
    if (csfState.busy) return;
    const confirmed = await confirmDialog(
      'بازیابی پشتیبان',
      'اطلاعات فعلی با فایل پشتیبان جایگزین می‌شود. پیش از بازیابی یک نسخهٔ ایمنی خودکار ساخته خواهد شد. ادامه می‌دهید؟',
      'بازیابی',
      true,
    );
    if (!confirmed) return;

    await csfRunBusy(csfEl('csfRestore'), 'در حال بازیابی…', async () => {
      const safetyPath = await invoke('restore_database');
      if (!safetyPath) return;
      setActiveFiscalYear(null);
      state.accountsLoaded = false;
      state.groupsLoaded = false;
      await loadSettings();
      state.voucherCount = await invoke('get_voucher_count').catch(() => 0);
      await refreshCompanySelector({ preserveSelection: false });
      state.filters = defaultFilters();
      csfSetNotice(`بازیابی انجام شد؛ نسخهٔ قبلی در ${safetyPath} نگهداری شد.`, 'ok');
      toast('بازیابی پشتیبان انجام شد.', 'ok');
    });
  })();
}

/* ── Filter ─────────────────────────────────────────────────── */

function csfClearSearch() {
  const input = csfEl('csfSearch');
  if (!input) return;
  input.value = '';
  csfApplyFilter();
  input.focus();
}

function csfEnterFirstVisible() {
  const rows = Array.from(csfEl('csfYearList')?.querySelectorAll('[data-csf-year]') || [])
    .filter((row) => !row.closest('li')?.hidden);
  if (!rows.length) return;
  const target = rows.find((row) => row.classList.contains('selected')) || rows[0];
  csfSelectYear(target.dataset.csfYear);
  void enterCsfProgram();
}

/* ── Bindings ───────────────────────────────────────────────── */

function bindCompanySelector() {
  if (csfState.bound) return;
  csfState.bound = true;

  csfEl('csfEnter')?.addEventListener('click', () => void enterCsfProgram());
  csfEl('csfNewYear')?.addEventListener('click', () => void csfCreateYear());
  csfEl('csfEditYear')?.addEventListener('click', () => csfEditYear());
  csfEl('csfDeleteYear')?.addEventListener('click', () => csfDeleteYear());
  csfEl('csfBackup')?.addEventListener('click', () => csfBackup());
  csfEl('csfRestore')?.addEventListener('click', () => csfRestore());
  csfEl('csfRefresh')?.addEventListener('click', () => {
    void csfRunBusy(csfEl('csfRefresh'), 'در حال تازه‌سازی…', async () => {
      await refreshCompanySelector({ preserveSelection: true });
      if (!csfEl('csfNotice')?.hidden) return;
      setStatus('فهرست سال‌های مالی تازه‌سازی شد.', 'ok');
    });
  });

  csfEl('csfSearchClear')?.addEventListener('click', csfClearSearch);
  csfEl('csfSearch')?.addEventListener('input', csfApplyFilter);
  csfEl('csfSearch')?.addEventListener('keydown', (event) => {
    if (event.key !== 'Enter' || event.isComposing || event.repeat) return;
    event.preventDefault();
    csfEnterFirstVisible();
  });

  csfEl('csfEmptyAction')?.addEventListener('click', () => {
    if (csfState.emptyMode === 'no-results') csfClearSearch();
    else if (csfState.emptyMode === 'none-years') void csfCreateYear();
    else void refreshCompanySelector({ preserveSelection: true });
  });

  const list = csfEl('csfYearList');
  list?.addEventListener('click', (event) => {
    const row = event.target.closest('[data-csf-year]');
    if (!row) return;
    csfSelectYear(row.dataset.csfYear);
  });
  list?.addEventListener('dblclick', (event) => {
    const row = event.target.closest('[data-csf-year]');
    if (!row) return;
    csfSelectYear(row.dataset.csfYear);
    void enterCsfProgram();
  });
  list?.addEventListener('keydown', (event) => {
    const row = event.target.closest('[data-csf-year]');
    if (!row) return;
    const rows = Array.from(list.querySelectorAll('[data-csf-year]:not([disabled])'))
      .filter((item) => !item.closest('li')?.hidden);
    const index = rows.indexOf(row);

    if (event.key === 'ArrowDown' || event.key === 'ArrowUp' || event.key === 'Home' || event.key === 'End') {
      if (!rows.length) return;
      event.preventDefault();
      const next = event.key === 'Home'
        ? 0
        : event.key === 'End'
          ? rows.length - 1
          : (index + (event.key === 'ArrowDown' ? 1 : -1) + rows.length) % rows.length;
      rows[next]?.focus();
      return;
    }

    if (event.key !== 'Enter' || event.repeat) return;
    event.preventDefault();
    if (row.classList.contains('selected')) void enterCsfProgram();
    else csfSelectYear(row.dataset.csfYear);
  });
}

/* ── Startup gate ───────────────────────────────────────────── */

function revealPopupsWithoutSelector() {
  document.body.classList.remove('is-booting');
}
