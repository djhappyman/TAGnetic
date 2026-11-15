// Side-effect-free seam: dialogs mount before the workflow controllers do.
let execute;
export function connectRibbonActions(handler) { execute = handler; }
export async function runRibbonAction(action, options = {}) {
  if (!execute) throw new Error('The app is still opening. Try again in a moment.');
  return execute(action, options);
}
