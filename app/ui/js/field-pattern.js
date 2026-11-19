// Reusable presentation of native patterns. This is not a mask interpreter:
// unknown placeholders/functions remain exact text and Rust validates meaning.
export function patternParts(value, fields) {
  const known = new Map(fields.map(field => [field.key.toLowerCase(), field]));
  const parts = [];
  let start = 0;
  for (const match of value.matchAll(/%([^%]+)%/g)) {
    const field = known.get(match[1].toLowerCase());
    if (!field) continue;
    if (match.index > start) parts.push({ text: value.slice(start, match.index) });
    parts.push({ text: match[0], field });
    start = match.index + match[0].length;
  }
  if (start < value.length) parts.push({ text: value.slice(start) });
  return parts;
}

export function fieldSuggestions(fields, query, limit = 8) {
  const term = query.replace(/^%/, '').toLocaleLowerCase();
  if (!term) return [];
  return fields.map((field, order) => {
    const label = field.label.toLocaleLowerCase();
    const key = field.key.toLocaleLowerCase();
    const score = label === term || key === term ? 0 : label.startsWith(term) || key.startsWith(term) ? 1 : label.includes(term) || key.includes(term) ? 2 : 3;
    return { field, score, order };
  }).filter(item => item.score < 3).sort((a, b) => a.score - b.score || a.order - b.order).slice(0, limit).map(item => item.field);
}

export function fieldOperation(mode, field, source, pattern) {
  return mode === 'extract'
    ? { kind: 'extract', source: `%${source}%`, pattern }
    : { kind: 'format', field, pattern };
}

// Only used to choose the UI's affected fields; the native engine owns parsing,
// extraction validity, duplicate-target detection, format validation and locks.
export function operationFields(operation, fields) {
  if (operation.kind === 'format') return [operation.field];
  return [...new Set(patternParts(operation.pattern, fields).flatMap(part => part.field ? [part.field.key] : []))];
}
