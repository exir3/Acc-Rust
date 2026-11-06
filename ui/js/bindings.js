'use strict';

/* ── Shared event binding ───────────────────────────────────── */

function bindFilterOnly() {
  // Filter forms and generic navigation are handled by bindPageCommon.
}

function bindPageCommon(root) {
  root.querySelectorAll('[data-go]').forEach((button) => {
    button.addEventListener('click', () => navigate(button.dataset.go));
  });
  root.querySelectorAll('[data-open-groups]').forEach((button) => {
    button.addEventListener('click', () => openGroupsWindow());
  });
  root.querySelectorAll('[data-new-voucher]').forEach((button) => {
    button.addEventListener('click', () => openVoucherWindow());
  });
  root.querySelectorAll('[data-open-voucher-list]').forEach((button) => {
    button.addEventListener('click', () => openVoucherListWindow());
  });
  root.querySelectorAll('[data-open-voucher]').forEach((button) => {
    button.addEventListener('click', () => openVoucherWindow(Number(button.dataset.openVoucher)));
  });
  root.querySelectorAll('[data-new-account]').forEach((button) => {
    button.addEventListener('click', () => openAccountDialog(null));
  });

  // Digits and date input hygiene.
  root.querySelectorAll('input[data-date]').forEach((input) => {
    input.addEventListener('input', () => {
      clearFieldError(input);
      const cleaned = toEn(input.value).replace(/[^\d/]/g, '');
      if (input.value !== cleaned) input.value = cleaned;
    });
  });

  // Filter forms.
  root.querySelectorAll('form[data-filter]').forEach((form) => {
    const key = form.dataset.filter;
    form.addEventListener('submit', (event) => {
      event.preventDefault();
      if (!validateDateFields(form)) return;
      const data = new FormData(form);
      if (key === 'accountFilters') {
        state.accountFilters = {
          search: String(data.get('search') || ''),
          group: String(data.get('group') || ''),
          level: String(data.get('level') || ''),
        };
        navigate('accounts', { force: true });
        return;
      }
      const target = state.filters[key];
      if (!target) return;
      if (state.listOffsets[key] !== undefined) state.listOffsets[key] = 0;
      for (const [name, value] of data.entries()) {
        target[name] = String(value);
      }
      navigate(state.page, { force: true });
    });

    const reset = form.querySelector('[data-reset]');
    if (reset) {
      reset.addEventListener('click', () => {
        if (key === 'accountFilters') {
          state.accountFilters = { search: '', group: '', level: '' };
          navigate('accounts', { force: true });
          return;
        }
        const defaults = defaultFilters()[key];
        if (defaults) Object.assign(state.filters[key], defaults);
        if (state.listOffsets[key] !== undefined) state.listOffsets[key] = 0;
        navigate(state.page, { force: true });
      });
    }
  });
}

