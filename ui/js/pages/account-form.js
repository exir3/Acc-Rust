'use strict';

/* ── Account form ───────────────────────────────────────────── */

function openAccountDialog(account) {
  const isEdit = Boolean(account);
  const groups = state.groups;
  if (!groups.length) {
    toast('ابتدا حداقل یک گروه حساب تعریف کنید.', 'info');
    return;
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
          <select id="acctGroup" class="input">${groupOptions}</select>
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
            <input id="acctPostable" type="checkbox" ${account?.isPostable ? 'checked' : ''} ${Number(level) === 1 ? 'disabled' : ''} />
            <span>قابل ثبت در سند</span>
          </label>
        </div>
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
  const levelSelect = $('#acctLevel', dialog.body);
  const groupSelect = $('#acctGroup', dialog.body);
  const parentSelect = $('#acctParent', dialog.body);
  const postable = $('#acctPostable', dialog.body);

  if (codeInput) {
    codeInput.addEventListener('input', () => {
      const cleaned = digitsOnly(codeInput.value);
      if (codeInput.value !== cleaned) codeInput.value = cleaned;
    });
  }

  const refreshParents = () => {
    if (!parentSelect) return;
    const currentLevel = Number(levelSelect?.value || level);
    const groupCode = groupSelect?.value || '';
    if (currentLevel <= 1) {
      parentSelect.innerHTML = '<option value="">— حساب سطح کل است —</option>';
      parentSelect.disabled = true;
      if (groupSelect) groupSelect.disabled = false;
      if (postable) postable.checked = false;
      if (postable) postable.disabled = true;
      if (!isEdit && codeInput) codeInput.value = toFa(nextAccountCode(state.accounts, currentLevel));
      return;
    }
    parentSelect.disabled = false;
    if (groupSelect) groupSelect.disabled = !isEdit;
    if (postable) postable.disabled = false;
    const candidates = state.accounts.filter((a) => Number(a.level) === currentLevel - 1 && a.groupCode === groupCode);
    const previous = parentSelect.value || (account && Number(account.level) === currentLevel
      ? account.parentCode
      : initialParentCode);
    parentSelect.innerHTML = candidates.length
      ? `<option value="">— انتخاب حساب بالاتر —</option>${candidates.map((a) => `<option value="${esc(a.code)}"${a.code === previous ? ' selected' : ''}>${toFa(a.code)} — ${esc(a.name)}</option>`).join('')}`
      : '<option value="">— حسابی در سطح بالاتر وجود ندارد —</option>';
    if (candidates.length && !parentSelect.value) parentSelect.value = candidates[0].code;
    if (!isEdit && codeInput) codeInput.value = toFa(nextAccountCode(state.accounts, currentLevel, parentSelect.value));
  };

  levelSelect?.addEventListener('change', refreshParents);
  groupSelect?.addEventListener('change', refreshParents);
  parentSelect?.addEventListener('change', () => {
    if (!isEdit && codeInput) codeInput.value = toFa(nextAccountCode(state.accounts, Number(levelSelect?.value || level), parentSelect.value));
  });
  refreshParents();
  if (account?.parentCode && parentSelect) parentSelect.value = account.parentCode;

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
  const postable = $('#acctPostable', ref.body);

  const showError = (message) => {
    if (!errorBox) return;
    errorBox.textContent = message;
    errorBox.hidden = false;
  };

  const code = digitsOnly(codeInput?.value || '');
  const name = (nameInput?.value || '').trim().replace(/\s+/g, ' ');
  const level = Number(levelSelect?.value || 1);
  const groupCode = groupSelect?.value || '';
  const nature = natureSelect?.value || 'debtor';
  const parentCode = level <= 1 ? '' : (parentSelect?.value || '');
  const isPostable = level > 1 && Boolean(postable?.checked);

  if (!account && !code) return showError('کد حساب الزامی است.');
  if (!name) return showError('نام حساب الزامی است.');
  if (!groupCode) return showError('گروه حساب را انتخاب کنید.');
  if (level > 1 && !parentCode) return showError('برای این سطح، انتخاب حساب بالاتر الزامی است.');

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
        isPostable,
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

