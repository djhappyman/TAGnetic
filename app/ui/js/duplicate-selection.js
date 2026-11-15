// Duplicate review uses the already loaded saved DTOs. It never reads files or
// merges pending editor values into the evidence shown in the sidebar.
export function duplicateGroupTitle(group, trackAt) {
  for (const path of group.paths) {
    const track = trackAt(path);
    if (track) return track.tags?.title?.trim() || path.split(/[\\/]/).pop() || group.label;
  }
  return group.label;
}

export function duplicateSelectionDetail(group, trackAt) {
  if (!group) return null;
  const tracks = group.paths.map(trackAt).filter(Boolean);
  if (!tracks.length) return null;
  const keys = [...new Set(tracks.flatMap(track => Object.keys(track.tags || {})))].sort();
  const shared = [], differingTags = [];
  for (const key of keys) {
    const value = tracks[0].tags?.[key] ?? '';
    if (tracks.every(track => (track.tags?.[key] ?? '') === value)) {
      if (value !== '') shared.push([key, value]);
    } else differingTags.push(key);
  }
  return { id: group.id, title: duplicateGroupTitle(group, trackAt), reason: group.reason,
    tracks, sharedTags: Object.fromEntries(shared), differingTags };
}
