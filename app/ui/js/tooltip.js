// Route native titles and icon hints through one delayed tooltip surface.
import { showControllerTooltip } from '../lib/controller-surfaces.js';
const titles = new WeakMap();
const descriptions = new WeakMap();
const labels = new WeakMap();
let initialized = false;
let currentAnchor = null;
let pendingHint;
let warmUntil = 0;
const OPEN_DELAY = 450;
const SKIP_DELAY = 400;
const RIBBON_BUTTON = '.command-ribbon button.ribbon-command';

function suppressTitle(target) {
  if (!(target instanceof Element) || !target.hasAttribute("title")) return;
  const text = target.getAttribute("title").trim();
  if (text) {
    titles.set(target, text);
    const previousDescription = descriptions.get(target);
    const description = target.getAttribute("aria-description");
    if (!description || description === previousDescription) {
      target.setAttribute("aria-description", text);
      descriptions.set(target, text);
    }

    // Some legacy icon buttons use title as their only accessible name. Give
    // those a durable name, without replacing visible labels or explicit ARIA.
    const previousLabel = labels.get(target);
    const label = target.getAttribute("aria-label");
    if (!target.hasAttribute("aria-labelledby") &&
        ((!label && !target.textContent.trim() &&
          target.matches('button, a[href], [role="button"], input[type="button"], input[type="submit"], input[type="reset"]')) ||
          (previousLabel && label === previousLabel))) {
      target.setAttribute("aria-label", text);
      labels.set(target, text);
    }
  } else {
    titles.delete(target);
    if (target.getAttribute("aria-description") === descriptions.get(target)) {
      // Validation and progress controls clear their explanatory text explicitly.
      target.removeAttribute("aria-description");
      descriptions.delete(target);
    }
  }
  target.removeAttribute("title");
}

function suppressTitles(root) {
  if (!(root instanceof Element)) return;
  suppressTitle(root);
  for (const target of root.querySelectorAll("[title]")) suppressTitle(target);
}

// Retain the controller API used when menus open; it never restores a title.
export function hideTooltip(keepWarm = false) {
  if (!keepWarm) warmUntil = 0;
  clearTimeout(pendingHint);
  currentAnchor = null;
  const tip = document.getElementById("tooltip");
  if (tip) tip.hidden = true;
}

function hintText(element) {
  const explicit = element.getAttribute('data-tooltip') || titles.get(element);
  if (explicit) return explicit;
  if (element.matches(RIBBON_BUTTON) ||
      (element.matches('button[aria-label], a[href][aria-label], [role="button"][aria-label]') && !element.textContent.trim())) {
    return element.getAttribute('aria-description') || element.getAttribute('aria-label');
  }
  return '';
}
function hintAnchor(target) {
  if (!(target instanceof HTMLElement || target instanceof SVGElement) || target.closest('.mode-tabs, #tooltip')) return null;
  for (let element = target; element && element !== document.body; element = element.parentElement) {
    if (!(element instanceof HTMLElement)) continue;
    suppressTitle(element);
    if (hintText(element)) return element;
  }
  return null;
}
function revealHint(anchor, delay = OPEN_DELAY) {
  if (!anchor || currentAnchor === anchor) return;
  const warm = performance.now() < warmUntil;
  hideTooltip(true);
  currentAnchor = anchor;
  pendingHint = setTimeout(() => {
    if (currentAnchor !== anchor || !anchor.isConnected || anchor.closest('[hidden], [inert]')) return;
    const text = hintText(anchor);
    if (text) {
      showControllerTooltip(anchor, text, '');
      warmUntil = Infinity;
    }
  }, delay === 0 || warm ? 0 : Number(anchor.getAttribute('data-tooltip-delay')) || delay);
}

export function initTooltips() {
  if (initialized) return;
  initialized = true;
  hideTooltip();
  suppressTitles(document.documentElement);

  document.addEventListener('pointerover', event => {
    if (event.pointerType !== 'touch') revealHint(hintAnchor(event.target));
  });
  document.addEventListener('pointerout', event => {
    if (currentAnchor?.contains(event.target) && !currentAnchor.contains(event.relatedTarget)) {
      if (warmUntil === Infinity) warmUntil = performance.now() + SKIP_DELAY;
      hideTooltip(true);
    }
  });
  document.addEventListener('focusin', event => revealHint(hintAnchor(event.target), 0));
  document.addEventListener('focusout', event => { if (currentAnchor?.contains(event.target)) hideTooltip(); });
  document.addEventListener('pointerdown', () => hideTooltip(), true);
  document.addEventListener('keydown', event => { if (event.key === 'Escape') hideTooltip(); }, true);
  document.addEventListener('scroll', () => hideTooltip(), true);
  window.addEventListener('resize', () => hideTooltip());
  window.addEventListener('blur', () => hideTooltip());

  // Menus, virtual rows, translations and button states all create or rewrite
  // titles after startup. Keep descriptions current without pointermove work.
  const observer = new MutationObserver((records) => {
    for (const record of records) {
      if (record.type === "attributes") suppressTitle(record.target);
      else for (const node of record.addedNodes) suppressTitles(node);
    }
  });
  observer.observe(document.documentElement, {
    subtree: true,
    childList: true,
    attributes: true,
    attributeFilter: ["title"],
  });
}
