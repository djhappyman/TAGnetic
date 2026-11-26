import type { AudioTreatment } from './audio-treatment';

export type DynamicsMode = 'compressor' | 'gate' | 'gain' | 'normalise';

export type DynamicsPlotOptions = {
  mode: DynamicsMode;
  inputPeakHold?: number;
  outputPeakHold?: number;
  normalisation?: { gainDb: number; loudnessLufs: number | null };
};

export const dynamicsPresets = {
  compressor: [
    { label: 'Tame loud peaks', patch: { compressorDb: -18, compressorRatio: 2, compressorAttackMs: 3, compressorReleaseMs: 100, compressorKneeDb: 6 } },
    { label: 'Even out speech', patch: { compressorDb: -24, compressorRatio: 3, compressorAttackMs: 15, compressorReleaseMs: 220, compressorKneeDb: 10 } },
  ],
  gate: [
    { label: 'Quiet gaps, keep room tone', patch: { gateDb: -52, gateReductionDb: 8 } },
    { label: 'Lower hiss between phrases', patch: { gateDb: -45, gateReductionDb: 12 } },
  ],
  gain: [
    { label: 'Trim source gain by 3 dB', patch: { gainDb: -3 } },
    { label: 'Lift quiet audio by 3 dB', patch: { gainDb: 3 } },
  ],
  normalise: [
    { label: 'Match the set to −18 LUFS', patch: { normalize: true, normalizeTargetDb: -18 } },
    { label: 'Match the set to −16 LUFS', patch: { normalize: true, normalizeTargetDb: -16 } },
  ],
} as const satisfies Record<DynamicsMode, readonly { label: string; patch: Partial<AudioTreatment> }[]>;

export function dynamicsMode(value: string): DynamicsMode {
  return value === 'gate' || value === 'gain' || value === 'normalise' ? value : 'compressor';
}
