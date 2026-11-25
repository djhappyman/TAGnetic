export type Launch = {
  mode: 'split' | 'convert'; paths: string[]; opener: HTMLElement | null; pendingPaths?: string[];
  refreshLibrary?: (paths: string[], replaced?: string[]) => Promise<boolean>;
  canReplace?: () => boolean;
};
export type Tags = Record<string, string>;
export type FrameRange = { start: number; end: number };
export type Source = {
  id: string;
  path: string;
  frames: number;
  sampleRate: number;
  originalFormat?: Format | null;
  sourceStamp?: string;
  overview?: number[];
  stereoOverview?: number[];
  overviewRms?: number[];
  stereoOverviewRms?: number[];
  tags?: Tags;
};
export type Loading = { frames: number; sampleRate: number; scannedFrames: number; overview: number[]; stereoOverview?: number[]; overviewRms?: number[] };
export type Track = {
  id: string;
  startFrame: number;
  include: boolean;
  tags: Tags;
  suggestedTags?: Tags;
  position?: string;
  leadIn?: boolean;
  pregapFrame?: number;
  pregap?: number;
  postgap?: number;
  index00?: number;
  index01?: number;
};
export type Format = 'mp3' | 'flac';
export const defaultSplitFilePattern = '%track:2% - %title%';
export type SplitMethod = 'copy' | 'exact';
export type SaveMode = 'beside' | 'replace' | 'folder';
export type Output = {
  path: string; tags?: Tags; startFrame?: number; endFrame?: number; sampleRate?: number;
  requestedStartFrame?: number | null; requestedEndFrame?: number | null;
};
export type Review = {
  id: string; outputs: Output[]; warnings: string[]; splitMethod?: SplitMethod | null;
  backups?: { source: string; path: string }[];
};
export type ExportResult = { paths: string[]; replaced?: string[]; backups?: string[]; warnings?: string[] };
export type ReplacementUndo = { paths: string[]; removed: string[]; warnings?: string[] };
export type Progress = { completed: number; total: number; phase: string; framesDone?: number; framesTotal?: number };
export type PeakTile = { start: number; end: number; values: Float32Array; rms?: Float32Array; stereo?: boolean };
export type AudioTreatment = { normalize: boolean; mono: 'off' | 'mix' | 'left' | 'right'; rumble: boolean; hum: 'off' | 'hz50' | 'hz60' };
export type TreatmentAnalysis = { gainDb: number; loudnessLufs: number | null; outputChannels: number };
export const noTreatment: AudioTreatment = { normalize: false, mono: 'off', rumble: false, hum: 'off' };
export const hasTreatment = ({ normalize, mono, rumble, hum }: AudioTreatment) =>
  normalize || mono !== 'off' || rumble || hum !== 'off';
export function readTreatment(value?: AudioTreatment): AudioTreatment {
  if (!value) return noTreatment;
  return {
    normalize: value.normalize === true,
    mono: value.mono === 'left' || value.mono === 'right' || value.mono === 'mix' ? value.mono : 'off',
    rumble: value.rumble === true,
    hum: value.hum === 'hz50' || value.hum === 'hz60' ? value.hum : 'off',
  };
}
export type Status = { frame: number; playing: boolean; loading?: boolean; sequence?: number };
export type Cue = {
  sourceName: string;
  markers: Array<{ frame: number; title?: string | null; artist?: string | null; tags?: Tags; pregapFrame?: number; pregap?: number; postgap?: number; index00?: number; index01?: number }>;
  tags?: Tags;
  warnings: string[];
};
export type Proposal = {
  tags?: Tags;
  rows: Array<{ position: string; title: string; performer?: string; durationSeconds?: number; frame?: number | null; side?: string }>;
  warnings: string[];
  spans?: Array<{ startIndex: number; endIndex: number; expectedEndFrame?: number | null; availableEndFrame?: number | null }>;
};
type DraftView = { playhead: number; start: number; end: number; selected: string; follow: boolean; stereo?: boolean };
export type Draft = { tracks: Track[]; tags: Tags; importedTags?: Tags; artworkPath: string; treatment?: AudioTreatment; view?: DraftView };
type StoredDraft = Draft & { frames: number; sampleRate: number; stamp?: string; version: 1 };
type Native = {
  core?: { invoke: (command: string, args: Record<string, unknown>) => Promise<unknown> };
  dialog?: {
    open: (options: Record<string, unknown>) => Promise<string | string[] | null>;
    save: (options: Record<string, unknown>) => Promise<string | null>;
  };
};

function native() {
  return (window as Window & { __TAURI__?: Native }).__TAURI__;
}

async function invokeNative<T>(command: string, args: Record<string, unknown>): Promise<T> {
  const core = native()?.core;
  if (!core) throw new Error('Audio tools are available in the TAGnetic desktop app.');
  try {
    return await core.invoke(command, args) as T;
  } catch (error) {
    if (typeof error === 'object' && error !== null && 'text' in error) throw new Error(String(error.text));
    if (error instanceof Error) throw error;
    throw new Error(String(error));
  }
}

export function audioCommand<T>(action: string, args: Record<string, unknown>) {
  return invokeNative<T>('audio_split_' + action, args);
}

const splitSessionKey = 'tagnetic.audio-split-session';
const splitAuditionKey = 'tagnetic.audio-split-audition';
let splitRecovery: Promise<void> | undefined;

function rememberedSplitSession(key: string) {
  try { return sessionStorage.getItem(key); }
  catch { return null; }
}

export function rememberAudioSplitSession(id: string, audition = false) {
  try { sessionStorage.setItem(audition ? splitAuditionKey : splitSessionKey, id); }
  catch { /* Storage can be unavailable in a restricted webview. */ }
}

export async function closeAudioSplitSession(id: string) {
  await audioCommand('close', { id });
  for (const key of [splitSessionKey, splitAuditionKey]) {
    if (rememberedSplitSession(key) === id) {
      try { sessionStorage.removeItem(key); }
      catch { /* Closing the same native session again is safe. */ }
    }
  }
}

export function recoverAudioSplitSession() {
  // A webview reload can leave the native preview running without its editor.
  if (!splitRecovery) {
    const ids = [rememberedSplitSession(splitAuditionKey), rememberedSplitSession(splitSessionKey)];
    splitRecovery = (async () => {
      for (const id of ids) if (id) await closeAudioSplitSession(id);
    })().catch(error => {
      splitRecovery = undefined;
      throw error;
    });
  }
  return splitRecovery;
}

type CoverImage = { mime: string; data_base64: string };

export async function recordingCover(path: string, replacement: string) {
  const image = replacement
    ? await invokeNative<CoverImage>('read_cover_image', { path: replacement })
    : (await invokeNative<{ samples: CoverImage[] }>('read_cover_summary', { paths: [path] })).samples[0];
  return image?.data_base64 ? `data:${image.mime};base64,${image.data_base64}` : '';
}

export function openDiscogsRelease(id: string) {
  return invokeNative('open_provider_page', { source: 'discogs', entity: 'release', id });
}

export async function choosePath(options: Record<string, unknown>, save = false) {
  const dialog = native()?.dialog;
  if (!dialog) throw new Error('The native file chooser is unavailable.');
  const selected = save ? await dialog.save(options) : await dialog.open({ multiple: false, ...options });
  return typeof selected === 'string' ? selected : null;
}

export const leaf = (path: string) => path.split(/[/\\]/).at(-1) || path;
export const stem = (path: string) => leaf(path).replace(/\.[^.]+$/, '');
export function directory(path: string) {
  const lastSeparator = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
  if (lastSeparator < 0) return '';
  const isRoot = lastSeparator === 0 || (lastSeparator === 2 && path[1] === ':');
  return path.slice(0, lastSeparator + Number(isRoot));
}

export function timeLabel(frame: number, sampleRate: number, decimals = 3) {
  const precision = 10 ** decimals;
  const units = Math.max(0, Math.round(frame * precision / sampleRate));
  const seconds = Math.floor(units / precision);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);
  const prefix = hours ? `${hours}:${String(minutes % 60).padStart(2, '0')}` : String(minutes);
  const fraction = String(units % precision).padStart(decimals, '0');
  return `${prefix}:${String(seconds % 60).padStart(2, '0')}${decimals ? '.' + fraction : ''}`;
}

export function frameAtTime(raw: string, rate: number) {
  const parts = raw.trim().split(':');
  if (parts.length > 3 || parts.some((part, index) => {
    const last = index === parts.length - 1;
    return !(last ? /^\d+(?:\.\d{1,9})?$/ : /^\d+$/).test(part) ||
      (index > 0 && Number(part) >= 60);
  })) return null;
  const seconds = parts.reduce((value, part) => value * 60 + Number(part), 0);
  const frame = Math.round(seconds * rate);
  return Number.isSafeInteger(frame) ? frame : null;
}

export function peakValues(value: number[] | ArrayBuffer | Uint8Array) {
  if (Array.isArray(value)) return Float32Array.from(value);
  const bytes = value instanceof ArrayBuffer ? value : Uint8Array.from(value).buffer;
  if (bytes.byteLength % Float32Array.BYTES_PER_ELEMENT) return new Float32Array();
  return new Float32Array(bytes);
}

const draftKey = (source: Source) => 'tagnetic.audio-split-draft:' + source.path;

export function saveDraft(source: Source, draft: Draft) {
  const saved: StoredDraft = {
    ...draft, frames: source.frames, sampleRate: source.sampleRate, stamp: source.sourceStamp, version: 1,
  };
  localStorage.setItem(draftKey(source), JSON.stringify(saved));
}

export function readDraft(source: Source) {
  const raw = localStorage.getItem(draftKey(source));
  if (!raw) return null;
  let draft: StoredDraft;
  try { draft = JSON.parse(raw); } catch { return null; }
  if (draft?.version !== 1 || !Array.isArray(draft.tracks) || !draft.tracks.length) return null;
  if (draft.frames !== source.frames || draft.sampleRate !== source.sampleRate ||
    draft.stamp !== source.sourceStamp) {
    throw new Error('The recording has changed since this draft was saved. Open a matching master to restore it.');
  }
  let last = -1;
  for (const track of draft.tracks) {
    if (!track || typeof track.id !== 'string' || !Number.isSafeInteger(track.startFrame) ||
      track.startFrame <= last || track.startFrame >= source.frames || !track.tags) return null;
    last = track.startFrame;
  }
  return draft;
}

export function hasDraft(source: Source) {
  try { return !!localStorage.getItem(draftKey(source)); } catch { return false; }
}
