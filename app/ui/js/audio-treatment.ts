export type AudioTreatment = {
  normalize: boolean;
  normalizeTargetDb: number;
  mono: 'off' | 'mix' | 'left' | 'right';
  rumble: boolean;
  rumbleHz: number;
  hum: 'off' | 'hz50' | 'hz60';
  humHarmonics: number;
  dcHz: number;
  highCutHz: number;
  notchHz: number;
  notchQ: number;
  bassDb: number;
  presenceDb: number;
  trebleDb: number;
  gainDb: number;
  compressorDb: number;
  compressorRatio: number;
  gateDb: number;
  gateReductionDb: number;
  balance: number;
  stereoWidth: number;
  swapChannels: boolean;
  polarity: 'off' | 'left' | 'right';
};

export const noTreatment: AudioTreatment = Object.freeze({
  normalize: false, normalizeTargetDb: 0, mono: 'off', rumble: false, rumbleHz: 0,
  hum: 'off', humHarmonics: 0, dcHz: 0, highCutHz: 0, notchHz: 0, notchQ: 0,
  bassDb: 0, presenceDb: 0, trebleDb: 0, gainDb: 0, compressorDb: 0, compressorRatio: 0,
  gateDb: 0, gateReductionDb: 0, balance: 0, stereoWidth: 0, swapChannels: false, polarity: 'off',
});

export type TreatmentNumber = {
  [Key in keyof AudioTreatment]: AudioTreatment[Key] extends number ? Key : never;
}[keyof AudioTreatment];

type NumberSpec = { label: string; min: number; max: number; initial: number; unit: string; step?: number; defaultIsZero?: boolean };

export const treatmentNumbers: Record<TreatmentNumber, NumberSpec> = {
  dcHz: { label: 'DC cutoff', min: 2, max: 15, initial: 5, unit: 'Hz' },
  rumbleHz: { label: 'Rumble cutoff', min: 15, max: 100, initial: 35, unit: 'Hz', defaultIsZero: true },
  humHarmonics: { label: 'Hum frequencies', min: 1, max: 5, initial: 1, unit: '', defaultIsZero: true },
  highCutHz: { label: 'High cutoff', min: 4000, max: 18000, initial: 10000, unit: 'Hz', step: 100 },
  notchHz: { label: 'Whistle frequency', min: 200, max: 12000, initial: 1000, unit: 'Hz', step: 10 },
  notchQ: { label: 'Notch narrowness', min: 5, max: 30, initial: 15, unit: 'Q', defaultIsZero: true },
  bassDb: { label: 'Bass', min: -6, max: 6, initial: 0, unit: 'dB' },
  presenceDb: { label: 'Presence', min: -6, max: 6, initial: 0, unit: 'dB' },
  trebleDb: { label: 'Treble', min: -6, max: 6, initial: 0, unit: 'dB' },
  gainDb: { label: 'Gain', min: -18, max: 12, initial: 0, unit: 'dB' },
  normalizeTargetDb: { label: 'Target loudness', min: -30, max: -8, initial: -18, unit: 'LUFS', defaultIsZero: true },
  compressorDb: { label: 'Compression threshold', min: -36, max: -6, initial: -18, unit: 'dBFS' },
  compressorRatio: { label: 'Compression ratio', min: 2, max: 6, initial: 2, unit: ':1', defaultIsZero: true },
  gateDb: { label: 'Quiet-gap threshold', min: -72, max: -30, initial: -48, unit: 'dBFS' },
  gateReductionDb: { label: 'Maximum reduction', min: 3, max: 24, initial: 12, unit: 'dB', defaultIsZero: true },
  balance: { label: 'Stereo balance', min: -100, max: 100, initial: 0, unit: '%', step: 5 },
  stereoWidth: { label: 'Stereo width change', min: -100, max: 50, initial: 0, unit: '%', step: 5 },
};

const numberKeys = Object.keys(treatmentNumbers) as TreatmentNumber[];
const settingKeys = Object.keys(noTreatment) as (keyof AudioTreatment)[];

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
  };
  for (const key of numberKeys) {
    const input = value[key];
    const { min, max, initial, defaultIsZero } = treatmentNumbers[key];
    if (input !== undefined && Number.isInteger(input) && input >= min && input <= max
      && !(defaultIsZero && input === initial)) settings[key] = input;
  }
  return settings;
}

export function sameTreatment(left: AudioTreatment, right: AudioTreatment): boolean {
  return left === right || settingKeys.every(key => left[key] === right[key]);
}

export const effectCategories = ['Repair', 'Tone', 'Level', 'Stereo'] as const;
export type EffectCategory = typeof effectCategories[number];

type EffectKey = 'dcHz' | 'rumble' | 'hum' | 'highCutHz' | 'notchHz' | 'bassDb' | 'presenceDb' | 'trebleDb'
  | 'gainDb' | 'normalize' | 'compressorDb' | 'gateDb' | 'mono' | 'swapChannels' | 'polarity' | 'balance' | 'stereoWidth';

type Effect = {
  key: EffectKey;
  category: EffectCategory;
  label: string;
  description: string;
  numbers?: readonly TreatmentNumber[];
  reset: Partial<AudioTreatment>;
};

export const audioEffects: readonly Effect[] = [
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
  { key: 'compressorDb', category: 'Level', label: 'Compressor', description: 'Bring louder sounds closer to the rest.', numbers: ['compressorDb', 'compressorRatio'], reset: { compressorDb: 0, compressorRatio: 0 } },
  { key: 'gateDb', category: 'Level', label: 'Quiet-gap reduction', description: 'Turn down noise in the quiet parts.', numbers: ['gateDb', 'gateReductionDb'], reset: { gateDb: 0, gateReductionDb: 0 } },
  { key: 'mono', category: 'Stereo', label: 'Convert to mono', description: 'Mix both channels or keep the cleaner one.', reset: { mono: 'off' } },
  { key: 'swapChannels', category: 'Stereo', label: 'Swap left and right', description: 'Correct channels that were recorded in reverse.', reset: { swapChannels: false } },
  { key: 'polarity', category: 'Stereo', label: 'Invert polarity', description: 'Invert one channel to correct a wiring issue.', reset: { polarity: 'off' } },
  { key: 'balance', category: 'Stereo', label: 'Balance', description: 'Turn down the stronger channel. Negative favours left.', numbers: ['balance'], reset: { balance: 0 } },
  { key: 'stereoWidth', category: 'Stereo', label: 'Stereo width', description: 'Narrow or widen the stereo image.', numbers: ['stereoWidth'], reset: { stereoWidth: 0 } },
];

export function effectIsActive(settings: AudioTreatment, key: EffectKey): boolean {
  const value = settings[key];
  return typeof value === 'string' ? value !== 'off' : !!value;
}

export const hasTreatment = (settings: AudioTreatment) => audioEffects.some(effect => effectIsActive(settings, effect.key));
export const audioTreatmentCount = (settings: AudioTreatment) => audioEffects.filter(effect => effectIsActive(settings, effect.key)).length;
export const activeEffectLabels = (settings: AudioTreatment) => audioEffects.filter(effect => effectIsActive(settings, effect.key)).map(effect => effect.label);
