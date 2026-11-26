import { eqBandBypassBits, maxEqBands, treatmentNumbers, type AudioTreatment, type EqBand, type TreatmentNumber } from './audio-treatment';

export const legacyEqBands = {
  low: { name: 'Low cut', enabled: 'rumble', frequency: 'rumbleHz', slope: 'rumbleSlope', bypassBit: eqBandBypassBits.rumble },
  bass: { name: 'Low shelf', enabled: 'bassDb', frequency: 'bassHz', gain: 'bassDb', bypassBit: eqBandBypassBits.bassDb },
  bell: { name: 'Bell', enabled: 'presenceDb', frequency: 'presenceHz', gain: 'presenceDb', q: 'presenceQ', bypassBit: eqBandBypassBits.presenceDb },
  notch: { name: 'Notch', enabled: 'notchHz', frequency: 'notchHz', q: 'notchQ', bypassBit: eqBandBypassBits.notchHz },
  treble: { name: 'High shelf', enabled: 'trebleDb', frequency: 'trebleHz', gain: 'trebleDb', bypassBit: eqBandBypassBits.trebleDb },
  high: { name: 'High cut', enabled: 'highCutHz', frequency: 'highCutHz', slope: 'highCutSlope', bypassBit: eqBandBypassBits.highCutHz },
} as const satisfies Record<string, {
  name: string;
  enabled: keyof AudioTreatment;
  frequency: TreatmentNumber;
  gain?: TreatmentNumber;
  q?: TreatmentNumber;
  slope?: TreatmentNumber;
  bypassBit: number;
}>;

export type EqShape = keyof typeof legacyEqBands;
export type ExtraEqShape = EqBand['kind'];

export type EqNode = {
  key: string;
  shape: EqShape;
  name: string;
  hz: number;
  gainDb: number;
  q: number;
  slope: number;
  bypassed: boolean;
  stored: boolean;
  extra?: { id?: number; index?: number };
};

const legacyKeys = Object.keys(legacyEqBands) as EqShape[];
const extrasOf = (settings: AudioTreatment) => settings.eqBands ?? [];
const bypassMask = (settings: AudioTreatment) => settings.eqBypassed ?? 0;
let nextBandId = 1;

export function isLegacyEqBand(key: string): key is EqShape {
  return Object.prototype.hasOwnProperty.call(legacyEqBands, key);
}

function extraKey(id: number | undefined, index?: number) {
  return id === undefined ? 'extra-draft-' + index : 'extra-' + id;
}

export function legacyEqNode(settings: AudioTreatment, shape: EqShape): EqNode {
  const spec = legacyEqBands[shape];
  const stored = Boolean(settings[spec.enabled]);
  const value = (key: TreatmentNumber) => settings[key] || treatmentNumbers[key].initial;
  return {
    key: shape,
    shape,
    name: spec.name,
    hz: value(spec.frequency),
    gainDb: 'gain' in spec ? settings[spec.gain] : 0,
    q: 'q' in spec ? value(spec.q) * (shape === 'notch' ? 100 : 1) : 80,
    slope: 'slope' in spec ? value(spec.slope) : 12,
    bypassed: (bypassMask(settings) & spec.bypassBit) !== 0,
    stored,
  };
}

export function findEqNode(settings: AudioTreatment, key: string | null): EqNode | undefined {
  if (!key) return;
  if (isLegacyEqBand(key)) return legacyEqNode(settings, key);
  return extraEqNodes(settings).find(node => node.key === key);
}

function extraEqNodes(settings: AudioTreatment): EqNode[] {
  return extrasOf(settings).map((band, index) => ({
    key: extraKey(band.id, index),
    name: legacyEqBands[band.kind].name + ' ' + (index + 1),
    shape: band.kind,
    hz: band.hz,
    gainDb: band.gainDb,
    q: band.q,
    slope: 12,
    bypassed: Boolean(band.bypassed),
    stored: true,
    extra: { id: band.id, index },
  }));
}

export function visibleEqNodes(settings: AudioTreatment, selected: string | null): EqNode[] {
  return legacyKeys.map(shape => legacyEqNode(settings, shape))
    .filter(node => node.stored || node.key === selected)
    .concat(extraEqNodes(settings));
}

export function makeExtraEqNode(settings: AudioTreatment, shape: ExtraEqShape): EqNode {
  const used = new Set(extrasOf(settings).map(band => band.id));
  while (used.has(nextBandId)) nextBandId++;
  const id = nextBandId++;
  const original = legacyEqNode(settings, shape);
  return { ...original, key: extraKey(id), stored: false, bypassed: false, extra: { id } };
}

export function eqFrequencyRange(node: EqNode, rate: number): [number, number] {
  const spec = treatmentNumbers[legacyEqBands[node.shape].frequency];
  const min = node.extra ? 20 : spec.min;
  const max = Math.min(node.extra ? 18000 : spec.max, Math.floor(rate * 0.45));
  return [min, max];
}

export function eqWidthRange(shape: EqShape): [number, number] {
  return shape === 'notch' ? [500, 3000] : [20, 1200];
}

const clamp = (value: number, min: number, max: number) => Math.round(Math.min(max, Math.max(min, value)));

export function eqBandPatch(settings: AudioTreatment, node: EqNode, rate: number): Partial<AudioTreatment> {
  const [min, max] = eqFrequencyRange(node, rate);
  if (max < min) return {};
  const hz = clamp(node.hz, min, max);
  const gainDb = clamp(node.gainDb, -6, 6);
  const [minQ, maxQ] = eqWidthRange(node.shape);
  const q = clamp(node.q, minQ, maxQ);
  if (node.extra && (node.shape === 'bell' || node.shape === 'notch')) {
    const bands = Array.from(extrasOf(settings));
    const found = node.extra.id === undefined ? node.extra.index ?? -1 : bands.findIndex(band => band.id === node.extra?.id);
    if (found === -1 && bands.length >= maxEqBands) return {};
    const band: EqBand = {
      id: node.extra.id,
      kind: node.shape,
      hz,
      gainDb: node.shape === 'notch' ? 0 : gainDb,
      q,
      bypassed: node.bypassed,
    };
    if (found === -1) bands.push(band);
    else bands[found] = band;
    return { eqBands: bands };
  }
  const spec = legacyEqBands[node.shape];
  const mask = bypassMask(settings);
  const result: Partial<AudioTreatment> = {
    [spec.frequency]: hz,
    eqBypassed: node.bypassed ? mask | spec.bypassBit : mask & ~spec.bypassBit,
  };
  if (node.shape === 'low') result.rumble = true;
  if ('gain' in spec) result[spec.gain] = gainDb;
  if ('slope' in spec) result[spec.slope] = clamp(node.slope / 12, 1, 4) * 12;
  if ('q' in spec) result[spec.q] = node.shape === 'notch' ? Math.round(q / 100) : q;
  return result;
}

export function removeEqBand(settings: AudioTreatment, node: EqNode): Partial<AudioTreatment> {
  if (node.extra) {
    const bands = extrasOf(settings).filter((band, index) => extraKey(band.id, index) !== node.key);
    return { eqBands: bands.length ? bands : null };
  }
  const spec = legacyEqBands[node.shape];
  return {
    [spec.enabled]: node.shape === 'low' ? false : 0,
    eqBypassed: bypassMask(settings) & ~spec.bypassBit,
  };
}

export function originalEqBand(settings: AudioTreatment, node: EqNode): Partial<AudioTreatment> {
  if (node.extra) return { eqBands: settings.eqBands };
  const spec = legacyEqBands[node.shape];
  return {
    [spec.enabled]: settings[spec.enabled],
    [spec.frequency]: settings[spec.frequency],
    ...('gain' in spec ? { [spec.gain]: settings[spec.gain] } : {}),
    ...('q' in spec ? { [spec.q]: settings[spec.q] } : {}),
    ...('slope' in spec ? { [spec.slope]: settings[spec.slope] } : {}),
    eqBypassed: bypassMask(settings),
  };
}

export function eqSoloRange(node: EqNode, sampleRate: number): { lowHz: number; highHz: number } | undefined {
  const max = Math.min(20_000, Math.floor(sampleRate * 0.45));
  if (max < 40) return;
  if (node.shape === 'low' || node.shape === 'bass') return { lowHz: 20, highHz: clamp(node.hz, 40, max) };
  if (node.shape === 'high' || node.shape === 'treble') return { lowHz: clamp(node.hz, 20, max - 20), highHz: max };
  const width = Math.max(10, node.hz * 50 / Math.max(1, node.q));
  const lowHz = clamp(node.hz - width, 20, max - 20);
  return { lowHz, highHz: clamp(node.hz + width, lowHz + 20, max) };
}
