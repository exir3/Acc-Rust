import { fileURLToPath } from 'node:url';
import { test, expect } from '@playwright/test';
import { buildSeed } from './fixtures.mjs';

const MOCK_TAURI = fileURLToPath(new URL('./mock-tauri.js', import.meta.url));

const VIEWPORT_HEIGHT = 520;
const DIALOG_MARGIN = 32;

const inferredGroupMappings = Array.from({ length: 24 }, (_, index) => ({
  accountCode: `5251${String(index + 1).padStart(2, '0')}`,
  accountName: `حساب حدسی شماره ${index + 1}`,
  level: 2,
  groupCode: '10',
  reason: 'گروه از پیشوند کد حساب کل حدس زده شد.',
}));

async function openBulkImportDialog(page) {
  const seed = buildSeed({
    bulkImportPreview: {
      fileName: 'Book1.xlsx',
      sheetName: 'اسناد',
      voucherCount: 5282,
      lineCount: 114699,
      accountCount: 1445,
      newAccountCount: 830,
      numberConflictCount: 0,
      conflictingVoucherNumbers: [],
      inferredGroupCount: inferredGroupMappings.length,
      inferredGroupMappings,
    },
  });
  await page.addInitScript((data) => {
    window.__E2E_SEED__ = data;
  }, seed);
  await page.addInitScript({ path: MOCK_TAURI });
  await page.goto('/');
  await expect(page.locator('#companySelector')).toBeVisible();
  await page.locator('#csfEnter').click();
  await expect(page.locator('.accounting-app')).toBeVisible();

  await page.getByRole('button', { name: 'اطلاعات پایه' }).click();
  await page.getByRole('menuitem', { name: 'ورود دسته‌جمعی اسناد از اکسل' }).click();
  await expect(page.locator('#dialogTitle')).toHaveText('ورود دسته‌جمعی اسناد از اکسل');
}

test.use({ viewport: { width: 1280, height: VIEWPORT_HEIGHT } });

test('the bulk import preview stays inside the window and scrolls its body', async ({ page }) => {
  await openBulkImportDialog(page);
  await page.locator('#dialogFoot').getByRole('button', { name: 'انتخاب فایل اکسل' }).click();
  await expect(page.locator('#dialogBody')).toContainText('تعداد سند');
  await expect(page.locator('#dialogBody')).toContainText('۵۲۸۲');

  for (const selector of ['#modal .dialog', '#dialogTitle', '#dialogFoot']) {
    const box = await page.locator(selector).boundingBox();
    expect(box, `${selector} should be rendered`).not.toBeNull();
    expect(box.y, `${selector} top`).toBeGreaterThanOrEqual(0);
    expect(box.y + box.height, `${selector} bottom`).toBeLessThanOrEqual(VIEWPORT_HEIGHT);
  }

  const dialogBox = await page.locator('#modal .dialog').boundingBox();
  expect(dialogBox.height).toBeLessThanOrEqual(VIEWPORT_HEIGHT - DIALOG_MARGIN);

  const bodyOverflow = await page.locator('#dialogBody').evaluate(
    (body) => body.scrollHeight - body.clientHeight,
  );
  expect(bodyOverflow, 'the preview should scroll instead of overflowing the window').toBeGreaterThan(0);

  await page.locator('#dialogBody').evaluate((body) => body.scrollTo(0, body.scrollHeight));
  await expect(page.locator('#bulkImportConfirmInferred')).toBeInViewport();
});
