'use strict';

/* ── Main menu bar ─────────────────────────────────────────── */

let openMenuIndex = null;
let menuOpenedByHover = false;

function buildMenubar() {
  const nav = $('#menubarNav');
  if (!nav) return;

  // eslint-disable-next-line no-unsanitized/property -- menu labels and icons are escaped/static
  nav.innerHTML = MENU_BAR.map((menu, menuIndex) => {
    const menuItems = menu.items.map((item, itemIndex) => {
      const pending = !item.page && !item.command;
      return `
        <button type="button" class="menu-item" role="menuitem"
          data-item-index="${itemIndex}"
          ${pending ? 'title="این فرم هنوز ساخته نشده است"' : ''}>
          <span class="menu-item-icon" aria-hidden="true">${esc(item.icon || '•')}</span>
          <span class="menu-item-label">${esc(item.label)}</span>
        </button>`;
    }).join('');

    const body = menuItems || `
      <div class="menu-empty" role="status">
        ${esc(menu.empty || 'برای این بخش هنوز گزینه‌ای تعریف نشده است')}
      </div>`;

    return `
      <div class="menu-wrap" data-menu-index="${menuIndex}">
        <button type="button" class="menu-root" aria-haspopup="menu" aria-expanded="false"
          data-menu-index="${menuIndex}">
          ${esc(menu.label)}
        </button>
        <div class="menu-dropdown" role="menu" aria-label="${esc(menu.label)}"
          data-size="${esc(menu.size || 'wide')}" hidden>
          <div class="menu-list">${body}</div>
        </div>
      </div>`;
  }).join('');
}

function openMenubarMenu(index, { fromHover = false } = {}) {
  $$('#menubarNav .menu-wrap').forEach((wrap, wrapIndex) => {
    const isOpen = wrapIndex === index;
    wrap.classList.toggle('is-open', isOpen);
    const trigger = $('.menu-root', wrap);
    if (trigger) trigger.setAttribute('aria-expanded', isOpen ? 'true' : 'false');
    const dropdown = $('.menu-dropdown', wrap);
    if (dropdown) dropdown.hidden = !isOpen;
  });
  openMenuIndex = index;
  menuOpenedByHover = fromHover;
}

function closeMenubarMenus() {
  $$('#menubarNav .menu-wrap').forEach((wrap) => {
    wrap.classList.remove('is-open');
    const trigger = $('.menu-root', wrap);
    if (trigger) trigger.setAttribute('aria-expanded', 'false');
    const dropdown = $('.menu-dropdown', wrap);
    if (dropdown) dropdown.hidden = true;
  });
  openMenuIndex = null;
  menuOpenedByHover = false;
}

function notifyPendingMenu(label) {
  toast(`«${label}» هنوز آماده نیست و در نسخه‌های بعدی اضافه می‌شود.`, 'info');
  setStatus('این بخش در دست ساخت است.', 'idle');
}

function openReorganizeVouchers() {
  if (!invoke) {
    toast('مرتب‌سازی و ادغام اسناد فقط در نسخه دسکتاپ ممکن است.', 'error');
    return;
  }

  const operations = {
    'merge-then-sort': 'ابتدا ادغام، سپس مرتب‌سازی',
    merge: 'فقط ادغام اسناد عادی هم‌تاریخ',
    sort: 'فقط مرتب‌سازی بر اساس تاریخ',
  };
  const dialog = openDialog({
    title: 'مرتب‌سازی و ادغام اسناد',
    wide: true,
    body: `
      <div class="form-grid">
        <label class="field">
          <span>عملیات</span>
          <select class="input" id="reorganizeOperation">
            ${Object.entries(operations).map(([value, label]) => `<option value="${value}"${value === 'merge-then-sort' ? ' selected' : ''}>${label}</option>`).join('')}
          </select>
        </label>
        <div class="note" id="reorganizeDescription">اسناد عادی هم‌تاریخ ادغام می‌شوند و سپس شماره‌ها بر اساس تاریخ بازچینی می‌شوند.</div>
      </div>
      <div id="reorganizePreview" class="reorganize-preview"></div>`,
    actions: [],
  });
  if (!dialog) return;

  let preview = null;
  let busy = false;
  const operationInput = dialog.body.querySelector('#reorganizeOperation');
  const description = dialog.body.querySelector('#reorganizeDescription');
  const previewArea = dialog.body.querySelector('#reorganizePreview');

  const setInitialActions = () => dialog.setActions([
    {
      label: 'پیش‌نمایش تغییرات',
      primary: true,
      icon: 'i-search',
      onClick: runPreview,
    },
  ]);

  const runPreview = async () => {
    if (busy) return;
    busy = true;
    const operation = operationInput.value;
    preview = null;
    operationInput.disabled = true;
    previewArea.innerHTML = '<div class="note">در حال تحلیل اسناد…</div>';
    dialog.setActions([]);
    try {
      preview = await invoke('preview_voucher_reorganization', { operation });
      const mergeGroups = preview.mergeGroups || [];
      const unbalancedGroups = preview.unbalancedGroups || [];
      const changes = preview.numberChanges || [];
      const noChanges = mergeGroups.length === 0 && changes.length === 0;
      const mergeRows = mergeGroups.slice(0, 20).map((group) => `
        <tr>
          <td>${esc(group.date)}</td>
          <td>${group.sourceVoucherNumbers.map(toFa).join('، ')}</td>
          <td>${toFa(group.keepVoucherNumber)}</td>
          <td>${toFa(group.lineCount)}</td>
        </tr>`).join('');
      const changeRows = changes.slice(0, 20).map((change) => `
        <tr>
          <td>${esc(change.date)}</td>
          <td>${toFa(change.oldNumber)}</td>
          <td>${toFa(change.newNumber)}</td>
        </tr>`).join('');
      const unbalancedRows = unbalancedGroups.slice(0, 10).map((group) => `
        <li>${esc(group.date)}: ${group.sourceVoucherNumbers.map(toFa).join('، ')} — ${esc(group.reason || 'گروه قابل ادغام نیست.')}</li>`).join('');
      // eslint-disable-next-line no-unsanitized/property -- preview values are escaped or formatted numbers
      previewArea.innerHTML = `
        <div class="info-list">
          <div><dt>تعداد اسناد</dt><dd>${toFa(preview.totalVouchers)}</dd></div>
          <div><dt>پس از ادغام</dt><dd>${toFa(preview.totalVouchersAfter)}</dd></div>
          <div><dt>گروه‌های قابل ادغام</dt><dd>${toFa(mergeGroups.length)}</dd></div>
          <div><dt>اسناد حذف‌شونده در ادغام</dt><dd>${toFa(preview.removedVoucherCount)}</dd></div>
          <div><dt>تغییر شماره</dt><dd>${toFa(changes.length)}</dd></div>
        </div>
        ${noChanges ? '<div class="note">تغییری برای این عملیات پیدا نشد.</div>' : ''}
        ${unbalancedGroups.length ? `<div class="note" data-tone="danger">${toFa(unbalancedGroups.length)} گروه هم‌تاریخ به دلیل نامتوازن‌بودن یا محدودیت داده ادغام نمی‌شوند.<ul>${unbalancedRows}</ul></div>` : ''}
        ${mergeGroups.length ? `
          <h3 class="reorganize-section-title">گروه‌های ادغام (${toFa(mergeGroups.length)})</h3>
          <div class="reorganize-table-wrap"><table><thead><tr><th>تاریخ</th><th>شماره اسناد</th><th>شماره باقی‌مانده</th><th>ردیف</th></tr></thead><tbody>${mergeRows}</tbody></table></div>
          ${mergeGroups.length > 20 ? `<div class="form-note">${toFa(mergeGroups.length - 20)} گروه دیگر در فهرست نمایش داده نشده است.</div>` : ''}` : ''}
        ${changes.length ? `
          <h3 class="reorganize-section-title">نمونه تغییر شماره‌ها (${toFa(changes.length)})</h3>
          <div class="reorganize-table-wrap"><table><thead><tr><th>تاریخ</th><th>شماره فعلی</th><th>شماره جدید</th></tr></thead><tbody>${changeRows}</tbody></table></div>
          ${changes.length > 20 ? `<div class="form-note">${toFa(changes.length - 20)} تغییر دیگر در فهرست نمایش داده نشده است.</div>` : ''}` : ''}
        ${mergeGroups.length ? '<label class="reorganize-confirm"><input id="reorganizeConfirmMerge" type="checkbox"><span>ادغام اسناد و حذف سرآیندهای تکراری را تأیید می‌کنم.</span></label>' : ''}
        <div class="note" data-reorganize-error hidden></div>`;

      dialog.setActions([
        {
          label: 'تأیید و اجرا',
          primary: true,
          icon: 'i-check',
          id: 'reorganizeApply',
          onClick: async (currentDialog, button) => {
            if (!preview || busy) return;
            if (preview.mergeGroups.length && !currentDialog.body.querySelector('#reorganizeConfirmMerge')?.checked) return;
            busy = true;
            button.disabled = true;
            try {
              const result = await invoke('apply_voucher_reorganization', {
                operation: preview.operation,
                planToken: preview.planToken,
              });
              currentDialog.close();
              const parts = [];
              if (result.mergedGroupCount) parts.push(`${toFa(result.mergedGroupCount)} گروه ادغام شد`);
              if (result.removedVoucherCount) parts.push(`${toFa(result.removedVoucherCount)} سند حذف شد`);
              if (result.renumberedCount) parts.push(`${toFa(result.renumberedCount)} شماره بازچینی شد`);
              toast(parts.length ? `${parts.join('، ')}.` : 'تغییری لازم نبود.', 'ok');
              await navigate('vouchers', { force: true });
            } catch (error) {
              const message = error instanceof Error ? error.message : String(error);
              const errorBox = currentDialog.body.querySelector('[data-reorganize-error]');
              if (errorBox) {
                errorBox.textContent = message;
                errorBox.hidden = false;
                errorBox.dataset.tone = 'danger';
              }
              const requiresMergeConfirmation = preview.mergeGroups.length > 0;
              const mergeConfirmed = Boolean(currentDialog.body.querySelector('#reorganizeConfirmMerge')?.checked);
              button.disabled = requiresMergeConfirmation && !mergeConfirmed;
            } finally {
              busy = false;
            }
          },
        },
        { label: 'پیش‌نمایش مجدد', icon: 'i-refresh', onClick: runPreview },
      ]);

      const applyButton = dialog.foot.querySelector('#reorganizeApply');
      const confirmMerge = dialog.body.querySelector('#reorganizeConfirmMerge');
      if (applyButton) applyButton.disabled = noChanges || mergeGroups.length > 0;
      confirmMerge?.addEventListener('change', () => {
        if (applyButton) applyButton.disabled = noChanges || !confirmMerge.checked;
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      previewArea.innerHTML = `<div class="note" data-tone="danger">${esc(message)}</div>`;
      setInitialActions();
    } finally {
      operationInput.disabled = false;
      busy = false;
    }
  };

  operationInput.addEventListener('change', () => {
    preview = null;
    previewArea.innerHTML = '';
    const descriptions = {
      'merge-then-sort': 'اسناد عادی هم‌تاریخ ادغام می‌شوند و سپس شماره‌ها بر اساس تاریخ بازچینی می‌شوند.',
      merge: 'فقط اسناد عادی با تاریخ یکسان ادغام می‌شوند؛ شماره‌ی سند باقی‌مانده تغییر نمی‌کند.',
      sort: 'شماره‌ی تمام اسناد بر اساس تاریخ، به‌صورت متوالی بازچینی می‌شود.',
    };
    description.textContent = descriptions[operationInput.value];
    setInitialActions();
  });
  setInitialActions();
}

function openBulkVoucherImport() {
  if (!invoke) {
    toast('ورود دسته‌جمعی اکسل فقط در نسخه دسکتاپ ممکن است.', 'error');
    return;
  }

  const steps = ['انتخاب فایل اکسل', 'اعتبارسنجی ساختار', 'تطبیق کدینگ حساب', 'صدور نهایی سند'];
  let unlistenProgress = null;
  let busy = false;
  let cancelRequested = false;
  const dialog = openDialog({
    title: 'ورود دسته‌جمعی اسناد از اکسل',
    wide: true,
    body: '',
    actions: [],
    canClose: () => !busy,
    onClose: () => {
      unlistenProgress?.();
      unlistenProgress = null;
    },
  });
  if (!dialog) return;

  let selecting = false;
  let dateSystem = 'auto';

  const renderWizard = (activeStep, content) => {
    const progress = activeStep === 4 ? 100 : Math.round(((activeStep + 1) / steps.length) * 100);
    const stepper = steps.map((label, index) => {
      const done = index < activeStep || activeStep === 4;
      const active = index === activeStep && activeStep !== 4;
      return `<div class="bulk-import-step${active ? ' is-active' : ''}${done ? ' is-done' : ''}">
        <span>${done ? icon('i-check') : toFa(index + 1)}</span><b>${label}</b>
      </div>`;
    }).join('');
    // eslint-disable-next-line no-unsanitized/property -- trusted fragments built above
    dialog.body.innerHTML = `
      <section class="bulk-import-wizard" aria-label="مراحل ورود اسناد">
        <header class="bulk-import-progress">
          <div class="bulk-import-progress-line">
            <strong>گام ${toFa(Math.min(activeStep + 1, steps.length))} از ${toFa(steps.length)}: ${activeStep === 4 ? 'تکمیل فرآیند' : steps[Math.min(activeStep, steps.length - 1)]}</strong>
            <span>پیشرفت فرآیند: ${toFa(progress)}٪</span>
          </div>
          <div class="bulk-import-progress-track"><i style="width:${progress}%"></i></div>
          <div class="bulk-import-stepper">${stepper}</div>
        </header>
        <div class="bulk-import-content">${content}</div>
      </section>`;
  };

  const setBulkProgress = (percent, label = '') => {
    const safePercent = Math.max(0, Math.min(100, Math.round(Number(percent) || 0)));
    const track = dialog.body.querySelector('.bulk-import-progress-track > i');
    if (track) track.style.width = `${safePercent}%`;
    const counter = dialog.body.querySelector('.bulk-import-progress-line > span');
    if (counter) counter.textContent = `پیشرفت فرآیند: ${toFa(safePercent)}٪`;
    const detail = dialog.body.querySelector('#bulkImportProgressText');
    if (detail && label) detail.textContent = label;
  };

  const showBusy = (step, title, detail) => {
    busy = true;
    renderWizard(step, `<div class="bulk-import-loading">
      <span class="bulk-import-wave"><i></i><i></i><i></i><i></i><i></i></span>
      <strong>${title}</strong>
      <small id="bulkImportProgressText">${detail}</small>
      <div class="form-actions" style="justify-content:center;margin-top:12px">
        <button type="button" class="btn" id="bulkImportCancel">${icon('i-close')}<span>لغو ورود</span></button>
      </div>
    </div>`);
    dialog.setActions([]);
    const cancelButton = dialog.body.querySelector('#bulkImportCancel');
    cancelButton?.addEventListener('click', async () => {
      cancelButton.disabled = true;
      cancelRequested = true;
      const label = cancelButton.querySelector('span');
      if (label) label.textContent = 'در حال لغو…';
      try {
        await invoke('cancel_bulk_voucher_import');
      } catch (error) {
        cancelButton.disabled = false;
        if (label) label.textContent = 'لغو ورود';
        toast(errorText(error), 'error');
      }
    });
  };

  const eventApi = window.__TAURI__?.event;
  if (eventApi && typeof eventApi.listen === 'function') {
    Promise.resolve(eventApi.listen('bulk-import-progress', (event) => {
      const payload = event?.payload || {};
      const processed = Math.max(0, Number(payload.processed) || 0);
      const total = Math.max(0, Number(payload.total) || 0);
      const percent = total > 0 ? (processed / total) * 100 : 0;
      const stage = payload.stage === 'commit' ? 'ثبت سند' : 'خواندن ردیف';
      setBulkProgress(percent, `${stage} ${toFa(processed)} از ${toFa(total)}`);
    })).then((unlisten) => {
      unlistenProgress = unlisten;
      if (dialog.modal.hidden) unlisten?.();
      return undefined;
    }).catch(() => {});
  }

  const selectContent = (message = '') => `
    ${message ? `<div class="note" data-tone="danger" role="alert">${esc(message)}</div>` : ''}
    <section class="bulk-import-file-panel">
      <div class="bulk-import-file-heading"><span class="bulk-import-file-icon">${icon('i-doc')}</span><div><h2>محل بارگذاری فایل دفاتر</h2><p>کاربرگ دارای اطلاعات به‌صورت خودکار شناسایی می‌شود.</p></div><span class="bulk-import-format">XLSX / XLS</span></div>
      <label class="bulk-import-date-system">
        <span>نوع تاریخ فایل اکسل</span>
        <select id="bulkImportDateSystem">
          <option value="auto"${dateSystem === 'auto' ? ' selected' : ''}>تشخیص خودکار (سال ۱۶۰۰ به بعد میلادی)</option>
          <option value="jalali"${dateSystem === 'jalali' ? ' selected' : ''}>شمسی</option>
          <option value="gregorian"${dateSystem === 'gregorian' ? ' selected' : ''}>میلادی</option>
        </select>
      </label>
      <div class="note">اگر تاریخ‌های فایل به‌صورت متنی نوشته شده‌اند، نوع آن‌ها را انتخاب کنید. در حالت تشخیص خودکار، سال‌های ۱۶۰۰ تا ۳۱۷۷ میلادی خوانده می‌شوند؛ این تاریخ‌ها در پیش‌نمایش به‌عنوان موارد مبهم هشدار داده می‌شوند. سلول‌های تاریخ واقعی اکسل همیشه میلادی و به شمسی تبدیل می‌شوند.</div>
      <div class="note">ستون‌های شماره سند، تاریخ، کد و عنوان کل، کد و عنوان معین، بدهکار و بستانکار الزامی هستند. در صورت استفاده از کد تفصیلی ۱، عنوان آن نیز لازم است. نوع سند، شرح سند و شرح ردیف اختیاری هستند. ستون کد گروه برای کدینگ‌های سفارشی اختیاری است؛ در صورت نبود آن، گروه از پیشوند کد حساب کل بر اساس کدینگ پیش‌فرض برنامه تشخیص داده می‌شود. ستون‌های ماهیت حساب نیز اختیاری‌اند؛ برای ساخت حساب در گروه‌های سایر درآمدها و هزینه‌ها، ماهیت حساب معین/تفصیلی را با مقدار بدهکار، بستانکار یا دوگانه مشخص کنید. تاریخ شمسی متنی یا تاریخ واقعی اکسل پذیرفته می‌شود و مبالغ باید عدد صحیح ریال باشند. کدهای دارای صفر ابتدایی یا بیش از ۱۵ رقم را در اکسل به‌صورت متن ذخیره کنید.</div>
    </section>`;

  function showReview(currentDialog, preview, message = '') {
    busy = false;
    cancelRequested = false;
    const conflicts = Number(preview.numberConflictCount) || 0;
    const inferred = Number(preview.inferredGroupCount) || 0;
    const dateWarningCount = Number(preview.dateWarningCount) || 0;
    const dateWarnings = Array.isArray(preview.dateWarnings) ? preview.dateWarnings : [];
    const inferredMappings = Array.isArray(preview.inferredGroupMappings) ? preview.inferredGroupMappings : [];
    const mappingRows = inferredMappings.map((item) => `
      <tr>
        <td>${toFa(item.accountCode)}</td>
        <td>${esc(item.accountName)}</td>
        <td>${toFa(item.level)}</td>
        <td>${toFa(item.groupCode)}</td>
        <td>${esc(item.reason || 'گروه از پیشوند کد حساب کل حدس زده شد.')}</td>
        <td><input class="bulk-import-group-input" data-account-code="${esc(item.accountCode)}" value="${esc(item.groupCode)}" inputmode="numeric" aria-label="گروه نهایی حساب ${esc(item.accountCode)}"></td>
      </tr>`).join('');
    renderWizard(2, `${message ? `<div class="note" data-tone="danger" role="alert">${esc(message)}</div>` : ''}
      <div class="note">فایل «${esc(preview.fileName)}» از کاربرگ «${esc(preview.sheetName || '—')}» شناسایی شد. ستون‌ها، تاریخ‌ها، تراز اسناد و ساختار اولیه حساب‌ها بررسی شده‌اند. شرح و نوع سند در صورت وجود در فایل حفظ می‌شود.</div>
      <dl class="info-list">
        <div><dt>کاربرگ</dt><dd>${esc(preview.sheetName || '—')}</dd></div>
        <div><dt>تعداد سند</dt><dd>${toFa(preview.voucherCount)}</dd></div>
        <div><dt>تعداد ردیف</dt><dd>${toFa(preview.lineCount)}</dd></div>
        <div><dt>حساب‌های شناسایی‌شده</dt><dd>${toFa(preview.accountCount)}</dd></div>
        <div><dt>حساب‌های جدید</dt><dd>${toFa(preview.newAccountCount)}</dd></div>
        <div><dt>شماره‌های تکراری</dt><dd>${toFa(conflicts)}</dd></div>
        <div><dt>تاریخ‌های مبهم</dt><dd>${toFa(dateWarningCount)}</dd></div>
        <div><dt>گروه‌های حدس‌زده‌شده</dt><dd>${toFa(inferred)}</dd></div>
      </dl>
      ${dateWarningCount ? `<div class="note" data-tone="warn" role="alert"><strong>${toFa(dateWarningCount)} تاریخ در حالت تشخیص خودکار میلادی تفسیر شد.</strong><span>اگر تاریخ‌های فایل شمسی هستند، نوع تاریخ را «شمسی» انتخاب کنید و فایل را دوباره تحلیل کنید. نمونه موارد:</span><ul>${dateWarnings.map((warning) => `<li>${esc(warning)}</li>`).join('')}</ul></div>` : ''}
      ${conflicts ? `<div class="bulk-import-conflicts"><strong>شماره‌های موجود در برنامه</strong><span>${(preview.conflictingVoucherNumbers || []).map((number) => toFa(number)).join('، ') || 'جزئیات شماره‌ها در دسترس نیست'}</span></div>` : ''}
      ${inferred ? `<div class="note" data-tone="warn" role="alert"><strong>گروه ${toFa(inferred)} حساب از روی پیشوند کد حدس زده شد.</strong><span>این حساب‌ها ستون «کد گروه» نداشتند. گروه پیشنهادی را در ستون «گروه نهایی» بررسی و در صورت نیاز اصلاح کنید؛ تغییر گروه یک حساب کل به همه حساب‌های حدسی همان شاخه اعمال می‌شود. تا زمانی که همه نگاشت‌ها تأیید نشوند، ثبت اسناد ممکن نیست. تصمیم نهایی در گزارش رویدادها (Audit) با منبع فایل، حدس یا اصلاح دستی ثبت می‌شود.</span></div>
      <div class="bulk-import-mappings-wrap">
        <table class="bulk-import-mappings">
          <thead><tr><th>کد حساب</th><th>عنوان</th><th>سطح</th><th>گروه پیشنهادی</th><th>علت پیشنهاد</th><th>گروه نهایی</th></tr></thead>
          <tbody>${mappingRows}</tbody>
        </table>
      </div>
      <label class="bulk-import-renumber"><input id="bulkImportConfirmInferred" type="checkbox"><span>همه نگاشت‌های گروه بالا را بررسی کردم و تأیید می‌کنم</span></label>` : ''}
      ${conflicts ? `<label class="bulk-import-renumber"><input id="bulkImportRenumber" type="checkbox"><span>برای ${toFa(conflicts)} سند دارای شماره‌ی تکراری، شماره‌ی آزاد جدید اختصاص بده</span></label>` : ''}`);

    currentDialog.setActions([
      {
        id: 'bulkImportCommit',
        label: 'تأیید و ثبت اسناد',
        primary: true,
        icon: 'i-check',
        onClick: async (activeDialog, button) => {
          const renumberConflicts = Boolean(activeDialog.body.querySelector('#bulkImportRenumber')?.checked);
          const confirmInferred = activeDialog.body.querySelector('#bulkImportConfirmInferred');
          const groupInputs = Array.from(activeDialog.body.querySelectorAll('.bulk-import-group-input'));
          if (inferred > 0 && !confirmInferred?.checked) {
            toast('ابتدا نگاشت گروه حساب‌های حدس‌زده‌شده را تأیید کنید.', 'error');
            return;
          }
          const groupOverrides = groupInputs.map((input) => ({
            accountCode: input.dataset.accountCode || '',
            groupCode: input.value.trim(),
          }));
          if (groupOverrides.some((mapping) => !mapping.accountCode || !mapping.groupCode)) {
            toast('گروه نهایی همه نگاشت‌ها را وارد کنید.', 'error');
            return;
          }
          button.disabled = true;
          activeDialog.body.querySelectorAll('button').forEach((item) => { item.disabled = true; });
          cancelRequested = false;
          showBusy(3, 'در حال ثبت اسناد و حساب‌های جدید…', 'ثبت در پایگاه داده به‌صورت تراکنشی انجام می‌شود؛ در صورت لغو، هیچ سندی ثبت نمی‌شود.');
          try {
            const count = await invoke('commit_bulk_voucher_excel', {
              renumberConflicts,
              groupOverrides,
              confirmInferredGroups: inferred === 0 ? true : Boolean(confirmInferred?.checked),
            });
            busy = false;
            const cancelNote = cancelRequested
              ? '<div class="note" data-tone="warn">درخواست لغو پس از تکمیل عملیات رسید؛ اسناد قبلاً ثبت شده‌اند و لغو دیگر ممکن نیست.</div>'
              : `<div class="note">فایل «${esc(preview.fileName)}» پردازش شد و اطلاعات ثبت‌شده در فهرست اسناد قابل مشاهده است.</div>`;
            renderWizard(4, `<section class="bulk-import-result" role="status"><span class="bulk-import-result-icon">${icon('i-check')}</span><h2>ورود اسناد با موفقیت تکمیل شد</h2><p><strong>${toFa(count)}</strong> سند حسابداری ثبت شد.</p>${cancelNote}</section>`);
            activeDialog.setActions([
              { label: 'مشاهده اسناد', primary: true, icon: 'i-journal', onClick: async (doneDialog) => { doneDialog.close(); await navigate('vouchers', { force: true }); } },
              { label: 'بستن', onClick: (doneDialog) => doneDialog.close() },
            ]);
            toast(
              cancelRequested
                ? `${toFa(count)} سند پیش از رسیدن درخواست لغو ثبت شده بود.`
                : `${toFa(count)} سند با موفقیت وارد شد.`,
              cancelRequested ? 'warn' : 'ok',
            );
          } catch (error) {
            showReview(activeDialog, preview, error instanceof Error ? error.message : String(error));
          }
        },
      },
      { label: 'انتخاب فایل دیگر', icon: 'i-refresh', onClick: selectFile },
    ]);

    const commitButton = currentDialog.foot.querySelector('#bulkImportCommit');
    const renumberCheckbox = currentDialog.body.querySelector('#bulkImportRenumber');
    const confirmInferred = currentDialog.body.querySelector('#bulkImportConfirmInferred');
    const groupInputs = Array.from(currentDialog.body.querySelectorAll('.bulk-import-group-input'));
    const updateCommitState = () => {
      if (!commitButton) return;
      const conflictsBlocked = conflicts > 0 && !renumberCheckbox?.checked;
      const inferredBlocked = inferred > 0 && !confirmInferred?.checked;
      commitButton.disabled = conflictsBlocked || inferredBlocked;
    };
    updateCommitState();
    renumberCheckbox?.addEventListener('change', updateCommitState);
    confirmInferred?.addEventListener('change', updateCommitState);
    groupInputs.forEach((input) => input.addEventListener('input', updateCommitState));
  }

  const selectFile = async () => {
    if (selecting) return;
    selecting = true;
    cancelRequested = false;
    showBusy(1, 'فایل در حال بررسی و اسناد در حال اعتبارسنجی هستند…', 'ساختار ستون‌ها، تاریخ‌ها، تراز اسناد و کدینگ حساب کنترل می‌شود.');
    try {
      const chosen = dialog.body.querySelector('#bulkImportDateSystem')?.value;
      if (chosen === 'auto' || chosen === 'jalali' || chosen === 'gregorian') {
        dateSystem = chosen;
      }
      const preview = await invoke('analyze_bulk_voucher_excel', { dateSystem });
      if (!preview) {
        busy = false;
        renderWizard(0, selectContent('انتخاب فایل لغو شد.'));
        dialog.setActions([{ label: 'انتخاب فایل اکسل', primary: true, icon: 'i-doc', onClick: selectFile }]);
        return;
      }
      showReview(dialog, preview);
    } catch (error) {
      busy = false;
      const message = error instanceof Error ? error.message : String(error);
      renderWizard(0, selectContent(message));
      dialog.setActions([{ label: 'انتخاب فایل اکسل', primary: true, icon: 'i-doc', onClick: selectFile }]);
    } finally {
      selecting = false;
    }
  };

  renderWizard(0, selectContent());
  dialog.setActions([{ label: 'انتخاب فایل اکسل', primary: true, icon: 'i-doc', onClick: selectFile }]);
}

async function openBulkVoucherDeleteWindow() {
  if (!invoke) {
    toast('حذف دسته‌جمعی اسناد فقط در نسخه دسکتاپ ممکن است.', 'error');
    return;
  }
  try {
    await invoke('open_bulk_voucher_delete_window');
    state.refreshOnFocus = true;
  } catch (error) {
    toast(errorText(error), 'error');
  }
}

function runMenuItem(item) {
  if (item.page) {
    navigate(item.page);
    return;
  }
  switch (item.command) {
    case 'groups-window':
      openGroupsWindow();
      return;
    case 'account-definition':
      window.openAccountDefinitionPopup?.();
      return;
    case 'new-voucher':
      openVoucherWindow();
      return;
    case 'voucher-list':
      openVoucherListWindow();
      return;
    case 'bulk-voucher-import':
      openBulkVoucherImport();
      return;
    case 'reorganize-vouchers':
      openReorganizeVouchers();
      return;
    case 'bulk-voucher-delete':
      openBulkVoucherDeleteWindow();
      return;
    case 'choose-wallpaper':
      $('#wallpaperPicker')?.click();
      return;
    case 'company-selector':
      void openCompanySelector();
      return;
    case 'help':
      showHelpDialog();
      return;
    case 'about':
      showAboutDialog();
      return;
    case 'shortcuts':
      showShortcutsDialog();
      return;
    default:
      notifyPendingMenu(item.label);
  }
}

function bindMenubar() {
  const nav = $('#menubarNav');
  if (!nav) return;

  nav.addEventListener('click', (event) => {
    const trigger = event.target.closest('.menu-root');
    if (trigger) {
      const index = Number(trigger.dataset.menuIndex);
      if (openMenuIndex === index) {
        // اگر منو با عبور ماوس باز شده، کلیک روی همان عنوان آن را نبندد.
        if (menuOpenedByHover) menuOpenedByHover = false;
        else closeMenubarMenus();
      } else {
        openMenubarMenu(index);
      }
      return;
    }

    const itemButton = event.target.closest('.menu-item');
    if (!itemButton) return;
    const wrap = itemButton.closest('.menu-wrap');
    const menuIndex = Number(wrap?.dataset.menuIndex);
    const itemIndex = Number(itemButton.dataset.itemIndex);
    const entry = MENU_BAR[menuIndex]?.items?.[itemIndex];
    closeMenubarMenus();
    if (entry) runMenuItem(entry);
  });

  // مثل رابط سوم: وقتی یک منو باز است، عبور ماوس روی منوی دیگر آن را باز می‌کند.
  nav.addEventListener('mouseover', (event) => {
    if (openMenuIndex === null) return;
    const wrap = event.target.closest('.menu-wrap');
    if (!wrap) return;
    const index = Number(wrap.dataset.menuIndex);
    if (Number.isInteger(index) && index !== openMenuIndex) {
      openMenubarMenu(index, { fromHover: true });
    }
  });

  // Keyboard support for the menu bar: arrows move between menus and items.
  nav.addEventListener('keydown', (event) => {
    const item = event.target.closest('.menu-item');
    const trigger = event.target.closest('.menu-root');
    if (!item && !trigger) return;

    const triggers = $$('#menubarNav .menu-root');
    const activeTrigger = trigger || item.closest('.menu-wrap')?.querySelector('.menu-root');
    const menuIndex = triggers.indexOf(activeTrigger);
    if (menuIndex < 0) return;

    const focusItems = (index, last = false) => {
      const wrap = $$('#menubarNav .menu-wrap')[index];
      const items = wrap ? $$('.menu-item', wrap) : [];
      const target = last ? items[items.length - 1] : items[0];
      if (target) target.focus();
    };
    const moveMenu = (offset, focusFirst = false) => {
      const nextIndex = (menuIndex + offset + triggers.length) % triggers.length;
      if (item) {
        closeMenubarMenus();
        openMenubarMenu(nextIndex);
        if (focusFirst) focusItems(nextIndex);
        else triggers[nextIndex].focus();
      } else {
        triggers[nextIndex].focus();
        if (openMenuIndex !== null) {
          openMenubarMenu(nextIndex);
          focusItems(nextIndex);
        }
      }
    };

    if (item) {
      const items = $$('.menu-item', item.closest('.menu-wrap'));
      const itemIndex = items.indexOf(item);
      if (event.key === 'ArrowDown') {
        event.preventDefault();
        items[(itemIndex + 1) % items.length]?.focus();
      } else if (event.key === 'ArrowUp') {
        event.preventDefault();
        items[(itemIndex - 1 + items.length) % items.length]?.focus();
      } else if (event.key === 'Home') {
        event.preventDefault();
        items[0]?.focus();
      } else if (event.key === 'End') {
        event.preventDefault();
        items[items.length - 1]?.focus();
      } else if (event.key === 'ArrowRight') {
        event.preventDefault();
        moveMenu(1, true);
      } else if (event.key === 'ArrowLeft') {
        event.preventDefault();
        moveMenu(-1, true);
      } else if (event.key === 'Escape') {
        event.preventDefault();
        closeMenubarMenus();
        triggers[menuIndex]?.focus();
      } else if (event.key === 'Tab') {
        closeMenubarMenus();
      }
      return;
    }

    if (event.key === 'ArrowRight') {
      event.preventDefault();
      moveMenu(1);
    } else if (event.key === 'ArrowLeft') {
      event.preventDefault();
      moveMenu(-1);
    } else if (event.key === 'ArrowDown') {
      event.preventDefault();
      openMenubarMenu(menuIndex);
      focusItems(menuIndex);
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      openMenubarMenu(menuIndex);
      focusItems(menuIndex, true);
    } else if (event.key === 'Escape' && openMenuIndex !== null) {
      event.preventDefault();
      closeMenubarMenus();
    }
  });

  document.addEventListener('pointerdown', (event) => {
    const bar = $('.menubar');
    if (bar && !bar.contains(event.target)) closeMenubarMenus();
  });

  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && openMenuIndex !== null) closeMenubarMenus();
  });
}

function showHelpDialog() {
  openDialog({
    title: 'راهنمای نرم‌افزار',
    body: `
      <div class="note">
        <b>دسترسی به بخش‌ها</b><br>
        از منوبار بالای پنجره، بخش مورد نظر را انتخاب کنید. بخش‌های «اطلاعات پایه»،
        «گزارشات حسابداری»، «صورت‌های مالی» و «دفاتر قانونی» صفحات موجود را باز می‌کنند
        و بقیه گزینه‌ها به‌تدریج تکمیل می‌شوند.
      </div>
      <div class="note">
        <b>ثبت سند جدید</b><br>
        از «عملیات مالی ← صدور سند حسابداری» یا کلید <span dir="ltr">Ctrl + N</span> استفاده کنید.
      </div>`,
  });
}

function showAboutDialog() {
  openDialog({
    title: 'درباره',
    body: `
      <div class="note">
        <b>نرم‌افزار حسابداری</b><br>
        نسخه ${esc(APP_VERSION)}<br>
        نرم‌افزار حسابداری دسکتاپ با پایگاه داده محلی SQLite.
      </div>`,
  });
}

function showShortcutsDialog() {
  openDialog({
    title: 'میانبرهای صفحه‌کلید',
    body: `
      <div class="note">
        <b><span dir="ltr">Ctrl + N</span></b> — صدور سند جدید<br>
        <b><span dir="ltr">Esc</span></b> — بستن منوها و پنجره‌ها
      </div>`,
  });
}

async function exitApplication() {
  const confirmed = await confirmDialog(
    'خروج از برنامه',
    'آیا از بستن نرم‌افزار مطمئن هستید؟',
    'خروج',
    true
  );
  if (!confirmed) return;

  try {
    const tauriWindow = window.__TAURI__ && window.__TAURI__.window;
    if (tauriWindow && typeof tauriWindow.getCurrentWindow === 'function') {
      await tauriWindow.getCurrentWindow().close();
    } else {
      window.close();
    }
  } catch (error) {
    toast(errorText(error), 'error');
  }
}

