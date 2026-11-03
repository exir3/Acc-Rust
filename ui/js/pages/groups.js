'use strict';

/* ── Groups overview ────────────────────────────────────────── */

PAGES.groups = {
  actions: () => [
    { label: 'مدیریت گروه‌ها', icon: 'i-folder', primary: true, onClick: () => openGroupsWindow() },
    { label: 'به‌روزرسانی', icon: 'i-refresh', onClick: () => navigate('groups', { force: true }) },
  ],
  load: async (force) => {
    const groups = await ensureGroups(force);
    return groups;
  },
  render: renderGroups,
  bind: bindFilterOnly,
};

function renderGroups(groups) {
  const rows = groups.map((group) => `
    <tr>
      <td class="center mono">${toFa(group.code)}</td>
      <td><b>${esc(group.name)}</b></td>
      <td class="center">${badge(group.type, typeTone(group.type))}</td>
      <td class="center">${toFa(group.accountCount)}</td>
      <td class="center muted">${faDate(group.updatedAt)}</td>
      <td class="center">
        <button type="button" class="btn btn-sm" data-open-groups title="مدیریت گروه‌ها">${icon('i-edit')}</button>
      </td>
    </tr>`).join('');

  return `
  <div class="info-strip">
    <span>گروه‌های حساب، دسته‌بندی اصلی نمودار حساب‌ها هستند و در گزارش‌های مالی تعیین‌کننده طبقه هر حساب‌اند.</span>
    <span class="sep"></span>
    <b>${toFa(groups.length)} گروه</b>
  </div>
  <div class="card">
    <div class="section-head">
      <h2>${icon('i-folder')} گروه‌های حساب</h2>
      <span class="badge" data-tone="info">${toFa(groups.reduce((sum, g) => sum + Number(g.accountCount || 0), 0))} حساب</span>
    </div>
    ${groups.length ? `
      <div class="table-wrap">
        <table class="data-table">
          <thead><tr>
            <th style="width:96px">کد گروه</th><th>نام گروه</th><th style="width:140px">نوع</th>
            <th style="width:110px">تعداد حساب</th><th style="width:140px">آخرین تغییر</th><th style="width:80px">عملیات</th>
          </tr></thead>
          <tbody>${rows}</tbody>
        </table>
      </div>` : `<div class="empty-state">
        ${icon('i-folder')}
        <b>گروه حسابی وجود ندارد</b>
        <span>برای شروع، حداقل یک گروه حساب تعریف کنید.</span>
        <button type="button" class="btn btn-primary" data-open-groups>${icon('i-plus')}<span>مدیریت گروه‌ها</span></button>
      </div>`}
  </div>`;
}

