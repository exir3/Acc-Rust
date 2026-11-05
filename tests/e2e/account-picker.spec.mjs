import { test, expect } from '@playwright/test';
import { openVoucherPage, pickerRow, pickerRows } from './helpers.mjs';

test('F2 drills into children and the back button returns to the root level', async ({ page }) => {
  await openVoucherPage(page);
  const row = page.locator('#voucherRows tr').first();
  await row.locator('.vf-code-input').click();
  await row.locator('.vf-code-input').press('Space');

  const modal = page.locator('#accountPickerModal');
  await expect(modal).toBeVisible();
  await expect(page.locator('#accountPickerPath')).toContainText('حساب‌های کل');
  await expect(pickerRows(page)).toHaveCount(2);

  await page.keyboard.press('F2');
  await expect(page.locator('#accountPickerPath')).toContainText('کل دارای معین');
  await expect(pickerRows(page)).toHaveCount(2);
  await expect(pickerRow(page, 'معین دارای تفصیلی')).toBeVisible();

  // Only direct children are shown: the detail accounts are a level deeper.
  await expect(pickerRow(page, 'تفصیلی یک')).toHaveCount(0);

  await page.locator('#accountPickerBack').click();
  await expect(page.locator('#accountPickerPath')).toContainText('حساب‌های کل');
  await expect(pickerRow(page, 'کل دارای معین')).toBeVisible();
  await expect(pickerRow(page, 'معین دارای تفصیلی')).toHaveCount(0);
  await expect(pickerRows(page)).toHaveCount(2);
});

test('double-click walks کل ← معین ← تفصیلی and fills both account columns', async ({ page }) => {
  await openVoucherPage(page);
  const row = page.locator('#voucherRows tr').first();
  await row.locator('.vf-code-input').click();
  await row.locator('.vf-code-input').press('Space');

  await pickerRow(page, 'کل دارای معین').dblclick();
  await expect(page.locator('#accountPickerPath')).toContainText('کل دارای معین');

  await pickerRow(page, 'معین دارای تفصیلی').dblclick();
  await expect(page.locator('#accountPickerPath')).toContainText('معین دارای تفصیلی');
  await expect(pickerRows(page)).toHaveCount(2);
  await expect(pickerRow(page, 'تفصیلی یک')).toBeVisible();
  await expect(pickerRow(page, 'تفصیلی دو')).toBeVisible();

  await pickerRow(page, 'تفصیلی یک').dblclick();
  await expect(page.locator('#accountPickerModal')).toBeHidden();

  // Choosing a detail from the base picker must keep the pair valid.
  await expect(row.locator('.vf-code-input')).toHaveValue('100001');
  await expect(row.locator('.vf-detail1-input')).toHaveValue('100001001');
});

test('F6 shows a flat کل/معین list and back leaves that mode first', async ({ page }) => {
  await openVoucherPage(page);
  const row = page.locator('#voucherRows tr').first();
  await row.locator('.vf-code-input').click();
  await row.locator('.vf-code-input').press('Space');

  await page.keyboard.press('F6');
  await expect(page.locator('#accountPickerPath')).toContainText('همه حساب‌های پایه');
  await expect(pickerRows(page)).toHaveCount(5);
  await expect(pickerRow(page, 'معین دوم')).toBeVisible();

  await page.locator('#accountPickerBack').click();
  await expect(page.locator('#accountPickerPath')).toContainText('حساب‌های کل');
  await expect(pickerRows(page)).toHaveCount(2);
  await expect(pickerRow(page, 'معین دوم')).toHaveCount(0);
});

test('search is independent of the drill level and Enter selects the match', async ({ page }) => {
  await openVoucherPage(page);
  const row = page.locator('#voucherRows tr').first();
  await row.locator('.vf-code-input').click();
  await row.locator('.vf-code-input').press('Space');

  await page.locator('#accountPickerSearch').fill('معین دوم');
  await expect(pickerRows(page)).toHaveCount(1);
  await expect(pickerRow(page, 'معین دوم')).toBeVisible();

  await page.keyboard.press('Enter');
  await expect(page.locator('#accountPickerModal')).toBeHidden();
  await expect(row.locator('.vf-code-input')).toHaveValue('200001');
  await expect(row.locator('.vf-detail1-input')).toHaveValue('');
});
