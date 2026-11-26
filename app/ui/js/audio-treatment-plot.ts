import type { AudioTreatment } from './audio-treatment';

export type TreatmentPlotMode = 'eq' | 'dynamics' | 'repair';
export type PlotBounds = { left: number; right: number; top: number; bottom: number };
export type TreatmentProbe = {
  frequencies: readonly number[];
  original: readonly number[];
  treated: readonly number[];
  reduction: readonly number[];
  input: number;
  output: number;
  spectrogram?: readonly (readonly number[])[];
  spectrogramStart?: number;
  spectrogramEnd?: number;
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
    if (from.input === to.input && from.output === to.output &&
        from.reduction.length === to.reduction.length &&
        to.reduction.every((db, index) => db === from.reduction[index])) return;
    const reduction = new Array<number>(to.reduction.length);
    const view = { ...to, reduction };
    return progress => {
      const weight = softly(progress);
      view.input = blendLevel(from.input, to.input, weight);
      view.output = blendLevel(from.output, to.output, weight);
      for (let index = 0; index < reduction.length; index++) {
        reduction[index] = blendLevel(from.reduction[index] ?? to.reduction[index], to.reduction[index], weight);
      }
      return view;
    };
  });
}

export function createSpectrumMotion() {
  return createProbeMotion(105, (from, to) => {
    const bins = to.frequencies.length;
    if (bins === 0 || from.frequencies.length !== bins || from.original.length !== bins || from.treated.length !== bins ||
        to.original.length !== bins || to.treated.length !== bins ||
        to.frequencies.some((hz, index) => hz !== from.frequencies[index])) return;
    if (to.original.every((db, index) => db === from.original[index] && to.treated[index] === from.treated[index])) return;
    const original = new Array<number>(bins);
    const treated = new Array<number>(bins);
    const view = { ...to, original, treated };
    return progress => {
      const weight = 1 - (1 - progress) ** 2;
      for (let index = 0; index < bins; index++) {
        original[index] = blendLevel(from.original[index], to.original[index], weight);
        treated[index] = blendLevel(from.treated[index], to.treated[index], weight);
      }
      return view;
    };
  });
}

type Coefficients = { numerator: readonly [number, number, number]; denominator: readonly [number, number, number] };
type CanvasPalette = { muted: string; grid: string; accent: string; ink: string };
const hzMin = 20;
const eqMin = -24;
const eqMax = 12;
const levelMin = -60;
const fftFloor = -96;
const hzTicks = [20, 30, 50, 80, 100, 150, 200, 300, 500, 800, 1000, 1500, 2000, 3000, 5000, 8000, 10000, 15000, 20000];
const labelledFrequencies = [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000];
const clamp = (value: number, minimum: number, maximum: number): number => Math.max(minimum, Math.min(maximum, value));
const widthOf = (bounds: PlotBounds): number => Math.max(1, bounds.right - bounds.left);
const heightOf = (bounds: PlotBounds): number => Math.max(1, bounds.bottom - bounds.top);
const frequencyLimit = (sampleRate: number): number => Math.max(hzMin + 1, Math.min(20_000, sampleRate * 0.45));
const frequencyFraction = (hz: number, rate: number): number =>
  Math.log(clamp(hz, hzMin, frequencyLimit(rate)) / hzMin) / Math.log(frequencyLimit(rate) / hzMin);
const dbFraction = (db: number, floor: number, ceiling: number): number => (clamp(db, floor, ceiling) - floor) / (ceiling - floor);

export function plotBounds(width: number, height: number): PlotBounds {
  const left = width < 460 ? 34 : 43;
  return { left, right: Math.max(left + 1, width - left), top: 13, bottom: Math.max(14, height - 25) };
}

export function frequencyX(hz: number, bounds: PlotBounds, rate: number): number {
  return bounds.left + frequencyFraction(hz, rate) * widthOf(bounds);
}

export function xFrequency(x: number, bounds: PlotBounds, rate: number): number {
  return hzMin * (frequencyLimit(rate) / hzMin) ** clamp((x - bounds.left) / widthOf(bounds), 0, 1);
}

export function gainY(gain: number, bounds: PlotBounds): number {
  return bounds.bottom - dbFraction(gain, eqMin, eqMax) * heightOf(bounds);
}

export function yGain(y: number, bounds: PlotBounds): number {
  return eqMin + clamp((bounds.bottom - y) / heightOf(bounds), 0, 1) * (eqMax - eqMin);
}

function dynamicsBounds(bounds: PlotBounds): PlotBounds {
  const strip = clamp(heightOf(bounds) * 0.34, 78, 126);
  return { ...bounds, bottom: Math.max(bounds.top + 1, bounds.bottom - strip) };
}

export function levelX(db: number, bounds: PlotBounds): number {
  return bounds.left + dbFraction(db, levelMin, 0) * widthOf(bounds);
}

export function xLevel(x: number, bounds: PlotBounds): number {
  return levelMin * (1 - clamp((x - bounds.left) / widthOf(bounds), 0, 1));
}

export function levelY(db: number, bounds: PlotBounds): number {
  const chart = dynamicsBounds(bounds);
  return chart.bottom - dbFraction(db, levelMin, 0) * heightOf(chart);
}

export function yLevel(y: number, bounds: PlotBounds): number {
  const chart = dynamicsBounds(bounds);
  return levelMin + clamp((chart.bottom - y) / heightOf(chart), 0, 1) * -levelMin;
}

// Mirror the Rust Audio EQ Cookbook sections. The graph moves during the
// gesture, and the independently measured traces arrive on the probe cadence.
function eqFilters(settings: AudioTreatment, rate: number): Coefficients[] {
  const filters: Coefficients[] = [];
  const angle = (hz: number) => 2 * Math.PI * hz / rate;
  function symmetric(hz: number, quality: number, shape: 'low-cut' | 'high-cut' | 'notch') {
    if (hz <= 0 || hz >= rate / 2) return;
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
  function shelf(hz: number, db: number, high: boolean) {
    if (!db || hz >= rate / 2) return;
    const omega = angle(hz);
    const cos = Math.cos(omega);
    const amplitude = 10 ** (db / 40);
    const slope = Math.SQRT2 * Math.sqrt(amplitude) * Math.sin(omega);
    const plus = amplitude + 1;
    const minus = amplitude - 1;
    if (high) {
      filters.push({
        numerator: [amplitude * (plus + minus * cos + slope), -2 * amplitude * (minus + plus * cos), amplitude * (plus + minus * cos - slope)],
        denominator: [plus - minus * cos + slope, 2 * (minus - plus * cos), plus - minus * cos - slope],
      });
    } else {
      filters.push({
        numerator: [amplitude * (plus - minus * cos + slope), 2 * amplitude * (minus - plus * cos), amplitude * (plus - minus * cos - slope)],
        denominator: [plus + minus * cos + slope, -2 * (minus + plus * cos), plus + minus * cos - slope],
      });
    }
  }
  symmetric(settings.dcHz, Math.SQRT1_2, 'low-cut');
  if (settings.rumble) cutoff(settings.rumbleHz || 35, settings.rumbleSlope || 12, 'low-cut');
  const hum = settings.hum === 'hz50' ? 50 : settings.hum === 'hz60' ? 60 : 0;
  if (hum) for (let harmonic = 1; harmonic <= Math.max(settings.humHarmonics, 1); harmonic++) {
    symmetric(hum * harmonic, 10, 'notch');
  }
  symmetric(settings.notchHz, settings.notchQ || 15, 'notch');
  if (settings.highCutHz) cutoff(settings.highCutHz, settings.highCutSlope || 12, 'high-cut');
  shelf(settings.bassHz || 120, settings.bassDb, false);
  const presenceHz = settings.presenceHz || 1500;
  if (settings.presenceDb && presenceHz < rate / 2) {
    const omega = angle(presenceHz);
    const amplitude = 10 ** (settings.presenceDb / 40);
    const alpha = Math.sin(omega) / (2 * ((settings.presenceQ || 80) / 100));
    const middle = -2 * Math.cos(omega);
    filters.push({
      numerator: [1 + alpha * amplitude, middle, 1 - alpha * amplitude],
      denominator: [1 + alpha / amplitude, middle, 1 - alpha / amplitude],
    });
  }
  shelf(settings.trebleHz || 4000, settings.trebleDb, true);
  return filters;
}

export function eqResponse(settings: AudioTreatment, sampleRate: number): (hz: number) => number {
  const filters = eqFilters(settings, sampleRate);
  return (hz) => {
    const omega = 2 * Math.PI * hz / sampleRate;
    const cos = Math.cos(omega);
    const sin = Math.sin(omega);
    const cos2 = 2 * cos * cos - 1;
    const sin2 = 2 * sin * cos;
    const magnitudeSquared = ([first, middle, last]: Coefficients['numerator']) =>
      (first + middle * cos + last * cos2) ** 2 + (middle * sin + last * sin2) ** 2;
    return settings.gainDb + filters.reduce((db, filter) => {
      const ratio = magnitudeSquared(filter.numerator) / Math.max(1e-24, magnitudeSquared(filter.denominator));
      return db + 10 * Math.log10(Math.max(1e-24, ratio));
    }, 0);
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

function drawSpectrum(ctx: CanvasRenderingContext2D, bounds: PlotBounds, rate: number, probe: TreatmentProbe, palette: CanvasPalette) {
  const original = new Path2D();
  const treated = new Path2D();
  const topHz = frequencyLimit(rate);
  const count = Math.min(probe.frequencies.length, probe.original.length, probe.treated.length);
  const y = (level: number) => bounds.bottom - dbFraction(level, fftFloor, 0) * heightOf(bounds);
  let first: number | undefined;
  let last = bounds.left;
  for (let index = 0; index < count; index++) {
    const hz = probe.frequencies[index];
    const before = probe.original[index];
    const after = probe.treated[index];
    if (!Number.isFinite(hz) || hz < hzMin || hz > topHz || !Number.isFinite(before) || !Number.isFinite(after)) continue;
    const x = frequencyX(hz, bounds, rate);
    if (first === undefined) {
      first = x;
      original.moveTo(x, y(before));
      treated.moveTo(x, y(after));
    } else {
      original.lineTo(x, y(before));
      treated.lineTo(x, y(after));
    }
    last = x;
  }
  if (first === undefined || last === first) return;

  const area = new Path2D(treated);
  area.lineTo(last, bounds.bottom);
  area.lineTo(first, bounds.bottom);
  area.closePath();
  ctx.fillStyle = palette.accent;
  ctx.globalAlpha = 0.11;
  ctx.fill(area);
  ctx.strokeStyle = palette.ink;
  ctx.globalAlpha = 0.63;
  ctx.stroke(treated);
  ctx.strokeStyle = palette.muted;
  ctx.globalAlpha = 0.74;
  ctx.setLineDash([3, 3]);
  ctx.stroke(original);
  ctx.setLineDash([]);
  ctx.globalAlpha = 1;
}

function drawEqGrid(ctx: CanvasRenderingContext2D, bounds: PlotBounds, rate: number, palette: CanvasPalette) {
  ctx.strokeStyle = palette.grid;
  ctx.fillStyle = palette.muted;
  ctx.textBaseline = 'middle';
  ctx.textAlign = 'right';
  const gainTicks = heightOf(bounds) < 160 ? [12, 0, -12, -24] : [12, 6, 0, -6, -12, -18, -24];
  for (const db of gainTicks) {
    const row = gainY(db, bounds);
    ctx.globalAlpha = db === 0 ? 0.92 : 0.48;
    stroke(ctx, bounds.left, row, bounds.right, row);
    ctx.globalAlpha = 1;
    ctx.fillText(db > 0 ? `+${db}` : String(db), bounds.left - 7, row);
  }
  ctx.textAlign = 'left';
  for (const db of [0, -24, -48, -72, -96]) {
    const row = bounds.bottom - dbFraction(db, fftFloor, 0) * heightOf(bounds);
    ctx.fillText(String(db), bounds.right + 7, row);
  }
  const topHz = frequencyLimit(rate);
  const compact = widthOf(bounds) < 425;
  const frequencyLabels = compact ? [20, 100, 500, 2000, 10000] : labelledFrequencies;
  const bottom = bounds.bottom + 13;
  ctx.textAlign = 'center';
  for (const hz of hzTicks) {
    if (hz >= topHz) break;
    const x = frequencyX(hz, bounds, rate);
    const labelled = frequencyLabels.includes(hz);
    ctx.globalAlpha = labelled ? 0.39 : 0.18;
    stroke(ctx, x, bounds.top, x, bounds.bottom);
    if (labelled && x <= bounds.right - 37) {
      ctx.globalAlpha = 1;
      ctx.fillText(formatHz(hz), x, bottom);
    }
  }
  ctx.globalAlpha = 0.39;
  stroke(ctx, bounds.right, bounds.top, bounds.right, bounds.bottom);
  ctx.globalAlpha = 1;
  ctx.textAlign = 'right';
  ctx.fillText(formatHz(topHz), bounds.right, bottom);
  ctx.textAlign = 'left';
  ctx.fillText('dB', 6, bottom);
  ctx.fillText('dBFS', bounds.right + 7, bottom);
}

type EqPaths = { key: string; curve: Path2D; fill: Path2D };
const eqPaths = new WeakMap<CanvasRenderingContext2D, EqPaths>();

function cachedEqPaths(ctx: CanvasRenderingContext2D, bounds: PlotBounds, settings: AudioTreatment, rate: number) {
  // FFT frames share the last response path until the controls or bounds move.
  const key = [bounds.left, bounds.top, bounds.right, bounds.bottom, rate, ...Object.values(settings)].join(',');
  const cached = eqPaths.get(ctx);
  if (cached?.key === key) return cached;

  const response = eqResponse(settings, rate);
  const samples = Math.round(clamp(widthOf(bounds) / 2.5, 80, 460));
  // Include narrow notch centers explicitly so they cannot fall between samples.
  const xs = Array.from({ length: samples + 1 }, (_, index) => bounds.left + index / samples * widthOf(bounds));
  const hum = settings.hum === 'hz50' ? 50 : settings.hum === 'hz60' ? 60 : 0;
  if (hum) for (let harmonic = 1; harmonic <= Math.max(settings.humHarmonics, 1); harmonic++) {
    xs.push(frequencyX(hum * harmonic, bounds, rate));
  }
  if (settings.notchHz) xs.push(frequencyX(settings.notchHz, bounds, rate));
  xs.sort((a, b) => a - b);
  const curve = new Path2D();
  xs.forEach((x, index) => {
    const y = gainY(response(xFrequency(x, bounds, rate)), bounds);
    if (index) curve.lineTo(x, y);
    else curve.moveTo(x, y);
  });
  const fill = new Path2D(curve);
  const zero = gainY(0, bounds);
  fill.lineTo(bounds.right, zero);
  fill.lineTo(bounds.left, zero);
  fill.closePath();
  const paths = { key, curve, fill };
  eqPaths.set(ctx, paths);
  return paths;
}

function drawEq(ctx: CanvasRenderingContext2D, bounds: PlotBounds, settings: AudioTreatment, rate: number, probe: TreatmentProbe | null | undefined, palette: CanvasPalette) {
  drawEqGrid(ctx, bounds, rate, palette);
  ctx.save();
  clipPlot(ctx, bounds);
  if (probe) drawSpectrum(ctx, bounds, rate, probe, palette);

  const { curve, fill } = cachedEqPaths(ctx, bounds, settings, rate);
  const zero = (gainY(0, bounds) - bounds.top) / heightOf(bounds);
  const gradient = ctx.createLinearGradient(0, bounds.top, 0, bounds.bottom);
  gradient.addColorStop(Math.max(0, zero - 0.12), palette.accent);
  gradient.addColorStop(zero, 'transparent');
  gradient.addColorStop(Math.min(1, zero + 0.21), palette.accent);
  ctx.globalAlpha = 0.27;
  ctx.fillStyle = gradient;
  ctx.fill(fill);
  ctx.globalAlpha = 1;
  ctx.lineWidth = 2.25;
  ctx.strokeStyle = palette.ink;
  ctx.stroke(curve);
  ctx.restore();
}

function meterText(db: number | undefined, unit: 'dBFS' | 'dB', precision = 0) {
  if (db === undefined || !Number.isFinite(db)) return '—';
  const floor = unit === 'dBFS' && db <= fftFloor ? '≤ ' : '';
  const sign = db < 0 && Math.abs(db) >= 0.5 * 10 ** -precision ? '−' : '';
  return floor + sign + Math.abs(db).toFixed(precision) + ' ' + unit;
}

export function describeDynamicsMeters(probe?: TreatmentProbe | null) {
  if (!probe || !Number.isFinite(probe.input) || !Number.isFinite(probe.output)) return undefined;
  const level = (db: number) => db <= fftFloor ? 'at or below −96 dBFS' : meterText(db, 'dBFS', 1);
  const reduction = probe.reduction.at(-1);
  const levels = 'Input ' + level(probe.input) + ', output ' + level(probe.output) + '.';
  return Number.isFinite(reduction) ? levels + ' Compressor gain reduction ' + meterText(reduction, 'dB', 1) + '.' : levels;
}

function drawMeterLabel(ctx: CanvasRenderingContext2D, left: number, right: number, y: number, label: string, readout: string, palette: CanvasPalette) {
  ctx.textBaseline = 'middle';
  ctx.fillStyle = palette.muted;
  ctx.textAlign = 'left';
  ctx.fillText(label, left, y);
  ctx.textAlign = 'right';
  ctx.fillText(readout, right, y);
}

function drawLevelMeter(ctx: CanvasRenderingContext2D, left: number, right: number, y: number, label: string, db: number | undefined, color: string, palette: CanvasPalette) {
  drawMeterLabel(ctx, left, right, y, label, meterText(db, 'dBFS'), palette);
  const bar = y + 13;
  ctx.lineWidth = 6;
  ctx.strokeStyle = palette.grid;
  ctx.globalAlpha = 0.6;
  stroke(ctx, left + 3, bar, right - 3, bar);
  if (db !== undefined && Number.isFinite(db)) {
    const end = left + 3 + (right - left - 6) * dbFraction(db, levelMin, 0);
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
}

function drawDynamicsMeters(ctx: CanvasRenderingContext2D, bounds: PlotBounds, chart: PlotBounds, probe: TreatmentProbe | null | undefined, palette: CanvasPalette) {
  const top = chart.bottom + 29;
  const split = bounds.left + widthOf(bounds) * 0.46;
  const meterEnd = split - 12;
  const historyStart = split + 12;
  const last = probe?.reduction.at(-1);
  drawLevelMeter(ctx, bounds.left, meterEnd, top, 'Input', probe?.input, palette.muted, palette);
  drawLevelMeter(ctx, bounds.left, meterEnd, top + 27, 'Output', probe?.output, palette.ink, palette);
  drawMeterLabel(ctx, historyStart, bounds.right, top, 'Gain reduction', meterText(last, 'dB'), palette);

  const traceTop = top + 10;
  const traceBottom = Math.max(traceTop + 1, bounds.bottom + 2);
  ctx.strokeStyle = palette.grid;
  ctx.globalAlpha = 0.65;
  stroke(ctx, historyStart, traceTop, bounds.right, traceTop);
  const reduction = probe?.reduction ?? [];
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

function drawDynamics(ctx: CanvasRenderingContext2D, bounds: PlotBounds, settings: AudioTreatment, probe: TreatmentProbe | null | undefined, palette: CanvasPalette) {
  const chart = dynamicsBounds(bounds);
  const active = Boolean(settings.compressorDb);
  const threshold = settings.compressorDb || -18;
  const ratio = active ? settings.compressorRatio || 2 : 1;
  ctx.strokeStyle = palette.grid;
  ctx.fillStyle = palette.muted;
  ctx.textBaseline = 'middle';
  const ticks = heightOf(chart) < 150 ? [-60, -40, -20, 0] : [-60, -48, -36, -24, -12, 0];
  for (const level of ticks) {
    const x = levelX(level, bounds);
    const y = levelY(level, bounds);
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

  const unity = chart.top;
  ctx.globalAlpha = 0.55;
  ctx.setLineDash([4, 4]);
  ctx.strokeStyle = palette.muted;
  stroke(ctx, chart.left, chart.bottom, chart.right, unity);
  ctx.setLineDash([]);
  const pivotX = levelX(threshold, bounds);
  const pivotY = levelY(threshold, bounds);
  const maximumY = levelY(threshold - threshold / ratio, bounds);
  if (active) {
    const gradient = ctx.createLinearGradient(0, unity, 0, pivotY);
    gradient.addColorStop(0, palette.accent);
    gradient.addColorStop(1, 'transparent');
    ctx.globalAlpha = 0.28;
    ctx.fillStyle = gradient;
    ctx.beginPath();
    ctx.moveTo(pivotX, pivotY);
    ctx.lineTo(chart.right, unity);
    ctx.lineTo(chart.right, maximumY);
    ctx.closePath();
    ctx.fill();
  }
  ctx.globalAlpha = 1;
  ctx.lineWidth = 2;
  ctx.strokeStyle = palette.ink;
  ctx.beginPath();
  ctx.moveTo(chart.left, chart.bottom);
  ctx.lineTo(pivotX, pivotY);
  ctx.lineTo(chart.right, maximumY);
  ctx.stroke();
  ctx.lineWidth = 1;
  if (probe && Number.isFinite(probe.input) && Number.isFinite(probe.output) && probe.input > levelMin) {
    const x = levelX(probe.input, bounds);
    const y = levelY(probe.output, bounds);
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

  drawDynamicsMeters(ctx, bounds, chart, probe, palette);
}

export function drawTreatmentPlot(
  canvas: HTMLCanvasElement,
  width: number,
  height: number,
  mode: Exclude<TreatmentPlotMode, 'repair'>,
  settings: AudioTreatment,
  sampleRate: number,
  probe?: TreatmentProbe | null,
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
  const bounds = plotBounds(width, height);
  if (mode === 'eq') drawEq(ctx, bounds, settings, sampleRate, probe, palette);
  else drawDynamics(ctx, bounds, settings, probe, palette);
}
