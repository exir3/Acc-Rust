import { fileURLToPath } from 'node:url';
import { expect } from '@playwright/test';
import { buildSeed } from './fixtures.mjs';

const MOCK_TAURI = fileURLToPath(new URL('./mock-tauri.js', import.meta.url));

/** Install the in-memory Tauri backend before the page scripts run. */
async function installMock(page, options) {
  const seed = buildSeed(options);
  await page.addInitScript((data) => {
    window.__E2E_SEED__ = data;
  }, seed);
  await page.addInitScript({ path: MOCK_TAURI });
}

export async function openVoucherPage(page, options = {}) {
  await installMock(page, options);
  await page.goto('/voucher.html');
  await expect(page.locator('#vfDocNo')).toHaveValue('1');
  await expect(page.locator('#vfDocStatusText')).toContainText('سند جدید');
}

export async function openAccountDefinitionPage(page, options = {}) {
  await installMock(page, options);
  await page.goto('/account-definition.html');
  await expect(page.locator('#adGridBody .ad-row').first()).toBeVisible();
  await expect(page.locator('#adSearch')).toBeVisible();
}

export async function openAccountGroupsPage(page, options = {}) {
  await installMock(page, options);
  await page.goto('/account-groups.html');
  await expect(page.locator('#agGridBody .ag-row').first()).toBeVisible();
  await expect(page.locator('#agNew')).toBeEnabled();
}

/** The trial balance renders inside the main window only in its popup mode. */
export async function openTrialBalancePage(page, options = {}) {
  await installMock(page, options);
  await page.addInitScript(() => {
    window.__IS_TRIAL_BALANCE_POPUP__ = true;
  });
  await page.goto('/index.html');
  await expect(page.locator('#tbpSearch')).toBeVisible();
  await expect(page.locator('.tbp-data-row[data-code]').first()).toBeVisible();
}

export const pickerRows = (page) => page.locator('#accountPickerRows .ap-row[data-picker-index]');

export const pickerRow = (page, text) => pickerRows(page).filter({ hasText: text });

export async function chooseBaseAccount(page, row, names) {
  const input = row.locator('.vf-code-input');
  await input.click();
  await input.press('Space');
  await expect(page.locator('#accountPickerModal')).toBeVisible();
  for (const name of names) {
    const option = pickerRow(page, name).first();
    await expect(option).toBeVisible();
    await option.dblclick();
  }
  await expect(page.locator('#accountPickerModal')).toBeHidden();
}

/** Shows the flat کل/معین list and picks a leaf account from it. */
export async function chooseFlatAccount(page, row, name) {
  const input = row.locator('.vf-code-input');
  await input.click();
  await input.press('Space');
  await expect(page.locator('#accountPickerModal')).toBeVisible();
  await page.keyboard.press('F6');
  await expect(page.locator('#accountPickerPath')).toContainText('همه حساب‌های پایه');
  const option = pickerRow(page, name).first();
  await expect(option).toBeVisible();
  await option.dblclick();
  await expect(page.locator('#accountPickerModal')).toBeHidden();
}

/** Two balanced lines: a detail account on the debit side, a معین on the credit side. */
export async function fillBalancedVoucher(page) {
  const first = page.locator('#voucherRows tr').nth(0);
  await chooseBaseAccount(page, first, ['کل دارای معین', 'معین دارای تفصیلی', 'تفصیلی یک']);
  await first.locator('.vf-debit-input').fill('1000');

  const second = page.locator('#voucherRows tr').nth(1);
  await chooseFlatAccount(page, second, 'معین بدون تفصیلی');
  await second.locator('.vf-credit-input').fill('1000');
}

export async function saveVoucher(page) {
  await page.locator('[data-action="save"]').first().click();
}

export async function backendVouchers(page) {
  return page.evaluate(() => window.__E2E__.backend.vouchers);
}

export async function backendAccounts(page) {
  return page.evaluate(() => window.__E2E__.backend.accounts);
}

export async function toastText(page) {
  return page.locator('#vfToast').textContent();
}
