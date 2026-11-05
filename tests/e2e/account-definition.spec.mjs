import { test, expect } from '@playwright/test';
import { openAccountDefinitionPage } from './helpers.mjs';

test('breadcrumb tracks the drill path and returns to the root level', async ({ page }) => {
  await openAccountDefinitionPage(page);

  const rootButton = page.locator('#adBreadcrumb [data-root]');
  await expect(rootButton).toHaveText('حساب‌های کل');
  await expect(rootButton).toBeDisabled();
  await expect(page.locator('#adGridBody .ad-row')).toHaveCount(2);

  await page.locator('#adGridBody .ad-row', { hasText: 'کل دارای معین' }).dblclick();
  await expect(page.locator('#adGridBody .ad-row', { hasText: 'معین دارای تفصیلی' })).toBeVisible();
  await expect(page.locator('#adGridBody .ad-row', { hasText: 'کل بدون معین' })).toHaveCount(0);
  await expect(page.locator('#adBreadcrumb [data-goto="1000"]')).toHaveAttribute('aria-current', 'location');

  await page.locator('#adGridBody .ad-row', { hasText: 'معین دارای تفصیلی' }).dblclick();
  await expect(page.locator('#adGridBody .ad-row', { hasText: 'تفصیلی یک' })).toBeVisible();
  await expect(page.locator('#adGridBody .ad-row')).toHaveCount(2);
  await expect(page.locator('#adBreadcrumb [data-goto="100001"]')).toHaveAttribute('aria-current', 'location');
  await expect(page.locator('#adBreadcrumb')).toContainText('کل دارای معین');
  await expect(page.locator('#adBreadcrumb')).toContainText('معین دارای تفصیلی');

  await rootButton.click();
  await expect(page.locator('#adBreadcrumb')).toContainText('حساب‌های کل');
  await expect(page.locator('#adBreadcrumb [data-goto]')).toHaveCount(0);
  await expect(page.locator('#adGridBody .ad-row')).toHaveCount(2);
  await expect(rootButton).toBeDisabled();
});

test('defines a new level-1 account after validating the form', async ({ page }) => {
  await openAccountDefinitionPage(page);

  await page.locator('#adNew').click();
  await expect(page.locator('#adEditActions')).toBeVisible();
  await expect(page.locator('#adCode')).toBeEnabled();
  await expect(page.locator('#adCode')).toHaveValue(/^[۰-۹]{4}$/);

  // The name is required; the form reports the problem instead of saving.
  await page.locator('#adSave').click();
  await expect(page.locator('#adError')).toBeVisible();
  await expect(page.locator('#adErrorText')).toContainText('کد و نام حساب');
  await expect.poll(async () => (await page.evaluate(() => window.__E2E__.backend.accounts)).length).toBe(7);

  await page.locator('#adName').fill('کل آزمایشی');
  await page.locator('#adSave').click();
  await expect(page.locator('#adToast')).toContainText('حساب جدید ثبت شد');
  await expect(page.locator('#adGridBody .ad-row', { hasText: 'کل آزمایشی' })).toBeVisible();
  await expect(page.locator('#adEditActions')).toBeHidden();

  const saved = await page.evaluate(() => window.__E2E__.backend.accounts
    .find((account) => account.name === 'کل آزمایشی'));
  expect(saved).toMatchObject({ level: 1, parentCode: '', isActive: true });
  expect(saved.code).toMatch(/^\d{4}$/);
});
