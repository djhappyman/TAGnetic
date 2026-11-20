// Interior-inspired continuity, implemented with native Web Animations. Only
// fixed chrome is observed: virtual table rows never acquire observers/players.
export function initWorkbenchMotion() {
  const reduced = matchMedia('(prefers-reduced-motion: reduce)');
  const cleanups = [];
  for (const list of document.querySelectorAll('.mode-tabs, .subtabs')) {
    const pill = document.createElement('span');
    pill.className = 'navigation-motion-pill';
    pill.setAttribute('aria-hidden', 'true');
    list.append(pill);
    list.dataset.motionNavigation = '';
    let selected = null;
    let animation;
    function place(animate = false) {
      const next = [...list.querySelectorAll('button')].find(button =>
        button.hasAttribute('aria-selected') ? button.getAttribute('aria-selected') === 'true' : button.classList.contains('active'));
      if (!next || !next.getClientRects().length) { pill.hidden = true; return; }
      const from = pill.hidden ? null : pill.getBoundingClientRect();
      const parent = list.getBoundingClientRect();
      const target = next.getBoundingClientRect();
      const box = { left: `${target.left - parent.left + list.scrollLeft - list.clientLeft}px`, top: `${target.top - parent.top + list.scrollTop - list.clientTop}px`, width: `${target.width}px`, height: `${target.height}px` };
      animation?.cancel();
      Object.assign(pill.style, box);
      pill.hidden = false;
      if (animate && selected && selected !== next && from && !reduced.matches && typeof pill.animate === 'function') {
        animation = pill.animate([
          { left: `${from.left - parent.left + list.scrollLeft - list.clientLeft}px`, top: `${from.top - parent.top + list.scrollTop - list.clientTop}px`, width: `${from.width}px`, height: `${from.height}px` }, box,
        ], { duration: 180, easing: 'cubic-bezier(.2,.8,.2,1)' });
      }
      selected = next;
    }
    const observer = new MutationObserver(() => place(true));
    observer.observe(list, { subtree: true, attributes: true, attributeFilter: ['aria-selected', 'class'] });
    const resize = new ResizeObserver(() => place());
    resize.observe(list);
    for (const button of list.querySelectorAll('button')) resize.observe(button);
    const settle = () => { animation?.cancel(); place(); };
    reduced.addEventListener('change', settle);
    place();
    cleanups.push(() => { observer.disconnect(); resize.disconnect(); reduced.removeEventListener('change', settle); animation?.cancel(); pill.remove(); delete list.dataset.motionNavigation; });
  }
  // A single restrained acknowledgement of a changed draft, rather than an
  // animation per edited cell or a success notification for every keystroke.
  for (const node of document.querySelectorAll('#staged-count, #staged-transformations, #staged-tags')) {
    let previous = node.textContent;
    let animation;
    const observer = new MutationObserver(() => {
      if (node.textContent === previous) return;
      previous = node.textContent;
      animation?.cancel();
      if (!reduced.matches && !node.closest('[hidden]') && typeof node.animate === 'function') {
        animation = node.animate([{ opacity: .55, transform: 'translateY(2px)' }, { opacity: 1, transform: 'translateY(0)' }], { duration: 140, easing: 'ease-out' });
      }
    });
    const settle = () => animation?.cancel();
    reduced.addEventListener('change', settle);
    observer.observe(node, { childList: true, characterData: true, subtree: true });
    cleanups.push(() => { observer.disconnect(); animation?.cancel(); reduced.removeEventListener('change', settle); });
  }
  window.addEventListener('pagehide', () => cleanups.forEach(cleanup => cleanup()), { once: true });
}
