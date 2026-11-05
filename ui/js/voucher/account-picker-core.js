'use strict';

/* ── Account picker hierarchy helpers ─────────────────────────
   Pure, DOM-free helpers shared by the voucher account picker and
   the Node unit tests. They only understand account records with
   `code`, `level` and `parentCode`, so they can be exercised without
   a browser or a Tauri backend.
   ──────────────────────────────────────────────────────────── */
(function attachAccountPickerCore(global) {
  const digits = (value) => String(value ?? '')
    .replace(/[۰-۹]/g, (digit) => String(digit.charCodeAt(0) - 0x06f0))
    .replace(/[٠-٩]/g, (digit) => String(digit.charCodeAt(0) - 0x0660))
    .replace(/[^0-9]/g, '');

  const levelOf = (account) => Number(account && account.level) || 0;

  function parentCodeOf(account) {
    return digits(account && account.parentCode);
  }

  function directChildren(accounts, parentCode, parentLevel) {
    const parent = digits(parentCode);
    if (!parent) return [];
    return accounts.filter((account) => parentCodeOf(account) === parent
      && levelOf(account) === Number(parentLevel) + 1);
  }

  function hasChildren(accounts, account) {
    if (!account || levelOf(account) >= 3) return false;
    return directChildren(accounts, account.code, levelOf(account)).length > 0;
  }

  function canDrill(accounts, account, mode) {
    if (!account || mode === 'detail') return false;
    return levelOf(account) < 3 && hasChildren(accounts, account);
  }

  function ancestors(account, accounts) {
    const chain = [];
    const seen = new Set();
    let parentCode = parentCodeOf(account);
    while (parentCode && !seen.has(parentCode)) {
      seen.add(parentCode);
      const parent = accounts.find((item) => digits(item.code) === parentCode);
      if (!parent) break;
      chain.unshift(parent);
      parentCode = parentCodeOf(parent);
    }
    return chain;
  }

  /// Accounts shown for the current drill level.
  /// - detail mode: details of the selected معین
  /// - base mode at the root: کل accounts
  /// - base mode after drilling: direct children of the last path account
  /// - show all: flat کل/معین list (F6)
  function visibleAccounts(accounts, options = {}) {
    const mode = options.mode === 'detail' ? 'detail' : 'base';
    if (mode === 'detail') {
      const baseCode = digits(options.baseCode);
      if (!baseCode) return [];
      return accounts.filter((account) => levelOf(account) === 3
        && parentCodeOf(account) === baseCode);
    }

    if (options.showAll) {
      return accounts.filter((account) => {
        const level = levelOf(account);
        return level === 1 || level === 2;
      });
    }

    const path = Array.isArray(options.path) ? options.path : [];
    if (path.length) {
      const parent = path[path.length - 1];
      const parentLevel = levelOf(parent);
      if (parentLevel < 1 || parentLevel >= 3) return [];
      return directChildren(accounts, parent.code, parentLevel);
    }

    return accounts.filter((account) => levelOf(account) === 1);
  }

  /// Search is intentionally independent of the drill level so a user can
  /// find an account at another level without walking the whole tree.
  function searchPool(accounts, options = {}) {
    const mode = options.mode === 'detail' ? 'detail' : 'base';
    if (mode === 'detail') {
      const baseCode = digits(options.baseCode);
      if (!baseCode) return [];
      return accounts.filter((account) => levelOf(account) === 3
        && parentCodeOf(account) === baseCode);
    }
    return accounts.filter((account) => {
      const level = levelOf(account);
      return level === 1 || level === 2;
    });
  }

  global.AccountPickerCore = {
    digits,
    directChildren,
    hasChildren,
    canDrill,
    ancestors,
    visibleAccounts,
    searchPool,
  };
})(typeof globalThis !== 'undefined' ? globalThis : this);
