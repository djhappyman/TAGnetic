// Search display/import options outlive the popup that edits them.
const storageKey = 'tagrex.searchPerPage';
let perPage = 5;
try {
  const stored = Number(localStorage.getItem(storageKey));
  if ([5, 10, 15].includes(stored)) perPage = stored;
} catch { /* Storage may be unavailable; retain the session preference. */ }
let vinylSidesToDisc = false;

export function getOnlineOptions() { return { perPage, vinylSidesToDisc }; }
export function setOnlineOptions(options) {
  const changedPage = [5, 10, 15].includes(options.perPage) && options.perPage !== perPage;
  if (changedPage) {
    perPage = options.perPage;
    try { localStorage.setItem(storageKey, String(perPage)); } catch { /* Session-only preference. */ }
  }
  if (typeof options.vinylSidesToDisc === 'boolean') vinylSidesToDisc = options.vinylSidesToDisc;
  if (changedPage) document.dispatchEvent(new CustomEvent('tagrex:online-page-size'));
}
