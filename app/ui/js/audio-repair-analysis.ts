import type { TreatmentProbe } from './audio-treatment-plot';

export type RepairAnalysisBounds = { startFrame: number; endFrame: number; lowHz: number; highHz: number };
export type RepairAnalysisKind = 'tone' | 'harmonics' | 'hum' | 'similar' | 'clicks' | 'noise';

type SpectralTile = {
  frequencies: readonly number[];
  columns: readonly (readonly number[])[];
  start: number;
  step: number;
  windowFrames?: number;
  clicks?: TreatmentProbe['spectrogramClicks'];
};

export type RepairTarget = 'left' | 'right';
export type RepairSuggestion = { label: string; bounds: RepairAnalysisBounds; primary?: boolean; channel?: RepairTarget };
type Result = { candidates: RepairSuggestion[]; detail: string };

function getTile(probe: TreatmentProbe | null): SpectralTile | undefined {
  if (!probe || probe.spectrogramTreated || probe.spectrogramDifference || !probe.spectrogram?.length || probe.frequencies.length < 3) return;
  const start = probe.spectrogramStart;
  const end = probe.spectrogramEnd;
  if (start === undefined || end === undefined || start >= end) return;
  if (probe.spectrogram.some(column => column.length !== probe.frequencies.length)) return;
  return { frequencies: probe.frequencies, columns: probe.spectrogram, start, step: (end - start) / probe.spectrogram.length,
    windowFrames: probe.spectrogramWindowFrames, clicks: probe.spectrogramClicks };
}

function median(values: readonly number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  return (sorted[Math.floor(sorted.length / 2)] + sorted[Math.floor((sorted.length - 1) / 2)]) / 2;
}

function selectedColumns(tile: SpectralTile, bounds: RepairAnalysisBounds): readonly (readonly number[])[] {
  const first = Math.max(0, Math.floor((bounds.startFrame - tile.start) / tile.step));
  const last = Math.min(tile.columns.length, Math.ceil((bounds.endFrame - tile.start) / tile.step));
  return first < last ? tile.columns.slice(first, last) : [];
}

function indices(tile: SpectralTile, low: number, high: number): number[] {
  return tile.frequencies.flatMap((hz, index) => hz >= low && hz <= high ? [index] : []);
}

function steadyPeaks(tile: SpectralTile, columns: readonly (readonly number[])[]) {
  const central = tile.frequencies.map((_, i) => median(columns.map(column => column[i])));
  return tile.frequencies.flatMap((_, i) => {
    const hz = tile.frequencies[i];
    const shoulder = Math.max(15, hz * .12);
    const neighbors = indices(tile, hz - shoulder * 2, hz + shoulder * 2)
      .filter(j => Math.abs(tile.frequencies[j] - hz) >= shoulder);
    if (neighbors.length < 2 || central[i] < -72 || central[i] < (central[i - 1] ?? -100)
      || central[i] < (central[i + 1] ?? -100)) return [];
    // Multiple narrow display bands may represent the same FFT bin.
    if (central[i] === central[i - 1]) return [];
    const prominence = central[i] - median(neighbors.map(j => central[j]));
    const stable = columns.filter(column => column[i] >= central[i] - 7).length / columns.length;
    return prominence >= 9 && stable >= .75 ? [{ hz: Math.round(hz), prominence }] : [];
  });
}

function ridge(peaks: ReturnType<typeof steadyPeaks>, center: number, tolerance: number) {
  return peaks.filter(peak => Math.abs(peak.hz - center) <= tolerance)
    .sort((a, b) => (b.prominence - a.prominence) || Math.abs(a.hz - center) - Math.abs(b.hz - center))[0];
}

function toneBounds(tile: SpectralTile, time: RepairAnalysisBounds, hz: number): RepairAnalysisBounds {
  const max = Math.floor(tile.frequencies.at(-1) ?? 20_000);
  const radius = Math.max(10, Math.ceil(hz * .022));
  const lowHz = Math.min(max - 20, Math.max(20, hz - radius));
  return { ...time, lowHz, highHz: Math.min(max, Math.max(lowHz + 20, hz + radius)) };
}

function tones(tile: SpectralTile, columns: readonly (readonly number[])[], selection: RepairAnalysisBounds, kind: 'tone' | 'harmonics' | 'hum'): Result {
  const peaks = steadyPeaks(tile, columns);
  const seed = Math.sqrt(Math.max(20, selection.lowHz) * selection.highHz);
  const wideSelection = selection.highHz >= Math.max(20, selection.lowHz) * 2;
  const fundamental = kind === 'hum'
    ? peaks.filter(peak => peak.hz >= 44 && peak.hz <= 67).sort((a, b) => b.prominence - a.prominence)[0]
    : wideSelection
      ? peaks.filter(peak => peak.hz >= selection.lowHz && peak.hz <= selection.highHz).sort((a, b) => b.prominence - a.prominence)[0]
      : ridge(peaks, seed, Math.max(22, seed * .16));
  if (!fundamental) return { candidates: [], detail: kind === 'hum'
    ? 'No steady peak stands out near 50 or 60 Hz. Try Tones detail over a passage with obvious hum.'
    : 'No steady ridge stands out here. Try a narrower selection on the source view.' };
  const candidates: RepairSuggestion[] = [{
    label: (kind === 'tone' ? 'Nearby ridge' : 'Fundamental') + ' near ' + fundamental.hz + ' Hz',
    bounds: toneBounds(tile, selection, fundamental.hz),
    primary: true,
  }];
  if (kind !== 'tone') {
    for (let n = 2; n <= 8; n++) {
      const expected = fundamental.hz * n;
      const partial = ridge(peaks, expected, Math.max(12, expected * .035));
      if (partial && candidates.every(candidate => partial.hz > candidate.bounds.highHz)) {
        candidates.push({ label: 'Harmonic ' + n + ' near ' + partial.hz + ' Hz', bounds: toneBounds(tile, selection, partial.hz), primary: true });
      }
    }
  }
  return { candidates, detail: 'Frequencies are estimated from the display. Listen before reducing, especially around nearby notes.' };
}

function transientProfile(tile: SpectralTile, column: number, bins: number[], context: number): number[] {
  const current = tile.columns[column];
  const before = tile.columns[column - context];
  const after = tile.columns[column + context];
  if (!before || !after) return [];
  return bins.map(i => Math.max(0, current[i] - Math.max(before[i], after[i])));
}

function similarity(seed: readonly number[], event: readonly number[]): number {
  const dot = seed.reduce((total, value, i) => total + value * event[i], 0);
  const norm = (values: readonly number[]) => Math.hypot(...values);
  const a = norm(seed);
  const b = norm(event);
  if (!a || !b) return 0;
  return dot / (a * b) * Math.min(a, b) / Math.max(a, b);
}

function similarEvents(tile: SpectralTile, selection: RepairAnalysisBounds, rate: number, sensitivity: number): Result {
  if (tile.windowFrames !== undefined && tile.windowFrames > rate * .035) {
    return { candidates: [], detail: 'Switch to Transients detail to distinguish brief events.' };
  }
  if (tile.step > rate * .025) {
    return { candidates: [], detail: 'Zoom in until the source view can distinguish brief events.' };
  }
  const bins = indices(tile, selection.lowHz, selection.highHz);
  if (bins.length < 5) return { candidates: [], detail: 'Widen the frequency selection to compare transient shapes.' };
  const seedIndex = Math.floor(((selection.startFrame + selection.endFrame) / 2 - tile.start) / tile.step);
  const context = Math.max(1, Math.ceil((tile.windowFrames ?? tile.step) / (2 * tile.step)));
  const seed = transientProfile(tile, seedIndex, bins, context);
  if (selection.endFrame - selection.startFrame > tile.step * 3 || seed.filter(value => value >= 6).length < 3) {
    return { candidates: [], detail: 'Select one brief event with a visible onset and surrounding context.' };
  }
  const cutoff = .91 - sensitivity * .045;
  const ranked = tile.columns.flatMap((_, index) => {
    if (Math.abs(index - seedIndex) <= context) return [];
    const startFrame = Math.round(tile.start + index * tile.step);
    const endFrame = Math.round(tile.start + (index + 1) * tile.step);
    const event = transientProfile(tile, index, bins, context);
    const score = similarity(seed, event);
    if (score < cutoff || endFrame <= startFrame) return [];
    return [{ index, score, rise: Math.hypot(...event), bounds: {
      startFrame, endFrame, lowHz: selection.lowHz, highHz: selection.highHz,
    } }];
  }).sort((a, b) => (b.score - a.score) || (b.rise - a.rise));
  const candidates: typeof ranked = [];
  for (const match of ranked) {
    if (candidates.every(candidate => Math.abs(candidate.index - match.index) > context)) candidates.push(match);
    if (candidates.length === 16) break;
  }
  candidates.sort((a, b) => a.index - b.index);
  return {
    candidates: candidates.map(({ bounds }, index) => ({ label: 'Match ' + (index + 1), bounds })),
    detail: candidates.length
      ? 'Review one event at a time. Percussion may resemble a fault. The suggested time covers one display column.'
      : 'No matching brief events stood out in the analysed range.',
  };
}

function clicks(tile: SpectralTile, selection: RepairAnalysisBounds, sensitivity: number, target?: RepairTarget): Result {
  if (!tile.clicks) return { candidates: [], detail: 'Switch to Transients detail to detect single-sample clicks.' };
  const highHz = Math.floor(tile.frequencies.at(-1) ?? 20_000);
  const candidates = tile.clicks
    .filter(click => click.sensitivity <= sensitivity && click.frame >= selection.startFrame && click.frame < selection.endFrame
      && (!target || click.channel === target))
    .map(({ frame, channel }, index) => ({ label: 'Click ' + (index + 1) + (channel ? ' · ' + (channel === 'left' ? 'Left' : 'Right') : ''), channel,
      bounds: { startFrame: frame, endFrame: frame + 1, lowHz: 20, highHz } }));
  return { candidates, detail: candidates.length
    ? 'Review each isolated spike in context. The suggested range marks its exact source sample.'
    : 'No isolated single-sample spikes stood out in the selected range.' };
}

function noise(columns: readonly (readonly number[])[], tile: SpectralTile, selection: RepairAnalysisBounds): Result {
  const bins = indices(tile, Math.max(250, selection.lowHz), selection.highHz);
  if (columns.length < 6 || bins.length < 8) return { candidates: [], detail: 'Select a longer noise-only passage across more frequencies.' };
  const profile = bins.map(i => {
    const levels = columns.map(column => column[i]);
    const floor = median(levels);
    const stable = floor > -95 && levels.filter(level => Math.abs(level - floor) <= 6).length / levels.length >= .8;
    return { floor, stable };
  });
  const stable = profile.filter(band => band.stable).length / profile.length;
  const typical = Math.round(median(profile.map(band => band.floor)));
  return {
    candidates: [],
    detail: stable >= .7 ? 'A steady spectral floor around ' + typical + ' dBFS per displayed band. Audition for music or speech before reducing.'
      : 'The spectral floor varies across this passage. Try a quieter, steadier section.',
  };
}

export function analyseRepair(probe: TreatmentProbe | null, selection: RepairAnalysisBounds, rate: number, kind: RepairAnalysisKind, sensitivity: number, target?: RepairTarget): Result {
  const tile = getTile(probe);
  if (!tile) return { candidates: [], detail: 'Switch to the source spectrogram and wait for the view to update.' };
  const columns = selectedColumns(tile, selection);
  if (columns.length < (kind === 'similar' || kind === 'clicks' ? 1 : 3) || selection.startFrame < tile.start - 1
    || selection.endFrame > tile.start + tile.step * tile.columns.length + 1) {
    return { candidates: [], detail: 'Fit the full selection in the source view or draw a longer region.' };
  }
  if (kind === 'tone' || kind === 'harmonics' || kind === 'hum') return tones(tile, columns, selection, kind);
  if (kind === 'noise') return noise(columns, tile, selection);
  const sensitivityLevel = Math.min(5, Math.max(1, sensitivity));
  if (kind === 'clicks') return clicks(tile, selection, sensitivityLevel, target);
  return similarEvents(tile, selection, rate, sensitivityLevel);
}
