/* Bulk voucher delete form for the Rust/Tauri client.
   It lives in its own native window, with the same classic accounting-form
   look as the account-definition window. */
(() => {
  'use strict';

  const tauriCore = window.__TAURI__?.core || null;
  const invoke = typeof tauriCore?.invoke === 'function'
    ? tauriCore.invoke.bind(tauriCore)
    : null;
  const appWindow = window.__TAURI__?.window?.getCurrentWindow?.() || null;

  const FA_DIGITS = '۰۱۲۳۴۵۶۷۸۹';
  const toFa = (value) => String(value ?? '').replace(/[0-9]/g, (d) => FA_DIGITS[Number(d)]);
  const toEn = (value) => String(value ?? '')
    .replace(/[۰-۹]/g, (ch) => String(ch.charCodeAt(0) - 0x06F0))
    .replace(/[٠-٩]/g, (ch) => String(ch.charCodeAt(0) - 0x0660));
  const esc = (value) => String(value ?? '').replace(/[&<>"']/g, (ch) => (
    { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[ch]
  ));
  const faDate = (value) => (value ? toFa(value) : '—');
  const errorText = (error) => {
    if (typeof error === 'string') return error;
    if (error && typeof error.message === 'string') return error.message;
    return String(error ?? 'خطای نامشخص');
  };
  const pad2 = (value) => String(value).padStart(2, '0');

  const shiftMonth = (year, month, delta) => {
    const index = year * 12 + (month - 1) + delta;
    return { year: Math.floor(index / 12), month: (index % 12) + 1 };
  };


  if (document.body.dataset.bulkVoucherDelete !== 'true') return;

  const state = {
    step: 'choose',
    busy: false,
    preview: null,
    previewInput: null,
    year: '',
    today: '',
    presets: [],
    form: {
      scope: 'dateRange',
      fromDate: '',
      toDate: '',
      fromNumber: '',
      toNumber: '',
      withBackup: true,
    },
  };

  const root = document.createElement('main');
  root.className = 'bvd2-window';
  root.innerHTML = `
    <div class="bvd2-content">
      <nav class="bvd2-stepper" data-stepper aria-label="مراحل حذف دسته‌جمعی"></nav>
      <div class="bvd2-scroll" data-scroll></div>
    </div>
    <footer class="bvd2-toolbar" data-toolbar></footer>
    <div class="bvd2-status" data-status role="status" aria-live="polite"></div>`;
  document.body.appendChild(root);

  const $ = (selector) => root.querySelector(selector);
  const scroll = $('[data-scroll]');
  const toolbar = $('[data-toolbar]');
  const stepper = $('[data-stepper]');
  const status = $('[data-status]');

  const say = (message, tone = '') => {
    status.textContent = message || '';
    status.dataset.tone = tone;
  };

  const closeWindow = () => {
    if (state.busy) return;
    if (appWindow && typeof appWindow.close === 'function') {
      void appWindow.close();
    } else {
      window.close();
    }
  };

  const setBusy = (value) => {
    state.busy = value;
    toolbar.querySelectorAll('button').forEach((button) => {
      button.disabled = value;
    });
    // Range inputs and presets stay editable otherwise, so a slow preview
    // could come back while the user is already looking at a different range.
    scroll.querySelectorAll('input, .bvd2-preset').forEach((element) => {
      element.disabled = value;
    });
    root.setAttribute('aria-busy', value ? 'true' : 'false');
  };

  const setActions = (actions, { showClose = true } = {}) => {
    toolbar.replaceChildren();
    actions.forEach((action) => {
      const button = document.createElement('button');
      button.type = 'button';
      button.className = `bvd2-button${action.primary ? ' primary' : ''}${action.danger ? ' danger' : ''}`;
      button.textContent = action.label;
      if (action.id) button.id = action.id;
      button.disabled = Boolean(action.disabled) || state.busy;
      button.addEventListener('click', () => action.onClick?.(button));
      toolbar.append(button);
    });

    const spacer = document.createElement('span');
    spacer.className = 'bvd2-spacer';
    toolbar.append(spacer);

    if (showClose) {
      const close = document.createElement('button');
      close.type = 'button';
      close.className = 'bvd2-button';
      close.textContent = 'بستن';
      close.disabled = state.busy;
      close.addEventListener('click', closeWindow);
      toolbar.append(close);
    }
  };

  const showError = (message) => {
    const box = scroll.querySelector('[data-error]');
    if (!box) {
      if (message) say(message, 'error');
      return;
    }
    box.textContent = message || '';
    box.hidden = !message;
  };

  const renderStepper = () => {
    const states = state.step === 'choose'
      ? ['current', 'todo', 'todo']
      : state.step === 'preview'
        ? ['done', 'current', 'todo']
        : ['done', 'done', 'current'];
    const item = (number, title, stepState) => `
      <span class="bvd2-step" data-state="${stepState}">
        <span class="bvd2-step-num">${number}</span>${title}
      </span>`;
    stepper.innerHTML = `
      ${item('۱', 'انتخاب محدوده', states[0])}
      <span class="bvd2-step-line" data-state="${states[1] === 'todo' ? 'todo' : 'done'}"></span>
      ${item('۲', 'پیش‌نمایش و تأیید', states[1])}
      <span class="bvd2-step-line" data-state="${states[2] === 'todo' ? 'todo' : 'done'}"></span>
      ${item('۳', 'حذف', states[2])}`;
  };

  const activePreset = () => (
    state.form.scope === 'dateRange'
      ? state.presets.find((preset) => (
        preset.from === state.form.fromDate && preset.to === state.form.toDate
      ))?.id || ''
      : ''
  );

  const scopeCard = (value, title, sub, badgeText, danger) => {
    const checked = state.form.scope === value;
    return `
      <label class="bvd2-scope-card${checked ? ' is-selected' : ''}${danger ? ' is-danger' : ''}">
        <input type="radio" name="bvd2Scope" value="${value}"${checked ? ' checked' : ''}>
        <span class="bvd2-scope-head">
          <b>${title}</b>
          ${badgeText ? `<span class="bvd2-scope-badge">${badgeText}</span>` : ''}
        </span>
        <span class="bvd2-scope-sub">${sub}</span>
      </label>`;
  };

  const rangePanel = () => {
    if (state.form.scope === 'dateRange') {
      const presetButtons = state.presets.map((preset) => `
        <button type="button" class="bvd2-preset${activePreset() === preset.id ? ' is-active' : ''}"
          data-preset="${preset.id}">${preset.label}</button>`).join('');
      return `
        <div class="bvd2-panel">
          <div class="bvd2-panel-head">
            <div class="bvd2-panel-title">
              <span class="bvd2-panel-icon">📅</span>
              <div>
                <h3>تعیین بازه تاریخی حذف</h3>
                <p>شامل اسناد از تاریخ شروع تا پایانِ تاریخ پایان.</p>
              </div>
            </div>
            ${state.presets.length
              ? `<div class="bvd2-presets"><span>بازه سریع:</span>${presetButtons}</div>`
              : ''}
          </div>
          <div class="bvd2-range-grid">
            <label class="bvd2-field">
              <span>از تاریخ</span>
              <input class="bvd2-input" data-date="fromDate" type="text" inputmode="numeric"
                value="${esc(state.form.fromDate)}" placeholder="۱۴۰۵/۰۱/۰۱" />
            </label>
            <label class="bvd2-field">
              <span>تا تاریخ</span>
              <input class="bvd2-input" data-date="toDate" type="text" inputmode="numeric"
                value="${esc(state.form.toDate)}" placeholder="۱۴۰۵/۱۲/۲۹" />
            </label>
          </div>
        </div>`;
    }

    if (state.form.scope === 'numberRange') {
      return `
        <div class="bvd2-panel">
          <div class="bvd2-panel-head">
            <div class="bvd2-panel-title">
              <span class="bvd2-panel-icon">🔢</span>
              <div>
                <h3>تعیین محدوده شماره سند</h3>
                <p>شامل همه اسناد از شماره شروع تا شماره پایان.</p>
              </div>
            </div>
          </div>
          <div class="bvd2-range-grid">
            <label class="bvd2-field">
              <span>از سند شماره</span>
              <input class="bvd2-input" data-number="fromNumber" type="text" inputmode="numeric"
                dir="ltr" value="${esc(state.form.fromNumber)}" placeholder="۱" />
            </label>
            <label class="bvd2-field">
              <span>تا سند شماره</span>
              <input class="bvd2-input" data-number="toNumber" type="text" inputmode="numeric"
                dir="ltr" value="${esc(state.form.toNumber)}" placeholder="۱۰۰" />
            </label>
          </div>
        </div>`;
    }

    return `
      <div class="bvd2-panel">
        <div class="bvd2-panel-head">
          <div class="bvd2-panel-title">
            <span class="bvd2-panel-icon is-danger">⚠</span>
            <div>
              <h3>حذف تمام اسناد حسابداری</h3>
              <p>همه اسناد دفتر کل هدف این عملیات هستند — با احتیاط ادامه دهید.</p>
            </div>
          </div>
        </div>
        <div class="bvd2-note" data-tone="danger">
          این گزینه تمام اسناد موجود را هدف قرار می‌دهد. پیش‌نمایش را با دقت بررسی کنید؛
          حذف نهایی غیرقابل‌بازگشت است.
        </div>
      </div>`;
  };

  const validateChoice = () => {
    if (state.form.scope === 'dateRange') {
      if (!state.form.fromDate.trim() || !state.form.toDate.trim()) {
        return 'وارد کردن هر دو تاریخ «از» و «تا» الزامی است.';
      }
      const pattern = /^\d{4}\/\d{1,2}\/\d{1,2}$/;
      const from = toEn(state.form.fromDate).trim();
      const to = toEn(state.form.toDate).trim();
      if (!pattern.test(from) || !pattern.test(to)) {
        return 'قالب تاریخ معتبر نیست؛ نمونه: ۱۴۰۵/۰۱/۰۱.';
      }
      const normalized = (value) => {
        const [year, month, day] = value.split('/');
        return `${year}/${pad2(month)}/${pad2(day)}`;
      };
      if (normalized(from) > normalized(to)) {
        return 'تاریخ «از» نمی‌تواند بعد از تاریخ «تا» باشد.';
      }
    }
    if (state.form.scope === 'numberRange') {
      if (!state.form.fromNumber || !state.form.toNumber) {
        return 'وارد کردن هر دو شماره سند «از» و «تا» الزامی است.';
      }
      const from = Number(state.form.fromNumber);
      const to = Number(state.form.toNumber);
      if (!Number.isSafeInteger(from) || !Number.isSafeInteger(to) || from <= 0 || to <= 0) {
        return 'شماره سند باید عدد صحیح و در محدوده مجاز باشد.';
      }
      if (from > to) {
        return 'شماره سند «از» نمی‌تواند بزرگ‌تر از «تا» باشد.';
      }
    }
    return null;
  };

  const currentInput = () => ({
    scope: state.form.scope,
    fromDate: state.form.scope === 'dateRange' ? state.form.fromDate : '',
    toDate: state.form.scope === 'dateRange' ? state.form.toDate : '',
    fromNumber: state.form.scope === 'numberRange' ? Number(state.form.fromNumber) : null,
    toNumber: state.form.scope === 'numberRange' ? Number(state.form.toNumber) : null,
  });

  const runPreview = async () => {
    if (state.busy) return;
    const validationError = validateChoice();
    if (validationError) {
      showError(validationError);
      say(validationError, 'error');
      return;
    }

    setBusy(true);
    showError('');
    say('در حال بررسی اسناد…');
    scroll.querySelector('[data-loading]')?.remove();
    scroll.insertAdjacentHTML(
      'beforeend',
      '<div class="bvd2-note" data-loading>در حال بررسی اسناد…</div>'
    );

    try {
      const input = currentInput();
      state.preview = await invoke('preview_bulk_voucher_delete', { input });
      // Keep the exact input the preview was built from. The form is not
      // edited on the preview step, but applying a snapshot means a later
      // change can never be sent with an older plan token.
      state.previewInput = input;
      state.step = 'preview';
      setBusy(false);
      renderPreview();
      say(`${toFa(state.preview.totalMatched)} سند در این محدوده پیدا شد.`, 'ok');
      return;
    } catch (error) {
      scroll.querySelector('[data-loading]')?.remove();
      showError(errorText(error));
      say(errorText(error), 'error');
    }
    setBusy(false);
  };

  const runApply = async () => {
    if (!state.preview || !state.previewInput || state.busy
      || Number(state.preview.deletableCount) === 0) return;
    const confirmBox = scroll.querySelector('#bvd2Confirm');
    if (!confirmBox?.checked) return;

    setBusy(true);
    showError('');
    say(state.form.withBackup ? 'در حال تهیه پشتیبان و حذف اسناد…' : 'در حال حذف اسناد…');
    scroll.querySelector('[data-loading]')?.remove();
    scroll.insertAdjacentHTML(
      'beforeend',
      `<div class="bvd2-note" data-loading>${state.form.withBackup
        ? 'در حال تهیه پشتیبان و حذف اسناد…'
        : 'در حال حذف اسناد…'}</div>`
    );

    try {
      const result = await invoke('apply_bulk_voucher_delete', {
        input: state.previewInput,
        planToken: state.preview.planToken,
        withBackup: state.form.withBackup,
      });
      state.preview = null;
      state.previewInput = null;
      state.step = 'done';
      setBusy(false);
      renderDone(result);
      say(result.message || 'حذف اسناد انجام شد.', 'ok');
      return;
    } catch (error) {
      scroll.querySelector('[data-loading]')?.remove();
      setBusy(false);
      showError(errorText(error));
      say(errorText(error), 'error');
      const applyButton = toolbar.querySelector('#bvd2Apply');
      const confirmNow = scroll.querySelector('#bvd2Confirm');
      if (applyButton) applyButton.disabled = !confirmNow?.checked;
    }
  };

  const renderChoose = () => {
    state.step = 'choose';
    renderStepper();
    scroll.innerHTML = `
      <div class="bvd2-intro">
        محدوده اسنادی که می‌خواهید حذف کنید را انتخاب کنید. ابتدا پیش‌نمایش تهیه می‌شود و
        حذف فقط پس از تأیید صریح شما انجام می‌شود.
      </div>
      <div class="bvd2-scope-grid" role="radiogroup" aria-label="محدوده حذف">
        ${scopeCard('dateRange', 'فیلتر بر اساس تاریخ ثبت', 'انتخاب محدوده دقیق روز و ماه', 'پیش‌فرض')}
        ${scopeCard('numberRange', 'محدوده شماره سند', 'از سند شماره X تا سند شماره Y')}
        ${scopeCard('all', 'کل اسناد حسابداری', 'تمام اسناد موجود در دفتر', '', true)}
      </div>
      ${rangePanel()}
      <div class="bvd2-note" data-tone="danger" data-error hidden></div>`;

    setActions([
      { label: 'پیش‌نمایش اسناد', primary: true, onClick: runPreview },
    ]);

    scroll.querySelectorAll('input[name="bvd2Scope"]').forEach((input) => {
      input.addEventListener('change', () => {
        if (state.busy) return;
        state.form.scope = input.value;
        state.preview = null;
        showError('');
        renderChoose();
      });
    });

    scroll.querySelectorAll('[data-date]').forEach((input) => {
      input.addEventListener('input', () => {
        const cleaned = toEn(input.value).replace(/[^\d/]/g, '');
        if (input.value !== cleaned) input.value = cleaned;
        state.form[input.dataset.date] = cleaned;
        showError('');
      });
    });

    scroll.querySelectorAll('[data-number]').forEach((input) => {
      input.addEventListener('input', () => {
        const cleaned = toEn(input.value).replace(/[^\d]/g, '');
        if (input.value !== cleaned) input.value = cleaned;
        state.form[input.dataset.number] = cleaned;
        showError('');
      });
    });

    scroll.querySelectorAll('[data-preset]').forEach((button) => {
      button.addEventListener('click', () => {
        if (state.busy) return;
        const preset = state.presets.find((item) => item.id === button.dataset.preset);
        if (!preset) return;
        state.form.fromDate = preset.from;
        state.form.toDate = preset.to;
        renderChoose();
      });
    });
  };

  const renderPreview = () => {
    if (!state.preview) return;
    renderStepper();
    const sample = Array.isArray(state.preview.deletableSample) ? state.preview.deletableSample : [];
    const deletableCount = Number(state.preview.deletableCount) || 0;
    const hasDeletable = deletableCount > 0;
    const rows = sample.map((row) => `
      <tr>
        <td class="num">${toFa(row.voucherNumber)}</td>
        <td class="num">${faDate(row.displayDate)}</td>
        <td class="desc">${esc(row.description || '—')}</td>
      </tr>`).join('');
    const rangeText = state.preview.dateRange
      ? `بازه انتخابی: ${faDate(state.preview.dateRange.from)} تا ${faDate(state.preview.dateRange.to)}`
      : state.preview.numberRange
        ? `بازه انتخابی: سند ${toFa(state.preview.numberRange.from)} تا ${toFa(state.preview.numberRange.to)}`
        : 'تمام اسناد حسابداری';

    scroll.innerHTML = `
      <div class="bvd2-intro">این فقط یک پیش‌نمایش است و هنوز هیچ سندی حذف نشده.</div>
      <dl class="bvd2-summary">
        <div><dt>سند در این محدوده</dt><dd>${toFa(state.preview.totalMatched)}</dd></div>
        <div><dt>قابل حذف</dt><dd class="danger">${toFa(deletableCount)}</dd></div>
        ${Number(state.preview.lockedCount)
          ? `<div><dt>دائمی (حذف نمی‌شود)</dt><dd>${toFa(state.preview.lockedCount)}</dd></div>`
          : ''}
      </dl>
      <div class="bvd2-range-note">${esc(rangeText)}</div>
      ${hasDeletable ? `
        <h3 class="bvd2-section-title">نمونه‌ای از اسناد قابل حذف</h3>
        <div class="bvd2-grid-wrap">
          <table class="bvd2-grid">
            <thead><tr><th>شماره سند</th><th>تاریخ</th><th>شرح</th></tr></thead>
            <tbody>${rows}</tbody>
          </table>
        </div>
        ${deletableCount > sample.length
          ? `<div class="bvd2-more">و ${toFa(deletableCount - sample.length)} سند دیگر…</div>`
          : ''}
        <label class="bvd2-check">
          <input type="checkbox" id="bvd2Backup"${state.form.withBackup ? ' checked' : ''}>
          <span>
            قبل از حذف، یک نسخه پشتیبان از پایگاه داده تهیه شود (پیش‌فرض فعال).
            ${state.preview.backupDirectory
              ? `<small>مسیر پشتیبان: <bdi dir="ltr">${esc(state.preview.backupDirectory)}</bdi></small>`
              : ''}
          </span>
        </label>
        <label class="bvd2-confirm">
          <input type="checkbox" id="bvd2Confirm">
          <span>
            تأیید می‌کنم که ${toFa(deletableCount)} سند برای همیشه حذف شوند.
            این عملیات <b>غیرقابل‌بازگشت</b> است.
          </span>
        </label>
      ` : '<div class="bvd2-note">هیچ سند قابل‌حذفی در این محدوده یافت نشد.</div>'}
      <div class="bvd2-note" data-tone="danger" data-error hidden></div>`;

    if (hasDeletable) {
      setActions([
        { id: 'bvd2Apply', label: 'تأیید و حذف', danger: true, onClick: runApply },
        {
          label: 'پیش‌نمایش مجدد',
          onClick: () => {
            if (state.busy) return;
            void runPreview();
          },
        },
        {
          label: 'بازگشت',
          onClick: () => {
            if (state.busy) return;
            state.preview = null;
            renderChoose();
          },
        },
      ]);
    } else {
      setActions([
        {
          label: 'پیش‌نمایش مجدد',
          onClick: () => {
            if (state.busy) return;
            void runPreview();
          },
        },
        {
          label: 'بازگشت',
          onClick: () => {
            if (state.busy) return;
            state.preview = null;
            renderChoose();
          },
        },
      ]);
    }

    const applyButton = toolbar.querySelector('#bvd2Apply');
    const confirmBox = scroll.querySelector('#bvd2Confirm');
    const backupBox = scroll.querySelector('#bvd2Backup');
    const syncApply = () => {
      if (applyButton) applyButton.disabled = !hasDeletable || !confirmBox?.checked;
    };
    confirmBox?.addEventListener('change', syncApply);
    backupBox?.addEventListener('change', () => {
      state.form.withBackup = backupBox.checked;
    });
    syncApply();
  };

  const renderDone = (result) => {
    renderStepper();
    scroll.innerHTML = `
      <div class="bvd2-done" role="status">
        <span class="bvd2-done-icon">✓</span>
        <h3>حذف اسناد انجام شد</h3>
        <p>${esc(result.message)}</p>
      </div>
      <dl class="bvd2-summary">
        <div><dt>سند حذف‌شده</dt><dd>${toFa(result.deletedCount)}</dd></div>
        ${Number(result.lockedCount)
          ? `<div><dt>دائمی (دست‌نخورده)</dt><dd>${toFa(result.lockedCount)}</dd></div>`
          : ''}
      </dl>
      ${result.withBackup && result.preDeleteBackupFile
        ? `<div class="bvd2-note" data-tone="ok">قبل از حذف، پشتیبان در مسیر
            <bdi dir="ltr">${esc(result.preDeleteBackupFile)}</bdi> ذخیره شد.</div>`
        : ''}`;
    setActions([], { showClose: true });
    toolbar.querySelector('button')?.focus();
  };

  const buildPresets = async () => {
    if (!state.year) {
      state.presets = [];
      return;
    }
    const year = Number(state.year);
    const month = Number(toEn(state.today).split('/')[1] || '1');
    const threeMonthsAgo = shiftMonth(year, month, -2);

    // Use the same Jalali calendar implementation as the Rust backend instead
    // of duplicating a 33-year leap-year approximation in JavaScript.
    let lastDayOfYear = 29;
    try {
      await invoke('validate_voucher_date', { date: `${state.year}/12/30` });
      lastDayOfYear = 30;
    } catch {
      // Esfand 30 is invalid in a non-leap year; Esfand 29 is always valid.
    }

    state.presets = [
      {
        id: 'last3',
        label: '۳ ماهه اخیر',
        from: `${threeMonthsAgo.year}/${pad2(threeMonthsAgo.month)}/01`,
        to: state.today,
      },
      {
        id: 'h1',
        label: `شش ماهه اول ${toFa(state.year)}`,
        from: `${state.year}/01/01`,
        to: `${state.year}/06/31`,
      },
      {
        id: 'full',
        label: 'کل سال',
        from: `${state.year}/01/01`,
        to: `${state.year}/12/${pad2(lastDayOfYear)}`,
      },
    ];
  };

  const init = async () => {
    renderStepper();
    scroll.innerHTML = '<div class="bvd2-note">در حال بارگذاری تنظیمات…</div>';
    setActions([]);

    if (!invoke) {
      scroll.innerHTML = '<div class="bvd2-note" data-tone="danger">این فرم فقط در نسخه دسکتاپ قابل استفاده است.</div>';
      setActions([]);
      say('ارتباط با برنامه برقرار نشد.', 'error');
      return;
    }

    try {
      const settings = await invoke('get_settings');
      state.today = settings?.today || '';
    } catch (error) {
      say(`دریافت تنظیمات ناموفق بود: ${errorText(error)}`, 'error');
    }

    const year = toEn(state.today).slice(0, 4);
    state.year = /^\d{4}$/.test(year) ? year : '';
    state.form.fromDate = state.year ? `${state.year}/01/01` : '';
    state.form.toDate = state.today;
    await buildPresets();
    renderChoose();
    say('محدوده حذف را انتخاب کنید.');
  };

  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      closeWindow();
    }
  });

  if (appWindow && typeof appWindow.onCloseRequested === 'function') {
    void appWindow.onCloseRequested((event) => {
      if (state.busy) event.preventDefault();
    });
  }

  void init();
})();
