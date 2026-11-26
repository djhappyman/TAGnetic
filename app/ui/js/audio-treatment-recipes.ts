import { readTreatment, type AudioTreatment } from './audio-treatment';

export type TreatmentRecipe = { name: string; settings: AudioTreatment };

const recipeKey = 'tagnetic.audioTreatment.recipes';
const copyKey = 'tagnetic.audioTreatment.copied';

export function portableTreatment(settings: Partial<AudioTreatment>): AudioTreatment {
  return readTreatment({ ...settings, spectralRepairs: null });
}

export function readTreatmentRecipes(): TreatmentRecipe[] {
  try {
    const stored: unknown = JSON.parse(localStorage.getItem(recipeKey) ?? '[]');
    return Array.isArray(stored) ? stored.flatMap(recipe =>
      typeof recipe?.name === 'string' && recipe.name.trim() && recipe.settings && typeof recipe.settings === 'object'
        ? [{ name: recipe.name, settings: portableTreatment(recipe.settings) }] : []) : [];
  } catch {
    return [];
  }
}

export function saveTreatmentRecipes(recipes: TreatmentRecipe[]) {
  localStorage.setItem(recipeKey, JSON.stringify(recipes));
}

export function copyTreatmentSettings(settings: AudioTreatment) {
  localStorage.setItem(copyKey, JSON.stringify(portableTreatment(settings)));
}

export function readCopiedTreatment(): AudioTreatment | null {
  try {
    const stored: unknown = JSON.parse(localStorage.getItem(copyKey) ?? 'null');
    return stored && typeof stored === 'object' ? portableTreatment(stored) : null;
  } catch {
    return null;
  }
}

/** Fit reusable settings to a new recording while keeping its own repair regions. */
export function treatmentForRecording(
  recipe: AudioTreatment, current: AudioTreatment, sampleRate: number, stereo: boolean,
) {
  const limit = Math.floor(sampleRate * 0.45);
  const settings = {
    ...recipe,
    spectralRepairs: current.spectralRepairs,
    eqBands: recipe.eqBands?.filter(band => band.hz <= limit) ?? null,
  };
  if (limit < (settings.rumbleHz || 35)) settings.rumble = false;
  if (limit < settings.dcHz) settings.dcHz = 0;
  if (limit < settings.highCutHz) settings.highCutHz = limit >= 4000 ? limit : 0;
  if (limit < settings.notchHz) settings.notchHz = limit >= 200 ? limit : 0;
  if (limit < (settings.bassHz || 120)) settings.bassDb = 0;
  if (limit < (settings.presenceHz || 1500)) settings.presenceDb = 0;
  if (limit < (settings.trebleHz || 4000)) settings.trebleDb = 0;
  if (settings.hum !== 'off') {
    const fundamental = settings.hum === 'hz50' ? 50 : 60;
    if (limit < fundamental) settings.hum = 'off';
    else settings.humHarmonics = Math.min(settings.humHarmonics || 1, Math.floor(limit / fundamental));
  }
  if (!stereo) {
    settings.mono = 'off';
    settings.polarity = 'off';
    settings.swapChannels = false;
    settings.balance = 0;
    settings.stereoWidth = 0;
  }
  return readTreatment(settings);
}
