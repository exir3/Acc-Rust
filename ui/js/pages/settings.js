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
  import: 'ورود گروهی',
  transfer: 'کپی/انتقال ردیف',
  reorganize: 'ادغام/بازشماره‌گذاری',
};

const SECTION_LABELS = {
  non_current_asset: 'دارایی غیرجاری',
  current_asset: 'دارایی جاری',
  equity: 'حقوق مالکانه',
  non_current_liability: 'بدهی غیرجاری',
  current_liability: 'بدهی جاری',
  revenue: 'درآمد عملیاتی',
  cogs: 'بهای تمام‌شده',
  selling_expense: 'هزینه‌های فروش',
  admin_expense: 'هزینه‌های اداری',
  other_operating: 'سایر درآمد/هزینه عملیاتی',
  finance_cost: 'هزینه‌های مالی',
  other_non_operating: 'سایر درآمد/هزینه غیرعملیاتی',
  income_tax: 'مالیات بر درآمد',
  memorandum: 'حساب‌های انتظامی',
};

// Which statement class each report section feeds. Mirrors
// `classify_activity` in src-tauri/src/vouchers/accounting.rs.
const SECTION_CLASSES = {
  current_asset: 'asset',
  non_current_asset: 'asset',
  current_liability: 'liability',
  non_current_liability: 'liability',
  equity: 'equity',
  revenue: 'income',
  cogs: 'expense',
  selling_expense: 'expense',
  admin_expense: 'expense',
  finance_cost: 'expense',
  income_tax: 'expense',
  other_operating: 'dual',
  other_non_operating: 'dual',
  memorandum: 'memorandum',
};

const GROUP_TYPE_CLASSES = {
  'دارایی': 'asset',
  'بدهی': 'liability',
  'سرمایه': 'equity',
  'درآمد': 'income',
  'هزینه': 'expense',
  'سایر': 'other',
};

PAGES.settings = {
  actions: () => [
    { label: 'ویرایش مشخصات', icon: 'i-edit', primary: true, onClick: () => openSettingsDialog() },
  ],
  load: async () => {
    const [settings, fiscalYears, auditLogs, legacy, groups] = await Promise.all([
      invoke('get_settings'),
      invoke('list_fiscal_years'),
      invoke('get_audit_logs', { limit: 100 }),
      invoke('analyze_legacy_level4').catch(() => null),
      invoke('list_account_groups').catch(() => ({ groups: [] })),
    ]);
    state.settings = settings;
    state.fiscalYears = fiscalYears.years || [];
    state.legacyLevel4 = legacy;
    state.auditLogs = auditLogs;
    state.accountGroups = groups.groups || [];
    updateBranding(settings);
    return {
      settings,
      fiscalYears: state.fiscalYears,
      auditLogs,
      legacy,
      accountGroups: state.accountGroups,
    };
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
        <button type="button" class="btn" data-backup-encrypted>${icon('i-lock')}<span>پشتیبان رمزنگاری‌شده</span></button>
        <button type="button" class="btn" data-restore-database>${icon('i-refresh')}<span>بازیابی پشتیبان</span></button>
      </div>
      <div class="form-note" style="margin-top:10px">
        برنامه ۳۰ نسخهٔ پشتیبان آخر و ۵ نسخهٔ ایمنی پیش از بازیابی را نگه می‌دارد.
        پشتیبان رمزنگاری‌شده با AES-256-GCM محافظت می‌شود و برای انتقال روی رسانهٔ قابل حمل مناسب است.
        پوشهٔ پشتیبان عادی حاوی داده مالی است؛ دسترسی آن را به حساب کاربری ویندوز محدود و روی درایو رمزنگاری‌شده نگهداری کنید.
      </div>
    </div>
  </div>
  ${fiscalYearsCard(data.fiscalYears || [])}
  ${classificationCard(data.accountGroups || state.accountGroups || [])}
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

function classificationWarnings(group) {
  const warnings = [];
  const section = group.defaultSection || '';
  const typeClass = GROUP_TYPE_CLASSES[group.type] || 'other';
  const sectionClass = section ? SECTION_CLASSES[section] : '';
  if (!section) {
    if (group.type === 'سایر') {
      warnings.push({
        tone: 'warn',
        text: 'بخش پیش‌فرض ندارد؛ طبقه‌بندی از پیشوند کد حساب حدس زده می‌شود.',
      });
    } else if (group.code === '10' || group.code === '11') {
      warnings.push({
        tone: 'warn',
        text: 'گروه با ماهیت دوگانه است و بخش پیش‌فرض ندارد؛ مانده بدهکار/بستانکار به‌طور خودکار تفسیر می‌شود.',
      });
    } else {
      warnings.push({
        tone: 'info',
        text: 'بخش پیش‌فرض تعیین نشده؛ طبقه‌بندی فقط از نوع گروه استفاده می‌کند.',
      });
    }
  } else if (typeClass !== 'other' && sectionClass !== 'dual' && sectionClass !== typeClass) {
    warnings.push({
      tone: 'error',
      text: `بخش «${SECTION_LABELS[section] || section}» با نوع گروه «${group.type}» ناسازگار است.`,
    });
  } else if (typeClass === 'other' && sectionClass === 'dual') {
    warnings.push({
      tone: 'info',
      text: 'گروه سایر با بخش عملیاتی/غیرعملیاتی؛ جهت مانده، درآمد یا هزینه بودن را تعیین می‌کند.',
    });
  }
  return warnings;
}

function classificationCard(groups) {
  const items = (groups || []).map((group) => {
    const warnings = classificationWarnings(group);
    const severity = warnings.some((item) => item.tone === 'error')
      ? 'error'
      : (warnings.some((item) => item.tone === 'warn') ? 'warn' : 'ok');
    const status = severity === 'error'
      ? badge('ناسازگار', 'error')
      : (severity === 'warn' ? badge('نیازمند بازبینی', 'warn') : badge('سازگار', 'ok'));
    const detail = warnings.length
      ? warnings.map((item) => `<div class="form-note" style="margin:2px 0">${esc(item.text)}</div>`).join('')
      : '<span class="muted">—</span>';
    return `<tr>
      <td class="mono center">${toFa(group.code)}</td>
      <td>${esc(group.name)}</td>
      <td>${esc(group.type)}</td>
      <td>${group.defaultSection ? esc(SECTION_LABELS[group.defaultSection] || group.defaultSection) : '—'}</td>
      <td class="center">${toFa(group.accountCount || 0)}</td>
      <td class="center">${status}</td>
      <td>${detail}</td>
    </tr>`;
  }).join('');
  const warnings = (groups || []).filter((group) => classificationWarnings(group)
    .some((item) => item.tone === 'error' || item.tone === 'warn')).length;

  return `
  <div class="card" style="grid-column:1/-1;margin-top:12px">
    <div class="section-head">
      <h2>${icon('i-balance')} ماتریس طبقه‌بندی صورت‌های مالی</h2>
      ${warnings ? badge(`${toFa(warnings)} گروه نیازمند بازبینی`, 'warn') : badge('طبقه‌بندی کامل', 'ok')}
    </div>
    <div class="table-wrap short">
      <table class="data-table">
        <thead><tr>
          <th style="width:80px">کد</th><th>گروه</th><th style="width:90px">نوع</th>
          <th>بخش پیش‌فرض</th><th style="width:70px">حساب</th>
          <th style="width:110px">وضعیت</th><th>توضیح</th>
        </tr></thead>
        <tbody>${items || `<tr><td colspan="7" class="center muted">گروه حسابی تعریف نشده است.</td></tr>`}</tbody>
      </table>
    </div>
    <div class="form-note">
      نوع گروه و بخش پیش‌فرض، جای هر حساب را در ترازنامه و صورت سود و زیان تعیین می‌کنند.
      گروه‌های «سایر» بدون بخش پیش‌فرض بر اساس پیشوند کد حساب طبقه‌بندی می‌شوند؛ برای کدینگ سفارشی این بخش را صریح تعیین کنید.
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
      <td class="center"><button type="button" class="btn btn-sm" data-legacy-resolve="${esc(account.code)}">تعیین مقصد</button></td>
    </tr>`).join('') : `<tr><td colspan="6" class="center muted">حساب سطح ۴ ثبت نشده است.</td></tr>`;

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
          <th style="width:70px">ردیف</th><th style="width:120px">وضعیت</th><th style="width:110px">عملیات</th>
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

function auditDiff(entry) {
  const render = (value, label) => {
    if (!value) return '';
    let text = value;
    try {
      text = JSON.stringify(JSON.parse(value), null, 2);
    } catch {
      // Keep the stored text as-is when it is not valid JSON.
    }
    if (text.length > 1800) text = `${text.slice(0, 1800)}\n…`;
    return `<div class="audit-diff-part"><span>${label}</span><pre>${esc(text)}</pre></div>`;
  };
  const before = render(entry.beforeValue, 'قبل');
  const after = render(entry.afterValue, 'بعد');
  if (!before && !after) return '—';
  return `<details class="audit-diff"><summary>مشاهده</summary>${before}${after}</details>`;
}

function auditLogCard(logs) {
  const entries = logs?.entries || [];
  const total = Number(logs?.total ?? entries.length);
  const entities = [...new Set(entries.map((entry) => entry.entityType))].sort();
  const entityOptions = entities
    .map((entity) => `<option value="${esc(entity)}">${esc(AUDIT_ENTITY_LABELS[entity] || entity)}</option>`)
    .join('');
  const rows = entries.length ? entries.map((entry) => {
    const entityLabel = AUDIT_ENTITY_LABELS[entry.entityType] || entry.entityType;
    const actionLabel = AUDIT_ACTION_LABELS[entry.action] || entry.action;
    const searchable = `${entry.createdAt} ${entityLabel} ${actionLabel} ${entry.details || ''} ${entry.beforeValue || ''} ${entry.afterValue || ''}`.slice(0, 6000);
    return `
    <tr data-audit-row data-entity="${esc(entry.entityType)}" data-search="${esc(searchable.toLowerCase())}">
      <td class="nowrap ltr">${toFa(entry.createdAt)}</td>
      <td class="nowrap">${esc(entityLabel)}</td>
      <td class="nowrap">${esc(actionLabel)}</td>
      <td class="desc-cell">${esc(entry.details || '—')}</td>
      <td class="desc-cell">${auditDiff(entry)}</td>
    </tr>`;
  }).join('') : `<tr><td colspan="5" class="center muted">هنوز تغییری ثبت نشده است.</td></tr>`;

  return `
  <div class="card" id="auditLogCard" style="grid-column:1/-1;margin-top:12px">
    <div class="section-head">
      <h2>${icon('i-doc')} تاریخچهٔ تغییرات</h2>
      <span class="badge" data-tone="info">${toFa(entries.length)} از ${toFa(total)} رویداد</span>
    </div>
    <div class="audit-filters">
      <select class="input" data-audit-entity aria-label="فیلتر موضوع">
        <option value="">همه موضوع‌ها</option>
        ${entityOptions}
      </select>
      <input class="input" type="search" data-audit-search placeholder="جستجو در جزئیات یا مقادیر..." aria-label="جستجو در تاریخچه" />
      <button type="button" class="btn btn-sm" data-audit-clear>پاک کردن فیلتر</button>
    </div>
    <div class="table-wrap short">
      <table class="data-table">
        <thead><tr>
          <th style="width:150px">زمان</th><th style="width:110px">موضوع</th>
          <th style="width:130px">عملیات</th><th>جزئیات</th><th style="width:120px">تغییرات</th>
        </tr></thead>
        <tbody>${rows}</tbody>
      </table>
    </div>
    ${total > entries.length ? `<div class="form-actions"><button type="button" class="btn btn-sm" data-audit-more>${icon('i-plus')}<span>نمایش رویدادهای بیشتر</span></button></div>` : ''}
    <div class="form-note">
      تاریخچه فقط افزودنی است و تغییر تعریف حساب‌ها، گروه‌ها، سال‌های مالی، تنظیمات، اسناد، ورود گروهی، انتقال ردیف، ادغام و بازشماره‌گذاری، بازیابی پایگاه داده و مهاجرت سطح ۴ را نگه می‌دارد. برای هر تغییر، تصویر قبل و بعد سند ذخیره می‌شود.
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
      let reason = '';
      let confirm;
      if (year.status === 'final') {
        const typed = await promptReopenYear(year);
        if (typed === null) return;
        reason = typed;
        confirm = true;
      } else {
        const confirmed = await confirmDialog(
          status === 'closed' ? 'بستن سال مالی' : 'باز کردن سال مالی',
          status === 'closed'
            ? `در سال مالی «${year.title}» پس از بستن، ثبت و تغییر اسناد مجاز نیست. ادامه می‌دهید؟`
            : `سال مالی «${year.title}» دوباره برای ثبت اسناد باز می‌شود. ادامه می‌دهید؟`,
          status === 'closed' ? 'بستن سال' : 'باز کردن سال',
          status === 'closed',
        );
        if (!confirmed) return;
        confirm = true;
      }
      button.disabled = true;
      try {
        const updated = await invoke('set_fiscal_year_status', { id, status, reason, confirm });
        if (updated?.id && Number(state.activeFiscalYear?.id) === Number(updated.id)) {
          setActiveFiscalYear({ ...state.activeFiscalYear, ...updated });
        }
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
        if (Number(state.activeFiscalYear?.id) === id) setActiveFiscalYear(null);
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
  root.querySelectorAll('[data-legacy-resolve]').forEach((button) => {
    button.addEventListener('click', () => {
      void openLegacyResolutionDialog(button.dataset.legacyResolve);
    });
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
  root.querySelector('[data-backup-encrypted]')?.addEventListener('click', async (event) => {
    const button = event.currentTarget;
    const password = await newPasswordDialog(
      'پشتیبان رمزنگاری‌شده',
      'این فایل فقط با همین رمز عبور قابل بازیابی است. رمز را در جای امن نگهداری کنید؛ در صورت فراموشی، بازیابی پشتیبان ممکن نیست.'
    );
    if (!password) return;
    button.disabled = true;
    try {
      const path = await invoke('backup_database_encrypted', { password });
      if (path) toast(`پشتیبان رمزنگاری‌شده ذخیره شد: ${path}`, 'ok');
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
    try {
      const choice = await invoke('choose_restore_backup');
      if (!choice) return;
      let password = null;
      if (choice.encrypted) {
        password = await passwordDialog(
          'رمز پشتیبان',
          `فایل «${choice.fileName}» رمزنگاری‌شده است. رمز عبور آن را وارد کنید.`
        );
        if (!password) return;
      }
      button.disabled = true;
      const safetyPath = await invoke('restore_database_file', {
        token: choice.token,
        password,
      });
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
  bindAuditControls(root);
}

function bindAuditControls(root) {
  const card = root.querySelector('#auditLogCard');
  if (!card) return;
  const entityFilter = card.querySelector('[data-audit-entity]');
  const searchInput = card.querySelector('[data-audit-search]');
  const apply = () => {
    const entity = entityFilter?.value || '';
    const query = (searchInput?.value || '').trim().toLowerCase();
    card.querySelectorAll('[data-audit-row]').forEach((row) => {
      const matchesEntity = !entity || row.dataset.entity === entity;
      const matchesQuery = !query || String(row.dataset.search || '').includes(query);
      row.hidden = !(matchesEntity && matchesQuery);
    });
  };
  entityFilter?.addEventListener('change', apply);
  searchInput?.addEventListener('input', apply);
  card.querySelector('[data-audit-clear]')?.addEventListener('click', () => {
    if (entityFilter) entityFilter.value = '';
    if (searchInput) searchInput.value = '';
    apply();
  });
  const more = card.querySelector('[data-audit-more]');
  more?.addEventListener('click', async () => {
    const current = state.auditLogs || { entries: [], total: 0 };
    const shown = (current.entries || []).length;
    more.disabled = true;
    try {
      const logs = await invoke('get_audit_logs', { limit: Math.max(shown + 100, 200) });
      state.auditLogs = logs;
      card.outerHTML = auditLogCard(logs);
      bindAuditControls(root);
    } catch (error) {
      toast(errorText(error), 'error');
      more.disabled = false;
    }
  });
}

function promptReopenYear(year) {
  return new Promise((resolve) => {
    const dialog = openDialog({
      title: 'بازگشایی سال مالی نهایی‌شده',
      body: `
        <div class="note" data-tone="warn">
          سال مالی «${esc(year.title)}» نهایی شده است. برای تغییر آن، دلیل بازگشایی را ثبت کنید.
        </div>
        <div class="field" style="margin-top:12px">
          <label for="fyReopenReason">دلیل بازگشایی <span class="req">*</span></label>
          <textarea id="fyReopenReason" class="input" rows="3" maxlength="500" placeholder="مثلاً اصلاح سند اشتباه پس از بستن سال"></textarea>
        </div>
        <div class="form-error" id="fyReopenError" hidden></div>`,
      hideClose: true,
      onClose: () => resolve(null),
      actions: [
        {
          label: 'بازگشایی سال',
          primary: true,
          onClick: () => {
            const reason = (dialog?.body.querySelector('#fyReopenReason')?.value || '').trim();
            const errorBox = dialog?.body.querySelector('#fyReopenError');
            if (!reason) {
              if (errorBox) {
                errorBox.textContent = 'ثبت دلیل بازگشایی الزامی است.';
                errorBox.hidden = false;
              }
              return;
            }
            dialogCloseHandler = null;
            closeDialog();
            resolve(reason);
          },
        },
        {
          label: 'انصراف',
          onClick: () => {
            dialogCloseHandler = null;
            closeDialog();
            resolve(null);
          },
        },
      ],
    });
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

function openFiscalYearDialog(year = null, { onSaved } = {}) {
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
        ${editing && year?.status === 'final' ? `
        <div class="field" style="grid-column:1/-1">
          <label for="fyReopenReason">دلیل بازگشایی (در صورت تغییر وضعیت از نهایی)</label>
          <textarea id="fyReopenReason" class="input" name="reopenReason" rows="3" maxlength="500" placeholder="مثلاً اصلاح سند اشتباه پس از بستن سال"></textarea>
          <div class="form-note">تا زمانی که وضعیت «نهایی» باقی بماند، ثبت دلیل لازم نیست.</div>
        </div>` : ''}
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
    const reopenReason = (form.elements.reopenReason?.value || '').trim();
    const reopeningFinal = editing && year.status === 'final' && status !== 'final';
    if (!title) {
      if (errorBox) {
        errorBox.textContent = 'عنوان سال مالی الزامی است.';
        errorBox.hidden = false;
      }
      return;
    }
    if (reopeningFinal && !reopenReason) {
      if (errorBox) {
        errorBox.textContent = 'برای تغییر وضعیت سال مالی نهایی‌شده، دلیل بازگشایی را ثبت کنید.';
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
      let saved;
      if (editing) {
        saved = await invoke('update_fiscal_year', {
          id: year.id,
          title,
          startDate,
          endDate,
          status,
          reason: reopeningFinal ? reopenReason : '',
          confirm: reopeningFinal,
        });
      } else {
        saved = await invoke('create_fiscal_year', { title, startDate, endDate, status });
      }
      closeDialog();
      toast(editing ? 'سال مالی ویرایش شد.' : 'سال مالی ایجاد شد.', 'ok');
      if (saved?.id && Number(state.activeFiscalYear?.id) === Number(saved.id)) {
        setActiveFiscalYear({ ...state.activeFiscalYear, ...saved });
      }
      if (typeof onSaved === 'function') {
        await onSaved(saved);
      } else {
        await navigate('settings', { force: true });
      }
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

async function openLegacyResolutionDialog(accountCode) {
  let accounts;
  try {
    accounts = await ensureAccounts();
  } catch (error) {
    toast(errorText(error), 'error');
    return;
  }
  const account = accounts.find((item) => item.code === accountCode);
  if (!account) {
    toast('حساب سطح ۴ انتخاب‌شده پیدا نشد.', 'error');
    return;
  }
  const activeBaseCodes = new Set(
    accounts
      .filter((item) => item.level === 2 && item.isActive !== false)
      .map((item) => item.code),
  );
  const candidates = accounts
    .filter((item) => item.level === 3
      && item.isActive !== false
      && activeBaseCodes.has(item.parentCode))
    .sort((a, b) => a.code.localeCompare(b.code));
  if (!candidates.length) {
    toast('تفصیلی ۱ فعالی زیر یک معین فعال وجود ندارد؛ ابتدا ساختار حساب را کامل کنید.', 'warn');
    return;
  }

  const options = candidates
    .map((item) => `<option value="${esc(item.code)}">${esc(item.code)} — ${esc(item.name)}</option>`)
    .join('');
  let previewed = false;

  const dialog = openDialog({
    title: 'تعیین مقصد حساب سطح ۴',
    wide: true,
    body: `
      <div class="note" data-tone="info">
        حساب سطح ۴ «${esc(account.code)} — ${esc(account.name)}» به تفصیلی ۱ انتخابی و معین والد آن متصل می‌شود. مبالغ ردیف‌ها تغییر نمی‌کنند.
      </div>
      <label class="field" style="display:block;margin-top:10px">
        <span>حساب تفصیلی ۱ مقصد</span>
        <select class="input" id="legacyResolutionTarget">${options}</select>
      </label>
      <div class="report-summary" id="legacyResolutionPreview" style="margin-top:10px" hidden></div>
      <label class="tbp-filter-check" style="display:flex;gap:7px;align-items:center;margin-top:12px">
        <input type="checkbox" id="legacyResolutionConfirm" disabled />
        از تهیه پشتیبان و تبدیل این حساب مطمئن هستم.
      </label>
      <div class="form-error" id="legacyResolutionError" hidden></div>`,
    actions: [
      {
        id: 'legacyResolutionPreviewButton',
        label: 'پیش‌نمایش',
        onClick: async (_result, button) => {
          const target = dialog?.body.querySelector('#legacyResolutionTarget')?.value || '';
          const errorBox = dialog?.body.querySelector('#legacyResolutionError');
          const previewBox = dialog?.body.querySelector('#legacyResolutionPreview');
          const confirm = dialog?.body.querySelector('#legacyResolutionConfirm');
          button.disabled = true;
          if (errorBox) errorBox.hidden = true;
          try {
            const preview = await invoke('preview_legacy_level4_resolution', {
              accountCode,
              detail1Code: target,
            });
            previewed = true;
            if (previewBox) {
              previewBox.hidden = false;
              // eslint-disable-next-line no-unsanitized/property -- preview values are formatted numbers
              previewBox.innerHTML = `
                <div class="kpi"><span>ردیف‌های متأثر</span><strong>${toFa(preview.lineCount)}</strong></div>
                <div class="kpi"><span>اسناد</span><strong>${toFa(preview.voucherCount)}</strong></div>
                <div class="kpi"><span>جمع بدهکار</span><strong>${toFa(preview.totalDebit)}</strong></div>
                <div class="kpi"><span>جمع بستانکار</span><strong>${toFa(preview.totalCredit)}</strong></div>
                <div class="kpi"><span>مانده</span><strong>${toFa(preview.balance)}</strong></div>
                <div class="kpi"><span>مقصد</span><strong>${esc(preview.targetAccountCode)} / ${esc(preview.targetDetail1Code)}</strong></div>`;
            }
            if (confirm) {
              confirm.checked = false;
              confirm.disabled = false;
            }
            const run = dialog?.foot.querySelector('#legacyResolutionRun');
            if (run) run.disabled = true;
          } catch (error) {
            previewed = false;
            if (errorBox) {
              errorBox.textContent = errorText(error);
              errorBox.hidden = false;
            }
          } finally {
            button.disabled = false;
          }
        },
      },
      {
        id: 'legacyResolutionRun',
        label: 'اجرای تبدیل',
        primary: true,
        onClick: async (_result, button) => {
          const target = dialog?.body.querySelector('#legacyResolutionTarget')?.value || '';
          const confirm = dialog?.body.querySelector('#legacyResolutionConfirm');
          const errorBox = dialog?.body.querySelector('#legacyResolutionError');
          if (!previewed || !confirm?.checked) return;
          button.disabled = true;
          if (errorBox) errorBox.hidden = true;
          try {
            const result = await invoke('resolve_legacy_level4_account', {
              accountCode,
              detail1Code: target,
              confirm: true,
            });
            closeDialog();
            toast(
              `تبدیل انجام شد: ${toFa(result.convertedLines)} ردیف، ${toFa(result.pendingLines)} ردیف باقی‌مانده. پشتیبان: ${result.backupPath}`,
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
  dialog.foot.querySelector('#legacyResolutionRun').disabled = true;
  dialog.body.querySelector('#legacyResolutionTarget')?.addEventListener('change', () => {
    previewed = false;
    const previewBox = dialog.body.querySelector('#legacyResolutionPreview');
    if (previewBox) previewBox.hidden = true;
    const confirm = dialog.body.querySelector('#legacyResolutionConfirm');
    if (confirm) {
      confirm.checked = false;
      confirm.disabled = true;
    }
    dialog.foot.querySelector('#legacyResolutionRun').disabled = true;
  });
  dialog.body.querySelector('#legacyResolutionConfirm')?.addEventListener('change', (event) => {
    dialog.foot.querySelector('#legacyResolutionRun').disabled = !(previewed && event.currentTarget.checked);
  });
}
