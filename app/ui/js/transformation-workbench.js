import { el, fileName } from './dom.js';
import { t } from './i18n.js';
import { invoke } from './invoke.js';
import { actionGroups, builtinGroups, selectedPaths, trackAt } from './state.js';
import { liveInputs } from './live-preview.js';
import { fieldLabel } from './fields.js';
import { describeRule, presetExample, previewFields, sampleRequest } from './preset-presentation.js';
import { presetLabel, presetNote } from './chain.js';
import { openPresetEditor, openNewPreset } from './presets.js';

// Inspection never enables a preset, stages edits, or writes an audio file.
export function createTransformationWorkbench({ rerender, isActive }) {
  let selected = null, field = 'title', sampleIndex = 0, typed = false, sequence = 0, timer, syncingField = false;
  const identity = group => `${!!group.builtin}:${group.name}`;
  const groups = () => [...actionGroups, ...builtinGroups];
  const current = () => groups().find(group => identity(group) === selected);
  const targetLabel = key => key === 'filename' ? t('presets.target.file') : key === 'fileext' ? t('presets.target.extension') : fieldLabel(key);
  function matches(group) {
    const query = el('transform-search').value.trim().toLocaleLowerCase();
    return (!el('transform-enabled').checked || isActive(group)) && (!query || `${presetLabel(group)} ${presetNote(group) || ''}`.toLocaleLowerCase().includes(query));
  }
  function inspect(group) {
    selected = identity(group); sampleIndex = 0; typed = false; refresh();
    const port = el('transform-inspect').closest('.transform-workspace-scroll');
    const bottom = el('transform-inspect').getBoundingClientRect().bottom - port.getBoundingClientRect().bottom;
    if (bottom > 0) port.scrollTop += bottom;
  }
  function refresh() {
    let group = current();
    if (!group) {
      group = groups().find(isActive) || groups()[0];
      selected = group ? identity(group) : null;
    }
    el('transform-inspect').hidden = !group;
    el('transform-sample').hidden = !group;
    if (!group) return;
    for (const row of el('groups-menu').querySelectorAll('.preset-row')) {
      const inspected = `${row.dataset.builtin}:${row.dataset.presetName}` === selected;
      row.classList.toggle('inspected', inspected);
      row.querySelector('.preset-apply').setAttribute('aria-pressed', String(inspected));
    }
    el('transform-inspect-title').textContent = presetLabel(group);
    el('transform-inspect-note').textContent = presetNote(group) || '';
    el('transform-inspect-note').hidden = !presetNote(group);
    el('transform-inspect-edit').textContent = group.builtin ? t('workbench.customize') : t('workbench.edit');
    el('transform-inspect-rules').replaceChildren(...group.rules.map(rule => {
      const item = document.createElement('li');
      const title = document.createElement('span'), detail = document.createElement('small');
      const description = describeRule(rule, t);
      title.textContent = description.title;
      detail.textContent = `${targetLabel(rule.scope || group.scope || 'tags')} · ${description.help}`;
      item.append(title, detail); item.classList.toggle('disabled', rule.enabled === false);
      return item;
    }));
    const targets = previewFields(group, ['title', 'artist', 'album', 'albumartist', 'genre', 'comment', 'key']);
    if (!targets.includes(field)) { field = targets[0]; typed = false; }
    el('transform-sample-field').replaceChildren(...targets.map(key => { const option = document.createElement('option'); option.value = key; option.textContent = targetLabel(key); return option; }));
    el('transform-sample-field').value = field;
    // Keep the Svelte native binding and its themed select proxy in sync.
    syncingField = true;
    el('transform-sample-field').dispatchEvent(new Event('change'));
    syncingField = false;
    scheduleSample();
  }
  function scheduleSample() {
    clearTimeout(timer); const revision = ++sequence;
    timer = setTimeout(() => renderSample(revision), 100);
  }
  async function renderSample(revision) {
    const group = current(); if (!group) return;
    const paths = selectedPaths(); sampleIndex = Math.min(sampleIndex, Math.max(0, paths.length - 1));
    const path = paths[sampleIndex], track = path && trackAt(path);
    for (const id of ['transform-sample-prev','transform-sample-next']) el(id).disabled = paths.length < 2;
    const input = el('transform-sample-before'), output = el('transform-sample-after'), status = el('transform-sample-status');
    if (!typed) {
      if (track) {
        const name = fileName(path);
        const raw = liveInputs().edits.find(edit => edit.path === path && edit.field === field);
        input.value = field === 'filename' ? name.replace(/\.[^.]+$/, '') : field === 'fileext' ? name.match(/\.([^.]+)$/)?.[1] || '' : raw ? raw.value ?? '' : track.tags[field] ?? '';
      } else input.value = presetExample(group);
    }
    el('transform-sample-source').textContent = typed ? t('workbench.customSample') : track ? fileName(path) : t('workbench.example');
    output.setAttribute('aria-busy','true'); status.textContent = t('presets.previewUpdating');
    const before = input.value;
    try {
      const after = await invoke('transform_sample', {...sampleRequest(group, field), text:before});
      if (revision !== sequence) return;
      output.textContent = after;
      status.textContent = after === before ? t('presets.unchanged') : t('workbench.sampleOnly');
    } catch (error) {
      if (revision !== sequence) return;
      output.textContent = ''; status.textContent = String(error);
    } finally { if (revision === sequence) output.removeAttribute('aria-busy'); }
  }
  el('transform-search').addEventListener('input', rerender);
  el('transform-enabled').addEventListener('change', rerender);
  el('transform-new').addEventListener('click', openNewPreset);
  el('transform-manage').addEventListener('click', () => openPresetEditor());
  el('transform-inspect-edit').addEventListener('click', () => { const group=current(); if(group) openPresetEditor(group.name); });
  el('transform-sample-field').addEventListener('change', () => { if(syncingField)return; field=el('transform-sample-field').value; typed=false; scheduleSample(); });
  el('transform-sample-before').addEventListener('input', () => { typed=true; scheduleSample(); });
  for (const [id, direction] of [['transform-sample-prev',-1],['transform-sample-next',1]]) el(id).addEventListener('click', () => {
    const count=selectedPaths().length; sampleIndex=count ? (sampleIndex+direction+count)%count : 0; typed=false; scheduleSample();
  });
  document.addEventListener('tagrex:transform-scope', () => {
    if (el('subtab-transformations').hidden) return;
    if (!typed) scheduleSample();
  });
  return {inspect, refresh, matches, isInspected:group => identity(group) === selected};
}
