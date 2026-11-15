import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { functionSource } from './interaction-codex-harness.js';

const source = readFileSync(new URL('../../app/ui/js/cover.js', import.meta.url), 'utf8');
const names = ['isMissingCoverPathCommand', 'previewCoverPath', 'embedCoverFromPath', 'adoptCoverEmbedPlan', 'embedCoverDto', 'chooseCover', 'readImageFile', 'embedCoverFile'];
const functions = names.map(name => functionSource(source, name)).join('\n');
function harness(invoke, native = true) {
  const paths = ['/music/selected.flac'];
  const subject = { paths };
  const calls = { adopted: [], toast: [], fileReader: 0 };
  let current = true;
  const context = vm.createContext({
    TAURI: native ? {} : null, invoke, nativePickActive: false,
    window: { __TAURI__: { dialog: { open: async () => '/elsewhere/picked.png' } } },
    captureSelection: () => subject, selectedPaths: () => paths,
    isSelectionCurrent: captured => current && captured === subject,
    adoptLiveCoverPlan: (...args) => calls.adopted.push(args),
    toast: (...args) => calls.toast.push(args), t: key => key, tn: (_, n) => n,
    FileReader: class {
      readAsDataURL() { calls.fileReader++; this.result = 'data:image/png;base64,aW1hZ2U='; this.onload(); }
    },
  });
  vm.runInContext(functions, context);
  return { context, calls, subject, supersede: () => { current = false; } };
}
const plan = { changes: [{ path: '/music/selected.flac' }] };

test('native picker and native drop stage through one path command with captured targets', async () => {
  for (const picker of [false, true]) {
    const requests = [];
    const { context, calls } = harness(async (command, args) => { requests.push({ command, args }); return plan; });
    if (picker) await context.chooseCover();
    else await context.embedCoverFromPath('/elsewhere/picked.png');
    assert.equal(requests.length, 1);
    assert.equal(requests[0].command, 'preview_cover_embed_path');
    assert.equal(requests[0].args.imagePath, '/elsewhere/picked.png');
    assert.deepEqual(requests[0].args.paths, ['/music/selected.flac']);
    assert.equal(calls.adopted.length, 1);
    assert.equal(calls.fileReader, 0);
  }
});

test('older native builds use the existing command pair only for the exact missing command', async () => {
  const requests = [];
  const { context, calls } = harness(async (command, args) => {
    requests.push({ command, args });
    if (command === 'preview_cover_embed_path') throw { failure: 'Command preview_cover_embed_path not found' };
    if (command === 'read_cover_image') return { mime: 'image/png', data_base64: 'image' };
    return plan;
  });
  await context.embedCoverFromPath('/elsewhere/picked.png');
  assert.deepEqual(requests.map(r => r.command), ['preview_cover_embed_path', 'read_cover_image', 'preview_cover_embed']);
  assert.equal(requests[2].args.paths[0], '/music/selected.flac');
  assert.equal(requests[2].args.cover.kind, 'front');
  assert.equal(calls.adopted.length, 1);
});

test('source, I/O, validation and unrelated missing-command errors never trigger fallback', async () => {
  for (const failure of [
    { text: 'Image not found', message: { code: 'error.notAnImage' } },
    'Permission denied', 'Command another_command not found',
    'The open library changed. Choose the artwork again.',
    { text: 'Command preview_cover_embed_path not found' },
  ]) {
    const requests = [];
    const { context, calls } = harness(async command => { requests.push(command); throw { failure }; });
    await context.embedCoverFromPath('/elsewhere/picked.png');
    assert.deepEqual(requests, ['preview_cover_embed_path']);
    assert.equal(calls.adopted.length, 0);
    assert.equal(calls.toast.length, 1);
  }
});

test('superseded native results and late compatibility reads never stage a plan', async () => {
  for (const compatibility of [false, true]) {
    let release;
    const requests = [];
    const h = harness(async command => {
      requests.push(command);
      if (compatibility && command === 'preview_cover_embed_path') throw { failure: 'Command preview_cover_embed_path not found' };
      return new Promise(resolve => { release = resolve; });
    });
    const pending = h.context.embedCoverFromPath('/elsewhere/picked.png');
    await new Promise(resolve => setImmediate(resolve));
    h.supersede();
    release(compatibility ? { mime: 'image/png', data_base64: 'image' } : plan);
    await pending;
    assert.equal(h.calls.adopted.length, 0);
    assert.equal(h.calls.toast.length, 0);
    assert.deepEqual(requests, compatibility ? ['preview_cover_embed_path', 'read_cover_image'] : ['preview_cover_embed_path']);
  }
});

test('a superseded missing-command response cannot begin a compatibility read', async () => {
  let reject;
  const requests = [];
  const h = harness(command => { requests.push(command); return new Promise((_, fail) => { reject = fail; }); });
  const pending = h.context.embedCoverFromPath('/elsewhere/picked.png');
  h.supersede();
  reject({ failure: 'Command preview_cover_embed_path not found' });
  await pending;
  assert.deepEqual(requests, ['preview_cover_embed_path']);
  assert.equal(h.calls.adopted.length, 0);
});

test('browser FileReader fallback retains the inline cover command', async () => {
  const requests = [];
  const h = harness(async (command, args) => { requests.push({ command, args }); return plan; }, false);
  await h.context.embedCoverFile({ type: 'image/png' }, h.subject);
  assert.equal(h.calls.fileReader, 1);
  assert.equal(requests[0].command, 'preview_cover_embed');
  assert.equal(requests[0].args.cover.data_base64, 'aW1hZ2U=');
  assert.equal(h.calls.adopted.length, 1);
});
