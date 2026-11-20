// Keep the established controller seam while the visible editor becomes chips.
// Programmatic writes do not emit input (same contract as a native input).
export function connectPatternController(node, { read, write, selection, select }) {
  Object.defineProperties(node, {
    value: { configurable:true, get:read, set:value => write(String(value ?? '')) },
    selectionStart: { configurable:true, get:() => selection().start },
    selectionEnd: { configurable:true, get:() => selection().end },
  });
  node.setSelectionRange = (start, end) => select(start, end);
  node.select = () => { node.focus(); select(0, read().length); };
  // Labels cannot natively target contenteditable. Preserve click-to-focus.
  const labelClick = event => {
    const label = event.target.closest?.('label');
    if (label?.htmlFor === node.id || (label?.contains(node) && !event.target.closest('button,input,select'))) node.focus();
  };
  document.addEventListener('click', labelClick);
  const actions = {
    mask:['preview','diff-apply'], 'from-name-mask':['from-name-preview','diff-apply'],
    'from-text-mask':['from-text-preview','diff-apply'], 'mb-expr':['mb-ok'],
    'export-mask':['export-run'], 'export-name':['export-run'],
    'colmask-pattern':['colmask-ok'], 'artwork-import-pattern':['ribbon-import-stage','ribbon-import-write'],
  };
  let blocked = [];
  const release = () => {
    for(const {button,inert,aria} of blocked) {button.inert=inert;delete button.dataset.patternBlocked;if(aria==null)button.removeAttribute('aria-disabled');else button.setAttribute('aria-disabled',aria);}
    blocked=[];
  };
  const pending = event => {
    release();
    if(!event.detail)return;
    for(const id of actions[node.id]||[]) {
      const button=document.getElementById(id);if(!button)continue;
      blocked.push({button,inert:button.inert,aria:button.getAttribute('aria-disabled')});
      button.inert=true;button.setAttribute('aria-disabled','true');button.dataset.patternBlocked='';
    }
  };
  const guard = event => {
    if(blocked.some(({button})=>button===event.target||button.contains(event.target))){event.preventDefault();event.stopImmediatePropagation();}
  };
  node.addEventListener('pattern:pending',pending);
  document.addEventListener('click',guard,true);
  return () => {
    document.removeEventListener('click', labelClick);
    node.removeEventListener('pattern:pending',pending);document.removeEventListener('click',guard,true);release();
    for (const key of ['value','selectionStart','selectionEnd','setSelectionRange','select']) delete node[key];
  };
}

export function patternFields(entries, direction, label) {
  return entries.filter(entry => /^%[^%]+%$/.test(entry.token) && (direction === 'render' ? entry.render : entry.extract))
    .map(entry => ({ key:entry.name, label:label(entry.name) }));
}
