import { test, expect } from '@playwright/test';
import { openAccountDefinitionPage } from './helpers.mjs';

test('cancelling an edited account asks before discarding the changes', async ({ page }) => {
  await openAccountDefinitionPage(page);

  await page.locator('#adNew').click();
  await expect(page.locator('#adEditActions')).toBeVisible();
  await page.locator('#adName').fill('حساب لغو‌شده');

  // «خیر» keeps the form and the typed name.
  await page.locator('#adCancel').click();
  const confirm = page.locator('#adConfirm');
  await expect(confirm).toBeVisible();
  await expect(page.locator('#adConfirmTitle')).toHaveText('لغو ویرایش');
  await page.locator('#adConfirmNo').click();
  await expect(confirm).toBeHidden();
  await expect(page.locator('#adEditActions')).toBeVisible();
  await expect(page.locator('#adName')).toHaveValue('حساب لغو‌شده');

  // Escape follows the same path, and confirming returns to view mode.
  await page.locator('#adName').press('Escape');
  await expect(confirm).toBeVisible();
  await page.locator('#adConfirmYes').click();
  await expect(confirm).toBeHidden();
  await expect(page.locator('#adEditActions')).toBeHidden();
  await expect(page.locator('#adName')).toHaveValue('کل دارای معین');

  const accountCount = await page.evaluate(() => window.__E2E__.backend.accounts.length);
  expect(accountCount).toBe(7);
});

test('cross-level search opens a result in its tree position and restore returns to it', async ({ page }) => {
  await openAccountDefinitionPage(page);

  const search = page.locator('#adSearch');
  await search.fill('تفصیلی یک');
  await expect(page.locator('#adSearchStatus')).toContainText('حساب یافت شد');
  await expect(page.locator('#adGridBody .ad-row')).toHaveCount(1);

  const resultRow = page.locator('#adGridBody .ad-row', { hasText: 'تفصیلی یک' });
  await expect(resultRow).toHaveAttribute('title', /کل دارای معین ← معین دارای تفصیلی/);
  await expect(resultRow.locator('.ad-path'))
    .toHaveText('کل دارای معین ← معین دارای تفصیلی');

  // Activating a result moves the drill path to the account's parent level.
  await resultRow.click();
  await expect(search).toHaveValue('');
  await expect(page.locator('#adBreadcrumb [data-goto="100001"]'))
    .toHaveAttribute('aria-current', 'location');
  await expect(page.locator('#adBreadcrumb')).toContainText('کل دارای معین');
  await expect(page.locator('#adBreadcrumb')).toContainText('معین دارای تفصیلی');
  await expect(page.locator('#adGridBody .ad-row', { hasText: 'تفصیلی یک' }))
    .toHaveClass(/selected/);

  // A new search starts from this level; clearing it restores that level.
  await search.fill('بدون معین');
  await expect(page.locator('#adGridBody .ad-row[data-id="2"]')).toBeVisible();
  await page.locator('#adSearchClear').click();
  await expect(search).toHaveValue('');
  await expect(page.locator('#adBreadcrumb [data-goto="100001"]'))
    .toHaveAttribute('aria-current', 'location');
  await expect(page.locator('#adGridBody .ad-row', { hasText: 'تفصیلی یک' }))
    .toHaveClass(/selected/);

  // Escape inside the search box clears the search the same way.
  await search.fill('دارایی');
  await expect(page.locator('#adGridBody .ad-row')).toHaveCount(7);
  await search.press('Escape');
  await expect(search).toHaveValue('');
  await expect(page.locator('#adBreadcrumb [data-goto="100001"]'))
    .toHaveAttribute('aria-current', 'location');
});

test('saving an account marks and focuses the invalid field', async ({ page }) => {
  await openAccountDefinitionPage(page);

  await page.locator('#adNew').click();
  await page.locator('#adSave').click();

  const name = page.locator('#adName');
  await expect(name).toHaveClass(/is-invalid/);
  await expect(name).toHaveAttribute('aria-invalid', 'true');
  await expect(name).toBeFocused();
  await expect(page.locator('.ad-field-name .ad-field-error'))
    .toHaveText('نام حساب الزامی است.');
  await expect(page.locator('#adCode')).not.toHaveClass(/is-invalid/);
  await expect(page.locator('#adErrorText')).toContainText('کد و نام حساب');

  // Typing in the field clears its own error, and the save then succeeds.
  await name.fill('کل اعتبارسنجی');
  await expect(name).not.toHaveClass(/is-invalid/);
  await expect(page.locator('.ad-field-name .ad-field-error')).toHaveCount(0);

  await page.locator('#adSave').click();
  await expect(page.locator('#adToast')).toContainText('حساب جدید ثبت شد');
  await expect(page.locator('#adGridBody .ad-row', { hasText: 'کل اعتبارسنجی' })).toBeVisible();
});
