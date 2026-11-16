// Presentation only: canonical grouping keys and file paths never change.
export const COMMON_GROUPS = ['folder', 'album', 'albumartist', 'artist', 'disc', 'year', 'genre', 'media'];
const ICONS = {folder:'folder', drop:'folder', album:'group-album', albumartist:'group-albumartist', artist:'group-artist', disc:'group-disc', year:'group-year', genre:'tag', media:'group-media', '':'group-none'};
export function groupIcon(key) { return ICONS[key] || 'tag'; }

// Compare complete path components, retaining at least one component per group.
// In particular, an ancestor group and its child must not acquire empty labels.
export function folderDisplayLabels(directories) {
  const entries = [...new Set(directories)].map(key => ({key, parts:key.replace(/\\/g, '/').replace(/\/$/, '').split('/')}));
  if (!entries.length) return new Map();
  const first = entries[0].parts;
  let shared = Math.max(0, first.length - 1);
  for (const {parts} of entries) {
    shared = Math.min(shared, Math.max(0, parts.length - 1));
    let i = 0;
    while (i < shared && parts[i] === first[i]) i++;
    shared = i;
    if (!shared) break;
  }
  return new Map(entries.map(({key, parts}) => [key, parts.slice(shared).join('/') || key]));
}
