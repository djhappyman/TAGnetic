export const eqFrequencyLimit = 20_000;
export const eqGainLimit = 24;
export const eqGainStep = 0.1;

/** @param {number} db */
export function roundEqDb(db) {
  return Math.round(db * 10) / 10;
}

/** @param {number} qHundredths @param {number} gainDb @param {boolean | undefined} adaptive */
export function bellQuality(qHundredths, gainDb, adaptive) {
  const quality = qHundredths / 100;
  return adaptive ? Math.min(12, quality * (1 + Math.abs(gainDb) / 12)) : quality;
}
