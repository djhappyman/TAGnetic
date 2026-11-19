// Keep native title bubbles suppressed around dense editors and suggestions.
// The command ribbon and query mode toggle use our themed tooltip surface,
// including commands added later, without individual tooltip wrappers.
import { showControllerTooltip } from '../lib/controller-surfaces.js';
const descriptions = new WeakMap();
const labels = new WeakMap();
let initialized = false;
let ribbonAnchor = null;
let ribbonTimer;

function suppressTitle(target) {
  if (!(target instanceof Element) || !target.hasAttribute("title")) return;
  const text = target.getAttribute("title").trim();
  if (text) {
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
  } else if (target.getAttribute("aria-description") === descriptions.get(target)) {
    // Validation and progress controls clear their explanatory text explicitly.
    target.removeAttribute("aria-description");
    descriptions.delete(target);
  }
  target.removeAttribute("title");
}

function suppressTitles(root) {
  if (!(root instanceof Element)) return;
  suppressTitle(root);
  for (const target of root.querySelectorAll("[title]")) suppressTitle(target);
}

// Retain the controller API used when menus open; it never restores a title.
export function hideTooltip() {
  clearTimeout(ribbonTimer);
  ribbonAnchor = null;
  const tip = document.getElementById("tooltip");
  if (tip) tip.hidden = true;
}

function ribbonButton(target) {
  return target instanceof Element ? target.closest('.command-ribbon button.ribbon-command, #query-mode-toggle, [data-tooltip]') : null;
}
function revealRibbonHint(button, delay) {
  if (!button || ribbonAnchor === button) return;
  hideTooltip();
  ribbonAnchor = button;
  ribbonTimer = setTimeout(() => {
    if (ribbonAnchor !== button || !button.isConnected || button.closest('[hidden], [inert]')) return;
    const text = button.getAttribute('data-tooltip') || button.getAttribute('aria-description') || button.getAttribute('aria-label');
    if (text) showControllerTooltip(button, text, '');
  }, delay === 0 ? 0 : Number(button.getAttribute('data-tooltip-delay')) || delay);
}

export function initTooltips() {
  if (initialized) return;
  initialized = true;
  hideTooltip();
  suppressTitles(document.documentElement);

  document.addEventListener('pointerover', event => revealRibbonHint(ribbonButton(event.target), 1000));
  document.addEventListener('pointerout', event => {
    const button = ribbonButton(event.target);
    if (button && !button.contains(event.relatedTarget)) hideTooltip();
  });
  document.addEventListener('focusin', event => revealRibbonHint(ribbonButton(event.target), 0));
  document.addEventListener('focusout', event => { if (ribbonButton(event.target)) hideTooltip(); });
  document.addEventListener('pointerdown', hideTooltip, true);
  document.addEventListener('keydown', event => { if (event.key === 'Escape') hideTooltip(); }, true);
  document.addEventListener('scroll', hideTooltip, true);
  window.addEventListener('resize', hideTooltip);
  window.addEventListener('blur', hideTooltip);

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
