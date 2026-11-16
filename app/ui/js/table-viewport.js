// The desktop inspector covers part of the native scrollport. All table input
// and overlay chrome use the same unobscured bounds; table geometry stays fixed.
export function tableCoveredWidth(wrap) {
  if (wrap.id !== 'files-view') return 0;
  const frame = wrap.closest('.dock-frame');
  if (frame?.dataset.sidebarDrawer !== 'true' || frame.dataset.sidebarClosed === 'true') return 0;
  return frame.querySelector('.dock-right')?.clientWidth || 0;
}
export function tableViewportBounds(wrap) {
  const rect = wrap.getBoundingClientRect();
  const width = Math.max(0,wrap.clientWidth-tableCoveredWidth(wrap));
  const left = rect.left+wrap.clientLeft, top = rect.top+wrap.clientTop;
  return {left,top,right:left+width,bottom:top+wrap.clientHeight,width,height:wrap.clientHeight};
}
