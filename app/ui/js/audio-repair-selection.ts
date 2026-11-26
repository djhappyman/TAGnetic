export type RepairBounds = { startFrame: number; endFrame: number; lowHz: number; highHz: number };
export type RepairRectangle = { left: number; top: number; right: number; bottom: number };
export type RepairDrag = { x: -1 | 0 | 1; y: -1 | 0 | 1 };

export function limit(value: number, minimum: number, maximum: number) {
  return Math.max(minimum, Math.min(maximum, value));
}

// Hit only genuine edges. A rectangle clipped by zoom must not expose a
// resize handle on the edge that is currently off-screen.
export function repairDragAt(rect: RepairRectangle, plot: RepairRectangle, x: number, y: number): RepairDrag | undefined {
  const slopX = Math.min(7, Math.max(2, (rect.right - rect.left) / 3));
  const slopY = Math.min(7, Math.max(2, (rect.bottom - rect.top) / 3));
  if (x < rect.left - slopX || x > rect.right + slopX || y < rect.top - slopY || y > rect.bottom + slopY) return;
  const left = rect.left >= plot.left && Math.abs(x - rect.left) <= slopX;
  const right = rect.right <= plot.right && Math.abs(x - rect.right) <= slopX;
  const top = rect.top >= plot.top && Math.abs(y - rect.top) <= slopY;
  const bottom = rect.bottom <= plot.bottom && Math.abs(y - rect.bottom) <= slopY;
  if (left || right || top || bottom || (x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom)) {
    return { x: left ? -1 : right ? 1 : 0, y: top ? -1 : bottom ? 1 : 0 };
  }
}

export function repairDragCursor(drag?: RepairDrag) {
  if (!drag) return 'crosshair';
  if (!drag.x && !drag.y) return 'move';
  if (!drag.x) return 'ns-resize';
  if (!drag.y) return 'ew-resize';
  return drag.x === drag.y ? 'nwse-resize' : 'nesw-resize';
}

export function editRepairBounds(
  original: RepairBounds, drag: RepairDrag, framesDelta: number, logHzDelta: number,
  recordingFrames: number, maximumHz: number,
): RepairBounds {
  const { startFrame, endFrame, lowHz, highHz } = original;
  const width = Math.min(endFrame - startFrame, recordingFrames);
  const minimumBand = Math.min(20, maximumHz - 20);
  const scale = Math.exp(logHzDelta);
  if (drag.x === 0 && drag.y === 0) {
    const start = limit(Math.round(startFrame + framesDelta), 0, recordingFrames - width);
    const ratio = highHz / lowHz;
    const low = Math.round(limit(lowHz * scale, 20, maximumHz / ratio));
    const high = limit(Math.round(low * ratio), low + minimumBand, maximumHz);
    return { startFrame: start, endFrame: start + width, lowHz: Math.min(low, high - minimumBand), highHz: high };
  }
  return {
    startFrame: drag.x < 0 ? limit(Math.round(startFrame + framesDelta), 0, endFrame - 1) : startFrame,
    endFrame: drag.x > 0 ? limit(Math.round(endFrame + framesDelta), startFrame + 1, recordingFrames) : endFrame,
    lowHz: drag.y > 0 ? limit(Math.round(lowHz * scale), 20, highHz - minimumBand) : lowHz,
    highHz: drag.y < 0 ? limit(Math.round(highHz * scale), lowHz + minimumBand, maximumHz) : highHz,
  };
}

// The edge zones deliberately shrink on small windows. The quadratic ramp
// starts gently and reaches a steady rate beyond the plot.
export function edgePanSpeed(position: number, start: number, end: number) {
  const zone = Math.min(28, Math.max(4, (end - start) / 5));
  if (position < start + zone) return -(limit((start + zone - position) / zone, 0, 1) ** 2);
  if (position > end - zone) return limit((position - end + zone) / zone, 0, 1) ** 2;
  return 0;
}
