import { expect, test } from '@playwright/test';
import { openAccountDefinitionPage, openVoucherPage } from './helpers.mjs';

// Layout and accessibility regressions that a fixed desktop viewport would
// hide. The audit asked for the supported window sizes, browser zoom, full
// keyboard movement and popup focus handling to be covered automatically.

async function horizontalOverflow(page) {
  return page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
}

test.describe('supported window sizes', () => {
  for (const size of [
    { width: 1280, height: 720 },
    { width: 1366, height: 768 },
    { width: 1920, height: 1080 },
  ]) {
    test(`account definition fits ${size.width}x${size.height}`, async ({ page }) => {
      await page.setViewportSize(size);
      await openAccountDefinitionPage(page);

      await expect(page.locator('#adGridBody .ad-row').first()).toBeVisible();
      await expect(page.locator('#adNew')).toBeVisible();
      await expect(page.locator('#adSave')).toBeHidden();
      expect(await horizontalOverflow(page), 'page must not scroll sideways').toBeLessThanOrEqual(2);
    });

    test(`voucher form fits ${size.width}x${size.height}`, async ({ page }) => {
      await page.setViewportSize(size);
      await openVoucherPage(page);

      await expect(page.locator('#voucherRows tr').first()).toBeVisible();
      await expect(page.locator('[data-action="save"]').first()).toBeVisible();
      expect(await horizontalOverflow(page), 'page must not scroll sideways').toBeLessThanOrEqual(2);
    });
  }

  test('account definition still fits at 1024x768', async ({ page }) => {
    await page.setViewportSize({ width: 1024, height: 768 });
    await openAccountDefinitionPage(page);

    await expect(page.locator('#adGridBody .ad-row').first()).toBeVisible();
    await expect(page.locator('#adNew')).toBeVisible();
    expect(await horizontalOverflow(page), 'page must not scroll sideways').toBeLessThanOrEqual(2);
  });

  test('voucher form still fits at 1024x768', async ({ page }) => {
    await page.setViewportSize({ width: 1024, height: 768 });
    await openVoucherPage(page);

    await expect(page.locator('#voucherRows tr').first()).toBeVisible();
    await expect(page.locator('[data-action="save"]').first()).toBeVisible();
    expect(await horizontalOverflow(page), 'page must not scroll sideways').toBeLessThanOrEqual(2);
  });

  for (const zoom of ['1.25', '1.5']) {
    test(`account definition stays usable at ${zoom}x browser zoom`, async ({ page }) => {
      await page.setViewportSize({ width: 1366, height: 768 });
      await openAccountDefinitionPage(page);
      await page.evaluate((value) => {
        document.documentElement.style.zoom = value;
      }, zoom);

      await expect(page.locator('#adGridBody .ad-row').first()).toBeVisible();
      await expect(page.locator('#adNew')).toBeVisible();
      expect(await horizontalOverflow(page), 'page must not scroll sideways').toBeLessThanOrEqual(2);
    });
  }
});

test('keyboard moves selection through the account grid and drills in', async ({ page }) => {
  await openAccountDefinitionPage(page);

  const rows = page.locator('#adGridBody .ad-row');
  await rows.first().focus();
  await expect(rows.first()).toBeFocused();

  await page.keyboard.press('ArrowDown');
  await expect(rows.nth(1)).toHaveAttribute('aria-selected', 'true');
  await expect(rows.nth(1)).toBeFocused();

  await page.keyboard.press('ArrowUp');
  await expect(rows.first()).toHaveAttribute('aria-selected', 'true');

  // Enter drills into the selected account; Backspace returns to the root.
  await page.keyboard.press('Enter');
  await expect(page.locator('#adGridBody .ad-row', { hasText: 'معین دارای تفصیلی' })).toBeVisible();
  await expect(page.locator('#adGridBody .ad-row[data-id="1"]')).toHaveCount(0);
  await expect(page.locator('#adBack')).toBeEnabled();
  await page.keyboard.press('Backspace');
  await expect(page.locator('#adGridBody .ad-row')).toHaveCount(2);
  await expect(page.locator('#adBack')).toBeDisabled();
});

test('confirmation dialog traps focus and returns it to the trigger', async ({ page }) => {
  await openAccountDefinitionPage(page);

  // Drill to a leaf account so delete opens the confirmation instead of the
  // "account has children" error.
  await page.locator('#adGridBody .ad-row', { hasText: 'کل دارای معین' }).dblclick();
  await expect(page.locator('#adGridBody .ad-row', { hasText: 'معین دارای تفصیلی' })).toBeVisible();
  await page.locator('#adGridBody .ad-row', { hasText: 'معین دارای تفصیلی' }).dblclick();
  await expect(page.locator('#adGridBody .ad-row', { hasText: 'تفصیلی یک' })).toBeVisible();
  await page.locator('#adGridBody .ad-row', { hasText: 'تفصیلی یک' }).click();

  const trigger = page.locator('#adDelete');
  await trigger.click();
  const dialog = page.locator('#adConfirmBox');
  await expect(dialog).toBeVisible();
  await expect(page.locator('#adConfirmNo')).toBeFocused();

  await page.keyboard.press('Tab');
  await expect(page.locator('#adConfirmYes')).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(page.locator('#adConfirmNo')).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  await expect(page.locator('#adConfirmYes')).toBeFocused();

  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await expect(trigger).toBeFocused();
});
