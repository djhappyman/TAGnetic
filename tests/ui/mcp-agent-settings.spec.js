import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';

const root = resolve('.test-dist');
test.beforeEach(async ({ page }) => {
  await page.route('http://tagrex.test/**', async route => {
    const path = resolve(root, '.' + new URL(route.request().url()).pathname);
    if (!path.startsWith(root + sep)) return route.abort();
    try {
      await route.fulfill({ body: await readFile(path), contentType: path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css' : path.endsWith('.woff2') ? 'font/woff2' : 'text/html' });
    } catch { await route.fulfill({ status: 404, body: 'Missing fixture' }); }
  });
});

for (const theme of ['dark', 'light']) for (const width of [1280, 620]) {
  test(`agent setup stays contained and unavailable in the browser fixture (${theme}, ${width}px)`, async ({ page }) => {
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.setViewportSize({ width, height: 850 });
    await page.addInitScript(mode => localStorage.setItem('tagrex.theme', mode), theme);
    await page.goto('http://tagrex.test/app/ui/index.html');
    await expect.poll(() => page.evaluate(() => typeof window.tagrex?.setMode)).toBe('function');
    await page.locator('#settings-open').click();
    await page.getByRole('tab', { name: 'External agents', exact: true }).click();
    const panel = page.locator('.agent-connections');
    await expect(panel).toBeVisible();
    await expect(panel.getByText('Agent connections are available in the desktop app.')).toBeVisible();
    await expect(page.locator('#mcp-enable')).toBeDisabled();
    await expect(page.locator('#mcp-agent-label')).toHaveCount(0);
    await expect(page.locator('#mcp-connection-config')).toHaveCount(0);
    const panelBox = await panel.boundingBox();
    const buttonBox = await page.locator('#mcp-enable').boundingBox();
    expect(buttonBox.x).toBeGreaterThanOrEqual(panelBox.x);
    expect(buttonBox.x + buttonBox.width).toBeLessThanOrEqual(panelBox.x + panelBox.width + 1);
    await page.getByRole('tab', { name: 'Shortcuts', exact: true }).click();
    await expect(page.locator('#set-shortcuts')).toBeVisible();
    await expect(panel).toBeHidden();
    await page.getByRole('tab', { name: 'External agents', exact: true }).click();
    await expect(panel).toBeVisible();
    expect(errors).toEqual([]);
  });
}
