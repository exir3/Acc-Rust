import { test, expect } from '@playwright/test';
import { openAccountDefinitionPage } from './helpers.mjs';

test('drill-down opens levels and the back button returns to the root', async ({ page }) => {
  await openAccountDefinitionPage(page);

  await expect(page.locator('#adBreadcrumb')).toHaveCount(0);
  await expect(page.locator('#adGridBody .ad-row')).toHaveCount(2);
  await expect(page.locator('#adBack')).toBeDisabled();

  await page.locator('#adGridBody .ad-row', { hasText: 'کل دارای معین' }).dblclick();
  await expect(page.locator('#adGridBody .ad-row', { hasText: 'معین دارای تفصیلی' })).toBeVisible();
  await expect(page.locator('#adGridBody .ad-row', { hasText: 'کل بدون معین' })).toHaveCount(0);
  await expect(page.locator('#adBack')).toBeEnabled();

  await page.locator('#adGridBody .ad-row', { hasText: 'معین دارای تفصیلی' }).dblclick();
  await expect(page.locator('#adGridBody .ad-row', { hasText: 'تفصیلی یک' })).toBeVisible();
  await expect(page.locator('#adGridBody .ad-row')).toHaveCount(2);

  await page.locator('#adBack').click();
  await expect(page.locator('#adGridBody .ad-row', { hasText: 'معین دارای تفصیلی' })).toBeVisible();
  await expect(page.locator('#adBack')).toBeEnabled();

  await page.locator('#adBack').click();
  await expect(page.locator('#adGridBody .ad-row')).toHaveCount(2);
  await expect(page.locator('#adBack')).toBeDisabled();
});

test('the edit button opens the selected account in the modal form', async ({ page }) => {
  await openAccountDefinitionPage(page);

  await page.locator('#adGridBody .ad-row', { hasText: 'کل دارای معین' }).click();
  await page.locator('#adEdit').click();

  await expect(page.locator('#adFormModal')).toBeVisible();
  await expect(page.locator('#adFormTitle')).toHaveText('اصلاح حساب');
  await expect(page.locator('#adName')).toHaveValue('کل دارای معین');
  await expect(page.locator('#adCode')).toBeDisabled();

  await page.locator('#adCancel').click();
  await expect(page.locator('#adFormModal')).toBeHidden();
});

test('defines a new level-1 account in a modal form after validating it', async ({ page }) => {
  await openAccountDefinitionPage(page);

  await expect(page.locator('#adFormModal')).toBeHidden();
  await page.locator('#adNew').click();
  await expect(page.locator('#adFormModal')).toBeVisible();
  await expect(page.locator('#adFormTitle')).toHaveText('تعریف حساب جدید');
  await expect(page.locator('#adEditActions')).toHaveCount(0);
  await expect(page.locator('#adCode')).toBeEnabled();
  await expect(page.locator('#adCode')).toHaveValue(/^[۰-۹]{4}$/);

  // The name is required; the form reports the problem instead of saving.
  await page.locator('#adSave').click();
  await expect(page.locator('#adFormError')).toBeVisible();
  await expect(page.locator('#adFormErrorText')).toContainText('کد و نام حساب');
  await expect(page.locator('#adError')).toBeHidden();
  await expect.poll(async () => (await page.evaluate(() => window.__E2E__.backend.accounts)).length).toBe(7);

  await page.locator('#adName').fill('کل آزمایشی');
  await page.locator('#adSave').click();
  await expect(page.locator('#adToast')).toContainText('حساب جدید ثبت شد');
  await expect(page.locator('#adGridBody .ad-row', { hasText: 'کل آزمایشی' })).toBeVisible();
  await expect(page.locator('#adFormModal')).toBeHidden();

  const saved = await page.evaluate(() => window.__E2E__.backend.accounts
    .find((account) => account.name === 'کل آزمایشی'));
  expect(saved).toMatchObject({ level: 1, parentCode: '', isActive: true });
  expect(saved.code).toMatch(/^\d{4}$/);
});
