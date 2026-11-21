import { tableViewportBounds } from './table-viewport.js';

const duration = 260;
const easing = 'cubic-bezier(.35,0,.2,1)';
const clamp = (value, low, high) => Math.max(low, Math.min(high, value));
const translate = distance => `translateY(${distance}px)`;
const groupKey = key => `group:${key}`;
const rowKey = row => row.dataset.path
  ? `file:${row.dataset.group?.length ?? 0}:${row.dataset.group ?? ''}${row.dataset.path}`
  : groupKey(row.dataset.group);

// Commit virtual membership once. Live neighbours move by their layout delta;
// the only snapshots are rows intersecting the viewport that are leaving it.
export function createGroupMotion(wrap, viewportRows, rowLayout) {
  const reduced = matchMedia('(prefers-reduced-motion: reduce)');
  const animations = new Set();
  let finishLayer = null;
  let scrollTop = 0, scrollLeft = 0;
  let lastFinished = -Infinity;
  const caretOf = group => viewportRows().find(row => !row.dataset.path && row.dataset.group === group)?.querySelector('.group-caret > svg');
  function cancel() {
    for (const animation of animations) animation.cancel();
    animations.clear();
    finishLayer?.();
  }
  function animate(node, frames, time = duration) {
    const animation = node.animate(frames, {duration:time, easing});
    animations.add(animation);
    animation.finished.then(() => animations.delete(animation), () => animations.delete(animation));
    return animation;
  }
  function run(change, {group = null, collapsing, affected}) {
    const rapid = group !== null && performance.now() - lastFinished < 160;
    const oldCaret = rapid && group !== null && !reduced.matches ? caretOf(group) : null;
    const rotation = oldCaret ? getComputedStyle(oldCaret).transform : null;
    cancel();
    try {
      if (reduced.matches || typeof Element.prototype.animate !== 'function') { change(); return; }
      // Every request commits immediately even during a burst. Keep the caret
      // continuous without stacking snapshots on repeated reversals.
      if (rapid) {
        change();
        scrollTop = wrap.scrollTop;
        scrollLeft = wrap.scrollLeft;
        const caret = rotation && caretOf(group);
        if (caret) animate(caret,[{transform:rotation},{transform:collapsing?'rotate(-90deg)':'rotate(0)'}],80);
        return;
      }
      transition(change, collapsing, affected);
    } finally {
      lastFinished = performance.now();
    }
  }

  function transition(change, collapsing, affected) {
    const viewport = tableViewportBounds(wrap);
    const top = Math.max(viewport.top, wrap.querySelector('thead').getBoundingClientRect().bottom);
    const travel = Math.max(0, viewport.bottom - top);
    if (!travel) { change(); return; }
    const prior = rowLayout();
    const before = visibleRows(top, viewport.bottom);
    const snapshots = [];
    const cellStyles = new Map();
    const styleState = {widths:null, cellStyles};
    // An expansion can displace rows that would otherwise be unmounted. Save
    // the visible tail so it can slide down while the new group reveals.
    let expandingSeen = !collapsing && before.length > 0 &&
      affected.has(before[0].row.dataset.group);
    for (const entry of before) {
      const {row} = entry;
      const changes = affected.has(row.dataset.group);
      if (collapsing ? row.dataset.path && changes : expandingSeen) {
        snapshots.push({...entry, ...snapshotRow(entry, viewport.left, styleState)});
      }
      expandingSeen ||= !collapsing && changes && !row.dataset.path;
    }
    change();
    scrollTop = wrap.scrollTop;
    scrollLeft = wrap.scrollLeft;
    const current = rowLayout();
    const after = visibleRows(top, viewport.bottom);
    const previous = new Map(before.map(entry => [rowKey(entry.row), entry.rect.top]));
    const next = new Map(after.map(entry => [rowKey(entry.row), entry.rect.top]));
    const headerShift = key => {
      const id = groupKey(key);
      return clamp((previous.get(id) ?? prior.groupTop(key)) - (next.get(id) ?? current.groupTop(key)), -travel, travel);
    };
    const exits = new Map();
    const reveals = new Map();
    for (const entry of snapshots) {
      const key = entry.row.dataset.group;
      if (!collapsing && next.has(rowKey(entry.row))) continue;
      const shift = collapsing ? -headerShift(key) : clamp(current.rowTop(entry.row) - entry.rect.top, -travel, travel);
      if (!Number.isFinite(shift) || (!collapsing && Math.abs(shift) <= .5)) continue;
      // One strip per exiting group. On expansion, neighbours with the same
      // displacement share a strip even when they belong to different groups.
      const stripId = collapsing ? key : Math.round(shift*2)/2;
      let strip = exits.get(stripId);
      if (!strip) exits.set(stripId, strip = {from:0, to:shift, rows:[], motion:collapsing?'close':'move'});
      strip.rows.push(entry);
    }

    for (const {row, rect} of after) {
      const key = row.dataset.group;
      const changes = affected.has(key);
      if (!collapsing && changes && row.dataset.path && !previous.has(rowKey(row))) {
        let strip = reveals.get(key);
        if (!strip) reveals.set(key, strip = {from:headerShift(key), to:0, rows:[], motion:'open'});
        strip.rows.push({row, rect, ...snapshotRow({row,rect}, viewport.left, styleState)});
        continue;
      }
      const from = previous.get(rowKey(row)) ?? prior.rowTop(row);
      const shift = clamp(from - rect.top, -travel, travel);
      if (Math.abs(shift) > .5) animate(row, [{transform:translate(shift)}, {transform:translate(0)}]);
      if (changes && !row.dataset.path) {
        const caret = row.querySelector('.group-caret > svg');
        if (caret) animate(caret, [{transform:collapsing?'rotate(0)':'rotate(-90deg)'}, {transform:collapsing?'rotate(-90deg)':'rotate(0)'}]);
      }
    }
    if (exits.size || reveals.size) paintStrips([...exits.values(), ...reveals.values()], viewport, top);
  }

  function visibleRows(top, bottom) {
    const found = [];
    for (const row of viewportRows()) {
      const rect = row.getBoundingClientRect();
      if (rect.bottom > top && rect.top < bottom) found.push({row, rect});
    }
    return found;
  }

  function snapshotRow({row,rect}, left, settings) {
    const clone = row.cloneNode(true);
    for (const name of ['data-path','data-group','tabindex','aria-rowindex','aria-expanded']) clone.removeAttribute(name);
    Object.assign(clone.style, {position:'absolute',display:'table',tableLayout:'fixed',left:`${rect.left-left}px`,width:`${rect.width}px`,height:`${rect.height}px`,margin:'0'});
    const group = !row.dataset.path;
    if (!group && !settings.widths && !row.classList.contains('unreadable')) {
      settings.widths = Array.from(row.cells, cell => cell.getBoundingClientRect().width);
    }
    const rowStyle = `${row.className}|${Number(row.getAttribute('aria-rowindex')) % 2}|${row.matches(':hover')}`;
    for (const [index, cell] of Array.from(row.cells).entries()) {
      const cacheKey = `${rowStyle}|${cell.className}`;
      let style = settings.cellStyles.get(cacheKey);
      if (!style) {
        const computed = getComputedStyle(cell);
        const declaration = document.createElement('td').style;
        Object.assign(declaration,{
          boxSizing:'border-box', display:computed.display, padding:computed.padding,
          color:computed.color, background:computed.backgroundColor, opacity:computed.opacity,
          font:computed.font, lineHeight:computed.lineHeight, textAlign:computed.textAlign, verticalAlign:computed.verticalAlign,
          whiteSpace:computed.whiteSpace, overflow:'hidden', textOverflow:computed.textOverflow, border:'0',
        });
        style = declaration.cssText;
        settings.cellStyles.set(cacheKey,style);
      }
      const width = group ? rect.width : row.classList.contains('unreadable') ? cell.getBoundingClientRect().width : settings.widths[index];
      const copy = clone.cells[index];
      copy.style.cssText = `${style};width:${width}px`;
    }
    for (const node of clone.querySelectorAll('[id],[tabindex],[contenteditable]')) {
      node.removeAttribute('id');
      node.removeAttribute('tabindex');
      node.removeAttribute('contenteditable');
    }
    let artwork = null;
    if (group) {
      const original = row.querySelector('.group-heading-content');
      const heading = clone.querySelector('.group-heading-content');
      if (heading && original) heading.style.marginLeft = `${original.getBoundingClientRect().left - rect.left - parseFloat(getComputedStyle(row.cells[0]).paddingLeft)}px`;
    } else {
      const indicator = row.querySelector('.track-artwork');
      const copy = clone.querySelector('.track-artwork');
      if (indicator && copy) {
        // Keep the glyph at the live viewport coordinates. The copied table
        // does its own column layout and cannot inherit the sticky column.
        const bounds = indicator.getBoundingClientRect();
        artwork = {node:copy, left:bounds.left-left, top:bounds.top};
        copy.remove();
        if (wrap.scrollLeft && settings.widths) {
          const cell = indicator.parentElement;
          const naturalLeft = rect.left + settings.widths.slice(0, cell.cellIndex).reduce((total, width) => total + width, 0);
          const shift = cell.getBoundingClientRect().left - naturalLeft;
          if (Math.abs(shift) > .5) Object.assign(clone.cells[cell.cellIndex].style,{position:'relative',left:`${shift}px`,zIndex:'1'});
        }
      }
    }
    return {clone, artwork};
  }

  function paintStrips(strips, viewport, top) {
    const layer = document.createElement('div');
    layer.className = 'group-motion-exit';
    layer.classList.toggle('show-old',wrap.querySelector('#tracks').classList.contains('show-old'));
    layer.setAttribute('aria-hidden','true');
    layer.inert = true;
    Object.assign(layer.style,{left:`${viewport.left}px`,top:`${top}px`,width:`${viewport.width}px`,height:`${viewport.bottom-top}px`});
    const players = [];
    const concealed = new Map();
    for (const {from, to, rows, motion} of strips) {
      const start = Math.max(top, rows[0].rect.top);
      const end = Math.min(viewport.bottom, rows.at(-1).rect.bottom);
      const clipper = document.createElement('div');
      clipper.className = 'group-motion-strip';
      clipper.dataset.motion = motion;
      Object.assign(clipper.style,{top:`${start-top}px`,height:motion === 'open' ? '0px' : `${end-start}px`});
      const table = document.createElement('table');
      const body = document.createElement('tbody');
      for (const {row, clone, rect} of rows) {
        clone.style.top = `${rect.top-start}px`;
        body.append(clone);
        if (motion === 'open') {
          concealed.set(row, row.style.opacity);
          row.style.opacity = '0';
        }
      }
      table.append(body);
      clipper.append(table);
      for (const {artwork} of rows) {
        if (!artwork) continue;
        Object.assign(artwork.node.style,{position:'absolute',left:`${artwork.left}px`,top:`${artwork.top-start}px`});
        clipper.append(artwork.node);
      }
      layer.append(clipper);
      const full = `${end-start}px`;
      players.push({clipper, frames:[
        {transform:translate(Number.isFinite(from) ? from : 0), height:motion === 'open' ? '0px' : full},
        {transform:translate(to), height:motion === 'close' ? '0px' : full},
      ]});
    }
    document.body.append(layer);
    const finish = () => {
      for (const [row, opacity] of concealed) row.style.opacity = opacity;
      layer.remove();
      if (finishLayer === finish) finishLayer = null;
    };
    finishLayer = finish;
    // The isolated overflow box defines the visible edge at every height.
    // The live table has already committed its final layout.
    Promise.allSettled(players.map(({clipper,frames}) => animate(clipper, frames).finished)).then(() => {
      if (finishLayer === finish) finish();
    });
  }
  function scrolled() {
    // Layout clamping queues a scroll event for the offsets just captured.
    if ((animations.size || finishLayer) && (wrap.scrollTop !== scrollTop || wrap.scrollLeft !== scrollLeft)) cancel();
  }
  function pointerDown(event) {
    if (!event.target.closest('tr.group-head') || event.ctrlKey || event.metaKey || event.button !== 0) cancel();
  }
  wrap.addEventListener('scroll',scrolled,{passive:true});
  wrap.addEventListener('pointerdown',pointerDown,{capture:true,passive:true});
  window.addEventListener('resize',cancel,{passive:true});
  window.addEventListener('pagehide',() => {
    cancel();
    wrap.removeEventListener('scroll',scrolled);
    wrap.removeEventListener('pointerdown',pointerDown,true);
    window.removeEventListener('resize',cancel);
  },{once:true});
  return {run,cancel};
}
