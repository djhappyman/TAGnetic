import type { RepairMask, RepairMaskPoint, RepairMaskRegion } from './audio-treatment';

export type RepairBounds = { startFrame: number; endFrame: number; lowHz: number; highHz: number };
export type RepairRectangle = { left: number; top: number; right: number; bottom: number };
export type RepairDrag = { x: -1 | 0 | 1; y: -1 | 0 | 1 };
export type RepairSelectionTool = 'rectangle' | 'time' | 'frequency' | 'brush' | 'lasso';

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
  recordingFrames: number, maximumHz: number, minimumHz = 20,
): RepairBounds {
  const { startFrame, endFrame, lowHz, highHz } = original;
  const width = Math.min(endFrame - startFrame, recordingFrames);
  const minimumBand = Math.min(20, maximumHz - minimumHz);
  const scale = Math.exp(logHzDelta);
  if (drag.x === 0 && drag.y === 0) {
    const start = limit(Math.round(startFrame + framesDelta), 0, recordingFrames - width);
    if (lowHz === 0) {
      const low = limit(Math.round((scale - 1) * highHz), minimumHz, maximumHz - highHz);
      return { startFrame: start, endFrame: start + width, lowHz: low, highHz: low + highHz };
    }
    const ratio = highHz / lowHz;
    const low = Math.round(limit(lowHz * scale, minimumHz, maximumHz / ratio));
    const high = limit(Math.round(low * ratio), low + minimumBand, maximumHz);
    return { startFrame: start, endFrame: start + width, lowHz: Math.min(low, high - minimumBand), highHz: high };
  }
  return {
    startFrame: drag.x < 0 ? limit(Math.round(startFrame + framesDelta), 0, endFrame - 1) : startFrame,
    endFrame: drag.x > 0 ? limit(Math.round(endFrame + framesDelta), startFrame + 1, recordingFrames) : endFrame,
    lowHz: drag.y > 0 ? limit(Math.round(lowHz ? lowHz * scale : (scale - 1) * highHz), minimumHz, highHz - minimumBand) : lowHz,
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

export function repairRegionBounds(region: RepairMaskRegion): RepairBounds {
  if (region.kind === 'rectangle') return region;
  const radiusFrames = region.kind === 'brush' ? region.radiusFrames : 0;
  const radiusHz = region.kind === 'brush' ? region.radiusHz : 0;
  return {
    startFrame: Math.min(...region.points.map(point => point.frame)) - radiusFrames,
    endFrame: Math.max(...region.points.map(point => point.frame)) + radiusFrames,
    lowHz: Math.min(...region.points.map(point => point.hz)) - radiusHz,
    highHz: Math.max(...region.points.map(point => point.hz)) + radiusHz,
  };
}

export function repairMaskBounds(regions: readonly RepairMaskRegion[], frames: number, nyquist: number): RepairBounds | undefined {
  const additions = regions.filter(region => region.operation === 'add').map(repairRegionBounds);
  if (!additions.length) return;
  const lowHz = limit(Math.floor(Math.min(...additions.map(region => region.lowHz))), 0, Math.max(0, nyquist - 20));
  const highHz = limit(Math.ceil(Math.max(...additions.map(region => region.highHz))), lowHz + 20, nyquist);
  const startFrame = limit(Math.floor(Math.min(...additions.map(region => region.startFrame))), 0, frames - 1);
  const endFrame = limit(Math.ceil(Math.max(...additions.map(region => region.endFrame))), startFrame + 1, frames);
  return { startFrame, endFrame, lowHz, highHz };
}

export function remapRepairMask(
  mask: RepairMask, before: RepairBounds, after: RepairBounds,
  recordingFrames: number, nyquist: number, sampleRate: number,
): RepairMask | undefined {
  const timeRatio = (after.endFrame - after.startFrame) / (before.endFrame - before.startFrame);
  const bandRatio = (after.highHz - after.lowHz) / (before.highHz - before.lowHz);
  if (mask.regions.some(region => region.kind === 'brush'
    && (Math.round(region.radiusFrames * timeRatio) > sampleRate * 2 || Math.round(region.radiusHz * bandRatio) > 20_000))) return;
  const frame = (value: number) => Math.round(after.startFrame + (value - before.startFrame) * timeRatio);
  const hz = (value: number) => Math.round(after.lowHz + (value - before.lowHz) * bandRatio);
  const newRegion = (region: RepairMaskRegion): RepairMaskRegion[] => {
    if (region.kind === 'rectangle') {
      const first = frame(region.startFrame);
      const last = frame(region.endFrame);
      const low = hz(region.lowHz);
      const high = hz(region.highHz);
      if (region.operation === 'subtract' && (last <= 0 || first >= recordingFrames || high <= 0 || low >= nyquist)) return [];
      const startFrame = limit(first, 0, recordingFrames);
      const endFrame = limit(Math.max(startFrame + 1, last), 0, recordingFrames);
      const lowHz = limit(low, 0, nyquist);
      const highHz = limit(Math.max(lowHz + 1, high), 0, nyquist);
      return startFrame < endFrame && lowHz < highHz ? [{ ...region, startFrame, endFrame, lowHz, highHz }] : [];
    }
    const points = region.points.map(point => ({ frame: frame(point.frame), hz: hz(point.hz) }));
    if (region.kind === 'lasso') {
      const bounded = region.operation === 'subtract' ? clipLasso(points, recordingFrames, nyquist) : points;
      return bounded.length < 3 ? [] : [{ ...region, points: reducePath(bounded, points.length) }];
    }
    const radiusFrames = Math.max(1, Math.round(region.radiusFrames * timeRatio));
    const radiusHz = Math.max(1, Math.round(region.radiusHz * bandRatio));
    if (region.operation === 'subtract' && (
      points.every(point => point.frame < -radiusFrames) || points.every(point => point.frame > recordingFrames + radiusFrames)
      || points.every(point => point.hz < -radiusHz) || points.every(point => point.hz > nyquist + radiusHz)
    )) return [];
    return [{ ...region, radiusFrames, radiusHz, points: points.map(point => ({
      frame: limit(point.frame, 0, recordingFrames), hz: limit(point.hz, 0, nyquist),
    })) }];
  };
  return {
    ...mask,
    regions: mask.regions.flatMap(newRegion),
  };
}

function reducePath(points: RepairMaskPoint[], count: number) {
  if (points.length <= count) return points;
  return Array.from({ length: count }, (_, index) => points[Math.floor(index * points.length / count)]);
}

function clipLasso(points: RepairMaskPoint[], frames: number, nyquist: number) {
  type Coordinate = keyof RepairMaskPoint;
  const edges = [
    { axis: 'frame', limit: 0, direction: 1 }, { axis: 'frame', limit: frames, direction: -1 },
    { axis: 'hz', limit: 0, direction: 1 }, { axis: 'hz', limit: nyquist, direction: -1 },
  ] as const;
  return edges.reduce((shape, { axis, limit, direction }) => {
    if (!shape.length) return shape;
    const other: Coordinate = axis === 'frame' ? 'hz' : 'frame';
    const inside = (point: RepairMaskPoint) => direction * (point[axis] - limit) >= 0;
    return shape.flatMap((point, index) => {
      const preceding = shape[(index + shape.length - 1) % shape.length];
      const isInside = inside(point);
      if (isInside === inside(preceding)) return isInside ? [point] : [];
      const fraction = (limit - preceding[axis]) / (point[axis] - preceding[axis]);
      const crossing = { ...preceding, [axis]: limit, [other]: Math.round(preceding[other] + fraction * (point[other] - preceding[other])) };
      return isInside ? [crossing, point] : [crossing];
    });
  }, points);
}

function brushOutline(
  region: Extract<RepairMaskRegion, { kind: 'brush' }>,
  project: (point: RepairMaskPoint) => string,
) {
  const { radiusFrames, radiusHz, points } = region;
  const ellipse = (point: RepairMaskPoint) => path(Array.from({ length: 12 }, (_, index) => {
    const angle = index * Math.PI / 6;
    return project({ frame: point.frame + Math.cos(angle) * radiusFrames, hz: point.hz + Math.sin(angle) * radiusHz });
  }));
  function path(vertices: string[]) { return 'M' + vertices.join('L') + 'Z'; }
  return points.map((point, index) => {
    if (!index) return ellipse(point);
    const previous = points[index - 1];
    const angle = Math.atan2((point.hz - previous.hz) / radiusHz, (point.frame - previous.frame) / radiusFrames) + Math.PI / 2;
    const df = Math.cos(angle) * radiusFrames;
    const dh = Math.sin(angle) * radiusHz;
    const edge = (at: RepairMaskPoint, direction: number) => project({ frame: at.frame + df * direction, hz: at.hz + dh * direction });
    return path([edge(previous, 1), edge(previous, -1), edge(point, -1), edge(point, 1)]) + ellipse(point);
  }).join('');
}

export function repairRegionPath(region: RepairMaskRegion, project: (point: RepairMaskPoint) => string): string {
  if (region.kind === 'brush') return brushOutline(region, project);
  if (region.kind === 'lasso') return 'M' + region.points.map(project).join('L') + 'Z';
  return 'M' + [
    { frame: region.startFrame, hz: region.lowHz },
    { frame: region.endFrame, hz: region.lowHz },
    { frame: region.endFrame, hz: region.highHz },
    { frame: region.startFrame, hz: region.highHz },
  ].map(project).join('L') + 'Z';
}
