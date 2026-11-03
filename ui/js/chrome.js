'use strict';

/* ── Windows and chrome ─────────────────────────────────────── */

async function openVoucherWindow(number = null) {
  if (!invoke) return;
  try {
    await invoke('open_voucher_window', { number: number || null });
    state.refreshOnFocus = true;
  } catch (error) {
    toast(errorText(error), 'error');
  }
}

async function openVoucherListWindow() {
  if (!invoke) return;
  try {
    await invoke('open_voucher_list_window');
    state.refreshOnFocus = true;
  } catch (error) {
    toast(errorText(error), 'error');
  }
}

async function openTrialBalanceWindow() {
  if (!invoke) return;
  try {
    await invoke('open_trial_balance_window');
    state.refreshOnFocus = true;
  } catch (error) {
    toast(errorText(error), 'error');
  }
}

async function openGroupsWindow() {
  if (!invoke) return;
  try {
    await invoke('open_account_groups_window');
    state.refreshOnFocus = true;
  } catch (error) {
    toast(errorText(error), 'error');
  }
}

function bindChrome() {
  if (!IS_VOUCHER_LIST_POPUP) {
    const homeButton = $('[data-workspace-home]');
    homeButton?.addEventListener('click', () => void navigate('workspace'));
    homeButton?.addEventListener('keydown', (event) => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        void navigate('workspace');
      }
    });
  }
  $('#wallpaperPicker')?.addEventListener('change', (event) => {
    chooseWallpaper(event.target.files?.[0]);
    event.target.value = '';
  });
  window.addEventListener('account-definition-opened', () => {
    state.refreshOnFocus = true;
  });

  window.addEventListener('accounts-changed', () => {
    state.accountsLoaded = false;
    state.groupsLoaded = false;
    if (state.page === 'accounts') void navigate('accounts', { force: true });
  });
  const eventApi = window.__TAURI__?.event;
  if (typeof eventApi?.listen === 'function') {
    void eventApi.listen('accounts-changed', () => {
      window.dispatchEvent(new Event('accounts-changed'));
    }).catch(() => {});
  }

  // The bulk-delete form runs in its own window; when it finishes, any open
  // voucher list or voucher-derived report should reload its data.
  const voucherPages = new Set(['vouchers', 'journal', 'ledger', 'trial', 'income', 'balance']);
  window.addEventListener('vouchers-changed', () => {
    state.listOffsets.vouchers = 0;
    state.listOffsets.journal = 0;
    if (voucherPages.has(state.page)) void navigate(state.page, { force: true });
  });
  if (typeof eventApi?.listen === 'function') {
    void eventApi.listen('vouchers-changed', () => {
      window.dispatchEvent(new Event('vouchers-changed'));
    }).catch(() => {});
  }
  $('#btnExitApp')?.addEventListener('click', () => {
    void exitApplication();
  });
  $$('[data-quick]').forEach((button) => {
    button.addEventListener('click', () => {
      const quick = button.dataset.quick;
      if (quick === 'new-voucher') {
        openVoucherWindow();
      } else {
        navigate(quick);
      }
    });
  });

  $('#dialogClose')?.addEventListener('click', closeDialog);
  $('#modal')?.addEventListener('click', (event) => {
    if (event.target.id === 'modal') closeDialog();
  });
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && !$('#modal')?.hidden) {
      event.preventDefault();
      closeDialog();
      return;
    }
    // Ctrl+N opens a new voucher, as described in the README.
    if (!IS_TRIAL_POPUP && event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey
        && event.key.toLowerCase() === 'n' && $('#modal')?.hidden !== false) {
      event.preventDefault();
      openVoucherWindow();
    }
  });

  let focusTimer = 0;
  window.addEventListener('focus', () => {
    if (!state.refreshOnFocus || !state.ready) return;
    clearTimeout(focusTimer);
    focusTimer = setTimeout(async () => {
      state.refreshOnFocus = false;
      state.accountsLoaded = false;
      state.groupsLoaded = false;
      try {
        await loadSettings();
        await navigate(state.page, { force: true });
      } catch (_) { /* a later explicit refresh will report the error */ }
    }, 420);
  });
}

