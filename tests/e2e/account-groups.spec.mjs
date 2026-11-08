import { test, expect } from '@playwright/test';
import { openAccountGroupsPage } from './helpers.mjs';

test('the Cancel button confirms before discarding an edited group', async ({ page }) => {
  await openAccountGroupsPage(page);

  await page.locator('#agEdit').click();
  await expect(page.locator('#agCancel')).toBeVisible();
  await page.locator('#agName').fill('گروه ویرایش‌شده');

  // «خیر» keeps the form open with the typed name.
  await page.locator('#agCancel').click();
  const confirm = page.locator('#agConfirm');
  await expect(confirm).toBeVisible();
  await expect(page.locator('#agConfirmTitle')).toHaveText('لغو ویرایش');
  await page.locator('#agConfirmNo').click();
  await expect(confirm).toBeHidden();
  await expect(page.locator('#agCancel')).toBeVisible();
  await expect(page.locator('#agName')).toHaveValue('گروه ویرایش‌شده');

  // Confirming discards the draft and returns to the selected group.
  await page.locator('#agCancel').click();
  await expect(confirm).toBeVisible();
  await page.locator('#agConfirmYes').click();
  await expect(confirm).toBeHidden();
  await expect(page.locator('#agCancel')).toBeHidden();
  await expect(page.locator('#agName')).toHaveValue('دارایی‌ها');
  await expect(page.locator('#agGridBody .ag-row')).toHaveCount(1);

  const groupCount = await page.evaluate(() => window.__E2E__.backend.groups.length);
  expect(groupCount).toBe(1);
});

test('saving a group marks and focuses the invalid field', async ({ page }) => {
  await openAccountGroupsPage(page);

  await page.locator('#agNew').click();
  await page.locator('#agSave').click();

  const name = page.locator('#agName');
  await expect(name).toHaveClass(/is-invalid/);
  await expect(name).toHaveAttribute('aria-invalid', 'true');
  await expect(name).toBeFocused();
  await expect(page.locator('.ag-field-name .ag-field-error'))
    .toHaveText('نام گروه الزامی است.');
  await expect(page.locator('#agCode')).not.toHaveClass(/is-invalid/);
  await expect(page.locator('#agErrorText')).toContainText('کد و نام گروه');

  await name.fill('گروه آزمایشی');
  await expect(name).not.toHaveClass(/is-invalid/);
  await expect(page.locator('.ag-field-name .ag-field-error')).toHaveCount(0);
});
