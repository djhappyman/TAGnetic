// Reorder only listed paths, one step at a time. Each group is an independent
// lane; hidden/filtered tracks retain their slots in the underlying file order.
export function movableDirections(groups, selected) {
  let up = false, down = false;
  for (const paths of groups) for (let i = 1; i < paths.length; i++) {
    const before = selected.has(paths[i - 1]), current = selected.has(paths[i]);
    if (current && !before) up = true;
    if (before && !current) down = true;
  }
  return { up, down };
}
export function shiftSelectedOrder(tracks, groups, selected, direction) {
  const replacements = new Map();
  for (const paths of groups) {
    if (paths.length < 2) continue;
    const order = [...paths];
    const start = direction < 0 ? 1 : order.length - 2;
    const end = direction < 0 ? order.length : -1;
    const step = direction < 0 ? 1 : -1;
    for (let i = start; i !== end; i += step) {
      const other = i + direction;
      if (selected.has(order[i]) && !selected.has(order[other])) [order[i], order[other]] = [order[other], order[i]];
    }
    for (let i = 0; i < paths.length; i++) if (paths[i] !== order[i]) replacements.set(paths[i], order[i]);
  }
  if (!replacements.size) return null;
  const byPath = new Map(tracks.map(track => [track.path, track]));
  return tracks.map(track => byPath.get(replacements.get(track.path)) || track);
}
