import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';

const root = resolve('.test-dist');
async function openApp(page, kind) {
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    window.playerOverride = { path: null, is_paused: false, position_secs: 0, duration_secs: 0, wants_next: false, output_unavailable: true };
  });
  await page.route('http://tagrex.test/**', async route => {
    const path = resolve(root, '.' + new URL(route.request().url()).pathname);
    if (!path.startsWith(root + sep)) return route.abort();
    try {
      let body = await readFile(path);
      if (path.endsWith('.js')) {
        let source = body.toString();
        if (kind === 'unreadable') source = source.replace(/case([`"'])list_tracks\1:/g, match => `${match}return Promise.resolve([{path:'/music/broken.flac',format:'FLAC',tags:{},unreadable:true,read_error:{text:'Malformed frame <b>unsafe</b>'}}]);`);
        if (kind === 'player') source = source.replace(/case([`"'])player_status\1:/g, match => `${match}if(window.playerOverride)return Promise.resolve(window.playerOverride);`);
        if (kind === 'settings') source = source.replace(/case([`"'])load_settings\1:/g, match => `${match}return Promise.resolve({proxy:'',read_priority:[],action_groups:[],mask_presets:[],recovery_notice:'Saved settings could not be parsed. Original file will be preserved.'});`);
        body = Buffer.from(source);
      }
      await route.fulfill({ body, contentType: path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css' : path.endsWith('.woff2') ? 'font/woff2' : 'text/html' });
    } catch { await route.fulfill({ status: 404, body: 'Missing fixture' }); }
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(() => page.evaluate(() => typeof window.tagrex?.setMode)).toBe('function');
  return errors;
}
async function library(page) {
  await page.locator('#root-display').click();
  await page.locator('#root').fill('/music');
  await page.locator('#root').press('Enter');
}

test('unreadable rows expose backend detail safely without becoming editable', async ({ page }) => {
  const errors = await openApp(page, 'unreadable');
  await library(page);
  const row = page.locator('#tracks-body tr.unreadable');
  await expect(row).toHaveCount(1);
  await expect(row.locator('.unreadable-note')).toHaveText('Malformed frame <b>unsafe</b>');
  await expect(row.locator('.file')).toHaveAttribute('aria-description', '/music/broken.flac: Malformed frame <b>unsafe</b>');
  await expect(row.locator('b')).toHaveCount(0);
  await row.click();
  await expect(row).not.toHaveClass(/selected/);
  expect(errors).toEqual([]);
});

test('player explains missing output and recovers its normal status without repeated prompts', async ({ page }) => {
  const errors = await openApp(page, 'player');
  await library(page);
  await expect(page.locator('#pl-title')).toHaveText('Audio output unavailable');
  await expect(page.locator('#pl-title')).toHaveAttribute('aria-description', 'Connect an audio device, then press Play to try again.');
  await expect(page.locator('#pl-toggle')).toBeEnabled();
  await page.evaluate(() => { window.playerOverride = null; });
  await expect(page.locator('#pl-title')).toHaveText('No track loaded');
  await page.locator('#pl-toggle').click();
  await expect(page.locator('#pl-title')).toContainText('Desert Rain');
  expect(errors).toEqual([]);
});

test('opening settings exposes the recovery notice while retaining the normal settings workflow', async ({ page }) => {
  const errors = await openApp(page, 'settings');
  await page.locator('#settings-open').click();
  await expect(page.getByRole('dialog', { name: 'Settings', exact: true })).toBeVisible();
  await expect(page.getByText('Saved settings could not be parsed. Original file will be preserved.', { exact: true })).toBeVisible();
  await expect(page.locator('#settings-save')).toBeEnabled();
  expect(errors).toEqual([]);
});
