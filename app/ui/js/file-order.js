// Keep deliberately aligned rows in their slots when saved metadata is re-read.
export function captureFileOrder(tracks, manual, renamed = new Map()) {
  return manual ? tracks.map(track => renamed.get(track.path) || track.path) : null;
}

export function restoreFileOrder(tracks, order) {
  if (!order) return tracks;
  const byPath = new Map(tracks.map(track => [track.path, track]));
  const known = new Set(order);
  const placed = order.map(path => byPath.get(path)).filter(Boolean);
  let next = 0;
  // New files retain their sorted slots. No spread of a large library array.
  return tracks.map(track => known.has(track.path) ? placed[next++] : track);
}
