/* In-page Tauri mock for the Playwright UI tests.
 *
 * The UI is a set of static HTML pages whose only backend is
 * `window.__TAURI__.core.invoke`. This file implements the subset of the
 * Tauri commands used by the voucher and account-definition windows with an
 * in-memory store, so the real UI code can be driven in Chromium without a
 * Rust process. The real backend remains covered by the Rust test suite.
 *
 * Tests put the initial data on `window.__E2E_SEED__` with an earlier
 * addInitScript call; this script reads it before the page scripts run.
 */
(() => {
  'use strict';

  const seed = window.__E2E_SEED__ || {};
  const clone = (value) => JSON.parse(JSON.stringify(value ?? null));
  let clock = 0;
  const stamp = () => `2026-11-05T10:${String(clock++ % 60).padStart(2, '0')}:00.000Z`;
  const pad2 = (value) => String(value).padStart(2, '0');

  const backend = {
    accounts: clone(seed.accounts || []),
    groups: clone(seed.groups || []),
    vouchers: clone(seed.vouchers || []),
    settings: clone(seed.settings || {}),
    fiscalYears: clone(seed.fiscalYears || []),
    nextNumber: Number(seed.nextNumber) || 1,
    calls: [],
  };

  let nextAccountId = Math.max(0, ...backend.accounts.map((account) => Number(account.id) || 0)) + 1;
  let nextVoucherId = Math.max(0, ...backend.vouchers.map((voucher) => Number(voucher.id) || 0)) + 1;
  let nextFiscalYearId = Math.max(0, ...backend.fiscalYears.map((year) => Number(year.id) || 0)) + 1;
  let nextLineId = 1 + Math.max(
    0,
    ...backend.vouchers.flatMap((voucher) => voucher.lines || []).map((line) => Number(line.id) || 0),
  );

  const accountByCode = (code) => backend.accounts.find(
    (account) => String(account.code) === String(code),
  );
  const groupByCode = (code) => backend.groups.find(
    (group) => String(group.code) === String(code),
  );
  const voucherByNumber = (number) => backend.vouchers.find(
    (voucher) => Number(voucher.number) === Number(number),
  );
  const voucherById = (id) => backend.vouchers.find(
    (voucher) => Number(voucher.id) === Number(id),
  );

  function nextFreeNumber() {
    const numbers = backend.vouchers
      .map((voucher) => Number(voucher.number))
      .filter((number) => Number.isFinite(number));
    return numbers.length ? Math.max(...numbers) + 1 : 1;
  }

  function makeAccount(payload) {
    const level = Number(payload.level) || 1;
    const parentCode = level === 1 ? '' : String(payload.parentCode || '');
    const parent = accountByCode(parentCode);
    const group = groupByCode(payload.groupCode)
      || groupByCode(parent?.groupCode)
      || { code: '', name: '', type: 'asset' };
    const account = {
      id: nextAccountId++,
      code: String(payload.code || ''),
      name: String(payload.name || ''),
      level,
      groupCode: String(payload.groupCode || group.code || ''),
      groupName: group.name || '',
      groupType: group.type || 'asset',
      nature: payload.nature || 'debtor',
      parentCode,
      parentName: parent ? parent.name : '',
      isPostable: level <= 2,
      isActive: payload.isActive !== false,
      createdAt: stamp(),
      updatedAt: stamp(),
    };
    backend.accounts.push(account);
    return account;
  }

  function makeVoucher(input, existing) {
    const lines = (input.lines || []).map((line, index) => {
      const account = accountByCode(line.accountCode);
      const detail = line.detail1Code ? accountByCode(line.detail1Code) : null;
      return {
        id: line.id != null ? Number(line.id) : nextLineId++,
        rowNo: index + 1,
        accountCode: String(line.accountCode || ''),
        accountName: account ? account.name : '',
        detail1Code: line.detail1Code ? String(line.detail1Code) : null,
        detail1Name: detail ? detail.name : null,
        description: String(line.description || ''),
        debit: Number(line.debit) || 0,
        credit: Number(line.credit) || 0,
      };
    });
    const number = Number(input.number) || existing?.number
      || backend.nextNumber || nextFreeNumber();
    const voucher = {
      id: existing ? existing.id : nextVoucherId++,
      number,
      date: String(input.date || ''),
      updatedAt: stamp(),
      isTemporary: existing ? existing.isTemporary : true,
      voucherType: String(input.voucherType || ''),
      description: String(input.description || ''),
      lines,
      totalDebit: lines.reduce((sum, line) => sum + line.debit, 0),
      totalCredit: lines.reduce((sum, line) => sum + line.credit, 0),
    };
    if (existing) Object.assign(existing, voucher);
    else backend.vouchers.push(voucher);
    backend.nextNumber = nextFreeNumber();
    return existing || voucher;
  }

  const handlers = {
    get_settings: () => clone(backend.settings),
    get_trial_balance: () => ({
      rows: [
        {
          accountCode: '1000',
          accountName: 'کل دارای معین',
          groupName: 'دارایی‌ها',
          parentCode: '',
          level: 1,
          openingDebit: 1000,
          openingCredit: 0,
          periodDebit: 0,
          periodCredit: 0,
          closingDebit: 1000,
          closingCredit: 0,
        },
        {
          accountCode: '2000',
          accountName: 'کل بدون معین',
          groupName: 'دارایی‌ها',
          parentCode: '',
          level: 1,
          openingDebit: 0,
          openingCredit: 1000,
          periodDebit: 0,
          periodCredit: 0,
          closingDebit: 0,
          closingCredit: 1000,
        },
      ],
    }),
    save_settings: ({ companyName, fiscalYear }) => {
      backend.settings.companyName = String(companyName ?? backend.settings.companyName ?? '');
      backend.settings.fiscalYear = String(fiscalYear ?? backend.settings.fiscalYear ?? '');
      return clone(backend.settings);
    },
    list_fiscal_years: () => ({ years: clone(backend.fiscalYears), databasePath: 'e2e.db' }),
    get_voucher_count: () => backend.vouchers.length,

    create_fiscal_year: ({ title, startDate, endDate, status }) => {
      const year = {
        id: nextFiscalYearId++,
        title: String(title || ''),
        startDate: String(startDate || ''),
        endDate: String(endDate || ''),
        status: String(status || 'open'),
        voucherCount: 0,
        createdAt: stamp(),
        updatedAt: stamp(),
      };
      backend.fiscalYears.push(year);
      return clone(year);
    },

    update_fiscal_year: ({ id, title, startDate, endDate, status }) => {
      const year = backend.fiscalYears.find((item) => Number(item.id) === Number(id));
      if (!year) throw new Error('سال مالی یافت نشد.');
      year.title = String(title ?? year.title);
      if (Number(year.voucherCount) === 0) {
        year.startDate = String(startDate ?? year.startDate);
        year.endDate = String(endDate ?? year.endDate);
      }
      year.status = String(status ?? year.status);
      year.updatedAt = stamp();
      return clone(year);
    },

    set_fiscal_year_status: ({ id, status }) => {
      const year = backend.fiscalYears.find((item) => Number(item.id) === Number(id));
      if (!year) throw new Error('سال مالی یافت نشد.');
      year.status = String(status ?? year.status);
      year.updatedAt = stamp();
      return clone(year);
    },

    delete_fiscal_year: ({ id }) => {
      const year = backend.fiscalYears.find((item) => Number(item.id) === Number(id));
      if (!year) throw new Error('سال مالی یافت نشد.');
      if (Number(year.voucherCount) > 0) {
        throw new Error('سال مالی دارای سند قابل حذف نیست.');
      }
      backend.fiscalYears = backend.fiscalYears.filter((item) => Number(item.id) !== Number(id));
      return null;
    },

    backup_database: () => '/e2e/backups/hesabdari-backup.db',
    backup_database_encrypted: ({ password }) => {
      if (!password || String(password).length < 8) {
        throw new Error('رمز پشتیبان باید حداقل ۸ نویسه باشد.');
      }
      return '/e2e/backups/hesabdari-backup.hacc';
    },
    choose_restore_backup: () => ({
      token: 'e2e-restore-token',
      encrypted: true,
      fileName: 'hesabdari-backup.hacc',
    }),
    restore_database_file: ({ token, password }) => {
      if (token !== 'e2e-restore-token') throw new Error('توکن بازیابی معتبر نیست.');
      if (!password) throw new Error('رمز عبور لازم است.');
      return '/e2e/backups/hesabdari-safety.db';
    },

    list_accounts: () => ({ accounts: clone(backend.accounts), databasePath: 'e2e.db' }),
    list_account_groups: () => ({ groups: clone(backend.groups), databasePath: 'e2e.db' }),
    next_voucher_number: () => backend.nextNumber || nextFreeNumber(),
    take_voucher_request: () => null,
    get_voucher: ({ number }) => clone(voucherByNumber(number) || null),

    adjacent_voucher_number: ({ number, direction }) => {
      const numbers = [...new Set(backend.vouchers.map((voucher) => Number(voucher.number)))]
        .filter((item) => Number.isFinite(item))
        .sort((left, right) => left - right);
      if (direction === 'previous') {
        const previous = numbers.filter((item) => item < Number(number));
        return previous.length ? previous[previous.length - 1] : null;
      }
      const next = numbers.find((item) => item > Number(number));
      return next === undefined ? null : next;
    },

    validate_voucher_date: ({ date }) => {
      const value = String(date || '').trim().replace(/-/g, '/');
      const match = /^(\d{4})\/(\d{1,2})\/(\d{1,2})$/.exec(value);
      if (!match) throw new Error('تاریخ سند باید به شکل ۱۴۰۵/۰۱/۰۱ وارد شود.');
      const year = Number(match[1]);
      const month = Number(match[2]);
      const day = Number(match[3]);
      if (year < 1 || month < 1 || month > 12 || day < 1 || day > 31) {
        throw new Error('سال، ماه یا روز تاریخ سند معتبر نیست.');
      }
      return `${match[1]}/${pad2(month)}/${pad2(day)}`;
    },

    save_voucher: ({ input, expectedUpdatedAt }) => {
      const existing = input.id ? voucherById(input.id) : voucherByNumber(input.number);
      if (existing && existing.isTemporary === false) {
        throw new Error('سند دائمی است و قابل تغییر نیست.');
      }
      if (existing && expectedUpdatedAt && existing.updatedAt !== expectedUpdatedAt) {
        throw new Error('سند پس از بازکردن تغییر کرده است؛ آن را دوباره باز کنید.');
      }
      return clone(makeVoucher(input, existing));
    },

    set_voucher_permanent: ({ id, expectedUpdatedAt }) => {
      const voucher = voucherById(id);
      if (!voucher) throw new Error('سند یافت نشد.');
      if (expectedUpdatedAt && voucher.updatedAt !== expectedUpdatedAt) {
        throw new Error('سند پس از بازکردن تغییر کرده است؛ آن را دوباره باز کنید.');
      }
      voucher.isTemporary = false;
      voucher.updatedAt = stamp();
      return clone(voucher);
    },

    delete_voucher: ({ id }) => {
      const voucher = voucherById(id);
      if (!voucher) throw new Error('سند یافت نشد.');
      if (voucher.isTemporary === false) throw new Error('سند دائمی قابل حذف نیست.');
      backend.vouchers = backend.vouchers.filter((item) => item.id !== voucher.id);
      backend.nextNumber = nextFreeNumber();
      return null;
    },

    create_account: (payload) => clone(makeAccount(payload)),

    update_account: ({ id, name, groupCode, nature, parentCode, isActive }) => {
      const account = backend.accounts.find((item) => Number(item.id) === Number(id));
      if (!account) throw new Error('حساب یافت نشد.');
      if (name != null) account.name = String(name);
      if (groupCode != null) {
        account.groupCode = String(groupCode);
        account.groupName = groupByCode(groupCode)?.name || '';
      }
      if (nature != null) account.nature = String(nature);
      if (parentCode != null) account.parentCode = String(parentCode);
      if (isActive != null) account.isActive = Boolean(isActive);
      account.updatedAt = stamp();
      return clone(account);
    },

    delete_account: ({ id }) => {
      backend.accounts = backend.accounts.filter((item) => Number(item.id) !== Number(id));
      return null;
    },

    delete_accounts_bulk: () => ({ deletedCount: 0, blocked: [] }),
    export_account_excel: () => 'accounts.xlsx',
    import_account_excel: () => ({ skippedCount: 0, createdCount: 0, errorCount: 0, errors: [] }),
    analyze_bulk_voucher_excel: () => clone(seed.bulkImportPreview || null),
  };

  const invoke = async (command, args = {}) => {
    backend.calls.push({ command, args: clone(args) });
    const handler = handlers[command];
    if (!handler) throw new Error(`E2E mock: command «${command}» is not implemented.`);
    return handler(args || {});
  };

  window.__TAURI__ = {
    core: { invoke },
    event: { listen: async () => () => {} },
    window: {
      getCurrentWindow: () => ({
        setFocus: async () => {},
        close: async () => {},
        onCloseRequested: async () => () => {},
      }),
    },
  };

  window.__E2E__ = {
    backend,
    calls: backend.calls,
    accountByCode,
    voucherByNumber,
  };
})();
