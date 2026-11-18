import { initTableScrollbars } from './overlay-scrollbars.js';

// Local geometry only: resizing this table never changes the library viewport.
export function initTransformationPane(root, port, handle) {
  const key = 'tagrex.transformationsHeight';
  let preferred = 240;
  try { const saved = Number(localStorage.getItem(key)); if (saved >= 100 && saved <= 2000) preferred = saved; } catch { /* Session-only sizing. */ }
  let frame = 0, drag = null, collapsed = false, motion = 0, motionFrame = 0;
  const toggle = root.querySelector('#transform-collapse');
  const footer = root.querySelector('.transform-table-footer');
  try { collapsed = localStorage.getItem('tagrex.transformationsCollapsed') === '1'; } catch { /* Session-only state. */ }
  const parent = root.closest('#subtab-editor') || root.parentElement;
  const disposeScrollbars = initTableScrollbars(port,{host:root});
  const bounds = () => {
    const style = getComputedStyle(parent);
    const max = Math.max(124,Math.floor(parent.clientHeight*.85)-parseFloat(style.paddingTop)-parseFloat(style.paddingBottom));
    return {min:Math.min(124,max),max};
  };
  function paint() {
    const {min,max} = bounds(), height = Math.max(min,Math.min(max,preferred));
    const style = getComputedStyle(root);
    const foldedHeight = Math.max(32,root.querySelector('.transform-heading').getBoundingClientRect().height)+parseFloat(style.borderTopWidth)+parseFloat(style.borderBottomWidth);
    root.style.height = `${collapsed ? foldedHeight : height}px`;
    handle.setAttribute('aria-valuemin',String(Math.round(min)));
    handle.setAttribute('aria-valuemax',String(Math.round(max)));
    handle.setAttribute('aria-valuenow',String(Math.round(height)));
  }
  function save() { try {localStorage.setItem(key,String(preferred));} catch { /* Session-only sizing. */ } }
  function fades() {
    frame = 0;
    const top = port.scrollTop > 1, bottom = port.scrollHeight-port.clientHeight-port.scrollTop > 1;
    port.dataset.fadeTop = String(top); port.dataset.fadeBottom = String(bottom);
  }
  function scheduleFades() { if (!frame) frame = requestAnimationFrame(fades); }
  function stop() {
    if (!drag) return;
    drag = null; save();
    window.removeEventListener('pointermove',move);
    window.removeEventListener('pointerup',stop);
    window.removeEventListener('pointercancel',stop);
  }
  function move(event) {
    if (!drag) return;
    const {min,max} = bounds(); preferred = Math.max(min,Math.min(max,drag.height+event.clientY-drag.y)); paint();
  }
  function start(event) {
    if (event.button !== 0 || collapsed) return;
    event.preventDefault(); stop();
    drag = {y:event.clientY,height:root.getBoundingClientRect().height};
    handle.focus({preventScroll:true}); handle.setPointerCapture?.(event.pointerId);
    window.addEventListener('pointermove',move);
    window.addEventListener('pointerup',stop,{once:true});
    window.addEventListener('pointercancel',stop,{once:true});
  }
  function keyboard(event) {
    if (!['ArrowUp','ArrowDown','Home','End'].includes(event.key)) return;
    event.preventDefault(); event.stopPropagation();
    const {min,max} = bounds(), current = root.getBoundingClientRect().height;
    preferred = event.key === 'Home' ? min : event.key === 'End' ? max : Math.max(min,Math.min(max,current+(event.key === 'ArrowDown' ? 16 : -16)));
    paint(); save();
  }
  function syncCollapsed() {
    root.dataset.collapsed = String(collapsed);
    toggle.setAttribute('aria-expanded',String(!collapsed));
    port.setAttribute('aria-hidden',String(collapsed));
  }
  function setCollapsed(value) {
    if (value === collapsed) return;
    stop(); collapsed = value; const version = ++motion;
    cancelAnimationFrame(motionFrame);
    port.inert = false; footer.inert = false;
    root.dataset.collapseAnimating = 'true';
    syncCollapsed(); paint();
    try {localStorage.setItem('tagrex.transformationsCollapsed',collapsed ? '1' : '0');} catch { /* Session-only state. */ }
    motionFrame = requestAnimationFrame(async()=>{
      await Promise.all(root.getAnimations().filter(animation=>animation.transitionProperty === 'height').map(animation=>animation.finished.catch(()=>{})));
      if (version !== motion) return;
      delete root.dataset.collapseAnimating;
      port.inert = collapsed; footer.inert = collapsed;
      scheduleFades();
    });
  }
  function toggleCollapsed() { toggle.focus({preventScroll:true}); setCollapsed(!collapsed); }
  function reveal() { setCollapsed(false); }
  toggle.addEventListener('click',toggleCollapsed);
  root.querySelector('#transform-add').addEventListener('click',reveal);
  syncCollapsed(); port.inert = collapsed; footer.inert = collapsed;
  const resize = new ResizeObserver(()=>{paint();scheduleFades();});
  resize.observe(parent);
  const viewport = new ResizeObserver(scheduleFades); viewport.observe(port);
  const content = new MutationObserver(scheduleFades); content.observe(port,{childList:true,subtree:true,attributes:true,attributeFilter:['hidden']});
  port.addEventListener('scroll',scheduleFades,{passive:true});
  handle.addEventListener('pointerdown',start); handle.addEventListener('keydown',keyboard);
  paint(); scheduleFades();
  window.addEventListener('pagehide',()=>{
    stop(); ++motion;cancelAnimationFrame(motionFrame);toggle.removeEventListener('click',toggleCollapsed);root.querySelector('#transform-add').removeEventListener('click',reveal);disposeScrollbars(); if(frame)cancelAnimationFrame(frame); resize.disconnect();viewport.disconnect();content.disconnect();
    port.removeEventListener('scroll',scheduleFades);handle.removeEventListener('pointerdown',start);handle.removeEventListener('keydown',keyboard);
  },{once:true});
}
