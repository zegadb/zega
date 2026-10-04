import { expect, test } from '@playwright/test';

test('mounts the explorer in the host chrome and serves its editor and wasm locally', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator('html')).toHaveAttribute('data-ready', 'yes');
  await expect(page.locator('#dashboard-chrome #btn-run')).toBeVisible();
  await expect(page.locator('#explorer #panes')).toBeVisible();
  await expect(page.locator('#explorer .monaco-editor')).toHaveCount(4);
  await expect(page.locator('#dashboard-chrome #conn-label')).toHaveText('Local explorer');
  expect(await page.locator('iframe').count()).toBe(0);
  const external = await page.evaluate(() => performance.getEntriesByType('resource').filter((entry) => new URL(entry.name).origin !== location.origin).map((entry) => entry.name));
  expect(external).toEqual([]);
});
