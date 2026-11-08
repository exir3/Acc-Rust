import { fileURLToPath } from 'node:url';
import { expect, test } from '@playwright/test';
import { buildSeed } from './fixtures.mjs';

const MOCK_TAURI = fileURLToPath(new URL('./mock-tauri.js', import.meta.url));

test('both balance-sheet menu items open the balance-sheet page', async ({ page }) => {
  await page.addInitScript((seed) => {
    window.__E2E_SEED__ = seed;
  }, buildSeed());
  await page.addInitScript({ path: MOCK_TAURI });
  await page.goto('/');
  await expect(page.locator('#companySelector')).toBeVisible();
  await page.locator('#csfEnter').click();
  await expect(page.locator('#companySelector')).toBeHidden();

  const menuCases = [
    { menu: 'گزارشات حسابداری', item: 'ترازنامه' },
    { menu: 'صورت‌های مالی', item: 'ترازنامه (صورت وضعیت مالی)' },
  ];

  for (const { menu, item } of menuCases) {
    await page.locator('#menubarNav .menu-root', { hasText: menu }).click();
    const menuItem = page.getByRole('menuitem', { name: item, exact: true });
    await menuItem.click();

    await expect(page.locator('#pageTitle')).toHaveText('ترازنامه');
    await expect(page.locator('form[data-filter="balance"]')).toBeVisible();
    await expect(page.getByText('وضعیت ترازنامه')).toBeVisible();
  }
});
