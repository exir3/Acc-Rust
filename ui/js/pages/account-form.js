'use strict';

/* ── Account form ───────────────────────────────────────────── */

let fsCatalogCache = null;
let accountDialogPreparing = false;

async function ensureFsCatalog() {
  if (fsCatalogCache) return fsCatalogCache;
  try {
    const payload = await invoke('list_fs_lines');
    if (payload && Array.isArray(payload.lines) && payload.lines.length) {
      fsCatalogCache = payload;
    }
  } catch (error) {
    // The classification selector is an enhancement: a catalog failure must
    // not stop the user from editing the basic account fields.
  }
  return fsCatalogCache;
}

const CLASSIFICATION_SOURCE_LABELS = {
  seed: 'داده اولیه',
  inherit: 'ارث از حساب بالاتر',
  inherited: 'ارث از حساب بالاتر',
  auto: 'پیشنهاد گروه',
  manual: 'تعیین دستی',
};

function fsLineOptions(catalog, selected) {
  const statements = [
    { key: 'balance_sheet', label: 'صورت وضعیت مالی' },
    { key: 'income_statement', label: 'صورت سود و زیان' },
    { key: 'memorandum', label: 'حساب‌های انتظامی' },
  ];
  return statements.map((statement) => {
    const lines = catalog.lines.filter((line) => line.statement === statement.key);
    if (!lines.length) return '';
    return `<optgroup label="${statement.label}">${lines.map((line) => (
      `<option value="${esc(line.code)}"${line.code === selected ? ' selected' : ''}>${esc(line.code)} — ${esc(line.title)}</option>`
    )).join('')}</optgroup>`;
  }).join('');
}

function optionList(options, selected, emptyLabel) {
  return `<option value="">${esc(emptyLabel)}</option>${options.map((option) => (
    `<option value="${esc(option.value)}"${option.value === selected ? ' selected' : ''}>${esc(option.label)}</option>`
  )).join('')}`;
}

async function openAccountDialog(account) {
  if (accountDialogPreparing) return;
  const isEdit = Boolean(account);
  const groups = state.groups;
  if (!groups.length) {
    toast('ابتدا حداقل یک گروه حساب تعریف کنید.', 'info');
    return;
  }
  let catalog;
  if (fsCatalogCache) {
    catalog = await ensureFsCatalog();
  } else {
    accountDialogPreparing = true;
    toast('در حال آماده‌سازی فرم حساب…', 'info');
    try {
      catalog = await ensureFsCatalog();
    } finally {
      accountDialogPreparing = false;
    }
  }

  const hierarchy = state.accountDrillPath
    .map((code) => state.accounts.find((item) => item.code === code))
    .filter(Boolean);
  const requestedLevel = Number(state.accountFilters.level);
  const level = account
    ? Number(account.level)
    : requestedLevel || Math.min(3, hierarchy.length + 1);
  const initialParentCode = account?.parentCode || (level > 1 ? hierarchy[level - 2]?.code || '' : '');
  const initialGroupCode = account?.groupCode || state.accountFilters.group || (level > 1
    ? hierarchy[level - 2]?.groupCode || groups[0]?.code
    : groups[0]?.code);
  const groupOptions = groups.map((g) => `<option value="${esc(g.code)}"${initialGroupCode === g.code ? ' selected' : ''}>${toFa(g.code)} — ${esc(g.name)} (${esc(g.type)})</option>`).join('');
  const nature = account ? account.nature : 'debtor';
  const suggestedCode = account?.code || nextAccountCode(state.accounts, level, initialParentCode);

  const contraTargets = catalog
    ? state.accounts
      .filter((item) => !account || Number(item.id) !== Number(account.id))
      .map((item) => ({ value: item.code, label: `${toFa(item.code)} — ${item.name}` }))
    : [];
  const classificationPanel = catalog ? `
        <fieldset class="acct-classification">
          <legend>طبقه‌بندی صورت‌های مالی</legend>
          <div class="field wide">
            <label for="acctFsLine">سرفصل صورت‌های مالی</label>
            <select id="acctFsLine" class="input">
              <option value="">خودکار — از حساب بالاتر یا گروه</option>
              ${fsLineOptions(catalog, account?.fsLine || '')}
            </select>
          </div>
          <div class="field">
            <label for="acctSystemRole">نقش سیستمی</label>
            <select id="acctSystemRole" class="input">
              ${optionList(catalog.systemRoles || [], account?.systemRole || '', '— بدون نقش —')}
            </select>
          </div>
          <div class="field">
            <label for="acctContraOf">حساب کاهنده از</label>
            <select id="acctContraOf" class="input">
              ${optionList(contraTargets, account?.contraOf || '', '— حساب کاهنده نیست —')}
            </select>
          </div>
          <div class="field">
            <label for="acctContraKind">نوع کاهنده</label>
            <select id="acctContraKind" class="input">
              ${optionList(catalog.contraKinds || [], account?.contraKind || '', '— بدون نوع —')}
            </select>
          </div>
          <div class="field">
            <label for="acctOppositeFsLine">سرفصل مقابل (تفکیک علامت)</label>
            <select id="acctOppositeFsLine" class="input">
              <option value="">— بدون سرفصل مقابل —</option>
              ${fsLineOptions(catalog, account?.oppositeFsLine || '')}
            </select>
          </div>
          <div class="field">
            <label>&nbsp;</label>
            <label class="check-field" style="padding-top:0">
              <input id="acctSplitBySign" type="checkbox"${account?.splitBySign ? ' checked' : ''} />
              <span>تفکیک بر اساس علامت مانده</span>
            </label>
          </div>
          ${account?.effectiveFsLineTitle ? `<div class="form-note acct-effective">سرفصل مؤثر فعلی: <b>${esc(account.effectiveFsLineTitle)}</b> — ${esc(account.effectiveSection || '')} (${esc(CLASSIFICATION_SOURCE_LABELS[account.classificationSource] || account.classificationSource || '—')})</div>` : ''}
        </fieldset>` : '';

  const dialog = openDialog({
    title: isEdit ? 'ویرایش حساب' : 'تعریف حساب جدید',
    wide: true,
    body: `
      <form id="acctForm" class="acct-form" autocomplete="off">
        <div class="field">
          <label for="acctCode">کد حساب <span class="req">*</span></label>
          <input id="acctCode" class="input mono" type="text" inputmode="numeric" maxlength="50" value="${esc(toFa(suggestedCode))}" ${isEdit ? 'readonly' : ''} placeholder="کد پیشنهادی بر اساس حساب بالاتر" />
        </div>
        <div class="field wide">
          <label for="acctName">نام حساب <span class="req">*</span></label>
          <input id="acctName" class="input" type="text" value="${esc(account?.name || '')}" placeholder="نام حساب" />
        </div>
        <div class="field">
          <label for="acctLevel">سطح حساب</label>
          <select id="acctLevel" class="input" ${isEdit ? 'disabled' : ''}>
            ${(isEdit ? [Number(account.level)] : [1, 2, 3]).map((value) => `<option value="${value}"${Number(level) === value ? ' selected' : ''}>${levelLabel(value)}</option>`).join('')}
          </select>
        </div>
        <div class="field wide">
          <label for="acctGroup">گروه حساب <span class="req">*</span></label>
          <select id="acctGroup" class="input" ${isEdit ? 'disabled' : ''}>${groupOptions}</select>
        </div>
        <div class="field">
          <label for="acctNature">ماهیت حساب</label>
          <select id="acctNature" class="input">
            <option value="debtor"${nature === 'debtor' ? ' selected' : ''}>بدهکار</option>
            <option value="creditor"${nature === 'creditor' ? ' selected' : ''}>بستانکار</option>
            <option value="dual"${nature === 'dual' ? ' selected' : ''}>دوگانه</option>
          </select>
        </div>
        <div class="field wide">
          <label for="acctParent">حساب بالاتر</label>
          <select id="acctParent" class="input"></select>
        </div>
        <div class="field">
          <label>&nbsp;</label>
          <label class="check-field" style="padding-top:0">
            <input id="acctPostable" type="checkbox" ${Number(level) <= 2 ? 'checked' : ''} disabled />
            <span>قابل ثبت در سند (کل و معین)</span>
          </label>
        </div>
        ${classificationPanel}
        <div class="form-error" id="acctError" hidden></div>
      </form>
      <div class="form-note">
        کد پیشنهادی از روی آخرین هم‌سطح تولید می‌شود و برای حساب‌های پایین‌تر با کد حساب بالاتر آغاز می‌شود. کد حساب پس از ثبت قابل تغییر نیست؛ گروه حساب سطوح پایین‌تر از حساب بالاتر گرفته می‌شود.
      </div>`,
    actions: [
      {
        label: isEdit ? 'ذخیره تغییرات' : 'ثبت حساب',
        icon: 'i-save',
        primary: true,
        onClick: async (ref, button) => {
          await submitAccountForm(ref, button, account);
        },
      },
      { label: 'انصراف', onClick: () => closeDialog() },
    ],
  });

  if (!dialog) return;
  const codeInput = $('#acctCode', dialog.body);
  const nameInput = $('#acctName', dialog.body);
  const levelSelect = $('#acctLevel', dialog.body);
  const groupSelect = $('#acctGroup', dialog.body);
  const parentSelect = $('#acctParent', dialog.body);
  const natureSelect = $('#acctNature', dialog.body);
  const postable = $('#acctPostable', dialog.body);

  if (codeInput) {
    codeInput.addEventListener('input', () => {
      clearFieldError(codeInput);
      const cleaned = digitsOnly(codeInput.value);
      if (codeInput.value !== cleaned) codeInput.value = cleaned;
    });
  }
  nameInput?.addEventListener('input', () => clearFieldError(nameInput));
  levelSelect?.addEventListener('change', () => clearFieldError(levelSelect));
  groupSelect?.addEventListener('change', () => clearFieldError(groupSelect));
  parentSelect?.addEventListener('change', () => clearFieldError(parentSelect));
  natureSelect?.addEventListener('change', () => clearFieldError(natureSelect));
  ['#acctFsLine', '#acctSystemRole', '#acctContraOf', '#acctContraKind', '#acctOppositeFsLine', '#acctSplitBySign']
    .forEach((selector) => {
      const control = $(selector, dialog.body);
      control?.addEventListener('change', () => clearFieldError(control));
    });

  const refreshParents = () => {
    if (!parentSelect) return;
    const currentLevel = Number(levelSelect?.value || level);
    const groupCode = groupSelect?.value || '';
    if (currentLevel <= 1) {
      parentSelect.innerHTML = '<option value="">— حساب سطح کل است —</option>';
      parentSelect.disabled = true;
      // A level-1 (کل) account is postable in WebApp, just like level 2.
      if (groupSelect) groupSelect.disabled = false;
      if (postable) postable.checked = true;
      if (postable) postable.disabled = true;
      if (!isEdit && codeInput) codeInput.value = toFa(nextAccountCode(state.accounts, currentLevel));
      return;
    }
    // Match WebApp: the account's group and direct parent can be changed;
    // the backend validates the new branch and blocks unsafe historical moves.
    parentSelect.disabled = false;
    if (groupSelect) groupSelect.disabled = false;
    if (postable) postable.disabled = true;
    if (postable) postable.checked = currentLevel <= 2;
    const candidates = state.accounts.filter((a) => Number(a.level) === currentLevel - 1 && a.groupCode === groupCode);
    const selectedParent = parentSelect.value;
    const selectedStillValid = candidates.some((a) => a.code === selectedParent);
    const previous = selectedStillValid
      ? selectedParent
      : (isEdit
        ? (groupCode === account?.groupCode ? account?.parentCode || '' : '')
        : initialParentCode);
    // Preserve an inactive current parent only when it still belongs to the
    // selected group and is exactly one level above the edited account.
    if (isEdit && previous && !candidates.some((a) => a.code === previous)) {
      const existing = state.accounts.find((a) => a.code === previous);
      if (existing && existing.groupCode === groupCode && Number(existing.level) === currentLevel - 1) candidates.unshift(existing);
    }
    // eslint-disable-next-line no-unsanitized/property -- option values and labels are escaped
    parentSelect.innerHTML = candidates.length
      ? `<option value="">— انتخاب حساب بالاتر —</option>${candidates.map((a) => `<option value="${esc(a.code)}"${a.code === previous ? ' selected' : ''}>${toFa(a.code)} — ${esc(a.name)}</option>`).join('')}`
      : '<option value="">— حسابی در سطح بالاتر وجود ندارد —</option>';
    if (isEdit) {
      parentSelect.value = previous;
    } else {
      if (candidates.length && !parentSelect.value) parentSelect.value = candidates[0].code;
      if (codeInput) codeInput.value = toFa(nextAccountCode(state.accounts, currentLevel, parentSelect.value));
    }
  };

  levelSelect?.addEventListener('change', refreshParents);
  groupSelect?.addEventListener('change', refreshParents);
  parentSelect?.addEventListener('change', () => {
    if (!isEdit && codeInput) codeInput.value = toFa(nextAccountCode(state.accounts, Number(levelSelect?.value || level), parentSelect.value));
  });
  refreshParents();

  setTimeout(() => (isEdit ? $('#acctName', dialog.body) : codeInput)?.focus(), 40);
}

async function submitAccountForm(ref, button, account) {
  const errorBox = $('#acctError', ref.body);
  const codeInput = $('#acctCode', ref.body);
  const nameInput = $('#acctName', ref.body);
  const levelSelect = $('#acctLevel', ref.body);
  const groupSelect = $('#acctGroup', ref.body);
  const natureSelect = $('#acctNature', ref.body);
  const parentSelect = $('#acctParent', ref.body);
  const fsLineSelect = $('#acctFsLine', ref.body);
  const systemRoleSelect = $('#acctSystemRole', ref.body);
  const contraOfSelect = $('#acctContraOf', ref.body);
  const contraKindSelect = $('#acctContraKind', ref.body);
  const oppositeFsLineSelect = $('#acctOppositeFsLine', ref.body);
  const splitBySignCheck = $('#acctSplitBySign', ref.body);
  const hasClassification = Boolean(fsLineSelect);

  const showError = (message) => {
    if (!errorBox) return;
    errorBox.textContent = message;
    errorBox.hidden = false;
  };

  const focusInvalid = (input) => {
    if (!input || typeof input.focus !== 'function') return;
    input.focus({ preventScroll: true });
    input.scrollIntoView({ block: 'center' });
  };

  const fail = (input, message) => {
    showError(message);
    if (input) setFieldError(input, message);
    focusInvalid(input);
    return false;
  };

  const clearFormFieldErrors = () => {
    [codeInput, nameInput, levelSelect, groupSelect, natureSelect, parentSelect,
      fsLineSelect, systemRoleSelect, contraOfSelect, contraKindSelect,
      oppositeFsLineSelect, splitBySignCheck].forEach((control) => {
      if (control) clearFieldError(control);
    });
  };

  clearFormFieldErrors();
  if (errorBox) errorBox.hidden = true;

  const code = digitsOnly(codeInput?.value || '');
  const name = (nameInput?.value || '').trim().replace(/\s+/g, ' ');
  const level = Number(levelSelect?.value || 1);
  const groupCode = groupSelect?.value || '';
  const nature = natureSelect?.value || 'debtor';
  const parentCode = level <= 1 ? '' : (parentSelect?.value || '');
  const isPostable = level <= 2;
  const fsLine = fsLineSelect?.value || '';
  const systemRole = systemRoleSelect?.value || '';
  const contraOf = contraOfSelect?.value || '';
  const contraKind = contraKindSelect?.value || '';
  const splitBySign = Boolean(splitBySignCheck?.checked);
  const oppositeFsLine = splitBySign ? (oppositeFsLineSelect?.value || '') : '';

  if (!account && !code) return fail(codeInput, 'کد حساب الزامی است.');
  if (!name) return fail(nameInput, 'نام حساب الزامی است.');
  if (!groupCode) return fail(groupSelect, 'گروه حساب را انتخاب کنید.');
  if (level > 1 && !parentCode) return fail(parentSelect, 'برای این سطح، انتخاب حساب بالاتر الزامی است.');
  if (contraOf && !contraKind) {
    showError('برای حساب کاهنده، انتخاب نوع کاهنده الزامی است.');
    setFieldError(contraOfSelect, 'حساب کاهنده انتخاب شده است.');
    setFieldError(contraKindSelect, 'نوع کاهنده را انتخاب کنید.');
    focusInvalid(contraKindSelect);
    return;
  }
  if (!contraOf && contraKind) {
    showError('نوع کاهنده بدون انتخاب حساب کاهنده مجاز نیست.');
    setFieldError(contraKindSelect, 'نوع کاهنده بدون حساب کاهنده مجاز نیست.');
    setFieldError(contraOfSelect, 'حساب کاهنده را انتخاب کنید.');
    focusInvalid(contraOfSelect);
    return;
  }
  if (splitBySign && !oppositeFsLine) {
    return fail(oppositeFsLineSelect, 'برای تفکیک بر اساس علامت، انتخاب سرفصل مقابل الزامی است.');
  }
  if (oppositeFsLine && fsLine && oppositeFsLine === fsLine) {
    showError('سرفصل مقابل باید با سرفصل اصلی متفاوت باشد.');
    setFieldError(fsLineSelect, 'سرفصل اصلی است.');
    setFieldError(oppositeFsLineSelect, 'سرفصل مقابل باید متفاوت باشد.');
    focusInvalid(oppositeFsLineSelect);
    return;
  }

  const classification = hasClassification ? {
    fsLine,
    contraOf,
    contraKind,
    oppositeFsLine,
    splitBySign,
    systemRole,
  } : null;

  if (errorBox) errorBox.hidden = true;
  if (button) button.disabled = true;

  try {
    if (account) {
      await invoke('update_account', {
        id: account.id,
        name,
        groupCode,
        nature,
        parentCode,
        ...(classification || {}),
      });
      toast('حساب با موفقیت ویرایش شد.', 'ok');
    } else {
      await invoke('create_account', {
        code,
        name,
        level,
        groupCode,
        nature,
        parentCode,
        isPostable,
        ...(classification || {}),
      });
      toast('حساب جدید ثبت شد.', 'ok');
    }
    closeDialog();
    state.accountsLoaded = false;
    await navigate('accounts', { force: true });
  } catch (error) {
    showError(errorText(error));
    if (button) button.disabled = false;
  }
}

