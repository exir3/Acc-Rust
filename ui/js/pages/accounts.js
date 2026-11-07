'use strict';

/* ── Chart of accounts ─────────────────────────────────────── */

PAGES.accounts = {
  actions: () => [
    { label: 'تعریف و اصلاح حساب‌ها', icon: 'i-tree', primary: true, onClick: () => window.openAccountDefinitionPopup?.() },
    { label: 'حساب جدید سریع', icon: 'i-plus', onClick: () => openAccountDialog(null) },
    { label: 'گروه‌ها', icon: 'i-folder', onClick: () => openGroupsWindow() },
    { label: 'به‌روزرسانی', icon: 'i-refresh', onClick: () => navigate('accounts', { force: true }) },
  ],
  load: async (force) => {
    const [accounts, groups] = await Promise.all([ensureAccounts(force), ensureGroups(force)]);
    return { accounts, groups };
  },
  render: renderAccounts,
  bind: bindAccounts,
};

function renderAccounts(data) {
  const { accounts, groups } = data;
  const filters = state.accountFilters;
  const accountByCode = new Map(accounts.map((account) => [account.code, account]));
  const parentCodes = new Set(accounts.map((account) => account.parentCode).filter(Boolean));
  const groupOptions = groups.map((g) => `<option value="${esc(g.code)}"${g.code === filters.group ? ' selected' : ''}>${toFa(g.code)} — ${esc(g.name)}</option>`).join('');
  const drillChain = [];
  for (const code of state.accountDrillPath) {
    const expectedLevel = drillChain.length + 1;
    const parent = drillChain[drillChain.length - 1];
    const node = accountByCode.get(code);
    if (!node || Number(node.level) !== expectedLevel || (node.parentCode || '') !== (parent?.code || '')) break;
    drillChain.push(node);
  }
  if (drillChain.length !== state.accountDrillPath.length) {
    state.accountDrillPath = drillChain.map((account) => account.code);
  }
  const currentParent = drillChain[drillChain.length - 1] || null;
  const search = normalizeSearch(filters.search);
  const hasGlobalFilter = Boolean(search || filters.group || filters.level);
  const filtered = accounts.filter((account) => {
    if (!hasGlobalFilter && (currentParent
      ? account.parentCode !== currentParent.code
      : Number(account.level) !== 1)) return false;
    if (search || filters.group || filters.level) {
      if (filters.group && account.groupCode !== filters.group) return false;
      if (filters.level && Number(account.level) !== Number(filters.level)) return false;
    }
    const haystack = normalizeSearch(`${account.code} ${account.name} ${account.groupName} ${account.parentName} ${account.effectiveFsLineTitle || ''}`);
    if (search && !haystack.includes(search)) return false;
    return true;
  });

  const rows = filtered.map((account, index) => {
    const hasChildren = parentCodes.has(account.code);
    return `
    <tr>
      <td class="center row-index">${toFa(index + 1)}</td>
      <td class="center mono">${toFa(account.code)}</td>
      <td class="desc-cell"><b>${esc(account.name)}</b></td>
      <td class="center"><span class="level-pill ${levelClass(account.level)}">${levelLabel(account.level)}</span></td>
      <td>${esc(account.groupName)}</td>
      <td class="center">${badge(natureLabel(account.nature), account.nature === 'creditor' ? 'warn' : account.nature === 'debtor' ? 'info' : 'neutral')}</td>
      <td class="muted" title="${esc(account.effectiveFsLine || '')}">${esc(account.effectiveFsLineTitle) || '—'}</td>
      <td class="muted">${esc(account.parentName) || '—'}</td>
      <td class="center">${account.isPostable ? badge('قابل ثبت', 'ok') : badge('غیرقابل ثبت', 'neutral')}</td>
      <td class="center nowrap">
        ${hasChildren ? `<button type="button" class="btn btn-sm account-drill-btn" data-drill-account="${esc(account.code)}" title="نمایش زیرحساب‌ها">${icon('i-chevron')}</button>` : ''}
        <button type="button" class="btn btn-sm" data-edit-account="${account.id}" title="ویرایش حساب">${icon('i-edit')}</button>
        <button type="button" class="btn btn-sm" data-delete-account="${account.id}" data-account-label="${esc(account.name)}" data-account-code="${esc(account.code)}" title="حذف حساب">${icon('i-trash')}</button>
      </td>
    </tr>`;
  }).join('');
  const pathButtons = [
    `<button type="button" class="account-path-btn" data-account-path="" ${drillChain.length === 0 ? 'aria-current="location" disabled' : ''}>حساب‌های کل</button>`,
    ...drillChain.map((account, index) => {
      const path = drillChain.slice(0, index + 1).map((item) => item.code).join(',');
      return `<span aria-hidden="true">‹</span><button type="button" class="account-path-btn" data-account-path="${esc(path)}" ${index === drillChain.length - 1 ? 'aria-current="location" disabled' : ''}>${toFa(account.code)} - ${esc(account.name)}</button>`;
    }),
  ].join('');

  return `
  <form class="filter-bar" data-filter="accountFilters">
    <div class="field grow">
      <label for="acSearch">جست‌وجو</label>
      <input id="acSearch" class="input" name="search" type="search" value="${esc(filters.search)}" placeholder="کد، نام حساب، گروه یا حساب بالاتر…" />
    </div>
    <div class="field">
      <label for="acGroup">گروه حساب</label>
      <select id="acGroup" class="input wide" name="group">
        <option value="">همه گروه‌ها</option>
        ${groupOptions}
      </select>
    </div>
    <div class="field">
      <label for="acLevel">سطح</label>
      <select id="acLevel" class="input" name="level">
        <option value="">همه سطوح</option>
        ${[1, 2, 3].map((level) => `<option value="${level}"${String(filters.level) === String(level) ? ' selected' : ''}>${levelLabel(level)}</option>`).join('')}
      </select>
    </div>
    <button class="btn btn-primary" type="submit">${icon('i-search')}<span>فیلتر</span></button>
    <button class="btn" type="button" data-reset>${icon('i-refresh')}<span>بازنشانی</span></button>
  </form>

  <nav class="account-browser-path" aria-label="مسیر حساب‌ها">
    <div class="account-path-items">${pathButtons}</div>
    <div class="account-path-meta">${hasGlobalFilter ? 'نتایج فیلتر در همه سطح‌ها' : `سطح ${levelLabel(currentParent ? Number(currentParent.level) + 1 : 1)}`} · ${toFa(filtered.length)} حساب</div>
  </nav>

  <div class="card">
    <div class="section-head">
      <h2>${icon('i-tree')} نمودار حساب‌ها</h2>
      <span class="badge" data-tone="info">${toFa(filtered.length)} حساب در این سطح</span>
    </div>
    ${filtered.length ? `
      <div class="table-wrap">
        <table class="data-table">
          <thead><tr>
            <th style="width:52px">ردیف</th><th style="width:96px">کد حساب</th><th>نام حساب</th><th style="width:92px">سطح</th>
            <th style="width:180px">گروه</th><th style="width:92px">ماهیت</th>
            <th style="width:190px">سرفصل صورت‌های مالی</th>
            <th style="width:180px">حساب بالاتر</th><th style="width:96px">ثبت سند</th><th style="width:104px">عملیات</th>
          </tr></thead>
          <tbody>${rows}</tbody>
        </table>
      </div>` : `<div class="empty-state">
        ${icon('i-tree')}
        <b>${hasGlobalFilter ? 'حسابی با این فیلتر یافت نشد' : currentParent ? 'زیرحسابی در این سطح تعریف نشده است' : 'حساب کلی تعریف نشده است'}</b>
        <span>${hasGlobalFilter ? 'فیلترها را تغییر دهید یا بازنشانی کنید.' : 'برای ایجاد حساب در این سطح، حساب جدید را تعریف کنید.'}</span>
        <button type="button" class="btn btn-primary" data-new-account>${icon('i-plus')}<span>حساب جدید</span></button>
      </div>`}
  </div>`;
}

function bindAccounts(root) {
  root.querySelectorAll('[data-account-path]').forEach((button) => {
    button.addEventListener('click', () => {
      state.accountDrillPath = button.dataset.accountPath ? button.dataset.accountPath.split(',') : [];
      navigate('accounts', { force: true });
    });
  });
  root.querySelectorAll('[data-drill-account]').forEach((button) => {
    button.addEventListener('click', () => {
      const account = state.accounts.find((item) => item.code === button.dataset.drillAccount);
      state.accountDrillPath = account ? accountPathTo(state.accounts, account) : [];
      state.accountFilters = { ...state.accountFilters, search: '', level: '' };
      navigate('accounts', { force: true });
    });
  });
  root.querySelectorAll('tbody tr').forEach((row) => {
    row.addEventListener('dblclick', (event) => {
      if (event.target.closest('button')) return;
      const codeCell = row.querySelector('.mono');
      const code = codeCell ? toEn(codeCell.textContent) : '';
      const hasChildren = state.accounts.some((account) => account.parentCode === code);
      if (!hasChildren) return;
      const account = state.accounts.find((item) => item.code === code);
      state.accountDrillPath = account ? accountPathTo(state.accounts, account) : [];
      state.accountFilters = { ...state.accountFilters, search: '', level: '' };
      navigate('accounts', { force: true });
    });
  });
  // «حساب جدید» buttons are wired once by bindPageCommon.
  root.querySelectorAll('[data-edit-account]').forEach((button) => {
    const id = Number(button.dataset.editAccount);
    const account = state.accounts.find((a) => Number(a.id) === id);
    button.addEventListener('click', () => openAccountDialog(account || null));
  });
  root.querySelectorAll('[data-delete-account]').forEach((button) => {
    button.addEventListener('click', async () => {
      const id = Number(button.dataset.deleteAccount);
      const label = button.dataset.accountLabel || 'این حساب';
      const code = button.dataset.accountCode || '';
      const confirmed = await confirmDialog(
        'حذف حساب',
        `آیا از حذف حساب «${toFa(code)} — ${label}» مطمئن هستید؟ حسابی که در اسناد استفاده شده باشد قابل حذف نیست.`
      );
      if (!confirmed) return;
      try {
        await invoke('delete_account', { id });
        toast('حساب حذف شد.', 'ok');
        state.accountsLoaded = false;
        await navigate('accounts', { force: true });
      } catch (error) {
        toast(errorText(error), 'error');
      }
    });
  });
}

