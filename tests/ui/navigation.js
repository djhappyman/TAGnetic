// Follow the visible entry points after Online became a top-level workspace.
export async function openTagSource(page, source) {
  if (source === 'transformations') source = 'editor';
  if (source === 'online') {
    await page.locator('.mode-tab[data-mode="online"]').click();
  } else {
    await page.locator('.mode-tab[data-mode="tagger"]').click();
    await page.locator(`.subtab[data-subtab="${source}"]`).click();
  }
}

// Duplicates is a workspace tab; returning to search selects Tags.
export async function toggleDuplicates(page) {
  const tab = page.locator('#query-mode-toggle');
  if (await tab.getAttribute('aria-pressed') === 'true') await page.locator('.mode-tab[data-mode="tagger"]').click();
  else await tab.click();
}

// Preset management has one entry in the compact toolbar menu.
export async function openTransformationEditor(page) {
  await openTagSource(page, 'transformations');
  if (await page.locator('#transform-options').getAttribute('aria-expanded') !== 'true') await page.locator('#transform-options').click();
  await page.locator('#presets-edit').click();
}
