export const GROUPS = [
  {
    id: 1,
    code: '01',
    name: 'دارایی‌ها',
    type: 'asset',
    defaultSection: 'assets',
    accountCount: 2,
    postedLineCount: 0,
    createdAt: '2026-11-05T08:00:00.000Z',
    updatedAt: '2026-11-05T08:00:00.000Z',
  },
];

function account(id, code, name, level, parentCode, options = {}) {
  return {
    id,
    code,
    name,
    level,
    groupCode: '01',
    groupName: 'دارایی‌ها',
    groupType: 'asset',
    nature: options.nature || 'debtor',
    parentCode,
    parentName: options.parentName || '',
    isPostable: level <= 2,
    isActive: options.isActive !== false,
    createdAt: '2026-11-05T08:00:00.000Z',
    updatedAt: '2026-11-05T08:00:00.000Z',
  };
}

export const ACCOUNTS = [
  account(1, '1000', 'کل دارای معین', 1, ''),
  account(2, '2000', 'کل بدون معین', 1, ''),
  account(3, '100001', 'معین دارای تفصیلی', 2, '1000', { parentName: 'کل دارای معین' }),
  account(4, '100002', 'معین بدون تفصیلی', 2, '1000', { parentName: 'کل دارای معین' }),
  account(5, '200001', 'معین دوم', 2, '2000', { parentName: 'کل بدون معین' }),
  account(6, '100001001', 'تفصیلی یک', 3, '100001', { parentName: 'معین دارای تفصیلی' }),
  account(7, '100001002', 'تفصیلی دو', 3, '100001', { parentName: 'معین دارای تفصیلی' }),
];

export const SETTINGS = {
  companyName: 'شرکت آزمون',
  fiscalYear: '1405',
  today: '1405/08/14',
  limits: {
    maxVoucherLines: 1000,
    maxAmount: 999_999_999_999_999,
    maxVoucherNumber: Number.MAX_SAFE_INTEGER,
    maxDescription: 1000,
    maxLineDescription: 500,
    voucherPageSize: 50,
    journalPageSize: 50,
    maxPageSize: 200,
  },
};

export const FISCAL_YEARS = [
  {
    id: 1,
    title: 'سال مالی ۱۴۰۴',
    startDate: '1404/01/01',
    endDate: '1404/12/29',
    status: 'closed',
    voucherCount: 24,
    createdAt: '2026-11-05T08:00:00.000Z',
    updatedAt: '2026-11-05T08:00:00.000Z',
  },
  {
    id: 2,
    title: 'سال مالی ۱۴۰۵',
    startDate: '1405/01/01',
    endDate: '1405/12/29',
    status: 'open',
    voucherCount: 7,
    createdAt: '2026-11-05T08:00:00.000Z',
    updatedAt: '2026-11-05T08:00:00.000Z',
  },
];

/** A fresh copy of the seed so one test cannot leak state into another. */
export function buildSeed(overrides = {}) {
  return {
    accounts: structuredClone(ACCOUNTS),
    groups: structuredClone(GROUPS),
    settings: structuredClone(SETTINGS),
    fiscalYears: structuredClone(FISCAL_YEARS),
    vouchers: [],
    nextNumber: 1,
    ...overrides,
  };
}
