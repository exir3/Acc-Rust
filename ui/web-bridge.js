'use strict';

/* ── Local web bridge ────────────────────────────────────────── */

// The desktop webview exposes the Tauri API. In local browser mode this file
// supplies the small part of that API the UI uses and forwards commands to the
// loopback server. It never replaces a real Tauri object.
(function initWebBridge() {
  const tauri = window.__TAURI__;
  if (tauri && tauri.core && typeof tauri.core.invoke === 'function') return;

  const params = new URLSearchParams(window.location.search);
  const requestedPopup = params.get('popup') || '';
  if (requestedPopup === 'trial-balance') window.__IS_TRIAL_BALANCE_POPUP__ = true;
  if (requestedPopup === 'voucher-list') window.__IS_VOUCHER_LIST_POPUP__ = true;

  const label = pageLabel(requestedPopup);
  const channel = typeof BroadcastChannel === 'function'
    ? new BroadcastChannel('hesabdari-web-events')
    : null;
  const eventListeners = new Map();
  const closeListeners = new Set();

  // Browser popup blockers accept window.open only during the click itself.
  // Window commands therefore open a blank tab synchronously and navigate it
  // after the server confirms the URL.
  const windowCommands = {
    open_trial_balance_window: { name: 'trial-balance', width: 1200, height: 820 },
    open_voucher_list_window: { name: 'voucher-list', width: 1280, height: 860 },
    open_account_groups_window: { name: 'account-groups', width: 960, height: 760 },
    open_account_definition_window: { name: 'account-definition', width: 1040, height: 800 },
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

  if (channel) {
    channel.addEventListener('message', (message) => {
      const data = message.data;
      if (!data || typeof data.event !== 'string') return;
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

  async function invoke(command, args) {
    const target = windowCommands[command];
    let blank = null;
    if (target) {
      blank = openWindow('', target.name, popupFeatures(target));
      if (blank) {
        try {
          blank.document.title = 'در حال بازکردن…';
        } catch (_) {
          // The placeholder can already be gone.
        }
      }
    }

    try {
      const response = await fetch('/api/command', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: command, args: args || {}, window: label }),
      });
      const payload = await readPayload(response);
      if (!response.ok || !payload || payload.ok !== true) {
        const message = payload && typeof payload.error === 'string'
          ? payload.error
          : `اجرای دستور «${command}» ناموفق بود.`;
        throw new Error(message);
      }

      const data = payload.data ?? null;
      if (data && typeof data.webOpen === 'string') {
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

  async function closeWindow() {
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
    if (prevented) return;
    window.close();
  }

  function currentWindow() {
    return {
      label,
      close: () => closeWindow(),
      setFocus: () => {
        window.focus();
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
