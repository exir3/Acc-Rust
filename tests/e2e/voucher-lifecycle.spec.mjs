import { test, expect } from '@playwright/test';
import { FISCAL_YEARS } from './fixtures.mjs';
import {
  backendVouchers,
  fillBalancedVoucher,
  openVoucherPage,
  saveVoucher,
} from './helpers.mjs';

/** Stores the startup-gate selection the voucher window reads from localStorage. */
async function setActiveYearOnLoad(page, year) {
  await page.addInitScript((data) => {
    localStorage.setItem('hesabdari-active-fiscal-year', String(data.id));
    localStorage.setItem('hesabdari-active-fiscal-year-title', data.title);
    localStorage.setItem('hesabdari-active-fiscal-year-data', JSON.stringify(data));
  }, year);
}

test('an open selected year keeps today as the default date when it falls inside', async ({ page }) => {
  await setActiveYearOnLoad(page, FISCAL_YEARS[1]);
  await openVoucherPage(page);

  await expect(page.locator('#vfDocDate')).toHaveValue('1405/08/14');
  await expect(page.locator('#vfFiscalYearBanner')).toBeHidden();
  await expect(page.locator('[data-action="save"]').first()).toBeEnabled();
});

test('a new voucher defaults into the selected year and rejects a date outside it', async ({ page }) => {
  const selectedYear = {
    ...FISCAL_YEARS[1],
    id: 3,
    title: 'سال مالی ۱۴۰۶',
    startDate: '1406/01/01',
    endDate: '1406/12/29',
    voucherCount: 0,
  };
  await setActiveYearOnLoad(page, selectedYear);
  await openVoucherPage(page, {
    fiscalYears: [...FISCAL_YEARS.map((year) => ({ ...year })), selectedYear],
  });

  // Today (1405/08/14) is outside the selected year, so the default follows
  // the selected period instead of the system clock.
  await expect(page.locator('#vfDocDate')).toHaveValue('1406/01/01');
  await expect(page.locator('#vfFiscalYearBanner')).toBeHidden();

  await page.locator('#vfDocDate').fill('1405/09/01');
  await saveVoucher(page);

  await expect(page.locator('#vfToast')).toContainText('داخل بازه سال مالی');
  const saveCalls = await page.evaluate(() => window.__E2E__.calls
    .filter((call) => call.command === 'save_voucher').length);
  expect(saveCalls).toBe(0);
});

test('a closed selected year blocks a new voucher with a banner and disabled actions', async ({ page }) => {
  const closedYear = FISCAL_YEARS[0];
  await setActiveYearOnLoad(page, closedYear);
  await openVoucherPage(page);

  await expect(page.locator('#vfFiscalYearBanner')).toBeVisible();
  await expect(page.locator('#vfFiscalYearBanner')).toContainText('بسته');
  await expect(page.locator('[data-action="save"]').first()).toBeDisabled();
  await expect(page.locator('[data-action="add-row"]').first()).toBeDisabled();
});

test('creates, saves, reopens and deletes a temporary voucher', async ({ page }) => {
  await openVoucherPage(page);
  await fillBalancedVoucher(page);
  await saveVoucher(page);

  await expect(page.locator('#vfDocStatusText')).toContainText('سند شماره ۱ ذخیره‌شده — موقت');
  await expect(page.locator('#vfToast')).toContainText('ذخیره شد');

  let vouchers = await backendVouchers(page);
  expect(vouchers).toHaveLength(1);
  expect(vouchers[0]).toMatchObject({
    number: 1,
    isTemporary: true,
    totalDebit: 1000,
    totalCredit: 1000,
  });
  expect(vouchers[0].lines).toHaveLength(2);
  expect(vouchers[0].lines[0]).toMatchObject({
    accountCode: '100001',
    detail1Code: '100001001',
    debit: 1000,
    credit: 0,
  });
  expect(vouchers[0].lines[1]).toMatchObject({
    accountCode: '100002',
    debit: 0,
    credit: 1000,
  });

  // Leave the saved voucher through the documented Ctrl+N shortcut.
  await page.keyboard.press('Control+n');
  await expect(page.locator('#vfDocNo')).toHaveValue('2');
  await expect(page.locator('#vfDocStatusText')).toContainText('سند جدید');

  await page.locator('#vfPrevBtn').click();
  await expect(page.locator('#vfDocStatusText')).toContainText('سند شماره ۱ ذخیره‌شده — موقت');

  const first = page.locator('#voucherRows tr').first();
  // A saved detail row reopens with its detail code in the single account
  // column, while «تفصیلی ۲» shows zero.
  await expect(first.locator('.vf-code-input')).toHaveValue('100001001');
  await expect(first.locator('.vf-detail2')).toHaveText('0');
  await expect(first.locator('.vf-debit-input')).toHaveValue('1,000');
  await expect(page.locator('#vfTotalDebit')).toHaveText('۱,۰۰۰');

  await page.locator('[data-action="delete-voucher"]').click();
  await expect(page.locator('#vfConfirmModal')).toBeVisible();
  await expect(page.locator('#vfConfirmText')).toContainText('برای همیشه حذف شود');
  await page.locator('#vfConfirmOk').click();

  await expect(page.locator('#vfToast')).toContainText('حذف شد');
  await expect(page.locator('#vfDocStatusText')).toContainText('سند جدید');
  vouchers = await backendVouchers(page);
  expect(vouchers).toHaveLength(0);
});

test('a permanent voucher becomes read-only and cannot be deleted', async ({ page }) => {
  await openVoucherPage(page);
  await fillBalancedVoucher(page);

  await page.locator('#vfPermanentBtn').click();
  await expect(page.locator('#vfConfirmModal')).toBeVisible();
  await expect(page.locator('#vfConfirmText')).toContainText('دائمی');
  await page.locator('#vfConfirmOk').click();
  await expect(page.locator('#vfPermanentBtn')).toHaveClass(/is-active/);

  await saveVoucher(page);
  await expect(page.locator('#vfDocStatusText')).toContainText('سند شماره ۱ دائمی — فقط‌خواندنی');
  await expect(page.locator('#vfLockedBanner')).toBeVisible();

  const first = page.locator('#voucherRows tr').first();
  await expect(first.locator('.vf-code-input')).toHaveAttribute('readonly', '');
  await expect(first.locator('.vf-debit-input')).toHaveAttribute('readonly', '');
  await expect(page.locator('#vfDocDate')).toHaveAttribute('readonly', '');
  await expect(page.locator('[data-action="delete-voucher"]')).toBeDisabled();
  await expect(page.locator('[data-action="save"]').first()).toBeDisabled();

  const vouchers = await backendVouchers(page);
  expect(vouchers).toHaveLength(1);
  expect(vouchers[0].isTemporary).toBe(false);

  // Saving or deleting a locked voucher is refused before any backend call.
  const deleteCalls = await page.evaluate(() => window.__E2E__.calls
    .filter((call) => call.command === 'delete_voucher').length);
  expect(deleteCalls).toBe(0);
});
