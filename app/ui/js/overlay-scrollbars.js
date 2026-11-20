import { tableCoveredWidth } from './table-viewport.js';
// All scrollbar geometry is read together before any rail style is written.
// Independent observers must not force layout between each other's writes.
const pendingScrollbars = new Set();
let scrollbarFrame = 0;
function scheduleScrollbars(measure) {
  pendingScrollbars.add(measure);
  if (scrollbarFrame) return;
  scrollbarFrame = requestAnimationFrame(() => {
    scrollbarFrame = 0;
    const reads = [...pendingScrollbars];
    pendingScrollbars.clear();
    const writes = reads.map(read => read());
    for (const write of writes) write?.();
  });
}

// Wheel/trackpad scrolling stays native. Only the overlaid thumbs are custom.
export function initTableScrollbars(wrap, { host = null, fadeEdges = false } = {}) {
  let fadeHeader = 0;
  let dirty = true;
  const removers = [];
  const listen = (node,type,handler,options) => {
    node.addEventListener(type,handler,options);
    removers.push(() => node.removeEventListener(type,handler,options));
  };
  const style = (node,key,value) => { if (node.style[key] !== value) node.style[key] = value; };
  const attribute = (node,key,value) => { if (node.getAttribute(key) !== String(value)) node.setAttribute(key,String(value)); };
  function schedule(geometry = false) { dirty ||= geometry; scheduleScrollbars(measure); }
  function createAxis(vertical) {
    const rail = document.createElement('div'), thumb = document.createElement('div');
    rail.className = `${host ? 'inspector-scrollbar' : 'table-scrollbar'} ${vertical ? 'is-vertical' : 'is-horizontal'}`;
    rail.tabIndex = 0;
    for (const [name,value] of Object.entries({role:'scrollbar','aria-label':vertical?'Scroll tracks':'Scroll columns','aria-controls':wrap.id,'aria-orientation':vertical?'vertical':'horizontal','aria-valuemin':'0'})) rail.setAttribute(name,value);
    thumb.className = 'table-scrollbar-thumb'; thumb.setAttribute('aria-hidden','true');
    if (host) {
      const panel = wrap.closest('[role="tabpanel"]');
      const label = panel?.getAttribute('aria-labelledby');
      if (label) { rail.removeAttribute('aria-label'); rail.setAttribute('aria-labelledby', label); }
    }
    rail.append(thumb);
    if (host) host.appendChild(rail); else wrap.after(rail);
    const position = vertical ? 'scrollTop' : 'scrollLeft';
    const axis = {rail,thumb,vertical,extent:0,travel:0,last:wrap[position],hover:false,drag:null,timer:0,rtl:false};
    const coordinate = event => vertical ? event.clientY : event.clientX;
    const normalized = () => wrap[position]*(axis.rtl?-1:1);
    const write = value => { wrap[position] = value*(axis.rtl?-1:1); };
    const settle = () => {
      clearTimeout(axis.timer);
      axis.timer = setTimeout(() => { if (!axis.hover && !axis.drag && document.activeElement !== rail) rail.classList.remove('is-active'); },800);
    };
    axis.reveal = () => { if (!rail.classList.contains('is-active')) rail.classList.add('is-active'); settle(); };
    listen(rail,'pointerenter',() => { axis.hover = true; axis.reveal(); });
    listen(rail,'pointerleave',() => { axis.hover = false; settle(); });
    listen(rail,'focus',axis.reveal); listen(rail,'blur',settle);
    listen(rail,'pointerdown',event => {
      if (event.button !== 0) return;
      // Dragging must preserve the unfinished contentEditable value.
      event.preventDefault(); axis.reveal();
      if (event.target === thumb) {
        axis.drag = {at:coordinate(event),position:normalized()}; rail.setPointerCapture(event.pointerId);
      } else {
        const bounds = thumb.getBoundingClientRect();
        const direction = coordinate(event) < (vertical ? bounds.top : bounds.left) ? -1 : 1;
        write(normalized()+direction*(axis.rtl?-1:1)*(vertical?wrap.clientHeight:wrap.clientWidth-tableCoveredWidth(wrap)));
      }
    });
    listen(rail,'pointermove',event => {
      if (axis.drag && axis.travel > 0) write(axis.drag.position+(coordinate(event)-axis.drag.at)*(axis.rtl?-1:1)*axis.extent/axis.travel);
    });
    const release = () => { axis.drag = null; settle(); };
    for (const type of ['pointerup','pointercancel','lostpointercapture']) listen(rail,type,release);
    listen(rail,'keydown',event => {
      const page = vertical ? wrap.clientHeight : wrap.clientWidth-tableCoveredWidth(wrap);
      const moves = vertical ? {ArrowUp:-24,ArrowDown:24,PageUp:-page,PageDown:page}
        : {ArrowLeft:-24*(axis.rtl?-1:1),ArrowRight:24*(axis.rtl?-1:1),PageUp:-page,PageDown:page};
      if (!(event.key in moves) && event.key !== 'Home' && event.key !== 'End') return;
      event.preventDefault(); axis.reveal();
      write(event.key === 'Home' ? 0 : event.key === 'End' ? axis.extent : normalized()+moves[event.key]);
    });
    return axis;
  }
  const axes = [createAxis(true),createAxis(false)];
  function measure() {
    const inactive = host?.closest('[inert]');
    const height = inactive ? 0 : wrap.clientHeight;
    const width = inactive ? 0 : wrap.clientWidth;
    if (!height || !width) return () => {
      for (const {rail} of axes) if (!rail.hidden) rail.hidden = true;
    };
    let geometry = null;
    if (dirty) {
      const extents = [Math.max(0,wrap.scrollHeight-height),Math.max(0,wrap.scrollWidth-width)];
      const covered = host ? 0 : tableCoveredWidth(wrap);
      const table = host ? null : wrap.querySelector('table');
      const filler = table?.querySelector('th.table-fill')?.getBoundingClientRect().width || 0;
      const terminalSpace = table ? Math.ceil(Math.max(0,covered-filler)) : 0;
      const tableHead = !host || fadeEdges ? wrap.querySelector('thead')?.getBoundingClientRect().height || 0 : 0;
      const head = host ? 0 : tableHead;
      if (fadeEdges) fadeHeader = tableHead;
      const bounds = host ? wrap.getBoundingClientRect() : null;
      const origin = host?.getBoundingClientRect();
      geometry = {
        height,width:width-covered,covered,table,terminalSpace,extents,head,rtl:getComputedStyle(wrap).direction === 'rtl',
        top:(bounds ? bounds.top-origin.top : wrap.offsetTop)+wrap.clientTop,
        left:(bounds ? bounds.left-origin.left : wrap.offsetLeft)+wrap.clientLeft,
      };
    }
    const positions = [wrap.scrollTop,Math.abs(wrap.scrollLeft)];
    const focus = fadeEdges && document.activeElement !== wrap && wrap.contains(document.activeElement)
      ? document.activeElement.getBoundingClientRect() : null;
    const bounds = focus ? wrap.getBoundingClientRect() : null;
    const topFade = fadeEdges && positions[0] > 1 && !(focus && focus.top < bounds.top + fadeHeader + 8 && focus.bottom > bounds.top + fadeHeader);
    const bottomFade = fadeEdges && wrap.scrollHeight - positions[0] - height > 1 && !(focus && focus.bottom > bounds.bottom - 8 && focus.top < bounds.bottom);
    return () => {
      if (fadeEdges) {
        attribute(wrap, 'data-fade-top', topFade);
        attribute(wrap, 'data-fade-bottom', bottomFade);
        if (geometry) wrap.style.setProperty('--scroll-fade-head', `${fadeHeader}px`);
      }
      let remeasure = false;
      if (geometry?.table && geometry.table.style.marginInlineEnd !== `${geometry.terminalSpace}px`) {
        geometry.table.style.marginInlineEnd = `${geometry.terminalSpace}px`;
        // Overflow extents update once after the terminal margin changes.
        remeasure = true;
        schedule(true);
      }
      for (const [index,axis] of axes.entries()) {
        const {rail,thumb,vertical} = axis;
        if (geometry) {
          const {extents,head,height,width,top,left,covered} = geometry;
          axis.extent = extents[index];
          axis.rtl = !vertical && geometry.rtl;
          const track = vertical ? Math.max(0,height-head-(extents[1]?12:0)) : Math.max(0,width-(extents[0]?12:0));
          const visible = vertical ? Math.max(0,height-head) : width;
          const size = Math.min(track,Math.max(24,track*visible/Math.max(visible,visible+axis.extent)));
          style(rail,'top',`${top+(vertical?head:height-12)}px`);
          if (host && vertical) style(rail,'left',`${left+width-12}px`);
          if (!host && vertical) { style(rail,'right','auto'); style(rail,'left',`${left+width-12}px`); }
          style(rail,vertical?'height':'width',`${track}px`);
          if (!vertical) style(rail,'left',`${left}px`);
          style(thumb,vertical?'height':'width',`${size}px`);
          axis.travel = track-size;
        }
        const hidden = axis.extent <= 0;
        if (rail.hidden !== hidden) rail.hidden = hidden;
        const position = positions[index];
        const at = axis.extent ? position/axis.extent*axis.travel : 0;
        style(thumb,'transform',`translate${vertical?'Y':'X'}(${axis.rtl?axis.travel-at:at}px)`);
        attribute(rail,'aria-valuemax',Math.round(axis.extent));
        attribute(rail,'aria-valuenow',Math.round(position));
      }
      dirty = remeasure;
    };
  }
  if (fadeEdges) {
    listen(wrap, 'focusin', () => schedule());
    listen(wrap, 'focusout', () => schedule());
  }
  listen(wrap,'scroll',() => {
    for (const axis of axes) {
      const value = wrap[axis.vertical?'scrollTop':'scrollLeft'];
      if (value !== axis.last) { axis.last = value; axis.reveal(); }
    }
    schedule();
  },{passive:true});
  const resize = new ResizeObserver(() => schedule(true));
  resize.observe(wrap);
  const contents = host ? [...wrap.children] : [wrap.querySelector('table')];
  for (const child of contents) if (child) resize.observe(child);
  // Column drags can consume trailing fill without changing the table's
  // outer size. Its fill width is also an input to the overlay's terminal
  // scroll space, so observe it directly and rebind after header replacement.
  let observedFill = null;
  function observeFill() {
    if (host) return;
    const next = wrap.querySelector('th.table-fill');
    if (next === observedFill) return;
    if (observedFill) resize.unobserve(observedFill);
    observedFill = next;
    if (observedFill) resize.observe(observedFill);
  }
  observeFill();
  const content = new MutationObserver(() => { observeFill(); schedule(true); });
  content.observe(host ? wrap : wrap.querySelector('table'),{childList:true,subtree:true,characterData:true});
  const visibility = new MutationObserver(() => schedule(true));
  const drawerFrame = wrap.closest('.dock-frame');
  if (drawerFrame) visibility.observe(drawerFrame,{attributes:true,attributeFilter:['data-sidebar-closed','data-sidebar-drawer','data-dock-compact']});
  listen(window,'resize',() => schedule(true),{passive:true});
  const dispose = () => {
    pendingScrollbars.delete(measure); resize.disconnect(); content.disconnect(); visibility.disconnect(); removers.forEach(remove=>remove());
    for (const axis of axes) { clearTimeout(axis.timer); axis.rail.remove(); }
  };
  window.addEventListener('pagehide',dispose,{once:true}); schedule(true);
  return dispose;
}
const wrap = document.getElementById('files-view');
if (wrap) initTableScrollbars(wrap);

// The inspector shares the table's native scrolling and fading overlay rails.
// Rails live outside scrollports, so neither their presence nor content growth
// changes a field's width or the sidebar's usable height.
const inspector = document.getElementById('mode-col');
if (inspector) {
  const ports = inspector.querySelectorAll('.editor-scroll, .from-name-scroll, .online-scroll, :scope > .mode-panel:not(#panel-tagger)');
  for (const [index, port] of [...ports].entries()) {
    if (!port.id) port.id = `inspector-scroll-${index}`;
    port.classList.add('overlay-scrollport');
    initTableScrollbars(port, { host: inspector });
  }
}
