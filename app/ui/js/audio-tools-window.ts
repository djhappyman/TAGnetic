import type { FrameRange } from './audio-split';
import type { TreatmentPlotMode, TreatmentProbe } from './audio-treatment-plot';
import type { AudioTreatment } from './audio-treatment';

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
    'presenceHz', 'presenceQ', 'trebleDb', 'trebleHz', 'notchHz', 'notchQ'],
  dynamics: ['compressorDb', 'compressorRatio', 'gateDb', 'gateReductionDb', 'gainDb', 'normalize', 'normalizeTargetDb'],
  repair: ['hum', 'humHarmonics', 'dcHz', 'mono', 'swapChannels', 'polarity', 'balance', 'stereoWidth', 'spectralRepairs'],
} as const satisfies Record<TreatmentPlotMode, readonly (keyof AudioTreatment)[]>;

const inactiveSettings: readonly (keyof AudioTreatment)[] = [
  'rumbleHz', 'rumbleSlope', 'highCutSlope', 'bassHz', 'presenceHz', 'presenceQ', 'trebleHz', 'notchQ',
  'compressorRatio', 'gateReductionDb', 'normalizeTargetDb', 'humHarmonics',
];

export function toolIsActive(kind: TreatmentPlotMode, settings: AudioTreatment) {
  return toolKeys[kind].some(key => !inactiveSettings.includes(key) && !!settings[key] && settings[key] !== 'off');
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

export type ToolSnapshot = {
  sourceId: string;
  frame: number;
  sampleRate: number;
  frames: number;
  settings: AudioTreatment;
  stereo: boolean;
  playing: boolean;
  original: boolean;
  loop: boolean;
  revision?: string;
  theme: ToolTheme;
};

export type ToolRequest = { kind: TreatmentPlotMode } & (
  | { action: 'sync' }
  | { action: 'play'; sourceId: string }
  | { action: 'undo' | 'redo'; sourceId: string; revision: string }
  | { action: 'compare'; sourceId: string; original: boolean }
  | { action: 'loop'; sourceId: string; range?: FrameRange }
  | { action: 'viewport'; sourceId: string; range: FrameRange | null }
  | { action: 'edit'; sourceId: string; patch: Partial<AudioTreatment>; revision: string; gesture?: string }
  | { action: 'flushed'; sourceId: string; fence: string }
);

export type ToolAction = 'play' | 'original' | 'treated' | 'loop' | 'undo' | 'redo';
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

export function listenToolState(handler: (state: ToolSnapshot | null) => void) {
  return events()?.listen<ToolSnapshot | null>(stateEvent, ({ payload }) => handler(payload));
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

export function sendToolState(kind: TreatmentPlotMode, state: ToolSnapshot | null) {
  return events()?.emitTo(toolLabel(kind), stateEvent, state);
}

export function sendToolProbe(kind: TreatmentPlotMode, update: ToolProbeUpdate) {
  return events()?.emitTo(toolLabel(kind), probeEvent, update);
}

export function requestToolFlush(kind: TreatmentPlotMode, fence: string) {
  return events()?.emitTo(toolLabel(kind), flushEvent, fence);
}
