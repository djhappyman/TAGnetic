// Resize the viewport around the preview; table columns keep their own widths.
const SIDEBAR_KEY = 'tagrex.maskBuilderSidebarWidth';
const clamp = (value, min, max) => Math.min(max, Math.max(min, value));
export function connectBuilderSizing({ dialog, body, sidebar, divider, edges, onSizeChange }) {
  let preferred = null;
  let drag = null;
  const removers = [];
  const listen = (node, type, handler, options) => {
    node.addEventListener(type, handler, options);
    removers.push(() => node.removeEventListener(type, handler, options));
  };
  try { const saved = Number(localStorage.getItem(SIDEBAR_KEY)); if (saved > 0) preferred = saved; } catch { /* Session-only sizing. */ }
  const splitBounds = () => ({min:200, max:Math.max(200, body.clientWidth - 296)});
  const dialogBounds = () => ({min:Math.min(720, innerWidth - 24), max:innerWidth - 24});
  function sync() {
    if (!body.clientWidth) return;
    const {min,max} = splitBounds();
    const width = clamp(preferred ?? clamp(body.clientWidth * .28, 280, 360), min, max);
    body.style.setProperty('--mb-sidebar-width', `${width}px`);
    const preview = Math.max(0, body.clientWidth - width - 16);
    divider.setAttribute('aria-valuemin', String(Math.max(0, body.clientWidth - max - 16)));
    divider.setAttribute('aria-valuemax', String(Math.max(0, body.clientWidth - min - 16)));
    divider.setAttribute('aria-valuenow', String(Math.round(preview)));
    divider.setAttribute('aria-valuetext', `Preview ${Math.round(preview)} pixels, reference ${Math.round(width)} pixels`);
    const bounds = dialogBounds();
    for (const edge of edges) {
      edge.setAttribute('aria-valuemin', String(bounds.min)); edge.setAttribute('aria-valuemax', String(bounds.max));
      edge.setAttribute('aria-valuenow', String(Math.round(dialog.offsetWidth)));
    }
  }
  function saveSidebar() {
    try {
      if (preferred == null) localStorage.removeItem(SIDEBAR_KEY);
      else localStorage.setItem(SIDEBAR_KEY, String(preferred));
    } catch { /* Session-only sizing. */ }
  }
  function resizeSidebar(width) {
    const {min,max} = splitBounds(); preferred = clamp(width,min,max); sync();
  }
  function resizeDialog(width) {
    const {min,max} = dialogBounds(); dialog.style.width = `${clamp(width,min,max)}px`; sync();
  }
  function finish(cancel = false) {
    if (!drag) return;
    const previous = drag; drag = null;
    if (cancel) { preferred = previous.preferred; dialog.style.width = previous.inlineWidth; sync(); }
    else { saveSidebar(); onSizeChange(); }
    dialog.classList.remove('mb-is-resizing');
    if (previous.node.hasPointerCapture(previous.id)) previous.node.releasePointerCapture(previous.id);
    document.removeEventListener('keydown', escapeDrag, true);
  }
  function escapeDrag(event) {
    if (event.key !== 'Escape') return;
    event.preventDefault(); event.stopImmediatePropagation(); finish(true);
  }
  for (const node of [divider, ...edges]) {
    listen(node, 'pointerdown', event => {
      if (event.button !== 0 || drag) return;
      event.preventDefault();
      drag = {node, id:event.pointerId, x:event.clientX, width:dialog.offsetWidth,
        sidebar:sidebar.offsetWidth, preferred, inlineWidth:dialog.style.width};
      node.setPointerCapture(event.pointerId); dialog.classList.add('mb-is-resizing');
      document.addEventListener('keydown', escapeDrag, true);
    });
    listen(node, 'pointermove', event => {
      if (!drag || drag.node !== node || drag.id !== event.pointerId) return;
      const delta = event.clientX - drag.x;
      if (node === divider) resizeSidebar(drag.sidebar - delta);
      // The dialog stays centred, so each edge moves by half the width change.
      else resizeDialog(drag.width + delta * (node.dataset.resizeEdge === 'left' ? -2 : 2));
    });
    listen(node, 'pointerup', () => finish());
    listen(node, 'pointercancel', () => finish(true));
    listen(node, 'lostpointercapture', () => finish());
    listen(node, 'keydown', event => {
      if (!['ArrowLeft','ArrowRight','Home','End'].includes(event.key)) return;
      event.preventDefault(); event.stopPropagation();
      const step = event.shiftKey ? 40 : 10;
      const bounds = node === divider ? splitBounds() : dialogBounds();
      if (node === divider) {
        resizeSidebar(event.key === 'Home' ? bounds.max : event.key === 'End' ? bounds.min
          : sidebar.offsetWidth + (event.key === 'ArrowLeft' ? step : -step));
        saveSidebar();
      } else {
        const direction = node.dataset.resizeEdge === 'left' ? -1 : 1;
        resizeDialog(event.key === 'Home' ? bounds.min : event.key === 'End' ? bounds.max
          : dialog.offsetWidth + (event.key === 'ArrowRight' ? step : -step) * direction);
        onSizeChange();
      }
    });
  }
  listen(divider, 'dblclick', () => { preferred = null; sync(); saveSidebar(); });
  const resize = new ResizeObserver(sync); resize.observe(body); resize.observe(dialog);
  const visibility = new MutationObserver(() => { if (dialog.closest('.controller-dialog-root').hidden) finish(true); });
  visibility.observe(dialog.closest('.controller-dialog-root'), {attributes:true,attributeFilter:['hidden']});
  listen(window, 'resize', sync);
  const dispose = () => { finish(true); resize.disconnect(); visibility.disconnect(); removers.forEach(remove=>remove()); };
  window.addEventListener('pagehide',dispose,{once:true});
  return {sync, dispose};
}
