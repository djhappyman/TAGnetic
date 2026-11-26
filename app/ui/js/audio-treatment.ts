export type SpectralRepair = {
  startFrame: number;
  endFrame: number;
  lowHz: number;
  highHz: number;
  amountDb: number;
  channel?: 'left' | 'right';
};
export const maxSpectralRepairs = 8;

export type EqBand = {
  id?: number;
  kind: 'bell' | 'notch';
  hz: number;
  gainDb: number;
  q: number;
  bypassed?: boolean;
};
export const maxEqBands = 8;
export const eqBandBypassBits = { rumble: 1, bassDb: 2, presenceDb: 4, notchHz: 8, trebleDb: 16, highCutHz: 32 } as const;

export type AudioTreatment = {
  normalize: boolean;
  normalizeTargetDb: number;
  mono: 'off' | 'mix' | 'left' | 'right';
  rumble: boolean;
  rumbleHz: number;
  rumbleSlope: number;
  hum: 'off' | 'hz50' | 'hz60';
  humHarmonics: number;
  dcHz: number;
  highCutHz: number;
  highCutSlope: number;
  notchHz: number;
  notchQ: number;
  bassDb: number;
  bassHz: number;
  presenceDb: number;
  presenceHz: number;
  presenceQ: number;
  trebleDb: number;
  trebleHz: number;
  eqBypassed: number;
  eqBands: EqBand[] | null;
  gainDb: number;
  compressorInputDb: number;
  compressorDb: number;
  compressorRatio: number;
  compressorAttackMs: number;
  compressorReleaseMs: number;
  compressorKneeDb: number;
  gateDb: number;
  gateReductionDb: number;
  balance: number;
  stereoWidth: number;
  swapChannels: boolean;
  polarity: 'off' | 'left' | 'right';
  spectralRepairs: (SpectralRepair | null)[] | null;
};

export const noTreatment: AudioTreatment = Object.freeze({
  normalize: false, normalizeTargetDb: 0, mono: 'off', rumble: false, rumbleHz: 0, rumbleSlope: 0,
  hum: 'off', humHarmonics: 0, dcHz: 0, highCutHz: 0, highCutSlope: 0, notchHz: 0, notchQ: 0,
  bassDb: 0, bassHz: 0, presenceDb: 0, presenceHz: 0, presenceQ: 0, trebleDb: 0, trebleHz: 0, eqBypassed: 0, eqBands: null,
  gainDb: 0, compressorInputDb: 0, compressorDb: 0, compressorRatio: 0, compressorAttackMs: 0, compressorReleaseMs: 0, compressorKneeDb: 0,
  gateDb: 0, gateReductionDb: 0, balance: 0, stereoWidth: 0, swapChannels: false, polarity: 'off', spectralRepairs: null,
});

export type TreatmentNumber = Exclude<{
  [Key in keyof AudioTreatment]: AudioTreatment[Key] extends number ? Key : never;
}[keyof AudioTreatment], 'eqBypassed'>;

type NumberSpec = { label: string; min: number; max: number; initial: number; unit: string; step?: number; defaultIsZero?: boolean };

export const treatmentNumbers: Record<TreatmentNumber, NumberSpec> = {
  dcHz: { label: 'DC cutoff', min: 2, max: 15, initial: 5, unit: 'Hz' },
  rumbleHz: { label: 'Rumble cutoff', min: 15, max: 100, initial: 35, unit: 'Hz', defaultIsZero: true },
  rumbleSlope: { label: 'Low-cut slope', min: 12, max: 48, initial: 12, unit: 'dB/oct', defaultIsZero: true },
  highCutSlope: { label: 'High-cut slope', min: 12, max: 48, initial: 12, unit: 'dB/oct', defaultIsZero: true },
  bassHz: { label: 'Bass frequency', min: 20, max: 18000, initial: 120, unit: 'Hz', defaultIsZero: true },
  presenceHz: { label: 'Presence frequency', min: 20, max: 18000, initial: 1500, unit: 'Hz', defaultIsZero: true },
  presenceQ: { label: 'Bell Q ×100', min: 20, max: 1200, initial: 80, unit: '', defaultIsZero: true },
  trebleHz: { label: 'Treble frequency', min: 20, max: 18000, initial: 4000, unit: 'Hz', defaultIsZero: true },
  humHarmonics: { label: 'Hum frequencies', min: 1, max: 5, initial: 1, unit: '', defaultIsZero: true },
  highCutHz: { label: 'High cutoff', min: 4000, max: 18000, initial: 10000, unit: 'Hz', step: 100 },
  notchHz: { label: 'Whistle frequency', min: 200, max: 12000, initial: 1000, unit: 'Hz', step: 10 },
  notchQ: { label: 'Notch narrowness', min: 5, max: 30, initial: 15, unit: 'Q', defaultIsZero: true },
  bassDb: { label: 'Bass', min: -6, max: 6, initial: 0, unit: 'dB' },
  presenceDb: { label: 'Presence', min: -6, max: 6, initial: 0, unit: 'dB' },
  trebleDb: { label: 'Treble', min: -6, max: 6, initial: 0, unit: 'dB' },
  gainDb: { label: 'Gain', min: -18, max: 12, initial: 0, unit: 'dB' },
  compressorInputDb: { label: 'Input gain', min: -24, max: 24, initial: 0, unit: 'dB' },
  normalizeTargetDb: { label: 'Target loudness', min: -30, max: -8, initial: -18, unit: 'LUFS', defaultIsZero: true },
  compressorDb: { label: 'Compression threshold', min: -36, max: -6, initial: -18, unit: 'dBFS' },
  compressorRatio: { label: 'Compression ratio', min: 2, max: 6, initial: 2, unit: ':1', defaultIsZero: true },
  compressorAttackMs: { label: 'Attack', min: 1, max: 100, initial: 8, unit: 'ms', defaultIsZero: true },
  compressorReleaseMs: { label: 'Release', min: 20, max: 1000, initial: 160, unit: 'ms', defaultIsZero: true },
  compressorKneeDb: { label: 'Knee', min: 0, max: 24, initial: 0, unit: 'dB' },
  gateDb: { label: 'Quiet-gap threshold', min: -72, max: -30, initial: -48, unit: 'dBFS' },
  gateReductionDb: { label: 'Maximum reduction', min: 3, max: 24, initial: 12, unit: 'dB', defaultIsZero: true },
  balance: { label: 'Stereo balance', min: -100, max: 100, initial: 0, unit: '%', step: 5 },
  stereoWidth: { label: 'Stereo width change', min: -100, max: 50, initial: 0, unit: '%', step: 5 },
};

const numberKeys = Object.keys(treatmentNumbers) as TreatmentNumber[];
const settingKeys = (Object.keys(noTreatment) as (keyof AudioTreatment)[]).filter(key => key !== 'spectralRepairs' && key !== 'eqBands');

export function sameRepairBounds(left: Omit<SpectralRepair, 'amountDb'>, right: Omit<SpectralRepair, 'amountDb'>) {
  return left.startFrame === right.startFrame && left.endFrame === right.endFrame
    && left.lowHz === right.lowHz && left.highHz === right.highHz && left.channel === right.channel;
}

function readRepairs(value: AudioTreatment['spectralRepairs']) {
  if (!Array.isArray(value)) return null;
  const repairs = value.slice(0, maxSpectralRepairs).map(edit => {
    if (!edit) return null;
    const { startFrame, endFrame, lowHz, highHz, amountDb, channel } = edit;
    return [startFrame, endFrame, lowHz, highHz, amountDb].every(Number.isSafeInteger)
      && startFrame >= 0 && startFrame < endFrame && lowHz >= 20 && highHz - lowHz >= 20 && highHz <= 20_000
      && amountDb >= 1 && amountDb <= 36 ? { startFrame, endFrame, lowHz, highHz, amountDb,
        ...(channel === 'left' || channel === 'right' ? { channel } : {}) } : null;
  });
  while (repairs.length && !repairs.at(-1)) repairs.pop();
  return repairs.length ? repairs : null;
}

function readBands(value: AudioTreatment['eqBands']) {
  if (!Array.isArray(value)) return null;
  const ids = new Set<number>();
  const bands = value.slice(0, maxEqBands).flatMap(band => {
    if (!band || (band.kind !== 'bell' && band.kind !== 'notch')) return [];
    const { kind, hz, gainDb, q } = band;
    const notch = kind === 'notch';
    if (![hz, gainDb, q].every(Number.isInteger) || hz < 20 || hz > 18_000
      || gainDb < -6 || gainDb > 6 || q < (notch ? 500 : 20) || q > (notch ? 3000 : 1200)) return [];
    const id = band.id;
    const validId = typeof id === 'number' && Number.isInteger(id) && id > 0 && id <= 0xffff_ffff && !ids.has(id);
    if (validId) ids.add(id);
    return [{ kind, hz, gainDb: notch ? 0 : gainDb, q, ...(validId ? { id } : {}), ...(band.bypassed === true ? { bypassed: true } : {}) }];
  });
  return bands.length ? bands : null;
}

export function readTreatment(value?: Partial<AudioTreatment> | null): AudioTreatment {
  if (!value) return noTreatment;
  const settings: AudioTreatment = {
    ...noTreatment,
    normalize: value.normalize === true,
    mono: value.mono === 'mix' || value.mono === 'left' || value.mono === 'right' ? value.mono : 'off',
    rumble: value.rumble === true,
    hum: value.hum === 'hz50' || value.hum === 'hz60' ? value.hum : 'off',
    swapChannels: value.swapChannels === true,
    polarity: value.polarity === 'left' || value.polarity === 'right' ? value.polarity : 'off',
    spectralRepairs: readRepairs(value.spectralRepairs ?? null),
    eqBands: readBands(value.eqBands ?? null),
    eqBypassed: Number.isInteger(value.eqBypassed) ? (value.eqBypassed ?? 0) & 63 : 0,
  };
  for (const key of numberKeys) {
    const input = value[key];
    const { min, max, initial, defaultIsZero } = treatmentNumbers[key];
    if (input !== undefined && Number.isInteger(input) && input >= min && input <= max
      && (!(key === 'rumbleSlope' || key === 'highCutSlope') || input % 12 === 0)
      && !(defaultIsZero && input === initial)) settings[key] = input;
  }
  return settings;
}

export function sameTreatment(left: AudioTreatment, right: AudioTreatment): boolean {
  if (left === right) return true;
  if (!settingKeys.every(key => left[key] === right[key])) return false;
  const bands = left.eqBands;
  const others = right.eqBands;
  if (bands !== others && !(bands && others && bands.length === others.length && bands.every((band, index) => {
    const other = others[index];
    return band.id === other.id && band.kind === other.kind && band.hz === other.hz && band.q === other.q
      && band.gainDb === other.gainDb && !!band.bypassed === !!other.bypassed;
  }))) return false;
  const a = left.spectralRepairs;
  const b = right.spectralRepairs;
  return a === b || (!!a && !!b && a.length === b.length && a.every((edit, index) => {
    const other = b[index];
    return edit === other || (!!edit && !!other && sameRepairBounds(edit, other) && edit.amountDb === other.amountDb);
  }));
}

export const effectCategories = ['Repair', 'Tone', 'Level', 'Stereo'] as const;
export type EffectCategory = typeof effectCategories[number];

type EffectKey = 'dcHz' | 'rumble' | 'hum' | 'highCutHz' | 'notchHz' | 'bassDb' | 'presenceDb' | 'trebleDb'
  | 'gainDb' | 'normalize' | 'compressorDb' | 'gateDb' | 'mono' | 'swapChannels' | 'polarity' | 'balance' | 'stereoWidth'
  | 'spectralRepairs' | 'eqBands';

type Effect = {
  key: EffectKey;
  category: EffectCategory;
  label: string;
  description: string;
  numbers?: readonly TreatmentNumber[];
  reset: Partial<AudioTreatment>;
};

export const audioEffects: readonly Effect[] = [
  { key: 'eqBands', category: 'Tone', label: 'Additional EQ bands', description: 'Adjust additional frequency ranges.', reset: { eqBands: null } },
  { key: 'spectralRepairs', category: 'Repair', label: 'Selected frequency attenuation', description: 'Turn down selected frequency ranges at specific times.', reset: { spectralRepairs: null } },
  { key: 'dcHz', category: 'Repair', label: 'Remove DC offset', description: 'Recenter a waveform shifted above or below zero.', numbers: ['dcHz'], reset: { dcHz: 0 } },
  { key: 'rumble', category: 'Repair', label: 'Reduce rumble', description: 'Cut low vibration and handling noise.', numbers: ['rumbleHz'], reset: { rumble: false, rumbleHz: 0 } },
  { key: 'hum', category: 'Repair', label: 'Reduce mains hum', description: 'Choose the mains frequency and cut its harmonics.', numbers: ['humHarmonics'], reset: { hum: 'off', humHarmonics: 0 } },
  { key: 'notchHz', category: 'Repair', label: 'Whistle notch', description: 'Remove a steady tone while preserving nearby sound. Higher Q narrows the cut.', numbers: ['notchHz', 'notchQ'], reset: { notchHz: 0, notchQ: 0 } },
  { key: 'highCutHz', category: 'Repair', label: 'High cut', description: 'Soften hiss and harshness by cutting high frequencies.', numbers: ['highCutHz'], reset: { highCutHz: 0 } },
  { key: 'bassDb', category: 'Tone', label: 'Bass', description: 'Cut or boost the low end.', numbers: ['bassDb'], reset: { bassDb: 0 } },
  { key: 'presenceDb', category: 'Tone', label: 'Presence', description: 'Cut or boost the midrange for clarity.', numbers: ['presenceDb'], reset: { presenceDb: 0 } },
  { key: 'trebleDb', category: 'Tone', label: 'Treble', description: 'Cut or boost brightness.', numbers: ['trebleDb'], reset: { trebleDb: 0 } },
  { key: 'normalize', category: 'Level', label: 'Normalise', description: 'Set one loudness target for the whole recording.', numbers: ['normalizeTargetDb'], reset: { normalize: false, normalizeTargetDb: 0 } },
  { key: 'gainDb', category: 'Level', label: 'Gain', description: 'Turn the full recording up or down.', numbers: ['gainDb'], reset: { gainDb: 0 } },
  { key: 'compressorDb', category: 'Level', label: 'Compressor', description: 'Bring louder sounds closer to the rest.', numbers: ['compressorInputDb', 'compressorDb', 'compressorRatio'], reset: { compressorInputDb: 0, compressorDb: 0, compressorRatio: 0 } },
  { key: 'gateDb', category: 'Level', label: 'Quiet-gap reduction', description: 'Turn down noise in the quiet parts.', numbers: ['gateDb', 'gateReductionDb'], reset: { gateDb: 0, gateReductionDb: 0 } },
  { key: 'mono', category: 'Stereo', label: 'Convert to mono', description: 'Mix both channels or keep the cleaner one.', reset: { mono: 'off' } },
  { key: 'swapChannels', category: 'Stereo', label: 'Swap left and right', description: 'Correct channels that were recorded in reverse.', reset: { swapChannels: false } },
  { key: 'polarity', category: 'Stereo', label: 'Invert polarity', description: 'Invert one channel to correct a wiring issue.', reset: { polarity: 'off' } },
  { key: 'balance', category: 'Stereo', label: 'Balance', description: 'Turn down the stronger channel. Negative favours left.', numbers: ['balance'], reset: { balance: 0 } },
  { key: 'stereoWidth', category: 'Stereo', label: 'Stereo width', description: 'Narrow or widen the stereo image.', numbers: ['stereoWidth'], reset: { stereoWidth: 0 } },
];

export function effectIsActive(settings: AudioTreatment, key: EffectKey): boolean {
  if (Object.prototype.hasOwnProperty.call(eqBandBypassBits, key)
    && (settings.eqBypassed & eqBandBypassBits[key as keyof typeof eqBandBypassBits])) return false;
  if (key === 'eqBands') return !!settings.eqBands?.some(band => !band.bypassed && (band.kind === 'notch' || band.gainDb !== 0));
  if (key === 'compressorDb') return !!(settings.compressorDb || settings.compressorInputDb);
  const value = settings[key];
  return typeof value === 'string' ? value !== 'off' : !!value;
}

export const hasTreatment = (settings: AudioTreatment) => audioEffects.some(effect => effectIsActive(settings, effect.key));
export const audioTreatmentCount = (settings: AudioTreatment) => audioEffects.filter(effect => effectIsActive(settings, effect.key)).length;
export const activeEffectLabels = (settings: AudioTreatment) => audioEffects.filter(effect => effectIsActive(settings, effect.key)).map(effect => effect.label);
