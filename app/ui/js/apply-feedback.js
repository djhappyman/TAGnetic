// Pace confirmed write feedback, never the executor. A burst drains in at most
// eight frames; continuous slow writes still appear as their events arrive.
export function createApplyFeedback(review, changes, {
  reducedMotion = globalThis.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false,
  frame = callback => requestAnimationFrame(callback), cancelFrame = id => cancelAnimationFrame(id),
  timer = callback => setTimeout(callback, 40), clearTimer = id => clearTimeout(id),
} = {}) {
  let queue = [], cursor = 0, chunk = 1, frameId = null, timerId = null;
  let sealed = false, cancelled = false, resolveDrain;
  const received = new Map();
  function stopFrame() {
    if (frameId !== null) cancelFrame(frameId);
    if (timerId !== null) clearTimer(timerId);
    frameId = timerId = null;
  }
  function tick() {
    stopFrame();
    if (cancelled) return;
    chunk = Math.max(chunk, Math.ceil((queue.length - cursor) / 8));
    const end = globalThis.document?.hidden ? queue.length : Math.min(queue.length, cursor + chunk);
    while (cursor < end) review.progress(queue[cursor++]);
    if (cursor < queue.length) schedule();
    else { queue = []; cursor = 0; chunk = 1; resolveDrain?.(); resolveDrain = null; }
  }
  function schedule() {
    if (frameId !== null || cancelled || cursor === queue.length) return;
    frameId = frame(tick); timerId = timer(tick);
  }
  function enqueue(event) {
    if (reducedMotion) review.progress(event);
    else { queue.push(event); schedule(); }
  }
  function progress(event) {
    if (sealed || cancelled) return;
    const bit = event.completed === 'tags' ? 1 : event.completed === 'transfer' ? 2 : 0;
    if (bit) received.set(event.path, (received.get(event.path) || 0) | bit);
    enqueue(event);
  }
  function finish() {
    if (cancelled) return Promise.resolve();
    sealed = true;
    // An IPC result may arrive before its final event. Successful batch return
    // confirms these remaining steps; late events must not replay them twice.
    for (const change of changes) {
      const seen = received.get(change.path) || 0;
      if ((change.tag_changes?.length || change.cover_change || change.block_changes?.length) && !(seen & 1)) enqueue({path:change.path, completed:'tags'});
      if (change.rename_to && !(seen & 2)) enqueue({path:change.path, completed:'transfer'});
    }
    return cursor === queue.length ? Promise.resolve() : new Promise(resolve => { resolveDrain = resolve; });
  }
  function cancel() {
    cancelled = true; stopFrame(); queue = []; cursor = 0;
    resolveDrain?.(); resolveDrain = null;
  }
  return { progress, finish, cancel };
}
