import type { FrameRange } from './audio-split';
import { plotBounds, type PlotBounds, type TreatmentProbe } from './audio-treatment-plot';

export type FrequencyRange = { lowHz: number; highHz: number };
export type RepairViewport = { time: FrameRange; frequency: FrequencyRange };
export type RepairTexture = { bitmap: HTMLCanvasElement; time: FrameRange; frequency: FrequencyRange };
export type RepairChannel = 'combined' | 'left' | 'right';
export type RepairContrast = 'balanced' | 'detail' | 'strong';
export type RepairWaveform = { values: readonly number[]; time: FrameRange; stereo?: boolean; channel?: RepairChannel; opacity?: number; clipping?: boolean; edge?: 'top' | 'bottom' };
const repairPalettes = {
  theme: [[37, 37, 37], [102, 102, 102], [177, 177, 177], [251, 251, 251]],
  warm: [[39, 22, 25], [170, 72, 34], [238, 179, 88], [255, 240, 203]],
  neutral: [[35, 39, 43], [115, 126, 133], [182, 193, 198], [244, 248, 248]],
  blue: [[23, 37, 57], [52, 105, 155], [120, 194, 203], [244, 249, 211]],
};
export type RepairPaletteName = keyof typeof repairPalettes;
const frequencyLabel = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 });

const themeStops = {
  dark: [[.29, .04, 0], [.53, .085, 15], [.73, .115, 34], [.89, .065, 42]],
  light: [[.92, .025, 0], [.74, .075, 15], [.55, .115, 34], [.33, .085, 42]],
};

function paletteStops(palette: RepairPaletteName, light: boolean, accent: string) {
  if (palette === 'theme' && CSS.supports('color', 'oklch(from blue .5 .1 calc(h + 15))')) {
    // Lightness orders signal level. The neighbouring hues give that ramp
    // more separation without borrowing the waveform's primary hue.
    return themeStops[light ? 'light' : 'dark'].map(([brightness, chroma, hue]) =>
      `oklch(from ${accent} ${brightness} ${chroma} calc(h + ${hue}))`);
  }
  const share = light ? [.14, .38, .37, .31] : palette === 'theme' ? [.48, .76, .62, .32] : [.34, .58, .36, .14];
  const lightness = [226, 166, 104, 26];
  return repairPalettes[palette].map((colour, index) => {
    const luma = colour[0] * .2126 + colour[1] * .7152 + colour[2] * .0722;
    const base = light ? colour.map(channel => Math.round(clampRepair(lightness[index] + (channel - luma) * .5, 0, 255))) : colour;
    return `color-mix(in srgb, rgb(${base.join(' ')}), ${accent} ${Math.round(share[index] * 100)}%)`;
  });
}

export function repairPaletteGradient(palette: RepairPaletteName, light: boolean): string {
  return paletteStops(palette, light, 'var(--accent)').join(', ');
}

export function clampRepair(value: number, minimum: number, maximum: number): number {
  return Math.max(minimum, Math.min(maximum, value));
}

export function recordingFrequencyRange(rate: number): FrequencyRange {
  return { lowHz: 20, highHz: Math.max(21, Math.min(20_000, Math.floor(rate * .45))) };
}

export function repairPlotBounds(width: number, height: number, sharedTime = false): PlotBounds {
  const bounds = plotBounds(width, height);
  return sharedTime
    ? { ...bounds, left: 0, top: 7, bottom: Math.max(8, height - 7) }
    : { ...bounds, left: 1 };
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
  const span = clampRepair(size, Math.min(frames, rate / 4), frames);
  const first = clampRepair(start, 0, frames - span);
  return { start: first, end: first + span };
}

export function spectralWindowLimit(rate: number, stereo: boolean): number {
  // Keep viewport probes inside Rust's decoded sample and duration budgets.
  return Math.min(rate * 12, stereo ? 2_000_000 : 4_000_000);
}

export function interpolateRepairTime(from: FrameRange, to: FrameRange, progress: number): FrameRange {
  const fromSpan = from.end - from.start;
  const toSpan = to.end - to.start;
  const span = fromSpan * (toSpan / fromSpan) ** progress;
  // Move the window by the same proportion as its span, keeping the
  // pointer's source frame fixed when the endpoints share an anchor.
  const fraction = fromSpan === toSpan ? progress : (span - fromSpan) / (toSpan - fromSpan);
  const start = from.start + (to.start - from.start) * fraction;
  return { start, end: start + span };
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

export function repairDbScale(contrast: RepairContrast, offset = 0) {
  const floor = contrast === 'detail' ? -112 : contrast === 'strong' ? -72 : -94;
  const ceiling = contrast === 'detail' ? -28 : -22;
  return { floor: floor + offset, ceiling: ceiling + offset };
}

export function repairLevelAt(probe: TreatmentProbe, frame: number, hz: number): number | undefined {
  const range = spectrogramRange(probe);
  const columns = probe.spectrogram;
  if (!range || !columns?.length || frame < range.start || frame > range.end || hz <= 0) return;
  const column = clampRepair(Math.floor((frame - range.start) * columns.length / (range.end - range.start)), 0, columns.length - 1);
  const bin = probe.frequencies.findIndex((frequency, index, all) =>
    index === all.length - 1 || hz < Math.sqrt(frequency * all[index + 1]));
  return columns[column][bin];
}

export function buildRepairTexture(
  probe: TreatmentProbe, rate: number, palette: RepairPaletteName, contrast: RepairContrast, offset: number, accent: string, light: boolean,
): RepairTexture | null {
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
  const bounds = { left: 0, top: 0, right: bitmap.width, bottom: bitmap.height };
  const rows = frequencies.map((hz, index) => {
    const upper = index + 1 === frequencies.length ? frequency.highHz : Math.sqrt(hz * frequencies[index + 1]);
    return frequencyY(clampRepair(upper, frequency.lowHz, frequency.highHz), bounds, frequency);
  });
  const { floor, ceiling } = repairDbScale(contrast, offset);
  // Read the few CSS swatches once. Interpolate a small sRGB lookup table
  // so rendering cells and cached tiles never requires CSS colour parsing.
  const swatches = paletteStops(palette, light, accent);
  for (const [row, swatch] of swatches.entries()) {
    ctx.fillStyle = swatch;
    ctx.fillRect(0, row, 1, 1);
  }
  const pixels = ctx.getImageData(0, 0, 1, swatches.length).data;
  ctx.clearRect(0, 0, 1, swatches.length);
  const colours = swatches.map((_, index) => pixels.slice(index * 4, index * 4 + 3));
  const steps = 64;
  const lookup = Array.from({ length: steps }, (_, index) => {
    const fraction = index / (steps - 1) * (colours.length - 1);
    const from = Math.min(colours.length - 2, Math.floor(fraction));
    const colour = colours[from].map((value, channel) => Math.round(value + (fraction - from) * (colours[from + 1][channel] - value)));
    return 'rgb(' + colour.join(' ') + ')';
  });
  for (const [x, levels] of columns.entries()) {
    for (let bin = 0; bin < Math.min(levels.length, rows.length); bin++) {
      const strength = clampRepair((levels[bin] - floor) / (ceiling - floor), 0, 1);
      if (strength <= 0) continue;
      const top = rows[bin];
      const bottom = rows[bin - 1] ?? bitmap.height;
      ctx.fillStyle = lookup[Math.round(strength * (steps - 1))];
      ctx.globalAlpha = strength ** 1.42 * .88;
      ctx.fillRect(x, top, 1, Math.max(0, bottom - top) + .5);
    }
  }
  return { bitmap, time, frequency };
}

type RepairPalette = { grid: string; ink: string; waveform: string; guide: string; font: string };

export function drawRepairViewport(
  canvas: HTMLCanvasElement, width: number, height: number, viewport: RepairViewport,
  textures: readonly RepairTexture[], rate: number, palette: RepairPalette,
  guides: readonly number[], scale = 1,
  waveform?: RepairWaveform, waveformOnly = false, sharedTime = false,
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
  const bounds = repairPlotBounds(width, height, sharedTime);
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
  ctx.restore();

  ctx.save();
  ctx.beginPath();
  ctx.rect(bounds.left, 0, bounds.right - bounds.left, height);
  ctx.clip();
  if (waveform) {
    const inset = waveformOnly ? 0 : (bounds.bottom - bounds.top) * .34;
    const waveformBounds = waveform.edge === 'bottom' ? { ...bounds, top: height * .7, bottom: height }
      : waveform.edge === 'top' ? { ...bounds, top: 0, bottom: height * .3 }
      : { ...bounds, top: bounds.top + inset, bottom: bounds.bottom - inset };
    drawWaveform(ctx, waveform, waveformBounds,
      time, palette.waveform, waveform.opacity ?? .16);
  }

  // Fade the painted signal into the surface while leaving the axes crisp.
  const fade = Math.min(12, (bounds.right - bounds.left) / 4);
  const leftFade = ctx.createLinearGradient(bounds.left, 0, bounds.left + fade, 0);
  leftFade.addColorStop(0, '#000');
  leftFade.addColorStop(1, '#0000');
  const rightFade = ctx.createLinearGradient(bounds.right - fade, 0, bounds.right, 0);
  rightFade.addColorStop(0, '#0000');
  rightFade.addColorStop(1, '#000');
  ctx.globalCompositeOperation = 'destination-out';
  ctx.fillStyle = leftFade;
  ctx.fillRect(bounds.left, 0, fade, height);
  ctx.fillStyle = rightFade;
  ctx.fillRect(bounds.right - fade, 0, fade, height);
  ctx.globalCompositeOperation = 'source-over';

  ctx.strokeStyle = palette.guide;
  ctx.globalAlpha = .56;
  ctx.setLineDash([4, 4]);
  for (const hz of guides) {
    if (waveformOnly || hz < frequency.lowHz || hz > frequency.highHz) continue;
    const y = frequencyY(hz, bounds, frequency);
    strokeLine(ctx, bounds.left, y, bounds.right, y);
  }
  ctx.restore();

  ctx.lineWidth = 1;
  ctx.strokeStyle = palette.grid;
  ctx.fillStyle = palette.ink;
  ctx.font = '10px ' + palette.font;
  ctx.textBaseline = 'middle';
  ctx.textAlign = 'left';
  const ticks = waveformOnly ? [-1, -.5, 0, .5, 1] : [20, 50, 100, 200, 500, 1000, 2000, 5000, 10_000, 20_000]
    .filter(hz => hz >= frequency.lowHz && hz <= frequency.highHz);
  if (!waveformOnly && ticks.length < 2) {
    ticks.length = 0;
    for (let i = 0; i < 3; i++) ticks.push(frequency.lowHz * (frequency.highHz / frequency.lowHz) ** (i / 2));
  }
  for (const tick of ticks) {
    const y = waveformOnly ? (bounds.top + bounds.bottom) / 2 - (bounds.bottom - bounds.top) * .47 * tick
      : frequencyY(tick, bounds, frequency);
    ctx.globalAlpha = .45;
    strokeLine(ctx, bounds.left, y, bounds.right, y);
    ctx.globalAlpha = .9;
    ctx.fillText(waveformOnly ? String(tick) : hertzLabel(tick), bounds.right + 6, y);
  }
  if (!waveformOnly && !sharedTime) ctx.fillText('Hz', bounds.right + 6, bounds.bottom + 13);
  ctx.textAlign = 'center';
  for (const fraction of [0, .25, .5, .75, 1]) {
    const x = bounds.left + (bounds.right - bounds.left) * fraction;
    ctx.globalAlpha = .45;
    strokeLine(ctx, x, bounds.top, x, bounds.bottom);
    if (!sharedTime) {
      const frame = time.start + (time.end - time.start) * fraction;
      ctx.globalAlpha = .9;
      ctx.textAlign = fraction === 0 ? 'left' : fraction === 1 ? 'right' : 'center';
      ctx.fillText(repairTimeLabel(frame, rate, time.end - time.start < rate), x, bounds.bottom + 13);
    }
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
  const center = waveform.edge === 'top' ? bounds.top : waveform.edge === 'bottom' ? bounds.bottom : (bounds.top + bounds.bottom) / 2;
  const height = (bounds.bottom - bounds.top) * (waveform.edge ? .94 : .47);
  function peak(index: number, side: number) {
    let value = 0;
    const from = first + index * (last - first) / points;
    const to = first + (index + 1) * (last - first) / points;
    const start = clampRepair(Math.floor((from - waveform.time.start) * count / span), 0, count - 1);
    const end = clampRepair(Math.ceil((to - waveform.time.start) * count / span), start + 1, count);
    if (waveform.edge) {
      const rising = waveform.edge === 'bottom';
      if (!!side !== rising) return center;
      let magnitude = 0;
      for (let bin = start; bin < end; bin++) {
        const base = bin * stride + offset;
        magnitude = Math.max(magnitude, Math.abs(waveform.values[base] ?? 0), Math.abs(waveform.values[base + 1] ?? 0));
      }
      return center + (rising ? -1 : 1) * Math.min(magnitude, 1) * height;
    }
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
  if (waveform.clipping) {
    ctx.strokeStyle = '#ffaba0';
    ctx.globalAlpha = .9;
    for (let index = 0; index < points; index++) {
      const first = left + index * (right - left) / points;
      const last = left + (index + 1) * (right - left) / points;
      for (const side of [0, 1]) {
        const y = peak(index, side);
        if (Math.abs(y - center) >= height * .99999) strokeLine(ctx, first, y, last, y);
      }
    }
  }
  ctx.restore();
}

function strokeLine(ctx: CanvasRenderingContext2D, x1: number, y1: number, x2: number, y2: number) {
  ctx.beginPath();
  ctx.moveTo(x1, y1);
  ctx.lineTo(x2, y2);
  ctx.stroke();
}
