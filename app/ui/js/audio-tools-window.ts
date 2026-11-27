import type { FrameRange } from './audio-split';
import type { AnalyzerView, TreatmentPlotMode, TreatmentProbe } from './audio-treatment-plot';
import { effectIsActive, eqBandIsActive, type AudioTreatment, type EqChannel, type SpectralRepair } from './audio-treatment';

export const toolModes = ['eq', 'dynamics', 'repair'] as const satisfies readonly TreatmentPlotMode[];
export const toolNames = { eq: 'EQ', dynamics: 'Dynamics', repair: 'Repair' } as const;
export const toolLabel = (kind: TreatmentPlotMode) => `audio-tool-${kind}`;
const commandEvent = 'tagnetic:audio-tools-command';
const stateEvent = 'tagnetic:audio-tools-state';
const probeEvent = 'tagnetic:audio-tools-probe';
const flushEvent = 'tagnetic:audio-tools-flush';
const visibilityEvent = 'tagnetic:audio-tool-visibility';

const toolKeys = {
  eq: ['rumble', 'rumbleHz', 'rumbleSlope', 'highCutHz', 'highCutSlope', 'bassDb', 'bassHz', 'presenceDb',
    'presenceHz', 'presenceQ', 'trebleDb', 'trebleHz', 'notchHz', 'notchQ', 'eqBands', 'eqBypassed', 'eqStrength', 'eqInputDb', 'eqOutputDb'],
  dynamics: ['compressorInputDb', 'compressorDb', 'compressorRatio', 'compressorAttackMs', 'compressorReleaseMs', 'compressorKneeDb',
    'gateDb', 'gateReductionDb', 'gainDb', 'normalize', 'normalizeTargetDb'],
  repair: ['hum', 'humHarmonics', 'dcHz', 'mono', 'swapChannels', 'polarity', 'balance', 'stereoWidth', 'spectralRepairs'],
} as const satisfies Record<TreatmentPlotMode, readonly (keyof AudioTreatment)[]>;

const toolEffects = {
  eq: ['rumble', 'bassDb', 'presenceDb', 'notchHz', 'trebleDb', 'highCutHz', 'eqInputDb', 'eqOutputDb'],
  dynamics: ['compressorDb', 'gateDb', 'gainDb', 'normalize'],
  repair: ['hum', 'dcHz', 'mono', 'swapChannels', 'polarity', 'balance', 'stereoWidth'],
} as const;

export function toolEffectCount(kind: TreatmentPlotMode, settings: AudioTreatment) {
  const preset = toolEffects[kind].filter(key => effectIsActive(settings, key)).length;
  if (kind === 'eq') return preset + (settings.eqBands?.filter(band => eqBandIsActive(band, settings.eqStrength)).length ?? 0);
  return kind === 'repair' ? preset + activeRepairCount(settings) : preset;
}

export function activeRepairCount(settings: AudioTreatment) {
  return settings.spectralRepairs?.filter(repair => repair && !repair.bypassed).length ?? 0;
}

export function toolPatch(kind: TreatmentPlotMode, patch: Partial<AudioTreatment>) {
  return Object.fromEntries(toolKeys[kind].filter(key => Object.prototype.hasOwnProperty.call(patch, key)).map(key => [key, patch[key]]));
}

const accentTokens = ['--accent', '--accent-text', '--accent-ink'] as const;
type ToolTheme = { theme?: string; palette?: string; font?: string; accent: string[] };

export function toolTheme(): ToolTheme {
  const root = document.documentElement;
  return { theme: root.dataset.theme, palette: root.dataset.palette, font: root.dataset.uiFont,
    accent: accentTokens.map(key => root.style.getPropertyValue(key)) };
}

export function applyToolTheme(theme: ToolTheme) {
  const root = document.documentElement;
  if (theme.theme) root.dataset.theme = theme.theme;
  if (theme.palette) root.dataset.palette = theme.palette;
  if (theme.font) root.dataset.uiFont = theme.font;
  accentTokens.forEach((key, index) => {
    const value = theme.accent[index];
    if (value) root.style.setProperty(key, value);
    else root.style.removeProperty(key);
  });
}

export type EqReference = { status: 'choosing' | 'analysing' | 'ready' | 'error'; generation?: number; name?: string; count?: number; preview?: boolean; message?: string };

export type ToolSnapshot = {
  sourceId: string;
  frame: number;
  sampleRate: number;
  frames: number;
  settings: AudioTreatment;
  stereo: boolean;
  playing: boolean;
  clockRunning?: boolean;
  repairPlayback?: 'buffering' | 'updating' | 'failed';
  original: boolean;
  loop: boolean;
  loopRange?: FrameRange;
  overview?: number[];
  stereoOverview?: number[];
  cuts?: number[];
  pinned?: boolean;
  levelMatch?: boolean;
  eqLevelComparison?: boolean;
  eqLearning?: { index: number; status: 'learning' | 'error'; message?: string };
  monitor?: 'band' | 'bypass-compressor' | 'difference-compressor';
  repairListen?: RepairListenState;
  undoLabel?: string;
  redoLabel?: string;
  reference?: EqReference;
  normalisation?: { gainDb: number; loudnessLufs: number | null };
  revision?: string;
  theme: ToolTheme;
};
export type ToolStateUpdate = Omit<ToolSnapshot, 'settings'> & { settings?: AudioTreatment };

export type SpectrogramViewOptions = {
  overview?: boolean;
  treated?: boolean;
  difference?: boolean;
  channel?: 'combined' | 'left' | 'right' | 'stereo';
  detail?: 'balanced' | 'transients' | 'tones';
};
export type PreviewBand = { lowHz: number; highHz: number; channel?: Exclude<EqChannel, 'stereo'> };
export type RepairListenState = {
  signal: 'original' | 'repaired' | 'removed';
  channel: 'stereo' | 'left' | 'right';
  trial?: 'light' | 'full';
};
export type RepairLoopPreferences = { smooth: boolean; returnTo: 'hold' | 'loop' | 'start' };
export type RepairListenCommand =
  | (RepairLoopPreferences & { type: 'loop'; range: FrameRange; band?: PreviewBand; preservePosition?: boolean; play?: boolean })
  | (RepairLoopPreferences & { type: 'preferences' })
  | { type: 'signal'; signal: RepairListenState['signal'] }
  | { type: 'channel'; channel: RepairListenState['channel'] }
  | { type: 'trial'; strength: NonNullable<RepairListenState['trial']>; candidate: SpectralRepair; slot?: number }
  | { type: 'stop'; returnTo?: RepairLoopPreferences['returnTo'] }
  | { type: 'all' };
type DirectToolAction = 'play' | 'hide' | 'pin' | 'level-match' | 'seek' | 'solo-band' | 'retune-solo-band' | 'audition-band'
  | 'bypass-compressor' | 'difference-compressor' | 'stop-audition'
  | 'reference-start' | 'reference-preview' | 'reference-keep' | 'reference-cancel';
export type ToolRequest = { kind: TreatmentPlotMode } & (
  | { action: 'sync' }
  | { action: 'learn-threshold'; sourceId: string; index: number }
  | { action: 'repair-listen'; sourceId: string; command: RepairListenCommand }
  | { action: 'analyzer'; sourceId: string; enabled: boolean; view?: AnalyzerView }
  | { action: DirectToolAction; sourceId: string; range?: FrameRange; band?: PreviewBand }
  | { action: 'undo' | 'redo'; sourceId: string; revision: string; trial?: number }
  | { action: 'compare'; sourceId: string; original: boolean }
  | { action: 'loop'; sourceId: string; range?: FrameRange }
  | { action: 'viewport'; sourceId: string; range: FrameRange | null; options?: SpectrogramViewOptions }
  | { action: 'edit'; sourceId: string; patch: Partial<AudioTreatment>; revision: string; gesture?: string; trial?: number;
      compareEq?: Partial<AudioTreatment> | null }
  | { action: 'flushed'; sourceId: string; fence: string }
);

export type ToolAction = DirectToolAction | 'original' | 'treated' | 'loop' | 'undo' | 'redo';
export type ToolProbeUpdate = { sourceId: string; probe?: TreatmentProbe; error?: string };
export type ToolVisibility = { kind: TreatmentPlotMode; visible: boolean };

type Events = {
  listen<T>(event: string, handler: (event: { payload: T }) => void): Promise<() => void>;
  emitTo(target: string, event: string, payload: unknown): Promise<void>;
};

function events() {
  return (window as Window & { __TAURI__?: { event?: Events } }).__TAURI__?.event;
}

export function listenToolRequests(handler: (request: ToolRequest) => void) {
  return events()?.listen<ToolRequest>(commandEvent, ({ payload }) => handler(payload));
}

export function listenToolState(handler: (state: ToolStateUpdate | null) => void) {
  return events()?.listen<ToolStateUpdate | null>(stateEvent, ({ payload }) => handler(payload));
}

export function listenToolVisibility(handler: (visibility: ToolVisibility) => void) {
  return events()?.listen<ToolVisibility>(visibilityEvent, ({ payload }) => handler(payload));
}

export function listenToolProbe(handler: (update: ToolProbeUpdate) => void) {
  return events()?.listen<ToolProbeUpdate>(probeEvent, ({ payload }) => handler(payload));
}

export function listenToolFlush(handler: (fence: string) => void) {
  return events()?.listen<string>(flushEvent, ({ payload }) => handler(payload));
}

export function sendToolRequest(request: ToolRequest) {
  return events()?.emitTo('main', commandEvent, request);
}

export function sendToolState(kind: TreatmentPlotMode, state: ToolStateUpdate | null) {
  return events()?.emitTo(toolLabel(kind), stateEvent, state);
}

export function sendToolProbe(kind: TreatmentPlotMode, update: ToolProbeUpdate) {
  return events()?.emitTo(toolLabel(kind), probeEvent, update);
}

export function requestToolFlush(kind: TreatmentPlotMode, fence: string) {
  return events()?.emitTo(toolLabel(kind), flushEvent, fence);
}
