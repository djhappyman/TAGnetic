// The native adapter and UI consume the same capability catalogue.
export function parseSearchFields(catalogue, source) {
  return catalogue.split('\n').filter(line => line.split('\t')[0] === source).map(line => {
    const [, key, parameter, label, kind, values = ''] = line.split('\t');
    return { key, parameter, label, kind, values: values ? values.split('|') : [] };
  });
}

export function validFilterValue(field, value) {
  const text = value.trim();
  if (!text || new TextEncoder().encode(text).length > 1024) return false;
  if (field.kind === 'choice') return field.values.includes(text);
  if (field.kind === 'number') return /^\d+$/.test(text) && Number(text) > 0 && Number(text) <= 4294967295;
  return true;
}

export function filterSuggestions(fields, word) {
  const term = word.toLocaleLowerCase();
  if (term.length < 2) return [];
  return fields.filter(field => `${field.label} ${field.key} ${field.key === 'release_title' ? 'album' : ''} ${field.key === 'medium' ? 'format' : ''} ${field.key === 'label' ? 'publisher' : ''}`.toLocaleLowerCase().includes(term)).slice(0, 8);
}

export function createSearchDrafts() {
  const drafts = new Map();
  return {
    get(source) { return drafts.get(source) || { text: '', filter: null }; },
    set(source, text, filter) { drafts.set(source, { text, filter: filter ? { ...filter } : null }); },
  };
}
export const onlineSearchDrafts = createSearchDrafts();

export function searchRequestFor(source, text, medium = '') {
  const { filter } = onlineSearchDrafts.get(source);
  const filters = filter ? { [filter.key]: filter.value } : {};
  // One filter total: a chip takes precedence over the legacy medium selector.
  return { album: text.trim(), format: filter ? null : medium || null, filters };
}
