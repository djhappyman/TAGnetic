import { bellQuality, eqFrequencyLimit, eqGainLimit } from './audio-eq-range.js';
import type { AudioTreatment, EqBand, EqChannel } from './audio-treatment';
import type { DynamicsMode, DynamicsPlotOptions } from './audio-dynamics';

export type TreatmentPlotMode = 'eq' | 'dynamics' | 'repair';
export type EqView = { start: number; end: number; gainRange: 6 | 12 | 24 };
export type PlotBounds = { left: number; right: number; top: number; bottom: number; eq?: EqView };
export type TreatmentProbe = {
  frequencies: readonly number[];
  original: readonly number[];
  treated: readonly number[];
  reduction: readonly number[];
  input: number;
  output: number;
  inputPeak?: number;
  outputPeak?: number;
  detector?: number;
  gateReduction?: readonly number[];
  dynamicEqMix?: readonly number[];
  upwardEq?: boolean;
  dynamicAllShapes?: boolean;
  dynamicRatio?: boolean;
  spectrogram?: readonly (readonly number[])[];
  spectrogramStart?: number;
  spectrogramEnd?: number;
  spectrogramChannel?: 'combined' | 'left' | 'right';
  spectrogramTreated?: boolean;
  spectrogramWaveform?: readonly number[];
};

type ProbeTween = (progress: number) => TreatmentProbe;

function createProbeMotion(duration: number, prepare: (from: TreatmentProbe, to: TreatmentProbe) => ProbeTween | undefined) {
  let target: TreatmentProbe | null = null;
  let shown: TreatmentProbe | null = null;
  let tween: ProbeTween | undefined;
  let start = 0;
  return (measured: TreatmentProbe | null, animate: boolean, paint: (probe: TreatmentProbe | null) => void) => {
    if (!measured || !animate) {
      shown = target = measured;
      tween = undefined;
      paint(measured);
      return;
    }
    const now = performance.now();
    if (measured !== target) {
      tween = shown ? prepare(shown, measured) : undefined;
      target = measured;
      start = now;
    }
    let frame = 0;
    function render(time: number) {
      const progress = Math.min(1, Math.max(0, (time - start) / duration));
      if (tween && progress < 1) {
        shown = tween(progress);
        paint(shown);
        frame = requestAnimationFrame(render);
      } else {
        shown = measured;
        tween = undefined;
        paint(measured);
      }
    }
    render(now);
    return () => cancelAnimationFrame(frame);
  };
}

function softly(progress: number) {
  return progress * progress * (3 - 2 * progress);
}

const blendLevel = (from: number, to: number, weight: number) => from + (to - from) * weight;

export function createMeterMotion() {
  return createProbeMotion(110, (from, to) => {
    if (from.input === to.input && from.output === to.output && from.detector === to.detector &&
        from.reduction.length === to.reduction.length &&
        to.reduction.every((db, index) => db === from.reduction[index]) &&
        from.gateReduction?.length === to.gateReduction?.length &&
        (to.gateReduction?.every((db, index) => db === from.gateReduction?.[index]) ?? true)) return;
    const reduction = new Array<number>(to.reduction.length);
    const gateReduction = to.gateReduction ? new Array<number>(to.gateReduction.length) : undefined;
    const view = { ...to, reduction, gateReduction };
    return progress => {
      const weight = softly(progress);
      view.input = blendLevel(from.input, to.input, weight);
      view.output = blendLevel(from.output, to.output, weight);
      if (from.detector !== undefined && to.detector !== undefined) {
        view.detector = blendLevel(from.detector, to.detector, weight);
      }
      for (let index = 0; index < reduction.length; index++) {
        reduction[index] = blendLevel(from.reduction[index] ?? to.reduction[index], to.reduction[index], weight);
      }
      if (gateReduction && to.gateReduction) {
        for (let index = 0; index < gateReduction.length; index++) {
          const gain = to.gateReduction[index];
          gateReduction[index] = blendLevel(from.gateReduction?.[index] ?? gain, gain, weight);
        }
      }
      return view;
    };
  });
}

export function createSpectrumMotion() {
  return createProbeMotion(105, (from, to) => {
    const bins = to.frequencies.length;
    const comparable = bins > 0 && from.frequencies.length === bins && from.original.length === bins &&
      from.treated.length === bins && to.original.length === bins && to.treated.length === bins &&
      to.frequencies.every((hz, index) => hz === from.frequencies[index]);
    const spectrumChanged = comparable && to.original.some((db, index) =>
      db !== from.original[index] || to.treated[index] !== from.treated[index]);
    const nextMix = to.dynamicEqMix ?? [];
    const lastMix = from.dynamicEqMix ?? [];
    const mixesChanged = nextMix.some((mix, index) => mix !== (lastMix[index] ?? 0));
    if (!spectrumChanged && !mixesChanged) return;
    const original = spectrumChanged ? new Array<number>(bins) : undefined;
    const treated = spectrumChanged ? new Array<number>(bins) : undefined;
    const dynamicEqMix = mixesChanged ? new Array<number>(nextMix.length) : undefined;
    const view = { ...to, original: original ?? to.original, treated: treated ?? to.treated,
      dynamicEqMix: dynamicEqMix ?? to.dynamicEqMix };
    return progress => {
      const weight = 1 - (1 - progress) ** 2;
      if (original && treated) {
        for (let index = 0; index < bins; index++) {
          original[index] = blendLevel(from.original[index], to.original[index], weight);
          treated[index] = blendLevel(from.treated[index], to.treated[index], weight);
        }
      }
      if (dynamicEqMix) {
        for (let index = 0; index < nextMix.length; index++) {
          dynamicEqMix[index] = blendLevel(lastMix[index] ?? 0, nextMix[index], weight);
        }
      }
      return view;
    };
  });
}

type Coefficients = { numerator: readonly [number, number, number]; denominator: readonly [number, number, number] };
export type AnalyzerView = 'before' | 'after' | 'both' | 'off';
type ColoredEqBand = { band: EqBand; color: string; selected: boolean };
type EqPlotOptions = { analyzer?: AnalyzerView; selectedBand?: EqBand; bands?: readonly ColoredEqBand[]; ghostBand?: EqBand; dynamicMix?: readonly number[]; view?: EqView; bandSelection?: boolean };

type CanvasPalette = { muted: string; grid: string; accent: string; ink: string };
const hzMin = 20;
const eqMax = eqGainLimit;
const levelMin = -60;
const fftFloor = -96;
const hzTicks = [20, 30, 50, 80, 100, 150, 200, 300, 500, 800, 1000, 1500, 2000, 3000, 5000, 8000, 10000, 15000, 20000];
const labelledFrequencies = [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000];
const clamp = (value: number, minimum: number, maximum: number): number => Math.max(minimum, Math.min(maximum, value));
const widthOf = (bounds: PlotBounds): number => Math.max(1, bounds.right - bounds.left);
const heightOf = (bounds: PlotBounds): number => Math.max(1, bounds.bottom - bounds.top);
const frequencyLimit = (sampleRate: number): number => Math.max(hzMin + 1, Math.min(eqFrequencyLimit, sampleRate * 0.45));
const frequencyFraction = (hz: number, rate: number): number =>
  Math.log(clamp(hz, hzMin, frequencyLimit(rate)) / hzMin) / Math.log(frequencyLimit(rate) / hzMin);
const dbFraction = (db: number, floor: number, ceiling: number): number => (clamp(db, floor, ceiling) - floor) / (ceiling - floor);

export function plotBounds(width: number, height: number, mode?: 'eq' | 'dynamics', eq?: EqView): PlotBounds {
  const left = width < 460 ? 34 : 43;
  const top = mode === 'eq' ? 25 : 13;
  return { left, right: Math.max(left + 1, width - left), top, bottom: Math.max(top + 1, height - 25), eq };
}

export function frequencyX(hz: number, bounds: PlotBounds, rate: number): number {
  const { start = 0, end = 1 } = bounds.eq ?? {};
  return bounds.left + (frequencyFraction(hz, rate) - start) / (end - start) * widthOf(bounds);
}

export function xFrequency(x: number, bounds: PlotBounds, rate: number): number {
  const { start = 0, end = 1 } = bounds.eq ?? {};
  const fraction = clamp((x - bounds.left) / widthOf(bounds), 0, 1);
  return hzMin * (frequencyLimit(rate) / hzMin) ** (start + (end - start) * fraction);
}

export function gainY(gain: number, bounds: PlotBounds, limit = true): number {
  const range = bounds.eq?.gainRange ?? eqMax;
  const amount = limit ? clamp(gain, -range, range) : gain;
  return bounds.bottom - (amount + range) / (2 * range) * heightOf(bounds);
}

export function yGain(y: number, bounds: PlotBounds, limit = true): number {
  const range = bounds.eq?.gainRange ?? eqMax;
  const fraction = (bounds.bottom - y) / heightOf(bounds);
  return -range + (limit ? clamp(fraction, 0, 1) : fraction) * 2 * range;
}

export function spectrumY(db: number, bounds: PlotBounds): number {
  return bounds.bottom - dbFraction(db, fftFloor, 0) * heightOf(bounds);
}

export function moveEqView(view: EqView, anchor: number, scale: number, pan: number): EqView {
  const initial = view.end - view.start;
  const span = clamp(initial * scale, 1 / 16, 1);
  const start = clamp(view.start + (initial - span) * anchor + pan * span, 0, 1 - span);
  return { ...view, start, end: start + span };
}

function dynamicsBounds(bounds: PlotBounds): PlotBounds {
  const strip = clamp(heightOf(bounds) * 0.34, 78, 126);
  return { ...bounds, bottom: Math.max(bounds.top + 1, bounds.bottom - strip) };
}

export function dynamicsLevelFloor(mode: DynamicsMode): number {
  return mode === 'gate' ? -84 : levelMin;
}

function compressorExcess(input: number, threshold: number, knee: number): number {
  const over = input - threshold;
  if (knee > 0 && Math.abs(over) < knee / 2) return (over + knee / 2) ** 2 / (2 * knee);
  return Math.max(0, over);
}

/** Transfer function matching the native compressor for settled input. */
export function compressorOutput(input: number, threshold: number, ratio: number, knee: number): number {
  return input - (1 - 1 / ratio) * compressorExcess(input, threshold, knee);
}

export function compressionRatioAt(input: number, output: number, threshold: number, knee: number): number {
  const excess = compressorExcess(input, threshold, knee);
  if (!excess) return 1;
  return 1 / (1 - clamp((input - output) / excess, 0, 0.99));
}

/** The expander turns down signals below threshold up to its configured maximum. */
export function gateOutput(input: number, threshold: number, depth: number): number {
  return input - Math.min(depth, Math.max(0, threshold - input));
}

export function levelX(db: number, bounds: PlotBounds, floor = levelMin): number {
  return bounds.left + dbFraction(db, floor, 0) * widthOf(bounds);
}

export function xLevel(x: number, bounds: PlotBounds, floor = levelMin): number {
  return floor * (1 - clamp((x - bounds.left) / widthOf(bounds), 0, 1));
}

export function levelY(db: number, bounds: PlotBounds, floor = levelMin): number {
  const chart = dynamicsBounds(bounds);
  return chart.bottom - dbFraction(db, floor, 0) * heightOf(chart);
}

export function yLevel(y: number, bounds: PlotBounds, floor = levelMin): number {
  const chart = dynamicsBounds(bounds);
  return floor + clamp((chart.bottom - y) / heightOf(chart), 0, 1) * -floor;
}

// Mirror the Rust Audio EQ Cookbook sections. The graph moves during the
// gesture, and the independently measured traces arrive on the probe cadence.
function eqFilters(settings: Partial<AudioTreatment>, rate: number, channel: EqChannel, dynamicMix?: readonly number[]): Coefficients[] {
  const filters: Coefficients[] = [];
  const angle = (hz: number) => 2 * Math.PI * hz / rate;
  function symmetric(hz: number | undefined, quality: number, shape: 'low-cut' | 'high-cut' | 'notch') {
    if (!hz || hz < 0 || hz >= rate / 2) return;
    const omega = angle(hz);
    const cos = Math.cos(omega);
    const alpha = Math.sin(omega) / (2 * quality);
    if (shape === 'notch') {
      filters.push({ numerator: [1, -2 * cos, 1], denominator: [1 + alpha, -2 * cos, 1 - alpha] });
      return;
    }
    const sign = shape === 'low-cut' ? -1 : 1;
    const b = (1 - sign * cos) / 2;
    filters.push({ numerator: [b, sign * 2 * b, b], denominator: [1 + alpha, -2 * cos, 1 - alpha] });
  }
  function cutoff(hz: number, slope: number, shape: 'low-cut' | 'high-cut') {
    const sections = Math.floor(Math.max(12, slope) / 12);
    for (let section = 0; section < sections; section++) {
      symmetric(hz, 1 / (2 * Math.cos((2 * section + 1) * Math.PI / (4 * sections))), shape);
    }
  }
  function shelf(hz: number, db: number | undefined, high: boolean, mix = 1) {
    if (!db || hz >= rate / 2) return;
    const omega = angle(hz);
    const cos = Math.cos(omega);
    const amplitude = 10 ** (db / 40);
    const slope = Math.SQRT2 * Math.sqrt(amplitude) * Math.sin(omega);
    const plus = amplitude + 1;
    const minus = amplitude - 1;
    const shelfFilter: Coefficients = high ? {
      numerator: [amplitude * (plus + minus * cos + slope), -2 * amplitude * (minus + plus * cos), amplitude * (plus + minus * cos - slope)],
      denominator: [plus - minus * cos + slope, 2 * (minus - plus * cos), plus - minus * cos - slope],
    } : {
      numerator: [amplitude * (plus - minus * cos + slope), 2 * amplitude * (minus - plus * cos), amplitude * (plus - minus * cos - slope)],
      denominator: [plus + minus * cos + slope, -2 * (minus + plus * cos), plus + minus * cos - slope],
    };
    const { numerator, denominator } = shelfFilter;
    filters.push({ denominator, numerator: [
      blendLevel(denominator[0], numerator[0], mix), blendLevel(denominator[1], numerator[1], mix),
      blendLevel(denominator[2], numerator[2], mix),
    ] });
  }
  function bell(hz: number, db: number | undefined, quality: number, mix = 1) {
    if (!db || hz <= 0 || hz >= rate / 2) return;
    const omega = angle(hz);
    const amplitude = 10 ** (db / 40);
    const alpha = Math.sin(omega) / (2 * quality);
    const middle = -2 * Math.cos(omega);
    const dry = alpha / amplitude;
    const wet = blendLevel(dry, alpha * amplitude, mix);
    filters.push({ numerator: [1 + wet, middle, 1 - wet], denominator: [1 + dry, middle, 1 - dry] });
  }
  const bypassed = settings.eqBypassed ?? 0;
  symmetric(settings.dcHz, Math.SQRT1_2, 'low-cut');
  if (settings.rumble && !(bypassed & 1)) cutoff(settings.rumbleHz || 35, settings.rumbleSlope || 12, 'low-cut');
  const hum = settings.hum === 'hz50' ? 50 : settings.hum === 'hz60' ? 60 : 0;
  if (hum) for (let harmonic = 1; harmonic <= Math.max(settings.humHarmonics ?? 1, 1); harmonic++) {
    symmetric(hum * harmonic, 10, 'notch');
  }
  if (!(bypassed & 8)) symmetric(settings.notchHz, settings.notchQ || 15, 'notch');
  if (settings.highCutHz && !(bypassed & 32)) cutoff(settings.highCutHz, settings.highCutSlope || 12, 'high-cut');
  if (!(bypassed & 2)) shelf(settings.bassHz || 120, settings.bassDb, false);
  if (!(bypassed & 4)) bell(settings.presenceHz || 1500, settings.presenceDb, (settings.presenceQ || 80) / 100);
  if (!(bypassed & 16)) shelf(settings.trebleHz || 4000, settings.trebleDb, true);
  for (const [index, band] of (settings.eqBands ?? []).entries()) {
    if (band.bypassed || (band.channel && band.channel !== 'stereo' && band.channel !== channel)) continue;
    switch (band.kind) {
      case 'low': cutoff(band.hz, band.slope || 12, 'low-cut'); break;
      case 'high': cutoff(band.hz, band.slope || 12, 'high-cut'); break;
      case 'bass': shelf(band.hz, band.gainDb, false); break;
      case 'treble': shelf(band.hz, band.gainDb, true); break;
      case 'bell': bell(band.hz, band.gainDb, bellQuality(band.q, band.gainDb, band.adaptiveQ)); break;
      case 'notch': symmetric(band.hz, band.q / 100, 'notch'); break;
    }
    const mix = dynamicMix?.[index] ?? 0;
    if (!band.dynamic || mix <= 0) continue;
    const gain = -band.dynamic.amountDb;
    switch (band.kind) {
      case 'low': case 'bass': shelf(band.hz, gain, false, mix); break;
      case 'high': case 'treble': shelf(band.hz, gain, true, mix); break;
      case 'bell': bell(band.hz, gain, bellQuality(band.q, band.gainDb, band.adaptiveQ), mix); break;
      case 'notch': bell(band.hz, gain, band.q / 100, mix); break;
    }
  }
  return filters;
}

export function eqResponse(settings: Partial<AudioTreatment>, sampleRate: number, channel: EqChannel = 'stereo', dynamicMix?: readonly number[]): (hz: number) => number {
  const filters = eqFilters(settings, sampleRate, channel, dynamicMix);
  const gain = settings.gainDb ?? 0;
  return (hz) => {
    const omega = 2 * Math.PI * hz / sampleRate;
    const cos = Math.cos(omega);
    const sin = Math.sin(omega);
    const cos2 = 2 * cos * cos - 1;
    const sin2 = 2 * sin * cos;
    const magnitudeSquared = ([first, middle, last]: Coefficients['numerator']) =>
      (first + middle * cos + last * cos2) ** 2 + (middle * sin + last * sin2) ** 2;
    return filters.reduce((db, filter) => {
      const ratio = magnitudeSquared(filter.numerator) / Math.max(1e-24, magnitudeSquared(filter.denominator));
      return db + 10 * Math.log10(Math.max(1e-24, ratio));
    }, gain);
  };
}

function stroke(ctx: CanvasRenderingContext2D, x1: number, y1: number, x2: number, y2: number) {
  ctx.beginPath();
  ctx.moveTo(x1, y1);
  ctx.lineTo(x2, y2);
  ctx.stroke();
}

function formatHz(frequency: number): string {
  return frequency < 1000 ? String(Math.round(frequency)) : `${Number((frequency / 1000).toFixed(1))}k`;
}


function clipPlot(ctx: CanvasRenderingContext2D, bounds: PlotBounds) {
  ctx.beginPath();
  ctx.rect(bounds.left, bounds.top, widthOf(bounds), heightOf(bounds));
  ctx.clip();
}

type SpectrumPoint = { x: number; y: number };

function spectrumPath(points: SpectrumPoint[]) {
  // Older probes reuse FFT bins at the low end. Treat a run as one reading
  // on the log-frequency axis so interpolation does not draw a flat step.
  const anchors: SpectrumPoint[] = [points[0]];
  const last = points.length - 1;
  for (let start = 0; start <= last;) {
    let end = start;
    while (end < last && points[end + 1].y === points[start].y) end++;
    if (start > 0 && end < last) {
      anchors.push({ x: (points[start].x + points[end].x) / 2, y: points[start].y });
    }
    start = end + 1;
  }
  anchors.push(points[last]);

  // Keep the slope through rising or falling readings and round genuine
  // peaks and dips without overshooting either neighbouring value.
  const slopes = anchors.slice(1).map((right, index) => {
    const left = anchors[index];
    return (right.y - left.y) / (right.x - left.x);
  });
  const tangent = (index: number) => {
    const before = slopes[index - 1];
    const after = slopes[index];
    if (index === 0) return after;
    if (index === slopes.length) return before;
    return before * after > 0 ? 2 * before * after / (before + after) : 0;
  };
  const path = new Path2D();
  path.moveTo(anchors[0].x, anchors[0].y);
  for (let index = 0; index < slopes.length; index++) {
    const left = anchors[index];
    const right = anchors[index + 1];
    const span = (right.x - left.x) / 3;
    path.bezierCurveTo(left.x + span, left.y + tangent(index) * span,
      right.x - span, right.y - tangent(index + 1) * span, right.x, right.y);
  }
  return path;
}

function drawSpectrum(ctx: CanvasRenderingContext2D, bounds: PlotBounds, rate: number, probe: TreatmentProbe, palette: CanvasPalette, view: AnalyzerView) {
  const topHz = frequencyLimit(rate);
  const points = (levels: readonly number[]) => {
    const trace: SpectrumPoint[] = [];
    for (let index = 0; index < Math.min(probe.frequencies.length, levels.length); index++) {
      const hz = probe.frequencies[index];
      const level = levels[index];
      if (!Number.isFinite(hz) || hz < hzMin || hz > topHz || !Number.isFinite(level)) continue;
      const x = frequencyX(hz, bounds, rate);
      if (trace.length && x <= trace[trace.length - 1].x) continue;
      trace.push({ x, y: spectrumY(level, bounds) });
    }
    return trace;
  };

  const after = view === 'before' ? [] : points(probe.treated);
  if (after.length > 1) {
    const treated = spectrumPath(after);
    const area = new Path2D(treated);
    area.lineTo(after[after.length - 1].x, bounds.bottom);
    area.lineTo(after[0].x, bounds.bottom);
    area.closePath();
    const fill = ctx.createLinearGradient(0, bounds.top, 0, bounds.bottom);
    fill.addColorStop(0, palette.accent);
    fill.addColorStop(1, 'transparent');
    ctx.fillStyle = fill;
    ctx.globalAlpha = 0.20;
    ctx.fill(area);
    ctx.strokeStyle = palette.ink;
    ctx.globalAlpha = 0.46;
    ctx.stroke(treated);
  }
  const before = view === 'after' ? [] : points(probe.original);
  if (before.length > 1) {
    ctx.strokeStyle = palette.muted;
    ctx.globalAlpha = 0.55;
    if (view === 'both') ctx.setLineDash([3, 3]);
    ctx.stroke(spectrumPath(before));
    ctx.setLineDash([]);
  }
  ctx.globalAlpha = 1;
}

function drawEqGrid(ctx: CanvasRenderingContext2D, bounds: PlotBounds, rate: number, palette: CanvasPalette) {
  ctx.strokeStyle = palette.grid;
  ctx.fillStyle = palette.muted;
  ctx.textBaseline = 'middle';
  ctx.textAlign = 'right';
  const gainRange = bounds.eq?.gainRange ?? eqMax;
  const intervals = heightOf(bounds) < 160 ? 4 : 8;
  const gainTicks = Array.from({ length: intervals + 1 }, (_, i) => gainRange * (1 - 2 * i / intervals));
  for (const db of gainTicks) {
    const row = gainY(db, bounds);
    if (db === 0) {
      ctx.save();
      ctx.fillStyle = palette.accent;
      ctx.globalAlpha = 0.08;
      ctx.fillRect(bounds.left, row - 3, widthOf(bounds), 6);
      ctx.strokeStyle = palette.ink;
      ctx.globalAlpha = 0.36;
      stroke(ctx, bounds.left, row, bounds.right, row);
      ctx.restore();
    } else {
      ctx.globalAlpha = 0.48;
      stroke(ctx, bounds.left, row, bounds.right, row);
    }
    ctx.globalAlpha = 1;
    ctx.fillText(db > 0 ? `+${db}` : String(db), bounds.left - 7, row);
  }
  ctx.textAlign = 'left';
  for (const db of [0, -24, -48, -72, -96]) {
    const row = spectrumY(db, bounds);
    ctx.fillText(String(db), bounds.right + 7, row);
  }
  const lowHz = xFrequency(bounds.left, bounds, rate);
  const topHz = xFrequency(bounds.right, bounds, rate);
  const compact = widthOf(bounds) < 425;
  const zoomed = !!bounds.eq && bounds.eq.end - bounds.eq.start < 0.75;
  const ticks = zoomed ? frequencyRulerTicks(lowHz, topHz) : hzTicks;
  const frequencyLabels = zoomed ? ticks : compact ? [20, 100, 500, 2000, 10000] : labelledFrequencies;
  const bottom = bounds.bottom + 13;
  let lastLabel = -Infinity;
  ctx.textAlign = 'center';
  for (const hz of ticks) {
    if (hz < lowHz - 1e-6) continue;
    if (hz >= topHz) break;
    const x = frequencyX(hz, bounds, rate);
    const labelled = frequencyLabels.includes(hz);
    ctx.globalAlpha = labelled ? 0.39 : 0.18;
    stroke(ctx, x, bounds.top, x, bounds.bottom);
    if (labelled && x >= lastLabel + 37 && x <= bounds.right - 37) {
      ctx.globalAlpha = 1;
      ctx.fillText(formatHz(hz), x, bottom);
      lastLabel = x;
    }
  }
  ctx.globalAlpha = 0.39;
  stroke(ctx, bounds.right, bounds.top, bounds.right, bounds.bottom);
  ctx.globalAlpha = 1;
  ctx.textAlign = 'right';
  ctx.fillText(formatHz(topHz), bounds.right, bottom);
  ctx.textAlign = 'left';
  ctx.fillText('EQ dB', 4, bottom);
  ctx.fillText('dBFS', bounds.right + 7, bottom);
}

function frequencyRulerTicks(low: number, high: number) {
  const approximate = (high - low) / 8;
  const magnitude = 10 ** Math.floor(Math.log10(approximate));
  const step = ([1, 2, 5, 10].find(factor => factor * magnitude >= approximate) ?? 10) * magnitude;
  const ticks: number[] = [];
  for (let hz = Math.ceil(low / step) * step; hz < high; hz += step) ticks.push(hz);
  return ticks;
}

type EqPaths = { key: string; curve: Path2D; other?: Path2D };
const eqPaths = new WeakMap<CanvasRenderingContext2D, EqPaths>();

function cachedEqPaths(ctx: CanvasRenderingContext2D, bounds: PlotBounds, settings: AudioTreatment, rate: number, options: EqPlotOptions) {
  // FFT frames share the last response path until the controls or bounds move.
  const selectedBand = options.selectedBand;
  const split = settings.eqBands?.some(band => !band.bypassed && band.channel && band.channel !== 'stereo');
  const primary = split ? (selectedBand?.channel === 'right' ? 'right' : 'left') : 'stereo';
  const key = [
    bounds.left, bounds.top, bounds.right, bounds.bottom, bounds.eq?.start, bounds.eq?.end, bounds.eq?.gainRange, rate, settings.gainDb, settings.dcHz,
    settings.rumble, settings.rumbleHz, settings.rumbleSlope, settings.hum, settings.humHarmonics,
    settings.notchHz, settings.notchQ, settings.highCutHz, settings.highCutSlope, settings.bassHz,
    settings.bassDb, settings.presenceHz, settings.presenceDb, settings.presenceQ, settings.trebleHz,
    settings.trebleDb, settings.eqBypassed, ...(settings.eqBands ?? []).flatMap(band => [
      band.kind, band.hz, band.gainDb, band.q, Boolean(band.adaptiveQ), Boolean(band.bypassed), band.channel, band.slope, band.dynamic?.amountDb,
    ]), primary, ...(options.dynamicMix ?? []),
  ].join(',');
  const cached = eqPaths.get(ctx);
  if (cached?.key === key) return cached;

  const samples = Math.round(clamp(widthOf(bounds) / 2.5, 80, 460));
  // Include narrow notch centers explicitly so they cannot fall between samples.
  const xs = Array.from({ length: samples + 1 }, (_, index) => bounds.left + index / samples * widthOf(bounds));
  const hum = settings.hum === 'hz50' ? 50 : settings.hum === 'hz60' ? 60 : 0;
  if (hum) for (let harmonic = 1; harmonic <= Math.max(settings.humHarmonics, 1); harmonic++) {
    xs.push(frequencyX(hum * harmonic, bounds, rate));
  }
  if (settings.notchHz && !((settings.eqBypassed ?? 0) & 8)) xs.push(frequencyX(settings.notchHz, bounds, rate));
  for (const band of settings.eqBands ?? []) {
    if (!band.bypassed) xs.push(frequencyX(band.hz, bounds, rate));
  }
  const visible = xs.filter(x => x >= bounds.left && x <= bounds.right).sort((a, b) => a - b);
  const response = eqResponse(settings, rate, primary, options.dynamicMix);
  const curve = responsePath(visible, bounds, rate, response);
  const other = split ? responsePath(visible, bounds, rate, eqResponse(settings, rate, primary === 'left' ? 'right' : 'left', options.dynamicMix)) : undefined;
  const paths = { key, curve, other };
  eqPaths.set(ctx, paths);
  return paths;
}

function responsePath(xs: number[], bounds: PlotBounds, rate: number, response: (hz: number) => number, observeY?: (y: number) => void) {
  const curve = new Path2D();
  xs.forEach((x, index) => {
    const y = gainY(response(xFrequency(x, bounds, rate)), bounds, bounds.eq === undefined);
    observeY?.(clamp(y, bounds.top, bounds.bottom));
    if (index) curve.lineTo(x, y);
    else curve.moveTo(x, y);
  });
  return curve;
}

type BandDrawing = { line: Path2D; area: Path2D; wash: CanvasGradient; range?: Path2D };
const bandDrawings = new WeakMap<CanvasRenderingContext2D, Map<string, BandDrawing>>();

function eqBandDrawing(ctx: CanvasRenderingContext2D, bounds: PlotBounds, rate: number, band: EqBand, color: string,
  previous: Map<string, BandDrawing> | undefined, current: Map<string, BandDrawing>) {
  const key = JSON.stringify([bounds, rate, band, color]);
  const cached = previous?.get(key);
  if (cached) { current.set(key, cached); return cached; }
  const samples = Math.round(clamp(widthOf(bounds) / 3.5, 100, 240));
  const xs = Array.from({ length: samples + 1 }, (_, index) => bounds.left + index / samples * widthOf(bounds));
  const pivot = frequencyX(band.hz, bounds, rate);
  if (pivot > bounds.left && pivot < bounds.right) xs.push(pivot);
  xs.sort((a, b) => a - b);
  const response = (mix = 0) => eqResponse({
    eqBands: [{ ...band, channel: 'stereo', bypassed: false }],
  }, rate, 'stereo', [mix]);
  const zero = gainY(0, bounds);
  let top = zero - 1;
  let bottom = zero + 1;
  const base = response();
  const baseline = responsePath(xs, bounds, rate, base, y => {
    top = Math.min(top, y);
    bottom = Math.max(bottom, y);
  });
  const line = baseline;
  const range = band.dynamic && !band.bypassed ? new Path2D(baseline) : undefined;
  if (range) {
    const maxCut = response(1);
    for (let i = xs.length - 1; i >= 0; i--) range.lineTo(xs[i], gainY(maxCut(xFrequency(xs[i], bounds, rate)), bounds, bounds.eq === undefined));
    range.closePath();
  }
  const area = new Path2D(baseline);
  area.lineTo(bounds.right, zero);
  area.lineTo(bounds.left, zero);
  area.closePath();
  const wash = ctx.createLinearGradient(0, top, 0, bottom);
  const zeroStop = (zero - top) / (bottom - top);
  const fullColorInset = 0.15;
  wash.addColorStop(zeroStop * fullColorInset, color);
  wash.addColorStop(zeroStop, 'transparent');
  wash.addColorStop(1 - (1 - zeroStop) * fullColorInset, color);
  const drawing = { line, area, wash, range };
  current.set(key, drawing);
  return drawing;
}

function drawEq(ctx: CanvasRenderingContext2D, bounds: PlotBounds, settings: AudioTreatment, rate: number, probe: TreatmentProbe | null | undefined, palette: CanvasPalette, options: EqPlotOptions) {
  drawEqGrid(ctx, bounds, rate, palette);
  ctx.save();
  clipPlot(ctx, bounds);
  if (probe && options.analyzer !== 'off') drawSpectrum(ctx, bounds, rate, probe, palette, options.analyzer ?? 'both');

  const previous = bandDrawings.get(ctx);
  const current = new Map<string, BandDrawing>();
  for (const { band, color, selected } of options.bands ?? []) {
    if (band.bypassed && !selected) continue;
    if (band.kind === 'bell' && band.gainDb === 0 && !band.dynamic) continue;
    if ((band.kind === 'bass' || band.kind === 'treble') && band.gainDb === 0 && !band.dynamic) continue;
    const { line, area, wash, range } = eqBandDrawing(ctx, bounds, rate, band, color, previous, current);
    const visibility = options.bandSelection && !selected ? 0.6 : 1;
    if (!band.bypassed) {
      ctx.fillStyle = wash;
      ctx.globalAlpha = (selected ? 0.28 : 0.21) * visibility;
      ctx.fill(area);
      if (range) {
        ctx.fillStyle = color;
        ctx.globalAlpha = (selected ? 0.31 : 0.2) * visibility;
        ctx.fill(range);
      }
    }
    ctx.strokeStyle = color;
    ctx.globalAlpha = (band.bypassed ? 0.38 : selected ? 0.86 : 0.64) * visibility;
    ctx.lineWidth = selected ? 1.7 : 1.25;
    if (band.bypassed) ctx.setLineDash([3, 4]);
    ctx.stroke(line);
    ctx.setLineDash([]);
  }
  const { curve, other } = cachedEqPaths(ctx, bounds, settings, rate, options);
  if (other) {
    ctx.setLineDash([4, 3]);
    ctx.strokeStyle = palette.muted;
    ctx.globalAlpha = 0.61;
    ctx.lineWidth = 1.3;
    ctx.stroke(other);
    ctx.setLineDash([]);
  }
  ctx.globalAlpha = 1;
  ctx.lineWidth = 2.8;
  ctx.strokeStyle = palette.ink;
  ctx.stroke(curve);
  if (options.ghostBand) {
    const band = options.ghostBand;
    const { line, area, wash } = eqBandDrawing(ctx, bounds, rate, band, palette.ink, previous, current);
    const x = frequencyX(band.hz, bounds, rate);
    const span = Math.min(120, widthOf(bounds) * 0.2);
    const ink = ctx.createLinearGradient(x - span, 0, x + span, 0);
    ink.addColorStop(0, band.kind === 'low' || band.kind === 'bass' ? palette.ink : 'transparent');
    ink.addColorStop(0.5, palette.ink);
    ink.addColorStop(1, band.kind === 'high' || band.kind === 'treble' ? palette.ink : 'transparent');
    ctx.fillStyle = wash;
    ctx.globalAlpha = 0.18;
    ctx.fill(area);
    ctx.strokeStyle = ink;
    ctx.lineWidth = 3;
    ctx.globalAlpha = 0.7;
    ctx.stroke(line);
  }
  bandDrawings.set(ctx, current);

  ctx.restore();
}

function meterText(db: number | undefined, unit: 'dBFS' | 'dB' | 'LUFS', precision = 0) {
  if (db === undefined || !Number.isFinite(db)) return '—';
  const floor = unit === 'dBFS' && db <= fftFloor ? '≤ ' : '';
  const sign = db < 0 && Math.abs(db) >= 0.5 * 10 ** -precision ? '−' : '';
  return floor + sign + Math.abs(db).toFixed(precision) + ' ' + unit;
}

export function describeDynamicsMeters(probe?: TreatmentProbe | null, mode: DynamicsPlotOptions['mode'] = 'compressor') {
  if (!probe || !Number.isFinite(probe.input) || !Number.isFinite(probe.output)) return undefined;
  const level = (db: number) => db <= fftFloor ? 'at or below −96 dBFS' : meterText(db, 'dBFS', 1);
  const reduction = mode === 'gate' ? probe.gateReduction?.at(-1) : probe.reduction.at(-1);
  const levels = 'Input ' + level(probe.input) + ', output ' + level(probe.output) + '.';
  if (mode !== 'compressor' && mode !== 'gate') return levels;
  return Number.isFinite(reduction) ? levels + ' Gain reduction ' + meterText(reduction, 'dB', 1) + '.' : levels;
}

function drawMeterLabel(ctx: CanvasRenderingContext2D, left: number, right: number, y: number, label: string, readout: string, palette: CanvasPalette) {
  ctx.textBaseline = 'middle';
  ctx.fillStyle = palette.muted;
  ctx.textAlign = 'left';
  ctx.fillText(label, left, y);
  ctx.textAlign = 'right';
  ctx.fillText(readout, right, y);
}

function drawLevelMeter(ctx: CanvasRenderingContext2D, left: number, right: number, y: number, label: string, db: number | undefined, color: string, palette: CanvasPalette, peak?: number, unit: 'dBFS' | 'LUFS' = 'dBFS') {
  drawMeterLabel(ctx, left, right, y, label, meterText(db, unit, 1), palette);
  const bar = y + 13;
  const floor = unit === 'LUFS' ? -42 : levelMin;
  ctx.lineWidth = 6;
  ctx.strokeStyle = palette.grid;
  ctx.globalAlpha = 0.6;
  stroke(ctx, left + 3, bar, right - 3, bar);
  if (db !== undefined && Number.isFinite(db)) {
    const end = left + 3 + (right - left - 6) * dbFraction(db, floor, 0);
    if (end > left + 3) {
      const gradient = ctx.createLinearGradient(left, 0, right, 0);
      gradient.addColorStop(0, 'transparent');
      gradient.addColorStop(0.36, color);
      gradient.addColorStop(1, color);
      ctx.strokeStyle = gradient;
      ctx.globalAlpha = label === 'Input' ? 0.87 : 1;
      stroke(ctx, left + 3, bar, end, bar);
    }
  }
  ctx.globalAlpha = 1;
  ctx.lineWidth = 1;
  if (peak !== undefined && Number.isFinite(peak)) {
    const position = left + 3 + (right - left - 6) * dbFraction(peak, floor, 0);
    ctx.strokeStyle = palette.ink;
    stroke(ctx, position, bar - 5, position, bar + 5);
  }
}

function drawDynamicsMeters(ctx: CanvasRenderingContext2D, bounds: PlotBounds, chart: PlotBounds, probe: TreatmentProbe | null | undefined, palette: CanvasPalette, options: DynamicsPlotOptions) {
  const top = chart.bottom + 29;
  const split = bounds.left + widthOf(bounds) * 0.46;
  const meterEnd = split - 12;
  const historyStart = split + 12;
  const reduction = (options.mode === 'gate' ? probe?.gateReduction : probe?.reduction) ?? [];
  drawLevelMeter(ctx, bounds.left, meterEnd, top, 'Input', probe?.input, palette.muted, palette, options.inputPeakHold);
  drawLevelMeter(ctx, bounds.left, meterEnd, top + 27, 'Output', probe?.output, palette.ink, palette, options.outputPeakHold);
  drawMeterLabel(ctx, historyStart, bounds.right, top, 'Gain reduction', meterText(reduction.at(-1), 'dB'), palette);

  const traceTop = top + 10;
  const traceBottom = Math.max(traceTop + 1, bounds.bottom + 2);
  ctx.strokeStyle = palette.grid;
  ctx.globalAlpha = 0.65;
  stroke(ctx, historyStart, traceTop, bounds.right, traceTop);
  if (reduction.length > 1) {
    const trace = new Path2D();
    reduction.forEach((db, index) => {
      const x = historyStart + index / (reduction.length - 1) * (bounds.right - historyStart);
      const y = traceTop + dbFraction(-db, 0, 24) * (traceBottom - traceTop - 1);
      if (index) trace.lineTo(x, y);
      else trace.moveTo(x, y);
    });
    const fill = new Path2D(trace);
    fill.lineTo(bounds.right, traceTop);
    fill.lineTo(historyStart, traceTop);
    fill.closePath();
    const gradient = ctx.createLinearGradient(0, traceTop, 0, traceBottom);
    gradient.addColorStop(0, 'transparent');
    gradient.addColorStop(1, palette.accent);
    ctx.fillStyle = gradient;
    ctx.globalAlpha = 0.46;
    ctx.fill(fill);
    ctx.strokeStyle = palette.ink;
    ctx.globalAlpha = 0.82;
    ctx.lineWidth = 1.5;
    ctx.stroke(trace);
  }
  ctx.globalAlpha = 1;
  ctx.lineWidth = 1;
}

function drawDynamicsLevels(ctx: CanvasRenderingContext2D, bounds: PlotBounds, settings: AudioTreatment, probe: TreatmentProbe | null | undefined, palette: CanvasPalette, options: DynamicsPlotOptions) {
  const start = bounds.top + Math.max(17, (heightOf(bounds) - 126) / 2);
  const gap = Math.min(52, heightOf(bounds) / 4);
  const { left, right } = bounds;
  if (options.mode === 'normalise') {
    const measured = options.normalisation?.loudnessLufs ?? undefined;
    const target = settings.normalizeTargetDb || -18;
    const projected = measured !== undefined && options.normalisation ? measured + options.normalisation.gainDb : undefined;
    drawLevelMeter(ctx, left, right, start, 'Whole-set loudness', measured, palette.muted, palette, undefined, 'LUFS');
    drawLevelMeter(ctx, left, right, start + gap, 'Target', target, palette.accent, palette, undefined, 'LUFS');
    drawLevelMeter(ctx, left, right, start + 2 * gap, 'Estimated output loudness', projected, palette.ink, palette, undefined, 'LUFS');
    return;
  }
  const outputPeak = probe?.outputPeak;
  drawLevelMeter(ctx, left, right, start, 'Input', probe?.input, palette.muted, palette, options.inputPeakHold);
  drawLevelMeter(ctx, left, right, start + gap, 'Output', probe?.output, palette.ink, palette, options.outputPeakHold);
  drawMeterLabel(ctx, left, right, start + 2 * gap, 'Output headroom', meterText(outputPeak === undefined ? undefined : Math.max(0, -outputPeak), 'dB', 1), palette);
}

function drawDynamics(ctx: CanvasRenderingContext2D, bounds: PlotBounds, settings: AudioTreatment, probe: TreatmentProbe | null | undefined, palette: CanvasPalette, options: DynamicsPlotOptions) {
  if (options.mode === 'gain' || options.mode === 'normalise') {
    drawDynamicsLevels(ctx, bounds, settings, probe, palette, options);
    return;
  }
  const chart = dynamicsBounds(bounds);
  const gate = options.mode === 'gate';
  const floor = dynamicsLevelFloor(options.mode);
  const active = gate ? Boolean(settings.gateDb) : Boolean(settings.compressorDb);
  const threshold = gate ? settings.gateDb || -48 : settings.compressorDb || -18;
  const ratio = settings.compressorRatio || 2;
  const knee = settings.compressorKneeDb || 0;
  const depth = settings.gateReductionDb || 12;
  const output = (input: number) => !active ? input : gate
    ? gateOutput(input, threshold, depth) : compressorOutput(input, threshold, ratio, knee);
  ctx.strokeStyle = palette.grid;
  ctx.fillStyle = palette.muted;
  ctx.textBaseline = 'middle';
  const ticks = heightOf(chart) < 150 ? [floor, floor * 2 / 3, floor / 3, 0] : [floor, -48, -36, -24, -12, 0];
  for (const level of ticks) {
    const x = levelX(level, bounds, floor);
    const y = levelY(level, bounds, floor);
    ctx.globalAlpha = 0.49;
    stroke(ctx, x, chart.top, x, chart.bottom);
    stroke(ctx, chart.left, y, chart.right, y);
    ctx.globalAlpha = 1;
    ctx.textAlign = 'center';
    ctx.fillText(String(level), x, chart.bottom + 12);
    ctx.textAlign = 'right';
    ctx.fillText(String(level), chart.left - 7, y);
  }
  ctx.textAlign = 'right';
  ctx.fillText('dBFS', bounds.right + 30, chart.bottom + 12);

  const thresholdX = levelX(threshold, bounds, floor);
  const detector = gate ? undefined : probe?.detector;
  const triggered = gate ? (probe?.gateReduction?.at(-1) ?? 0) < 0 : (detector ?? floor) >= threshold;
  ctx.globalAlpha = 0.52;
  ctx.setLineDash([4, 4]);
  ctx.strokeStyle = palette.muted;
  stroke(ctx, chart.left, chart.bottom, chart.right, chart.top);
  ctx.strokeStyle = active && triggered ? palette.ink : palette.muted;
  stroke(ctx, thresholdX, chart.top, thresholdX, chart.bottom);
  ctx.setLineDash([]);
  const curve = new Path2D();
  const transition = gate ? [threshold - depth, threshold] : [threshold - knee / 2, threshold, threshold + knee / 2];
  const columns = 30;
  const inputs = [...Array.from({ length: columns + 1 }, (_, index) => floor * (1 - index / columns)),
    ...transition.filter(level => level > floor && level < 0)].sort((a, b) => a - b);
  for (const [index, input] of inputs.entries()) {
    const x = levelX(input, bounds, floor);
    const y = levelY(output(input), bounds, floor);
    if (index) curve.lineTo(x, y);
    else curve.moveTo(x, y);
  }
  if (active) {
    const gradient = ctx.createLinearGradient(0, chart.top, 0, chart.bottom);
    gradient.addColorStop(0, palette.accent);
    gradient.addColorStop(1, 'transparent');
    ctx.globalAlpha = 0.28;
    ctx.fillStyle = gradient;
    const fill = new Path2D(curve);
    fill.lineTo(chart.right, chart.top);
    fill.lineTo(chart.left, chart.bottom);
    fill.closePath();
    ctx.fill(fill);
  }
  ctx.globalAlpha = 1;
  ctx.lineWidth = 2;
  ctx.strokeStyle = palette.ink;
  ctx.stroke(curve);
  ctx.lineWidth = 1;
  if (detector !== undefined && Number.isFinite(detector) && detector > floor) {
    const x = levelX(detector, bounds, floor);
    const y = levelY(output(detector), bounds, floor);
    ctx.globalAlpha = 0.47;
    ctx.strokeStyle = palette.ink;
    stroke(ctx, x, chart.bottom, x, y);
    stroke(ctx, chart.left, y, x, y);
    ctx.globalAlpha = 1;
    ctx.fillStyle = palette.ink;
    ctx.beginPath();
    ctx.arc(x, y, 3.5, 0, Math.PI * 2);
    ctx.fill();
  }

  drawDynamicsMeters(ctx, bounds, chart, probe, palette, options);
}

export function drawTreatmentPlot(
  canvas: HTMLCanvasElement,
  width: number,
  height: number,
  mode: Exclude<TreatmentPlotMode, 'repair'>,
  settings: AudioTreatment,
  sampleRate: number,
  probe?: TreatmentProbe | null,
  dynamics: DynamicsPlotOptions = { mode: 'compressor' },
  eq: EqPlotOptions = {},
): void {
  if (width <= 0 || height <= 0 || sampleRate <= 0) return;
  const ctx = canvas.getContext('2d');
  if (!ctx) return;
  const scale = Math.min(3, window.devicePixelRatio || 1);
  const pixelsWide = Math.round(width * scale);
  const pixelsHigh = Math.round(height * scale);
  if (canvas.width !== pixelsWide) canvas.width = pixelsWide;
  if (canvas.height !== pixelsHigh) canvas.height = pixelsHigh;
  ctx.setTransform(scale, 0, 0, scale, 0, 0);
  ctx.clearRect(0, 0, width, height);
  ctx.lineWidth = 1;
  ctx.lineJoin = 'round';
  ctx.lineCap = 'round';
  const styles = getComputedStyle(canvas);
  const color = (variable: string, fallback: string) => styles.getPropertyValue(variable).trim() || fallback;
  const palette = {
    muted: color('--muted', '#aeb7d0'),
    grid: color('--border', '#3d435b'),
    accent: color('--accent', '#86a7ff'),
    ink: color('--accent-ink', '#a9c0ff'),
  };
  ctx.font = `10px ${color('--font-literal', 'monospace')}`;
  const bounds = plotBounds(width, height, mode, eq.view);
  if (mode === 'eq') drawEq(ctx, bounds, settings, sampleRate, probe, palette, eq);
  else drawDynamics(ctx, bounds, settings, probe, palette, dynamics);
}
