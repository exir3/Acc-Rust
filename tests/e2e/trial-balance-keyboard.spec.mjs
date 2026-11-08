import { test, expect } from '@playwright/test';
import { openTrialBalancePage } from './helpers.mjs';

test('arrow keys in the search box keep the caret and do not move the grid selection', async ({ page }) => {
  await openTrialBalancePage(page);

  const search = page.locator('#tbpSearch');
  await search.focus();
  await expect(search).toBeFocused();

  const first = page.locator('.tbp-data-row[data-code="1000"]');
  const second = page.locator('.tbp-data-row[data-code="2000"]');
  await expect(first).toHaveClass(/selected/);

  // The search field is a text input: ArrowDown must stay inside it.
  await search.press('ArrowDown');
  await expect(search).toBeFocused();
  await expect(first).toHaveClass(/selected/);
  await expect(second).not.toHaveClass(/selected/);

  // Once focus is on the grid, the same key moves the selected row.
  await first.click();
  await expect(first).toBeFocused();
  await page.keyboard.press('ArrowDown');
  await expect(second).toHaveClass(/selected/);
  await expect(second).toBeFocused();
  await expect(first).not.toHaveClass(/selected/);

  // And the search box still owns its arrow keys afterwards.
  await search.focus();
  await search.press('ArrowDown');
  await expect(second).toHaveClass(/selected/);
});
