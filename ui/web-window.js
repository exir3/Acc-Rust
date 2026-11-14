'use strict';

/* ── Local web windows ─────────────────────────────────────────
   In local browser mode the application forms do not open as browser
   popups. They open inside the page as windows with the same custom
   title bar as the account selector, so neither the browser nor
   Windows supplies the form title bar.

   The top-most page owns the windows. A form that itself runs in one
   of those windows forwards its window commands to that host, which
   keeps every form in the same browser tab.
   ─────────────────────────────────────────────────────────────── */

(function initWebWindows() {
  const tauri = window.__TAURI__;
  if (tauri && tauri.core && typeof tauri.core.invoke === 'function') return;

  const HOST_SOURCE = 'hesabdari-web-window-host';
  const CLIENT_SOURCE = 'hesabdari-web-window-client';
  const SVG_NS = 'http://www.w3.org/2000/svg';
  const APP_EXE_SUFFIX = /\s*[—–-]\s*نرم‌افزار حسابداری\s*$/;
  const isEmbedded = window.parent !== window;

  function cleanTitle(value) {
    return String(value || '').replace(APP_EXE_SUFFIX, '').trim();
  }

  function svgIcon(paths) {
    const svg = document.createElementNS(SVG_NS, 'svg');
    svg.setAttribute('class', 'ico web-window-ico');
    svg.setAttribute('viewBox', '0 0 16 16');
    svg.setAttribute('aria-hidden', 'true');
    svg.setAttribute('focusable', 'false');
    for (const pathData of paths) {
      const path = document.createElementNS(SVG_NS, 'path');
      path.setAttribute('d', pathData);
      path.setAttribute('fill', 'none');
      path.setAttribute('stroke', 'currentColor');
      path.setAttribute('stroke-width', '1.4');
      path.setAttribute('stroke-linecap', 'round');
      path.setAttribute('stroke-linejoin', 'round');
      svg.appendChild(path);
    }
    return svg;
  }

  const CLOSE_PATHS = ['M4.5 4.5l7 7M11.5 4.5l-7 7'];
  const MAXIMIZE_PATHS = ['M4 4h8v8H4z'];
  const RESTORE_PATHS = ['M5.5 5.5h7v7h-7z', 'M5.5 5.5V4h6.5v6.5h-1.5'];

  /* ── Embedded form: talk to the page that owns the windows ─── */

  function initClient() {
    let closeGuard = null;

    function post(message) {
      window.parent.postMessage(
        { source: CLIENT_SOURCE, ...message },
        window.location.origin,
      );
    }

    function publishTitle() {
      const title = cleanTitle(document.title);
      if (title) post({ type: 'title', title });
    }

    function observeTitle() {
      const title = document.querySelector('title');
      if (!title || typeof MutationObserver !== 'function') return;
      const observer = new MutationObserver(publishTitle);
      observer.observe(title, { childList: true, characterData: true, subtree: true });
    }

    async function answerCloseRequest(token) {
      let canClose;
      try {
        canClose = closeGuard ? await closeGuard() : true;
      } catch (_) {
        canClose = true;
      }
      post({
        type: canClose ? 'close-approved' : 'close-cancelled',
        token,
      });
    }

    window.addEventListener('message', (event) => {
      if (event.origin !== window.location.origin) return;
      const data = event.data;
      if (!data || data.source !== HOST_SOURCE) return;

      if (data.type === 'close-request') {
        answerCloseRequest(data.token).catch(() => {});
      } else if (data.type === 'focus') {
        window.focus();
      }
    });

    if (document.readyState === 'loading') {
      document.addEventListener('DOMContentLoaded', () => {
        publishTitle();
        observeTitle();
      }, { once: true });
    } else {
      publishTitle();
      observeTitle();
    }

    window.__HESABDARI_WEB_WINDOWS__ = {
      embedded: true,
      isHost: false,
      open(request) {
        post({ type: 'open', request });
      },
      closeSelf() {
        post({ type: 'close-now' });
      },
      focusSelf() {
        post({ type: 'focus' });
      },
      setCloseGuard(guard) {
        closeGuard = typeof guard === 'function' ? guard : null;
      },
    };
  }

  /* ── Host page: create, focus and close the in-page windows ── */

  function initHost() {
    const records = new Map();
    let layer = null;
    let openCount = 0;
    let zCounter = 0;
    let closeSequence = 0;

    function ensureLayer() {
      if (layer) return layer;
      layer = document.createElement('div');
      layer.className = 'web-window-layer';
      layer.hidden = true;
      layer.setAttribute('aria-live', 'polite');
      (document.body || document.documentElement).appendChild(layer);
      return layer;
    }

    function normalizeRequest(request) {
      if (!request || typeof request !== 'object') return null;
      const name = typeof request.name === 'string' && /^[a-z0-9-]+$/.test(request.name)
        ? request.name
        : '';
      if (!name) return null;

      let url = '';
      if (typeof request.url === 'string' && request.url.trim()) {
        try {
          const parsed = new URL(request.url, window.location.href);
          if (parsed.origin === window.location.origin) {
            url = `${parsed.pathname}${parsed.search}${parsed.hash}`;
          }
        } catch (_) {
          // A malformed URL is treated as an empty request.
        }
        if (!url) return null;
      }

      const width = Number(request.width);
      const height = Number(request.height);
      const title = cleanTitle(request.title);
      return {
        name,
        url,
        title: title || 'نرم‌افزار حسابداری',
        width: Number.isFinite(width) ? width : 1000,
        height: Number.isFinite(height) ? height : 720,
      };
    }

    function makeTitleButton(className, label, paths) {
      const button = document.createElement('button');
      button.type = 'button';
      button.className = className;
      button.title = label;
      button.setAttribute('aria-label', label);
      button.appendChild(svgIcon(paths));
      return button;
    }

    function setTitle(record, title) {
      const clean = cleanTitle(title);
      if (!clean) return;
      record.titleElement.textContent = clean;
      record.window.setAttribute('aria-label', clean);
      record.frame.title = clean;
    }

    function focusWindow(record) {
      if (!record || record.removed) return;
      zCounter += 1;
      record.window.style.zIndex = String(zCounter);
      for (const other of records.values()) {
        other.window.classList.toggle('is-active', other === record);
      }
      if (record.loaded) {
        try {
          record.frame.contentWindow.focus();
        } catch (_) {
          // A focus call can be denied; the window is still in front.
        }
      }
    }

    function removeWindow(record) {
      if (!record || record.removed) return;
      record.removed = true;
      clearTimeout(record.closeTimer);
      records.delete(record.name);
      record.window.remove();
      if (!records.size && layer) layer.hidden = true;
      document.dispatchEvent(new CustomEvent('web-window-closed', {
        detail: { name: record.name },
      }));
    }

    function settleClose(record, token, approved) {
      if (!record.pendingClose || record.pendingClose.token !== token) return;
      clearTimeout(record.closeTimer);
      record.pendingClose = null;
      record.closing = false;
      if (approved) removeWindow(record);
    }

    function requestClose(record) {
      if (!record || record.removed || record.closing) return;
      const client = record.frame.contentWindow;
      const hasGuard = Boolean(client && client.__HESABDARI_WEB_WINDOWS__);
      if (!record.loaded || !hasGuard) {
        removeWindow(record);
        return;
      }

      record.closing = true;
      const token = String(++closeSequence);
      record.pendingClose = { token };
      try {
        client.postMessage(
          { source: HOST_SOURCE, type: 'close-request', token },
          window.location.origin,
        );
      } catch (_) {
        settleClose(record, token, true);
      }
    }

    function toggleMaximize(record) {
      if (!record || record.removed) return;
      record.maximized = !record.maximized;
      record.window.classList.toggle('is-maximized', record.maximized);
      const label = record.maximized ? 'بازگردانی اندازه' : 'بزرگ‌نمایی';
      record.maxButton.title = label;
      record.maxButton.setAttribute('aria-label', label);
      record.maxButton.replaceChildren(
        svgIcon(record.maximized ? RESTORE_PATHS : MAXIMIZE_PATHS),
      );
      focusWindow(record);
    }

    function bindDrag(record) {
      const bar = record.titlebar;
      let drag = null;

      bar.addEventListener('pointerdown', (event) => {
        if (event.button !== 0 || record.maximized) return;
        if (event.target.closest('button')) return;
        const rect = record.window.getBoundingClientRect();
        drag = {
          pointerId: event.pointerId,
          startX: event.clientX,
          startY: event.clientY,
          left: rect.left,
          top: rect.top,
          width: rect.width,
          height: rect.height,
        };
        record.window.classList.add('is-dragging');
        try {
          bar.setPointerCapture(event.pointerId);
        } catch (_) {
          // Pointer capture is an enhancement; dragging still follows moves.
        }
        event.preventDefault();
      });

      function finishDrag() {
        if (!drag) return;
        drag = null;
        record.window.classList.remove('is-dragging');
      }

      bar.addEventListener('pointermove', (event) => {
        if (!drag || event.pointerId !== drag.pointerId) return;
        const maxLeft = Math.max(8, window.innerWidth - drag.width - 8);
        const maxTop = Math.max(8, window.innerHeight - drag.height - 8);
        const left = Math.max(8, Math.min(drag.left + event.clientX - drag.startX, maxLeft));
        const top = Math.max(8, Math.min(drag.top + event.clientY - drag.startY, maxTop));
        record.window.style.left = `${Math.round(left)}px`;
        record.window.style.top = `${Math.round(top)}px`;
      });

      bar.addEventListener('pointerup', finishDrag);
      bar.addEventListener('pointercancel', finishDrag);

      bar.addEventListener('dblclick', (event) => {
        if (event.target.closest('button')) return;
        toggleMaximize(record);
      });
    }

    function createWindow(spec) {
      const win = document.createElement('section');
      win.className = 'web-window';
      win.dataset.window = spec.name;
      win.setAttribute('role', 'dialog');
      win.setAttribute('aria-modal', 'false');
      win.setAttribute('aria-label', spec.title);
      win.dir = 'rtl';

      const titlebar = document.createElement('div');
      titlebar.className = 'ap-titlebar web-window-titlebar';

      const closeButton = makeTitleButton(
        'ap-close web-window-btn',
        'بستن',
        CLOSE_PATHS,
      );
      const maxButton = makeTitleButton(
        'web-window-max web-window-btn',
        'بزرگ‌نمایی',
        MAXIMIZE_PATHS,
      );
      const titleElement = document.createElement('span');
      titleElement.className = 'ap-title web-window-title';
      titleElement.textContent = spec.title;
      titlebar.append(closeButton, maxButton, titleElement);

      const body = document.createElement('div');
      body.className = 'web-window-body';

      const frame = document.createElement('iframe');
      frame.className = 'web-window-frame';
      frame.title = spec.title;
      frame.name = spec.name;
      frame.src = spec.url;

      const loading = document.createElement('div');
      loading.className = 'web-window-loading';
      const spinner = document.createElement('span');
      spinner.className = 'web-window-spinner';
      spinner.setAttribute('aria-hidden', 'true');
      const loadingText = document.createElement('span');
      loadingText.textContent = 'در حال بازکردن…';
      loading.append(spinner, loadingText);

      body.append(frame, loading);
      win.append(titlebar, body);

      const record = {
        name: spec.name,
        window: win,
        titlebar,
        titleElement,
        maxButton,
        frame,
        loaded: false,
        removed: false,
        closing: false,
        closeTimer: 0,
        pendingClose: null,
        maximized: false,
      };

      closeButton.addEventListener('click', (event) => {
        event.stopPropagation();
        requestClose(record);
      });
      maxButton.addEventListener('click', (event) => {
        event.stopPropagation();
        toggleMaximize(record);
      });
      win.addEventListener('pointerdown', () => focusWindow(record));

      frame.addEventListener('load', () => {
        if (record.removed) return;
        record.loaded = true;
        win.classList.add('is-loaded');
        try {
          const innerTitle = frame.contentDocument ? frame.contentDocument.title : '';
          if (innerTitle) setTitle(record, innerTitle);
        } catch (_) {
          // Same-origin pages always allow this; keep the requested title.
        }
        try {
          frame.contentWindow.focus();
        } catch (_) {
          // Focus is best-effort.
        }
      });

      const viewportWidth = window.innerWidth;
      const viewportHeight = window.innerHeight;
      const width = Math.max(360, Math.min(spec.width, viewportWidth - 24));
      const height = Math.max(300, Math.min(spec.height, viewportHeight - 24));
      const offset = (openCount % 5) * 22;
      openCount += 1;
      const left = Math.max(8, Math.round((viewportWidth - width) / 2) - offset);
      const top = Math.max(8, Math.round((viewportHeight - height) / 2) - offset);
      win.style.width = `${width}px`;
      win.style.height = `${height}px`;
      win.style.left = `${left}px`;
      win.style.top = `${top}px`;

      bindDrag(record);
      ensureLayer().appendChild(win);
      layer.hidden = false;
      return record;
    }

    function open(request) {
      const spec = normalizeRequest(request);
      if (!spec) return null;

      const existing = records.get(spec.name);
      if (existing) {
        // Desktop replaces the single print preview with the newest report.
        if (spec.name === 'print-preview' && spec.url) {
          removeWindow(existing);
        } else {
          if (spec.url && !existing.loaded) existing.frame.src = spec.url;
          focusWindow(existing);
          return existing;
        }
      }

      if (!spec.url) return null;
      const record = createWindow(spec);
      records.set(spec.name, record);
      focusWindow(record);
      return record;
    }

    function recordForSource(source) {
      for (const record of records.values()) {
        if (record.frame.contentWindow === source) return record;
      }
      return null;
    }

    window.addEventListener('message', (event) => {
      if (event.origin !== window.location.origin) return;
      const data = event.data;
      if (!data || data.source !== CLIENT_SOURCE) return;
      const record = recordForSource(event.source);
      if (!record) return;

      if (data.type === 'open') {
        open(data.request);
      } else if (data.type === 'close-now') {
        removeWindow(record);
      } else if (data.type === 'close-approved') {
        settleClose(record, data.token, true);
      } else if (data.type === 'close-cancelled') {
        settleClose(record, data.token, false);
      } else if (data.type === 'focus') {
        focusWindow(record);
      } else if (data.type === 'title') {
        setTitle(record, data.title);
      }
    });

    window.addEventListener('resize', () => {
      for (const record of records.values()) {
        if (record.maximized) continue;
        const rect = record.window.getBoundingClientRect();
        const width = Math.min(rect.width, window.innerWidth - 16);
        const height = Math.min(rect.height, window.innerHeight - 16);
        const left = Math.max(8, Math.min(rect.left, window.innerWidth - width - 8));
        const top = Math.max(8, Math.min(rect.top, window.innerHeight - height - 8));
        record.window.style.width = `${Math.round(width)}px`;
        record.window.style.height = `${Math.round(height)}px`;
        record.window.style.left = `${Math.round(left)}px`;
        record.window.style.top = `${Math.round(top)}px`;
      }
    });

    window.__HESABDARI_WEB_WINDOWS__ = {
      embedded: false,
      isHost: true,
      open,
      has(name) {
        return records.has(String(name || ''));
      },
      focus(name) {
        focusWindow(records.get(String(name || '')));
      },
      close(name) {
        requestClose(records.get(String(name || '')));
      },
    };
  }

  if (isEmbedded) initClient();
  else initHost();
})();
