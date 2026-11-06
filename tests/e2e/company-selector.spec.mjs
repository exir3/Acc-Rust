import { fileURLToPath } from 'node:url';
import { test, expect } from '@playwright/test';
import { buildSeed, FISCAL_YEARS } from './fixtures.mjs';

const MOCK_TAURI = fileURLToPath(new URL('./mock-tauri.js', import.meta.url));

async function openMainWindow(page, overrides = {}) {
  const seed = buildSeed(overrides);
  await page.addInitScript((data) => {
    window.__E2E_SEED__ = data;
  }, seed);
  await page.addInitScript({ path: MOCK_TAURI });
  await page.goto('/');
  await expect(page.locator('#companySelector')).toBeVisible();
  await expect(page.locator('#csfYearList .csf-row').first()).toBeVisible();
}

test('startup selector shows the company, preselects the running year, and enters the app', async ({ page }) => {
  await openMainWindow(page);

  await expect(page.locator('#csfCompanyName')).toHaveText('شرکت آزمون');
  await expect(page.locator('#csfYearList .csf-row')).toHaveCount(2);
  await expect(page.locator('#csfYearList .csf-row.selected')).toContainText('سال مالی ۱۴۰۵');
  await expect(page.locator('#csfEnter')).toBeEnabled();
  await expect(page.locator('#statusFiscal')).toHaveText('سال مالی: ۱۴۰۵');

  await page.locator('#csfEnter').click();

  await expect(page.locator('#companySelector')).toBeHidden();
  await expect(page.locator('.accounting-app')).toBeVisible();
  await expect(page.locator('#statusFiscal')).toContainText('۱۴۰۵');
  await expect(page.locator('#statusText')).toContainText('سال مالی');
});

test('fiscal-year search narrows the list and clears back to all years', async ({ page }) => {
  await openMainWindow(page);

  await page.locator('#csfSearch').fill('۱۴۰۴');
  await expect(page.locator('#csfYearList > li:not([hidden])')).toHaveCount(1);
  await expect(page.locator('#csfYearList > li:not([hidden])')).toContainText('سال مالی ۱۴۰۴');
  await expect(page.locator('#csfCount')).toHaveText('۱ از ۲ سال مالی');

  await page.locator('#csfSearchClear').click();
  await expect(page.locator('#csfSearch')).toHaveValue('');
  await expect(page.locator('#csfYearList > li:not([hidden])')).toHaveCount(2);
});

test('a new fiscal year can be created from the selector and becomes selected', async ({ page }) => {
  await openMainWindow(page);

  await page.locator('#csfNewYear').click();
  await expect(page.locator('#modal')).toBeVisible();
  await page.locator('#fyTitle').fill('سال مالی ۱۴۰۶');
  await page.locator('#fyStart').fill('1406/01/01');
  await page.locator('#fyEnd').fill('1406/12/29');
  await page.locator('#fiscalYearForm [type="submit"]').click();

  await expect(page.locator('#modal')).toBeHidden();
  await expect(page.locator('#csfYearList .csf-row')).toHaveCount(3);
  await expect(page.locator('#csfYearList .csf-row.selected')).toContainText('سال مالی ۱۴۰۶');
  await expect(page.locator('#csfNotice')).toContainText('سال مالی «سال مالی ۱۴۰۶» ایجاد شد.');
  await expect(page.locator('#csfCount')).toHaveText('۳ سال مالی');
});

test('backup action reports the saved backup path without leaving the selector', async ({ page }) => {
  await openMainWindow(page);

  await page.locator('#csfBackup').click();

  await expect(page.locator('#csfNotice')).toContainText('/e2e/backups/hesabdari-backup.db');
  await expect(page.locator('#companySelector')).toBeVisible();
});

test('restore asks for confirmation and reports the safety copy', async ({ page }) => {
  await openMainWindow(page);

  await page.locator('#csfRestore').click();
  await expect(page.locator('#modal')).toBeVisible();
  await expect(page.locator('#dialogTitle')).toHaveText('بازیابی پشتیبان');
  await page.locator('#dialogFoot').getByRole('button', { name: 'بازیابی' }).click();

  await expect(page.locator('#csfNotice')).toContainText('/e2e/backups/hesabdari-safety.db');
  await expect(page.locator('#companySelector')).toBeVisible();
});

test('a year with vouchers cannot be deleted, while an empty year can', async ({ page }) => {
  const emptyYear = {
    ...FISCAL_YEARS[1],
    id: 3,
    title: 'سال مالی ۱۴۰۶',
    startDate: '1406/01/01',
    endDate: '1406/12/29',
    voucherCount: 0,
  };
  await openMainWindow(page, {
    fiscalYears: [...FISCAL_YEARS.map((year) => ({ ...year })), emptyYear],
  });

  await expect(page.locator('#csfYearList .csf-row.selected')).toContainText('سال مالی ۱۴۰۵');
  await expect(page.locator('#csfDeleteYear')).toBeDisabled();

  await page.locator('#csfYearList .csf-row', { hasText: 'سال مالی ۱۴۰۶' }).click();
  await expect(page.locator('#csfDeleteYear')).toBeEnabled();
  await page.locator('#csfDeleteYear').click();
  await page.locator('#dialogFoot').getByRole('button', { name: 'حذف' }).click();

  await expect(page.locator('#csfYearList .csf-row')).toHaveCount(2);
  await expect(page.locator('#csfNotice')).toContainText('سال مالی «سال مالی ۱۴۰۶» حذف شد.');
});

test('the settings menu reopens the selector on the current year', async ({ page }) => {
  await openMainWindow(page);

  await page.locator('#csfEnter').click();
  await expect(page.locator('#companySelector')).toBeHidden();

  await page.getByRole('button', { name: 'تنظیمات', exact: true }).click();
  await page.getByRole('menuitem', { name: 'انتخاب سال مالی' }).click();

  await expect(page.locator('#companySelector')).toBeVisible();
  await expect(page.locator('#csfYearList .csf-row.selected')).toContainText('سال مالی ۱۴۰۵');
  await expect(page.locator('#csfEnter')).toBeEnabled();
});
