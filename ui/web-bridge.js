'use strict';

/* ── Local web bridge ────────────────────────────────────────── */

// The desktop webview exposes the Tauri API. In local browser mode this file
// supplies the small part of that API the UI uses and forwards commands to the
// loopback server. It never replaces a real Tauri object.
(function initWebBridge() {
  const tauri = window.__TAURI__;
  if (tauri && tauri.core && typeof tauri.core.invoke === 'function') return;

  // Marks pages that run on the loopback host instead of the desktop webview.
  // For example, the print preview saves its HTML with a browser download.
  window.__HESABDARI_WEB__ = true;

  const params = new URLSearchParams(window.location.search);
  const requestedPopup = params.get('popup') || '';
  if (requestedPopup === 'trial-balance') window.__IS_TRIAL_BALANCE_POPUP__ = true;
  if (requestedPopup === 'voucher-list') window.__IS_VOUCHER_LIST_POPUP__ = true;

  const label = pageLabel(requestedPopup);
  const isVoucherPage = label === 'voucher';
  const channel = typeof BroadcastChannel === 'function'
    ? new BroadcastChannel('hesabdari-web-events')
    : null;
  const eventListeners = new Map();
  const closeListeners = new Set();

  let voucherRequestTaken = false;
  let pendingVoucherRequest = null;
  let hasPendingVoucherRequest = false;
  let probeSequence = 0;
  let activeProbe = null;

  if (isVoucherPage && params.has('number')) {
    const requestedNumber = Number(params.get('number'));
    if (Number.isInteger(requestedNumber) && requestedNumber > 0) {
      pendingVoucherRequest = requestedNumber;
      hasPendingVoucherRequest = true;
    }
    // Keep the address bar clean so a refresh starts a fresh form instead of
    // replaying a request the form already consumed.
    params.delete('number');
    const query = params.toString();
    window.history.replaceState(
      null,
      '',
      `${window.location.pathname}${query ? `?${query}` : ''}${window.location.hash}`,
    );
  }

  // Browser popup blockers accept window.open only during the click itself.
  // When the in-page window layer is unavailable (or the page was loaded
  // without web-window.js), window commands fall back to a blank popup that
  // is navigated after the server confirms the URL.
  const windowCommands = {
    open_trial_balance_window: {
      name: 'trial-balance', title: 'تراز آزمایشی', width: 1200, height: 820,
    },
    open_voucher_list_window: {
      name: 'voucher-list', title: 'لیست اسناد حسابداری', width: 1280, height: 860,
    },
    open_voucher_window: {
      name: 'voucher', title: 'صدور سند حسابداری', width: 1240, height: 800,
    },
    open_account_groups_window: {
      name: 'account-groups', title: 'تعریف گروه حساب‌ها', width: 960, height: 760,
    },
    open_account_definition_window: {
      name: 'account-definition',
      title: 'تعریف حساب‌های کل، معین و تفصیلی',
      width: 1040,
      height: 800,
    },
    open_bulk_voucher_delete_window: {
      name: 'bulk-voucher-delete', title: 'حذف دسته‌جمعی اسناد', width: 960, height: 760,
    },
    open_voucher_preview: {
      name: 'print-preview', title: 'پیش‌نمایش چاپ', width: 900, height: 980,
    },
    open_ledger_preview: {
      name: 'print-preview', title: 'پیش‌نمایش چاپ', width: 900, height: 980,
    },
    open_journal_preview: {
      name: 'print-preview', title: 'پیش‌نمایش چاپ', width: 900, height: 980,
    },
    open_trial_balance_preview: {
      name: 'print-preview', title: 'پیش‌نمایش چاپ', width: 900, height: 980,
    },
    open_income_statement_preview: {
      name: 'print-preview', title: 'پیش‌نمایش چاپ', width: 900, height: 980,
    },
    open_balance_sheet_preview: {
      name: 'print-preview', title: 'پیش‌نمایش چاپ', width: 900, height: 980,
    },
    open_equity_changes_preview: {
      name: 'print-preview', title: 'پیش‌نمایش چاپ', width: 900, height: 980,
    },
    open_cogs_preview: {
      name: 'print-preview', title: 'پیش‌نمایش چاپ', width: 900, height: 980,
    },
    open_profit_appropriation_preview: {
      name: 'print-preview', title: 'پیش‌نمایش چاپ', width: 900, height: 980,
    },
    open_cash_flow_preview: {
      name: 'print-preview', title: 'پیش‌نمایش چاپ', width: 900, height: 980,
    },
  };

  function pageLabel(popupName) {
    if (popupName === 'trial-balance' || popupName === 'voucher-list') return popupName;
    const path = window.location.pathname.toLowerCase();
    if (path.endsWith('/voucher.html')) return 'voucher';
    if (path.endsWith('/account-groups.html')) return 'account-groups';
    if (path.endsWith('/account-definition.html')) return 'account-definition';
    if (path.endsWith('/bulk-voucher-delete.html')) return 'bulk-voucher-delete';
    if (path.endsWith('/print.html')) return 'print-preview-web';
    return 'main';
  }

  // The in-page window layer is created by web-window.js. Pages that run
  // inside one of its frames receive the client API and forward their window
  // commands to the page that owns the windows.
  function webWindowApi() {
    const api = window.__HESABDARI_WEB_WINDOWS__;
    return api && typeof api.open === 'function' ? api : null;
  }

  function popupFeatures(command) {
    const width = command ? command.width : 1100;
    const height = command ? command.height : 780;
    const left = Math.max(0, Math.round((window.screen.availWidth - width) / 2));
    const top = Math.max(0, Math.round((window.screen.availHeight - height) / 2));
    return `popup=yes,width=${width},height=${height},left=${left},top=${top},resizable=yes,scrollbars=yes`;
  }

  function openWindow(url, name, features) {
    const opened = window.open(url, name, features);
    if (opened) {
      try {
        opened.focus();
      } catch (_) {
        // Focus can be denied; the window is still open.
      }
    }
    return opened;
  }

  function deliverEvent(name, payload) {
    const handlers = eventListeners.get(name);
    if (!handlers) return;
    for (const handler of Array.from(handlers)) {
      try {
        const result = handler({ event: name, id: 0, payload });
        if (result && typeof result.catch === 'function') result.catch(() => {});
      } catch (_) {
        // One failing listener must not stop the others.
      }
    }
  }

  function listen(name, handler) {
    if (typeof handler !== 'function') return Promise.resolve(() => {});
    if (!eventListeners.has(name)) eventListeners.set(name, new Set());
    eventListeners.get(name).add(handler);
    return Promise.resolve(() => {
      eventListeners.get(name)?.delete(handler);
    });
  }

  function emit(name, payload) {
    deliverEvent(name, payload);
    if (channel) channel.postMessage({ event: name, payload });
    return Promise.resolve();
  }

  function broadcast(name, payload) {
    if (channel) channel.postMessage({ event: name, payload });
  }

  function takeVoucherRequest() {
    if (!isVoucherPage) return null;
    voucherRequestTaken = true;
    if (!hasPendingVoucherRequest) return null;
    hasPendingVoucherRequest = false;
    const value = pendingVoucherRequest;
    pendingVoucherRequest = null;
    return value;
  }

  function requestedVoucherNumber(args) {
    const number = Number(args && args.number);
    return Number.isInteger(number) && number > 0 ? number : null;
  }

  // A live voucher tab answers right away; when none does, the caller opens a
  // new tab without unloading a form that may hold unsaved data.
  function probeVoucherTab() {
    if (!channel) return Promise.resolve(false);
    return new Promise((resolve) => {
      const id = String(++probeSequence);
      let settled = false;
      let timer = 0;
      const finish = (alive) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        if (activeProbe && activeProbe.id === id) activeProbe = null;
        resolve(alive);
      };
      timer = setTimeout(() => finish(false), 150);
      activeProbe = { id, finish };
      channel.postMessage({ event: 'voucher-presence-probe', payload: { id } });
    });
  }

  if (channel) {
    channel.addEventListener('message', (message) => {
      const data = message.data;
      if (!data || typeof data.event !== 'string') return;

      if (data.event === 'voucher-presence') {
        if (activeProbe && data.payload && data.payload.id === activeProbe.id) {
          activeProbe.finish(true);
        }
        return;
      }
      if (data.event === 'voucher-presence-probe') {
        if (isVoucherPage) {
          channel.postMessage({
            event: 'voucher-presence',
            payload: { id: data.payload ? data.payload.id : null },
          });
        }
        return;
      }
      if (data.event === 'voucher-request' && isVoucherPage && !voucherRequestTaken) {
        // The request can arrive while the form is still booting; keep it for
        // the first take_voucher_request call and also deliver it to any
        // listener that is already bound.
        pendingVoucherRequest = data.payload ?? null;
        hasPendingVoucherRequest = true;
      }
      deliverEvent(data.event, data.payload);
    });
  }

  async function readPayload(response) {
    try {
      return await response.json();
    } catch (_) {
      return null;
    }
  }

  // The browser cannot use the desktop file dialog for restore, so the bridge
  // picks a file locally and uploads it to the loopback server. The server
  // keeps only a token and hash; the UI never supplies a path.
  function pickFile(accept) {
    return new Promise((resolve) => {
      const input = document.createElement('input');
      input.type = 'file';
      input.accept = accept;
      input.style.display = 'none';
      document.body.appendChild(input);
      let settled = false;
      const finish = (file) => {
        if (settled) return;
        settled = true;
        input.remove();
        resolve(file);
      };
      input.addEventListener('change', () => {
        finish(input.files && input.files.length ? input.files[0] : null);
      });
      input.addEventListener('cancel', () => finish(null));
      input.click();
    });
  }

  function pickRestoreFile() {
    return pickFile('.hacc,.sqlite3,.db,application/octet-stream');
  }

  function pickExcelFile() {
    return pickFile(
      '.xlsx,.xlsm,.xls,.xlsb,.ods,'
      + 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet,'
      + 'application/vnd.ms-excel,application/vnd.oasis.opendocument.spreadsheet',
    );
  }

  async function uploadFile(url, file) {
    const response = await fetch(url, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/octet-stream',
        'X-File-Name': encodeURIComponent(file.name || 'file'),
      },
      body: file,
    });
    const payload = await readPayload(response);
    if (!response.ok || !payload || payload.ok !== true) {
      const message = payload && typeof payload.error === 'string'
        ? payload.error
        : 'بارگذاری فایل ناموفق بود.';
      throw new Error(message);
    }
    return payload.data ?? null;
  }

  async function uploadRestoreBackup() {
    const file = await pickRestoreFile();
    if (!file) return null;
    return uploadFile('/api/restore/upload', file);
  }

  // Excel commands receive an upload token instead of a path. The workbook is
  // parsed by the same Rust code the desktop app uses.
  async function uploadExcelWorkbook() {
    const file = await pickExcelFile();
    if (!file) return null;
    return uploadFile('/api/excel/upload', file);
  }

  // The server writes the export in the data folder and returns a one-time
  // link. The browser downloads that link; the UI receives a display path.
  function downloadFile(data) {
    const link = document.createElement('a');
    link.href = data.webDownload;
    if (data.fileName) link.download = data.fileName;
    document.body.appendChild(link);
    link.click();
    link.remove();
  }

  const excelUploadCommands = new Set([
    'preview_account_excel_import',
    'import_voucher_excel',
    'analyze_bulk_voucher_excel',
  ]);

  async function invoke(command, args) {
    if (command === 'take_voucher_request') return takeVoucherRequest();
    if (command === 'choose_restore_backup') return uploadRestoreBackup();

    let requestInput = args || {};
    if (excelUploadCommands.has(command)) {
      const upload = await uploadExcelWorkbook();
      if (!upload) return null;
      requestInput = { ...requestInput, uploadToken: upload.token };
    }

    const target = windowCommands[command];
    const opensVoucher = command === 'open_voucher_window';
    const inPageWindows = webWindowApi();
    let blank = null;
    let reuseVoucherTab = false;
    if (target && !inPageWindows) {
      if (opensVoucher) reuseVoucherTab = await probeVoucherTab();
      if (!reuseVoucherTab) {
        blank = openWindow('', target.name, popupFeatures(target));
        if (blank) {
          try {
            blank.document.title = 'در حال بازکردن…';
          } catch (_) {
            // The placeholder can already be gone.
          }
        }
      }
    }

    try {
      // The preview tab receives its document token in the URL; the server only
      // hands the stored HTML to a window whose label is a print preview.
      const requestArgs = command === 'get_print_doc'
        ? { ...requestInput, token: params.get('doc') || '' }
        : requestInput;
      const response = await fetch('/api/command', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: command, args: requestArgs, window: label }),
      });
      const payload = await readPayload(response);
      if (!response.ok || !payload || payload.ok !== true) {
        const message = payload && typeof payload.error === 'string'
          ? payload.error
          : `اجرای دستور «${command}» ناموفق بود.`;
        throw new Error(message);
      }

      const data = payload.data ?? null;
      if (command === 'export_account_excel' && data && typeof data.webDownload === 'string') {
        downloadFile(data);
        return {
          path: data.path || data.fileName || '',
          skippedCodes: Array.isArray(data.skippedCodes) ? data.skippedCodes : [],
        };
      }
      if (data && typeof data.webDownload === 'string') {
        downloadFile(data);
        return data.path || data.fileName || null;
      }
      if (data && typeof data.webOpen === 'string') {
        if (inPageWindows) {
          inPageWindows.open({
            name: data.webName || (target ? target.name : 'window'),
            url: data.webOpen,
            title: target ? target.title : 'نرم‌افزار حسابداری',
            width: target ? target.width : undefined,
            height: target ? target.height : undefined,
          });
          // A form already open receives the requested voucher number through
          // the same broadcast the desktop window uses.
          if (opensVoucher) broadcast('voucher-request', requestedVoucherNumber(args));
          return null;
        }
        if (reuseVoucherTab) {
          const existing = openWindow('', data.webName, popupFeatures(target));
          if (existing) {
            try {
              existing.focus();
            } catch (_) {
              // Focus can be denied; the request still reaches the tab.
            }
          }
          broadcast('voucher-request', requestedVoucherNumber(args));
          return null;
        }
        const features = popupFeatures(target || null);
        const popupWindow = blank || openWindow('', data.webName, features);
        if (popupWindow && !popupWindow.closed) {
          popupWindow.location.replace(data.webOpen);
        } else {
          const fallback = openWindow(data.webOpen, data.webName, features);
          if (!fallback) window.location.assign(data.webOpen);
        }
        return null;
      }
      if (command === 'save_voucher' || command === 'set_voucher_permanent'
          || command === 'delete_voucher' || command === 'transfer_voucher_lines'
          || command === 'execute_closing' || command === 'reverse_closing'
          || command === 'execute_carry_forward'
          || command === 'commit_bulk_voucher_excel') {
        if (command === 'commit_bulk_voucher_excel') broadcast('accounts-changed');
        broadcast('vouchers-changed');
      } else if (command === 'create_account' || command === 'assign_closing_roles'
          || command === 'import_account_excel') {
        broadcast('accounts-changed');
      } else if (command === 'restore_database_file') {
        // Other tabs must drop their stale chart, voucher lists and reports.
        broadcast('database-restored');
        broadcast('accounts-changed');
        broadcast('vouchers-changed');
      }
      return data;
    } catch (error) {
      if (blank && !blank.closed) {
        try {
          blank.close();
        } catch (_) {
          // Closing the placeholder is best-effort.
        }
      }
      throw error instanceof Error ? error : new Error(String(error));
    }
  }

  function registerCloseHandler(handler) {
    if (typeof handler !== 'function') return Promise.resolve(() => {});
    closeListeners.add(handler);
    return Promise.resolve(() => {
      closeListeners.delete(handler);
    });
  }

  async function closeListenersAllow() {
    let prevented = false;
    const request = {
      preventDefault: () => {
        prevented = true;
      },
    };
    const pending = [];
    for (const handler of Array.from(closeListeners)) {
      try {
        const result = handler(request);
        if (result && typeof result.then === 'function') pending.push(result);
      } catch (_) {
        // A failing close listener must not trap the window.
      }
    }
    await Promise.allSettled(pending);
    return !prevented;
  }

  // The window title bar asks the form whether it may close. A dirty form
  // can stop the request and show its own confirmation inside the frame.
  const closingWindows = webWindowApi();
  if (closingWindows && typeof closingWindows.setCloseGuard === 'function') {
    closingWindows.setCloseGuard(() => closeListenersAllow());
  }

  async function closeWindow() {
    if (!(await closeListenersAllow())) return;
    const inPageWindows = webWindowApi();
    if (inPageWindows && typeof inPageWindows.closeSelf === 'function') {
      inPageWindows.closeSelf();
      return;
    }
    window.close();
  }

  function currentWindow() {
    return {
      label,
      close: () => closeWindow(),
      setFocus: () => {
        const inPageWindows = webWindowApi();
        if (inPageWindows && typeof inPageWindows.focusSelf === 'function') {
          inPageWindows.focusSelf();
        } else {
          window.focus();
        }
        return Promise.resolve();
      },
      onCloseRequested: (handler) => registerCloseHandler(handler),
    };
  }

  window.__TAURI__ = {
    ...(window.__TAURI__ || {}),
    core: {
      ...(window.__TAURI__?.core || {}),
      invoke,
    },
    event: {
      ...(window.__TAURI__?.event || {}),
      listen,
      emit,
    },
    window: {
      ...(window.__TAURI__?.window || {}),
      getCurrentWindow: currentWindow,
    },
  };
})();
