'use strict';

/* ── Settings ───────────────────────────────────────────────── */

PAGES.settings = {
  actions: () => [
    { label: 'ویرایش مشخصات', icon: 'i-edit', primary: true, onClick: () => openSettingsDialog() },
  ],
  load: async () => {
    const settings = await invoke('get_settings');
    state.settings = settings;
    updateBranding(settings);
    return settings;
  },
  render: renderSettings,
  bind: bindSettings,
};

function renderSettings(settings) {
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
  </div>`;
}

function bindSettings(root) {
  root.querySelector('[data-edit-settings]')?.addEventListener('click', () => openSettingsDialog());
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

