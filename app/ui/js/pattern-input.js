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
  return () => {
    document.removeEventListener('click', labelClick);
    for (const key of ['value','selectionStart','selectionEnd','setSelectionRange','select']) delete node[key];
  };
}

export function patternFields(entries, direction, label) {
  return entries.filter(entry => /^%[^%]+%$/.test(entry.token) && (direction === 'render' ? entry.render : entry.extract))
    .map(entry => ({ key:entry.name, label:label(entry.name) }));
}
