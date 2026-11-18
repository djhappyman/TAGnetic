import { initTransformationPane } from './transformation-pane.js';
import { el, placeFloating } from './dom.js';
import { createComponentElement } from '../lib/component-elements.js';
import { t } from './i18n.js';
import { actionGroups, builtinGroups } from './state.js';
import { fieldLabel } from './fields.js';
import { describeRule } from './preset-presentation.js';
import { presetLabel } from './chain.js';
import { openPresetEditor, openNewPreset } from './presets.js';

// The sidebar configures the chain; Staged is its only results surface.
export function createTransformationWorkbench({ rerender, isActive, enable }) {
  initTransformationPane(el('transform-block'),el('transform-scroll'),el('transform-resize'));
  let selected = null;
  const added = new Set();
  const identity = group => `${!!group.builtin}:${group.name}`;
  const groups = () => [...actionGroups, ...builtinGroups];
  const isInspected = group => identity(group) === selected;
  const targetLabel = key => key === 'tags' ? t('chain.scopeAll') : key === 'filename' ? t('presets.target.file') : key === 'fileext' ? t('presets.target.extension') : fieldLabel(key);
  function matches(group) { return !group.builtin || isActive(group) || added.has(identity(group)); }
  function inspect(group) { selected = isInspected(group) ? null : identity(group); rerender(); }
  function renderDetails(group) {
    const details = document.createElement('div'); details.className = 'transform-rule-details';
    const rules = document.createElement('ol');
    for (const rule of group.rules || []) {
      const item = document.createElement('li'); item.classList.toggle('disabled', rule.enabled === false);
      const title = document.createElement('span'); title.textContent = describeRule(rule,t).title;
      const target = document.createElement('small'); target.textContent = targetLabel(rule.scope || group.scope || 'tags');
      item.append(title,target); rules.append(item);
    }
    details.append(rules); return details;
  }
  function closeLibrary(focus = true) {
    el('transform-library').hidden = true;
    el('transform-add').setAttribute('aria-expanded','false');
    if (focus) el('transform-add').focus({preventScroll:true});
  }
  function refreshLibrary() {
    const query = el('transform-search').value.trim().toLocaleLowerCase();
    const list = el('transform-library-list'); list.replaceChildren();
    for (const [title, items] of [['presets.yours',actionGroups],['chain.builtIn',builtinGroups]]) {
      const matches = items.filter(group => presetLabel(group).toLocaleLowerCase().includes(query));
      if (!matches.length) continue;
      const heading = document.createElement('h4'); heading.textContent = t(title); list.append(heading);
      for (const group of matches) {
        const button = createComponentElement('button', {variant:'ghost',size:'sm'});
        button.type = 'button'; button.className = 'transform-library-preset'; button.dataset.builtin = String(!!group.builtin); button.textContent = presetLabel(group);
        button.addEventListener('click', () => {
          added.add(identity(group)); selected = identity(group); closeLibrary(); enable(group);
        });
        list.append(button);
      }
    }
    if (!list.children.length) {
      const empty = document.createElement('p'); empty.className = 'preset-none'; empty.textContent = t('workbench.noResults'); list.append(empty);
    }
  }
  function refresh() { if (!el('transform-library').hidden) refreshLibrary(); }
  el('transform-add').addEventListener('click', () => {
    if (!el('transform-library').hidden) { closeLibrary(); return; }
    closeOptions(false);
    el('transform-library').hidden = false;
    el('transform-add').setAttribute('aria-expanded','true');
    refreshLibrary();
    placeFloating(el('transform-library'),el('transform-add'),{align:'right',width:320});
    el('transform-search').focus();
  });
  el('transform-library').addEventListener('keydown', event => {
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); closeLibrary(); }
  });
  function closeOptions(focus = true) {
    el('transform-options-menu').hidden = true;
    el('transform-options').setAttribute('aria-expanded','false');
    if (focus) el('transform-options').focus({preventScroll:true});
  }
  el('transform-options').addEventListener('click', () => {
    if (!el('transform-options-menu').hidden) { closeOptions(); return; }
    closeLibrary(false); el('transform-options-menu').hidden = false;
    el('transform-options').setAttribute('aria-expanded','true');
    placeFloating(el('transform-options-menu'),el('transform-options'),{align:'right',width:190});
    el('presets-edit').focus();
  });
  el('transform-options-menu').addEventListener('keydown',event=>{
    if(event.key === 'Escape') {event.preventDefault();event.stopPropagation();closeOptions();}
  });
  document.addEventListener('pointerdown',event=>{
    if (!(event.target instanceof Element)) return;
    if (!event.target.closest('#transform-library,#transform-add')) closeLibrary(false);
    if (!event.target.closest('#transform-options-menu,#transform-options')) closeOptions(false);
  });
  el('transform-search').addEventListener('input', refreshLibrary);
  el('transform-new').addEventListener('click', () => { closeLibrary(); openNewPreset(); });
  el('presets-edit').addEventListener('click', () => {
    closeOptions();
    openPresetEditor(groups().find(isInspected)?.name);
  });
  return { inspect, refresh, matches, isInspected, renderDetails };
}
