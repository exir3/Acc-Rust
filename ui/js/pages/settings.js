'use strict';

/* ── Settings ───────────────────────────────────────────────── */

const FISCAL_YEAR_STATUSES = [
  { value: 'open', label: 'باز', tone: 'ok' },
  { value: 'closed', label: 'بسته', tone: 'warn' },
  { value: 'final', label: 'نهایی', tone: 'info' },
];

const AUDIT_ENTITY_LABELS = {
  account: 'حساب',
  account_group: 'گروه حساب',
  voucher: 'سند',
  fiscal_year: 'سال مالی',
  settings: 'تنظیمات',
  database: 'پایگاه داده',
  legacy_level4: 'حساب سطح ۴',
};

const AUDIT_ACTION_LABELS = {
  create: 'ایجاد',
  update: 'ویرایش',
  delete: 'حذف',
  status: 'تغییر وضعیت',
  finalize: 'نهایی‌سازی',
  restore: 'بازیابی',
  migrate: 'مهاجرت',
};

PAGES.settings = {
  actions: () => [
    { label: 'ویرایش مشخصات', icon: 'i-edit', primary: true, onClick: () => openSettingsDialog() },
  ],
  load: async () => {
    const [settings, fiscalYears, auditLogs, legacy] = await Promise.all([
      invoke('get_settings'),
      invoke('list_fiscal_years'),
      invoke('get_audit_logs', { limit: 40 }),
      invoke('analyze_legacy_level4').catch(() => null),
    ]);
    state.settings = settings;
    state.fiscalYears = fiscalYears.years || [];
    state.legacyLevel4 = legacy;
    updateBranding(settings);
    return { settings, fiscalYears: state.fiscalYears, auditLogs, legacy };
  },
  render: renderSettings,
  bind: bindSettings,
};

function renderSettings(data) {
  const settings = data.settings || state.settings || {};
  const today = settings.today || '';
  return `
  <div class="settings-grid">
    <div class="card">
      <div class="section-head">
        <h2>${icon('i-settings')} مشخصات واحد اقتصادی</h2>
      </div>
      <dl class="info-list">
        <div><dt>نام شرکت / واحد اقتصادی</dt><dd>${esc(settings.companyName || '—')}</dd></div>
        <div><dt>سال مالی</dt><dd>${esc(settings.fiscalYear || '—')}</dd></div>
      </dl>
      <div class="form-actions">
        <button type="button" class="btn btn-primary" data-edit-settings>${icon('i-edit')}<span>ویرایش مشخصات</span></button>
      </div>
      <div class="form-note">
        نام شرکت در سرصفحه همه گزارش‌ها و در نوار وضعیت نمایش داده می‌شود.
        سال مالی به گزارش‌های چاپی اضافه می‌شود.
      </div>
    </div>

    <div class="card">
      <div class="section-head">
        <h2>${icon('i-info')} اطلاعات برنامه</h2>
      </div>
      <dl class="info-list">
        <div><dt>نام برنامه</dt><dd>نرم‌افزار حسابداری</dd></div>
        <div><dt>نسخه</dt><dd class="ltr">${APP_VERSION}</dd></div>
        <div><dt>تاریخ امروز</dt><dd class="ltr">${faDate(today)}</dd></div>
        <div><dt>تعداد حساب‌ها</dt><dd>${toFa(state.accountsLoaded ? state.accounts.length : '—')}</dd></div>
        <div><dt>تعداد اسناد</dt><dd>${toFa(state.voucherCount || 0)}</dd></div>
        <div><dt>مسیر پایگاه داده</dt><dd class="ltr" style="font-size:10px;max-width:260px;word-break:break-all">${esc(state.databasePath || '—')}</dd></div>
      </dl>
      <div class="form-note" style="margin-top:14px">
        همه اطلاعات به‌صورت محلی و آفلاین در پایگاه داده SQLite ذخیره می‌شود.
      </div>
      <div class="section-head">
        <h2>${icon('i-save')} پشتیبان‌گیری</h2>
      </div>
      <p class="form-note">بازیابی، اطلاعات فعلی را جایگزین می‌کند و پیش از آن یک نسخهٔ ایمنی نگه می‌دارد.</p>
      <div class="form-actions">
        <button type="button" class="btn" data-backup-database>${icon('i-save')}<span>تهیه پشتیبان</span></button>
        <button type="button" class="btn" data-restore-database>${icon('i-refresh')}<span>بازیابی پشتیبان</span></button>
      </div>
    </div>
  </div>
  ${fiscalYearsCard(data.fiscalYears || [])}
  ${legacyLevel4Card(data.legacy)}
  ${auditLogCard(data.auditLogs)}`;
}

function fiscalYearStatusBadge(status) {
  const meta = FISCAL_YEAR_STATUSES.find((item) => item.value === status) || FISCAL_YEAR_STATUSES[0];
  return badge(meta.label, meta.tone);
}

function fiscalYearsCard(years) {
  const rows = years.length ? years.map((year) => {
    const actionable = year.status === 'open' ? 'closed' : 'open';
    const actionTitle = actionable === 'closed' ? 'بستن سال مالی' : 'باز کردن سال مالی';
    return `<tr>
      <td>${esc(year.title)}</td>
      <td class="nowrap ltr">${faDate(year.startDate)}</td>
      <td class="nowrap ltr">${faDate(year.endDate)}</td>
      <td class="center">${fiscalYearStatusBadge(year.status)}</td>
      <td class="center">${toFa(year.voucherCount)}</td>
      <td class="center nowrap">
        <button type="button" class="btn btn-sm" data-edit-fiscal-year="${year.id}" title="ویرایش سال مالی">${icon('i-edit')}</button>
        <button type="button" class="btn btn-sm" data-toggle-fiscal-year="${year.id}" data-status="${actionable}" title="${actionTitle}">${icon('i-refresh')}</button>
        <button type="button" class="btn btn-sm" data-delete-fiscal-year="${year.id}" title="حذف سال مالی">${icon('i-trash')}</button>
      </td>
    </tr>`;
  }).join('') : `<tr><td colspan="6" class="center muted">هنوز سال مالی تعریف نشده است.</td></tr>`;

  return `
  <div class="card" style="grid-column:1/-1;margin-top:12px">
    <div class="section-head">
      <h2>${icon('i-calendar')} سال‌های مالی</h2>
      <button type="button" class="btn btn-primary btn-sm" data-add-fiscal-year>${icon('i-plus')}<span>سال مالی جدید</span></button>
    </div>
    <div class="table-wrap short">
      <table class="data-table">
        <thead><tr>
          <th>عنوان</th><th style="width:110px">شروع</th><th style="width:110px">پایان</th>
          <th style="width:84px">وضعیت</th><th style="width:64px">اسناد</th><th style="width:132px">عملیات</th>
        </tr></thead>
        <tbody>${rows}</tbody>
      </table>
    </div>
    <div class="form-note">
      هر سند به سال مالی تاریخ خود متصل می‌شود. در سال بسته یا نهایی، ثبت، ویرایش، نهایی‌سازی و حذف سند مجاز نیست.
    </div>
  </div>`;
}

function legacyLevel4Card(legacy) {
  if (!legacy || !legacy.accountCount) return '';
  const accounts = legacy.accounts || [];
  const rows = accounts.length ? accounts.map((account) => `
    <tr>
      <td class="mono center">${toFa(account.code)}</td>
      <td>${esc(account.name)}</td>
      <td class="mono center">${toFa(account.parentCode) || '—'}</td>
      <td class="center">${toFa(account.lineCount)}</td>
      <td class="center">${account.convertible ? badge('قابل تبدیل', 'ok') : badge('نیازمند بررسی', 'warn')}</td>
    </tr>`).join('') : `<tr><td colspan="5" class="center muted">حساب سطح ۴ ثبت نشده است.</td></tr>`;

  return `
  <div class="card" style="grid-column:1/-1;margin-top:12px">
    <div class="section-head">
      <h2>${icon('i-warning')} حساب‌های سطح ۴ باقی‌مانده</h2>
    </div>
    <div class="report-summary">
      <div class="kpi"><span>حساب سطح ۴</span><strong>${toFa(legacy.accountCount)}</strong></div>
      <div class="kpi"><span>ردیف سند</span><strong>${toFa(legacy.lineCount)}</strong></div>
      <div class="kpi"><span>ردیف قابل تبدیل</span><strong>${toFa(legacy.convertibleLineCount)}</strong></div>
      <div class="kpi" data-tone="${legacy.ambiguousLineCount ? 'warn' : 'ok'}"><span>ردیف مبهم</span><strong>${toFa(legacy.ambiguousLineCount)}</strong></div>
    </div>
    <div class="table-wrap short">
      <table class="data-table">
        <thead><tr>
          <th style="width:120px">کد</th><th>عنوان</th><th style="width:120px">حساب والد</th>
          <th style="width:70px">ردیف</th><th style="width:120px">وضعیت</th>
        </tr></thead>
        <tbody>${rows}</tbody>
      </table>
    </div>
    <p class="form-note">
      داده‌های سطح ۴ به‌صورت دست‌نخورده نگهداری می‌شوند. مهاجرت ابتدا یک پشتیبان معتبر می‌سازد و سپس ردیف‌های قابل تبدیل را به تفصیلی ۱ و معین تبدیل می‌کند؛ ردیف‌های مبهم بدون تغییر می‌مانند.
    </p>
    <div class="form-actions">
      <button type="button" class="btn btn-primary" data-legacy-migrate>${icon('i-refresh')}<span>بررسی و مهاجرت</span></button>
    </div>
  </div>`;
}

function auditLogCard(logs) {
  const entries = logs?.entries || [];
  const rows = entries.length ? entries.map((entry) => `
    <tr>
      <td class="nowrap ltr">${toFa(entry.createdAt)}</td>
      <td class="nowrap">${esc(AUDIT_ENTITY_LABELS[entry.entityType] || entry.entityType)}</td>
      <td class="nowrap">${esc(AUDIT_ACTION_LABELS[entry.action] || entry.action)}</td>
      <td class="desc-cell">${esc(entry.details || '—')}</td>
    </tr>`).join('') : `<tr><td colspan="4" class="center muted">هنوز تغییری ثبت نشده است.</td></tr>`;

  return `
  <div class="card" style="grid-column:1/-1;margin-top:12px">
    <div class="section-head">
      <h2>${icon('i-doc')} تاریخچهٔ تغییرات</h2>
      <span class="badge" data-tone="info">${toFa(logs?.total ?? entries.length)} رویداد</span>
    </div>
    <div class="table-wrap short">
      <table class="data-table">
        <thead><tr>
          <th style="width:150px">زمان</th><th style="width:110px">موضوع</th>
          <th style="width:110px">عملیات</th><th>جزئیات</th>
        </tr></thead>
        <tbody>${rows}</tbody>
      </table>
    </div>
    <div class="form-note">
      تاریخچه فقط افزودنی است و تغییر تعریف حساب‌ها، گروه‌ها، سال‌های مالی، تنظیمات، اسناد، بازیابی پایگاه داده و مهاجرت سطح ۴ را نگه می‌دارد.
    </div>
  </div>`;
}

function bindSettings(root, data) {
  root.querySelector('[data-edit-settings]')?.addEventListener('click', () => openSettingsDialog());
  root.querySelector('[data-add-fiscal-year]')?.addEventListener('click', () => openFiscalYearDialog(null));
  root.querySelectorAll('[data-edit-fiscal-year]').forEach((button) => {
    button.addEventListener('click', () => {
      const id = Number(button.dataset.editFiscalYear);
      const year = (state.fiscalYears || []).find((item) => item.id === id);
      if (year) openFiscalYearDialog(year);
    });
  });
  root.querySelectorAll('[data-toggle-fiscal-year]').forEach((button) => {
    button.addEventListener('click', async () => {
      const id = Number(button.dataset.toggleFiscalYear);
      const status = button.dataset.status === 'closed' ? 'closed' : 'open';
      const year = (state.fiscalYears || []).find((item) => item.id === id);
      if (!year) return;
      const confirmed = await confirmDialog(
        status === 'closed' ? 'بستن سال مالی' : 'باز کردن سال مالی',
        status === 'closed'
          ? `در سال مالی «${year.title}» پس از بستن، ثبت و تغییر اسناد مجاز نیست. ادامه می‌دهید؟`
          : `سال مالی «${year.title}» دوباره برای ثبت اسناد باز می‌شود. ادامه می‌دهید؟`,
        status === 'closed' ? 'بستن سال' : 'باز کردن سال',
        status === 'closed',
      );
      if (!confirmed) return;
      button.disabled = true;
      try {
        await invoke('set_fiscal_year_status', { id, status });
        toast(status === 'closed' ? 'سال مالی بسته شد.' : 'سال مالی باز شد.', 'ok');
        await navigate('settings', { force: true });
      } catch (error) {
        toast(errorText(error), 'error');
        button.disabled = false;
      }
    });
  });
  root.querySelectorAll('[data-delete-fiscal-year]').forEach((button) => {
    button.addEventListener('click', async () => {
      const id = Number(button.dataset.deleteFiscalYear);
      const year = (state.fiscalYears || []).find((item) => item.id === id);
      if (!year) return;
      const confirmed = await confirmDialog(
        'حذف سال مالی',
        `سال مالی «${year.title}» حذف می‌شود. فقط سال‌های بدون سند قابل حذف هستند. ادامه می‌دهید؟`,
      );
      if (!confirmed) return;
      button.disabled = true;
      try {
        await invoke('delete_fiscal_year', { id });
        toast('سال مالی حذف شد.', 'ok');
        await navigate('settings', { force: true });
      } catch (error) {
        toast(errorText(error), 'error');
        button.disabled = false;
      }
    });
  });
  root.querySelector('[data-legacy-migrate]')?.addEventListener('click', () => {
    openLegacyMigrationDialog(data?.legacy || state.legacyLevel4);
  });
  root.querySelector('[data-backup-database]')?.addEventListener('click', async (event) => {
    const button = event.currentTarget;
    button.disabled = true;
    try {
      const path = await invoke('backup_database');
      if (path) toast(`پشتیبان ذخیره شد: ${path}`, 'ok');
    } catch (error) {
      toast(errorText(error), 'error');
    } finally {
      button.disabled = false;
    }
  });
  root.querySelector('[data-restore-database]')?.addEventListener('click', async (event) => {
    // `currentTarget` is cleared once this handler awaits, so keep the
    // button reference before opening the confirmation dialog.
    const button = event.currentTarget;
    const confirmed = await confirmDialog(
      'بازیابی پشتیبان',
      'اطلاعات فعلی با فایل پشتیبان جایگزین می‌شود. پیش از بازیابی یک نسخهٔ ایمنی خودکار ساخته خواهد شد. ادامه می‌دهید؟'
    );
    if (!confirmed) return;
    button.disabled = true;
    try {
      const safetyPath = await invoke('restore_database');
      if (!safetyPath) return;
      await Promise.all([ensureAccounts(true), ensureGroups(true)]);
      state.voucherCount = await invoke('get_voucher_count');
      await navigate('settings', { force: true });
      toast(`بازیابی انجام شد؛ نسخهٔ قبلی در ${safetyPath} نگهداری شد.`, 'ok');
    } catch (error) {
      toast(errorText(error), 'error');
    } finally {
      button.disabled = false;
    }
  });
}

function openSettingsDialog() {
  const settings = state.settings || {};
  const dialog = openDialog({
    title: 'ویرایش مشخصات واحد اقتصادی',
    body: `
      <form id="settingsForm" class="settings-form" autocomplete="off">
        <div class="field">
          <label for="setCompany">نام شرکت / واحد اقتصادی <span class="req">*</span></label>
          <input id="setCompany" class="input" name="companyName" type="text" value="${esc(settings.companyName)}" placeholder="نام شرکت" />
        </div>
        <div class="field">
          <label for="setFiscal">سال مالی</label>
          <input id="setFiscal" class="input" name="fiscalYear" type="text" value="${esc(settings.fiscalYear)}" placeholder="۱۴۰۵" />
        </div>
        <div class="form-error" id="settingsError" hidden></div>
        <div class="form-actions">
          <button class="btn btn-primary" type="submit">${icon('i-save')}<span>ذخیره تنظیمات</span></button>
        </div>
      </form>`,
  });
  const form = dialog?.body.querySelector('#settingsForm');
  if (!form) return;
  form.addEventListener('submit', async (event) => {
    event.preventDefault();
    const errorBox = $('#settingsError', form);
    const companyName = (form.elements.companyName?.value || '').trim();
    const fiscalYear = (form.elements.fiscalYear?.value || '').trim();
    if (!companyName) {
      if (errorBox) {
        errorBox.textContent = 'نام شرکت الزامی است.';
        errorBox.hidden = false;
      }
      return;
    }
    const submit = form.querySelector('[type="submit"]');
    if (submit) submit.disabled = true;
    if (errorBox) errorBox.hidden = true;
    try {
      const saved = await invoke('save_settings', { companyName, fiscalYear });
      state.settings = saved;
      updateBranding(saved);
      closeDialog();
      toast('تنظیمات ذخیره شد.', 'ok');
      await navigate('settings', { force: true });
    } catch (error) {
      if (errorBox) {
        errorBox.textContent = errorText(error);
        errorBox.hidden = false;
      }
      if (submit) submit.disabled = false;
    }
  });
}

function openFiscalYearDialog(year = null) {
  const editing = Boolean(year);
  const lockedRange = editing && Number(year.voucherCount) > 0;
  const dialog = openDialog({
    title: editing ? 'ویرایش سال مالی' : 'تعریف سال مالی جدید',
    body: `
      <form id="fiscalYearForm" class="settings-form" autocomplete="off">
        <div class="field" style="grid-column:1/-1">
          <label for="fyTitle">عنوان سال مالی <span class="req">*</span></label>
          <input id="fyTitle" class="input" name="title" type="text" value="${esc(year?.title || '')}" placeholder="سال مالی ۱۴۰۵" />
        </div>
        <div class="field">
          <label for="fyStart">تاریخ شروع <span class="req">*</span></label>
          <input id="fyStart" class="input" name="startDate" data-date type="text" inputmode="numeric" value="${esc(year?.startDate || '')}" placeholder="۱۴۰۵/۰۱/۰۱"${lockedRange ? ' disabled' : ''} />
        </div>
        <div class="field">
          <label for="fyEnd">تاریخ پایان <span class="req">*</span></label>
          <input id="fyEnd" class="input" name="endDate" data-date type="text" inputmode="numeric" value="${esc(year?.endDate || '')}" placeholder="۱۴۰۵/۱۲/۲۹"${lockedRange ? ' disabled' : ''} />
        </div>
        <div class="field">
          <label for="fyStatus">وضعیت</label>
          <select id="fyStatus" class="input" name="status">
            ${FISCAL_YEAR_STATUSES.map((status) => `<option value="${status.value}"${status.value === (year?.status || 'open') ? ' selected' : ''}>${status.label}</option>`).join('')}
          </select>
        </div>
        ${lockedRange ? '<div class="form-note" style="grid-column:1/-1">این سال مالی سند ثبت‌شده دارد؛ بازه تاریخ آن قابل تغییر نیست. فقط عنوان یا وضعیت را می‌توانید تغییر دهید.</div>' : ''}
        <div class="form-error" id="fiscalYearError" hidden></div>
        <div class="form-actions">
          <button class="btn btn-primary" type="submit">${icon('i-save')}<span>${editing ? 'ذخیره تغییرات' : 'ایجاد سال مالی'}</span></button>
        </div>
      </form>`,
  });
  const form = dialog?.body.querySelector('#fiscalYearForm');
  if (!form) return;
  form.addEventListener('submit', async (event) => {
    event.preventDefault();
    const errorBox = $('#fiscalYearError', form);
    const title = (form.elements.title?.value || '').trim();
    const startDate = lockedRange ? year.startDate : (form.elements.startDate?.value || '').trim();
    const endDate = lockedRange ? year.endDate : (form.elements.endDate?.value || '').trim();
    const status = form.elements.status?.value || 'open';
    if (!title) {
      if (errorBox) {
        errorBox.textContent = 'عنوان سال مالی الزامی است.';
        errorBox.hidden = false;
      }
      return;
    }
    if (!lockedRange && (!startDate || !isValidJalaliDate(startDate) || !endDate || !isValidJalaliDate(endDate))) {
      if (errorBox) {
        errorBox.textContent = 'تاریخ شروع و پایان سال مالی باید معتبر باشند.';
        errorBox.hidden = false;
      }
      return;
    }
    const submit = form.querySelector('[type="submit"]');
    if (submit) submit.disabled = true;
    if (errorBox) errorBox.hidden = true;
    try {
      if (editing) {
        await invoke('update_fiscal_year', { id: year.id, title, startDate, endDate, status });
      } else {
        await invoke('create_fiscal_year', { title, startDate, endDate, status });
      }
      closeDialog();
      toast(editing ? 'سال مالی ویرایش شد.' : 'سال مالی ایجاد شد.', 'ok');
      await navigate('settings', { force: true });
    } catch (error) {
      if (errorBox) {
        errorBox.textContent = errorText(error);
        errorBox.hidden = false;
      }
      if (submit) submit.disabled = false;
    }
  });
}

function openLegacyMigrationDialog(analysis) {
  if (!analysis) return;
  const accounts = analysis.accounts || [];
  const rows = accounts.length ? accounts.map((account) => `
    <tr>
      <td class="mono center">${toFa(account.code)}</td>
      <td>${esc(account.name)}</td>
      <td class="center">${toFa(account.lineCount)}</td>
      <td class="center">${account.convertible ? badge('قابل تبدیل', 'ok') : badge('نیازمند بررسی', 'warn')}</td>
    </tr>`).join('') : `<tr><td colspan="4" class="center muted">حساب سطح ۴ وجود ندارد.</td></tr>`;

  const dialog = openDialog({
    title: 'مهاجرت حساب‌های سطح ۴',
    wide: true,
    body: `
      <div class="note" data-tone="warn">
        پیش از مهاجرت یک پشتیبان معتبر ساخته می‌شود. ردیف‌های قابل تبدیل به تفصیلی ۱ و معین تبدیل می‌شوند و ردیف‌های مبهم دست‌نخورده می‌مانند.
      </div>
      <div class="report-summary" style="margin-top:10px">
        <div class="kpi"><span>حساب سطح ۴</span><strong>${toFa(analysis.accountCount)}</strong></div>
        <div class="kpi"><span>ردیف قابل تبدیل</span><strong>${toFa(analysis.convertibleLineCount)}</strong></div>
        <div class="kpi" data-tone="${analysis.ambiguousLineCount ? 'warn' : 'ok'}"><span>ردیف مبهم</span><strong>${toFa(analysis.ambiguousLineCount)}</strong></div>
      </div>
      <div class="table-wrap short" style="margin-top:10px">
        <table class="data-table">
          <thead><tr><th style="width:120px">کد</th><th>عنوان</th><th style="width:70px">ردیف</th><th style="width:120px">وضعیت</th></tr></thead>
          <tbody>${rows}</tbody>
        </table>
      </div>
      <label class="tbp-filter-check" style="display:flex;gap:7px;align-items:center;margin-top:12px">
        <input type="checkbox" id="legacyConfirm" />
        از تهیه پشتیبان و اجرای مهاجرت مطمئن هستم.
      </label>
      <div class="form-error" id="legacyError" hidden></div>`,
    actions: [
      {
        id: 'legacyRun',
        label: 'اجرای مهاجرت',
        primary: true,
        onClick: async (_result, button) => {
          const confirm = dialog?.body.querySelector('#legacyConfirm');
          const errorBox = dialog?.body.querySelector('#legacyError');
          if (!confirm?.checked) return;
          button.disabled = true;
          if (errorBox) errorBox.hidden = true;
          try {
            const result = await invoke('apply_legacy_level4_migration', { confirm: true });
            closeDialog();
            toast(
              `مهاجرت انجام شد: ${toFa(result.convertedLines)} ردیف تبدیل، ${toFa(result.pendingLines)} ردیف در انتظار، ${toFa(result.removedAccounts)} حساب حذف شد.`,
              'ok',
            );
            await Promise.all([ensureAccounts(true), ensureGroups(true)]);
            await navigate('settings', { force: true });
          } catch (error) {
            if (errorBox) {
              errorBox.textContent = errorText(error);
              errorBox.hidden = false;
            }
            button.disabled = false;
          }
        },
      },
    ],
  });
  if (!dialog) return;
  dialog.foot.querySelector('#legacyRun').disabled = true;
  dialog.body.querySelector('#legacyConfirm')?.addEventListener('change', (event) => {
    dialog.foot.querySelector('#legacyRun').disabled = !event.currentTarget.checked;
  });
}
