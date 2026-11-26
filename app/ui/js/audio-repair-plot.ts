import type { FrameRange } from './audio-split';
import { plotBounds, type PlotBounds, type TreatmentProbe } from './audio-treatment-plot';

export type FrequencyRange = { lowHz: number; highHz: number };
export type RepairViewport = { time: FrameRange; frequency: FrequencyRange };
export type RepairTexture = { bitmap: HTMLCanvasElement; time: FrameRange; frequency: FrequencyRange };
export type RepairChannel = 'combined' | 'left' | 'right';
export type RepairContrast = 'balanced' | 'detail' | 'strong';
export type RepairWaveform = { values: readonly number[]; time: FrameRange; stereo?: boolean; channel?: RepairChannel };
const frequencyLabel = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 });

export function clampRepair(value: number, minimum: number, maximum: number): number {
  return Math.max(minimum, Math.min(maximum, value));
}

export function recordingFrequencyRange(rate: number): FrequencyRange {
  return { lowHz: 20, highHz: Math.max(21, Math.min(20_000, Math.floor(rate * .45))) };
}

export function spectrogramRange(probe?: TreatmentProbe | null): FrameRange | null {
  const start = probe?.spectrogramStart;
  const end = probe?.spectrogramEnd;
  return start !== undefined && end !== undefined && start < end ? { start, end } : null;
}

export function frameAtX(x: number, bounds: PlotBounds, time: FrameRange): number {
  const fraction = clampRepair((x - bounds.left) / (bounds.right - bounds.left), 0, 1);
  return time.start + fraction * (time.end - time.start);
}

export function frameX(frame: number, bounds: PlotBounds, time: FrameRange): number {
  return bounds.left + (frame - time.start) / (time.end - time.start) * (bounds.right - bounds.left);
}

export function frequencyAtY(y: number, bounds: PlotBounds, frequency: FrequencyRange): number {
  const fraction = clampRepair((y - bounds.top) / (bounds.bottom - bounds.top), 0, 1);
  return frequency.highHz * (frequency.lowHz / frequency.highHz) ** fraction;
}

export function frequencyY(hz: number, bounds: PlotBounds, frequency: FrequencyRange): number {
  return bounds.top + Math.log(frequency.highHz / hz) / Math.log(frequency.highHz / frequency.lowHz) * (bounds.bottom - bounds.top);
}

export function boundedTime(start: number, size: number, rate: number, frames: number): FrameRange {
  const span = clampRepair(size, Math.min(frames, rate / 4), Math.min(frames, rate * 12));
  const first = clampRepair(start, 0, frames - span);
  return { start: first, end: first + span };
}

export function repairTimeLabel(frame: number, rate: number, precise = false): string {
  const digits = precise ? 2 : 1;
  const precision = precise ? 100 : 10;
  const seconds = Math.round(frame / rate * precision) / precision;
  return Math.floor(seconds / 60) + ':' + (seconds % 60).toFixed(digits).padStart(3 + digits, '0');
}

function hertzLabel(hz: number): string {
  return hz >= 1000 ? frequencyLabel.format(hz / 1000) + 'k' : String(Math.round(hz));
}

export function buildRepairTexture(probe: TreatmentProbe, rate: number, accent: string, contrast: RepairContrast = 'balanced'): RepairTexture | null {
  const time = spectrogramRange(probe);
  const columns = probe.spectrogram;
  const frequencies = probe.frequencies;
  if (!time || !columns?.length || frequencies.length < 2) return null;
  const frequency = recordingFrequencyRange(rate);
  const bitmap = document.createElement('canvas');
  bitmap.width = columns.length;
  bitmap.height = 240;
  const ctx = bitmap.getContext('2d');
  if (!ctx) return null;
  ctx.fillStyle = accent;
  const bounds = { left: 0, top: 0, right: bitmap.width, bottom: bitmap.height };
  const rows = frequencies.map((hz, index) => {
    const upper = index + 1 === frequencies.length ? frequency.highHz : Math.sqrt(hz * frequencies[index + 1]);
    return frequencyY(clampRepair(upper, frequency.lowHz, frequency.highHz), bounds, frequency);
  });
  const floor = contrast === 'detail' ? -112 : contrast === 'strong' ? -72 : -94;
  const ceiling = contrast === 'detail' ? -28 : -22;
  for (const [x, levels] of columns.entries()) {
    for (let bin = 0; bin < Math.min(levels.length, rows.length); bin++) {
      const strength = clampRepair((levels[bin] - floor) / (ceiling - floor), 0, 1);
      if (strength <= 0) continue;
      const top = rows[bin];
      const bottom = rows[bin - 1] ?? bitmap.height;
      ctx.globalAlpha = strength ** 1.65 * .84;
      ctx.fillRect(x, top, 1, Math.max(0, bottom - top) + .5);
    }
  }
  return { bitmap, time, frequency };
}

type RepairPalette = { grid: string; ink: string; guide: string; font: string };

export function drawRepairViewport(
  canvas: HTMLCanvasElement, width: number, height: number, viewport: RepairViewport,
  textures: readonly RepairTexture[], rate: number, palette: RepairPalette,
  guides: readonly number[], scale = 1,
  waveform?: RepairWaveform,
): void {
  if (width <= 0 || height <= 0) return;
  const ctx = canvas.getContext('2d');
  if (!ctx) return;
  const pixelsWide = Math.round(width * scale);
  const pixelsHigh = Math.round(height * scale);
  if (canvas.width !== pixelsWide) canvas.width = pixelsWide;
  if (canvas.height !== pixelsHigh) canvas.height = pixelsHigh;
  ctx.setTransform(scale, 0, 0, scale, 0, 0);
  ctx.clearRect(0, 0, width, height);
  const { time, frequency } = viewport;
  const bounds = plotBounds(width, height);
  ctx.save();
  ctx.beginPath();
  ctx.rect(bounds.left, bounds.top, bounds.right - bounds.left, bounds.bottom - bounds.top);
  ctx.clip();
  ctx.imageSmoothingEnabled = true;
  for (const texture of textures) {
    const first = Math.max(time.start, texture.time.start);
    const last = Math.min(time.end, texture.time.end);
    const low = Math.max(frequency.lowHz, texture.frequency.lowHz);
    const high = Math.min(frequency.highHz, texture.frequency.highHz);
    if (last <= first || high <= low) continue;
    const x = frameX(first, bounds, time);
    const y = frequencyY(high, bounds, frequency);
    const tx = (first - texture.time.start) / (texture.time.end - texture.time.start) * texture.bitmap.width;
    const tw = (last - first) / (texture.time.end - texture.time.start) * texture.bitmap.width;
    const textureBounds = { left: 0, top: 0, right: texture.bitmap.width, bottom: texture.bitmap.height };
    const ty = frequencyY(high, textureBounds, texture.frequency);
    const th = frequencyY(low, textureBounds, texture.frequency) - ty;
    const w = frameX(last, bounds, time) - x;
    const h = frequencyY(low, bounds, frequency) - y;
    // Replace an overlapping tile without accumulating opacity where observations meet.
    ctx.clearRect(x, y, w, h);
    ctx.drawImage(texture.bitmap, tx, ty, tw, th, x, y, w, h);
  }
  if (waveform) drawWaveform(ctx, waveform, bounds, time, palette.ink, .52);

  ctx.strokeStyle = palette.guide;
  ctx.globalAlpha = .56;
  ctx.setLineDash([4, 4]);
  for (const hz of guides) {
    if (hz < frequency.lowHz || hz > frequency.highHz) continue;
    const y = frequencyY(hz, bounds, frequency);
    strokeLine(ctx, bounds.left, y, bounds.right, y);
  }
  ctx.restore();

  ctx.lineWidth = 1;
  ctx.strokeStyle = palette.grid;
  ctx.fillStyle = palette.ink;
  ctx.font = '10px ' + palette.font;
  ctx.textBaseline = 'middle';
  ctx.textAlign = 'right';
  const ticks = [20, 50, 100, 200, 500, 1000, 2000, 5000, 10_000, 20_000]
    .filter(hz => hz >= frequency.lowHz && hz <= frequency.highHz);
  if (ticks.length < 2) {
    ticks.length = 0;
    for (let i = 0; i < 3; i++) ticks.push(frequency.lowHz * (frequency.highHz / frequency.lowHz) ** (i / 2));
  }
  for (const hz of ticks) {
    const y = frequencyY(hz, bounds, frequency);
    ctx.globalAlpha = .45;
    strokeLine(ctx, bounds.left, y, bounds.right, y);
    ctx.globalAlpha = .9;
    ctx.fillText(hertzLabel(hz), bounds.left - 6, y);
  }
  ctx.textAlign = 'center';
  for (const fraction of [0, .25, .5, .75, 1]) {
    const x = bounds.left + (bounds.right - bounds.left) * fraction;
    const frame = time.start + (time.end - time.start) * fraction;
    ctx.globalAlpha = .45;
    strokeLine(ctx, x, bounds.top, x, bounds.bottom);
    ctx.globalAlpha = .9;
    ctx.fillText(repairTimeLabel(frame, rate, time.end - time.start < rate), x, bounds.bottom + 13);
  }
  ctx.globalAlpha = 1;
}

export function drawRepairOverview(
  canvas: HTMLCanvasElement, width: number, height: number, scale: number,
  waveform: RepairWaveform, ink: string,
) {
  if (width <= 0 || height <= 0) return;
  const ctx = canvas.getContext('2d');
  if (!ctx) return;
  const pixelsWide = Math.round(width * scale);
  const pixelsHigh = Math.round(height * scale);
  if (canvas.width !== pixelsWide) canvas.width = pixelsWide;
  if (canvas.height !== pixelsHigh) canvas.height = pixelsHigh;
  ctx.setTransform(scale, 0, 0, scale, 0, 0);
  ctx.clearRect(0, 0, width, height);
  drawWaveform(ctx, waveform, { left: 0, top: 3, right: width, bottom: height - 3 }, waveform.time, ink, .72);
}

function drawWaveform(
  ctx: CanvasRenderingContext2D, waveform: RepairWaveform, bounds: PlotBounds,
  time: FrameRange, ink: string, opacity: number,
) {
  const stride = waveform.stereo ? 4 : 2;
  const offset = waveform.stereo && waveform.channel === 'right' ? 2 : 0;
  const count = Math.floor(waveform.values.length / stride);
  const span = waveform.time.end - waveform.time.start;
  const first = Math.max(time.start, waveform.time.start);
  const last = Math.min(time.end, waveform.time.end);
  if (!count || span <= 0 || first >= last) return;
  const left = frameX(first, bounds, time);
  const right = frameX(last, bounds, time);
  const points = Math.max(1, Math.min(Math.ceil(right - left), Math.ceil((last - first) * count / span)));
  const center = (bounds.top + bounds.bottom) / 2;
  const height = (bounds.bottom - bounds.top) * .47;
  function peak(index: number, side: number) {
    let value = 0;
    const from = first + index * (last - first) / points;
    const to = first + (index + 1) * (last - first) / points;
    const start = clampRepair(Math.floor((from - waveform.time.start) * count / span), 0, count - 1);
    const end = clampRepair(Math.ceil((to - waveform.time.start) * count / span), start + 1, count);
    const compare = side ? Math.max : Math.min;
    for (let bin = start; bin < end; bin++) value = compare(value, waveform.values[bin * stride + offset + side] ?? 0);
    return center - clampRepair(value, -1, 1) * height;
  }
  ctx.save();
  ctx.fillStyle = ink;
  ctx.globalAlpha = opacity;
  ctx.beginPath();
  for (let index = 0; index < points; index++) {
    const y = peak(index, 1);
    ctx.lineTo(left + index * (right - left) / points, y);
    ctx.lineTo(left + (index + 1) * (right - left) / points, y);
  }
  for (let index = points - 1; index >= 0; index--) {
    const y = peak(index, 0);
    ctx.lineTo(left + (index + 1) * (right - left) / points, y);
    ctx.lineTo(left + index * (right - left) / points, y);
  }
  ctx.closePath();
  ctx.fill();
  ctx.restore();
}

function strokeLine(ctx: CanvasRenderingContext2D, x1: number, y1: number, x2: number, y2: number) {
  ctx.beginPath();
  ctx.moveTo(x1, y1);
  ctx.lineTo(x2, y2);
  ctx.stroke();
}
