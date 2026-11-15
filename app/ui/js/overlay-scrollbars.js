// Wheel/trackpad scrolling stays native. Only the overlaid thumbs are custom.
export function initTableScrollbars(wrap) {
  let frame = 0, dirty = true;
  const removers = [];
  const listen = (node,type,handler,options) => {
    node.addEventListener(type,handler,options);
    removers.push(() => node.removeEventListener(type,handler,options));
  };
  const style = (node,key,value) => { if (node.style[key] !== value) node.style[key] = value; };
  const attribute = (node,key,value) => { if (node.getAttribute(key) !== String(value)) node.setAttribute(key,String(value)); };
  function schedule(geometry = false) { dirty ||= geometry; if (!frame) frame = requestAnimationFrame(paint); }
  function createAxis(vertical) {
    const rail = document.createElement('div'), thumb = document.createElement('div');
    rail.className = `table-scrollbar ${vertical ? 'is-vertical' : 'is-horizontal'}`;
    rail.tabIndex = 0;
    for (const [name,value] of Object.entries({role:'scrollbar','aria-label':vertical?'Scroll tracks':'Scroll columns','aria-controls':wrap.id,'aria-orientation':vertical?'vertical':'horizontal','aria-valuemin':'0'})) rail.setAttribute(name,value);
    thumb.className = 'table-scrollbar-thumb'; thumb.setAttribute('aria-hidden','true');
    rail.append(thumb); wrap.after(rail);
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
        write(normalized()+direction*(axis.rtl?-1:1)*(vertical?wrap.clientHeight:wrap.clientWidth));
      }
    });
    listen(rail,'pointermove',event => {
      if (axis.drag && axis.travel > 0) write(axis.drag.position+(coordinate(event)-axis.drag.at)*(axis.rtl?-1:1)*axis.extent/axis.travel);
    });
    const release = () => { axis.drag = null; settle(); };
    for (const type of ['pointerup','pointercancel','lostpointercapture']) listen(rail,type,release);
    listen(rail,'keydown',event => {
      const page = vertical ? wrap.clientHeight : wrap.clientWidth;
      const moves = vertical ? {ArrowUp:-24,ArrowDown:24,PageUp:-page,PageDown:page}
        : {ArrowLeft:-24*(axis.rtl?-1:1),ArrowRight:24*(axis.rtl?-1:1),PageUp:-page,PageDown:page};
      if (!(event.key in moves) && event.key !== 'Home' && event.key !== 'End') return;
      event.preventDefault(); axis.reveal();
      write(event.key === 'Home' ? 0 : event.key === 'End' ? axis.extent : normalized()+moves[event.key]);
    });
    return axis;
  }
  const axes = [createAxis(true),createAxis(false)];
  function paint() {
    frame = 0;
    const height = wrap.clientHeight, width = wrap.clientWidth;
    const extents = [Math.max(0,wrap.scrollHeight-height),Math.max(0,wrap.scrollWidth-width)];
    const head = dirty ? wrap.querySelector('thead')?.getBoundingClientRect().height || 0 : 0;
    for (const [index,axis] of axes.entries()) {
      const {rail,thumb,vertical} = axis;
      axis.extent = extents[index];
      const hidden = axis.extent <= 0 || (vertical?height:width) === 0;
      if (rail.hidden !== hidden) rail.hidden = hidden;
      if (dirty) {
        axis.rtl = !vertical && getComputedStyle(wrap).direction === 'rtl';
        const track = vertical ? Math.max(0,height-head-(extents[1]?12:0)) : Math.max(0,width-(extents[0]?12:0));
        const visible = vertical ? Math.max(0,height-head) : width;
        const size = Math.min(track,Math.max(24,track*visible/Math.max(visible,visible+axis.extent)));
        style(rail,'top',`${wrap.offsetTop+wrap.clientTop+(vertical?head:height-12)}px`);
        style(rail,vertical?'height':'width',`${track}px`);
        if (!vertical) style(rail,'left',`${wrap.offsetLeft+wrap.clientLeft}px`);
        style(thumb,vertical?'height':'width',`${size}px`);
        axis.travel = track-size;
      }
      const position = vertical ? wrap.scrollTop : Math.abs(wrap.scrollLeft);
      const at = axis.extent ? position/axis.extent*axis.travel : 0;
      style(thumb,'transform',`translate${vertical?'Y':'X'}(${axis.rtl?axis.travel-at:at}px)`);
      attribute(rail,'aria-valuemax',Math.round(axis.extent));
      attribute(rail,'aria-valuenow',Math.round(position));
    }
    dirty = false;
  }
  listen(wrap,'scroll',() => {
    for (const axis of axes) {
      const value = wrap[axis.vertical?'scrollTop':'scrollLeft'];
      if (value !== axis.last) { axis.last = value; axis.reveal(); }
    }
    schedule();
  },{passive:true});
  const resize = new ResizeObserver(() => schedule(true));
  resize.observe(wrap); resize.observe(wrap.querySelector('table'));
  const content = new MutationObserver(() => schedule(true));
  content.observe(wrap.querySelector('table'),{childList:true,subtree:true,characterData:true});
  listen(window,'resize',() => schedule(true),{passive:true});
  const dispose = () => {
    cancelAnimationFrame(frame); resize.disconnect(); content.disconnect(); removers.forEach(remove=>remove());
    for (const axis of axes) { clearTimeout(axis.timer); axis.rail.remove(); }
  };
  window.addEventListener('pagehide',dispose,{once:true}); schedule(true);
  return dispose;
}
const wrap = document.getElementById('files-view');
if (wrap) initTableScrollbars(wrap);
