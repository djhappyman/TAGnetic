<script lang="ts">
  import { onMount } from 'svelte';
  import AudioSplitToolWindow from './AudioSplitToolWindow.svelte';
  import ControllerTooltip from './ControllerTooltip.svelte';
  import type { FrameRange } from '../js/audio-split';
  import { readTreatment, sameTreatment, type AudioTreatment } from '../js/audio-treatment';
  import type { AnalyzerView, TreatmentPlotMode, TreatmentProbe } from '../js/audio-treatment-plot';
  import {
    applyToolTheme, listenToolFlush, listenToolProbe, listenToolState, sendToolRequest, toolPatch,
    type PreviewBand, type SpectrogramViewOptions, type ToolAction, type ToolProbeUpdate, type ToolRequest, type ToolSnapshot,
  } from '../js/audio-tools-window';

  let { kind }: { kind: TreatmentPlotMode } = $props();
  let snapshot = $state.raw<ToolSnapshot | null>(null);
  let probe = $state.raw<TreatmentProbe | null>(null);
  let connectionError = $state('');
  let probeError = $state('');
  let received: ToolSnapshot | null = null;
  let earlyProbe: ToolProbeUpdate | undefined;
  let active = false;
  let lastTheme = '';
  const revisionPrefix = Math.random().toString(36).slice(2);
  let revision = 0;
  type Mutation = Extract<ToolRequest, { action: 'edit' | 'undo' | 'redo' }>;
  type QueuedRequest = Mutation | Extract<ToolRequest, { action: 'learn-threshold' }>;
  let sending: Mutation | undefined;
  const queued: QueuedRequest[] = [];
  let requestedFence: string | undefined;

  function send(request: ToolRequest) {
    const promise = sendToolRequest(request);
    if (!promise) connectionError = 'Open audio tools from TAGnetic.';
    return promise;
  }

  function flush() {
    if (sending) return;
    const request = queued.shift();
    if (!request) {
      if (requestedFence && snapshot) {
        const fence = requestedFence;
        requestedFence = undefined;
        void send({ kind, action: 'flushed', sourceId: snapshot.sourceId, fence })?.catch(reason => { connectionError = String(reason); });
      }
      return;
    }
    if (request.action === 'learn-threshold') {
      void send(request)?.catch(reason => { connectionError = String(reason); });
      flush();
      return;
    }
    sending = request;
    void send(request)?.catch(reason => {
      if (sending === request) {
        sending = undefined;
        connectionError = String(reason);
        flush();
      }
    });
  }

  function edit(patch: Partial<AudioTreatment>, gesture?: string, compareEq?: Partial<AudioTreatment> | null) {
    if (!snapshot) return;
    const accepted = toolPatch(kind, patch);
    const settings = readTreatment({ ...snapshot.settings, ...accepted });
    if (sameTreatment(snapshot.settings, settings) && compareEq === undefined) return;
    snapshot = { ...snapshot, settings };
    const trial = snapshot.reference?.preview ? snapshot.reference.generation : undefined;
    const tail = queued.at(-1);
    if (gesture && tail?.action === 'edit' && tail.gesture === gesture && tail.trial === trial) {
      Object.assign(tail.patch, accepted);
    } else {
      queued.push({
        kind, action: 'edit', sourceId: snapshot.sourceId, patch: accepted, revision: `${revisionPrefix}-${++revision}`, gesture, trial, compareEq,
      });
    }
    flush();
  }

  function action(action: ToolAction, range?: FrameRange, band?: PreviewBand) {
    if (!snapshot) return;
    const { sourceId } = snapshot;
    if (action === 'undo' || action === 'redo') {
      const trial = snapshot.reference?.preview ? snapshot.reference.generation : undefined;
      queued.push({ kind, action, sourceId, revision: `${revisionPrefix}-${++revision}`, trial });
      flush();
      return;
    }
    const message: ToolRequest = action === 'loop' ? { kind, action, sourceId, range }
      : action === 'original' || action === 'treated' ? { kind, action: 'compare', sourceId, original: action === 'original' }
      : { kind, action, sourceId, range, band };
    void send(message)?.catch(reason => { connectionError = String(reason); });
  }

  function learnThreshold(index: number) {
    if (!snapshot || kind !== 'eq') return;
    queued.push({ kind, sourceId: snapshot.sourceId, action: 'learn-threshold', index });
    flush();
  }

  function analyzer(view: AnalyzerView) {
    if (snapshot) void send({ kind, action: 'analyzer', sourceId: snapshot.sourceId, enabled: view !== 'off', view })?.catch(reason => { connectionError = String(reason); });
  }

  function viewport(range: FrameRange | null, options?: SpectrogramViewOptions) {
    if (snapshot) void send({ kind, action: 'viewport', sourceId: snapshot.sourceId, range, options })?.catch(reason => { connectionError = String(reason); });
  }

  function acceptProbe(update: ToolProbeUpdate) {
    if (!active) return;
    if (update.probe && kind === 'repair' && !update.probe.spectrogram?.length) return;
    if (received?.sourceId !== update.sourceId) {
      earlyProbe = update;
      return;
    }
    probe = update.probe ?? probe;
    probeError = update.error ?? '';
  }

  function accept(state: ToolSnapshot | null) {
    if (!active) return;
    if (state?.sourceId !== received?.sourceId) {
      sending = undefined;
      queued.length = 0;
      requestedFence = undefined;
      probe = null;
      probeError = '';
    }
    received = state;
    if (!state) { snapshot = null; return; }
    if (earlyProbe?.sourceId === state.sourceId) {
      acceptProbe(earlyProbe);
      earlyProbe = undefined;
    }
    connectionError = '';
    const theme = JSON.stringify(state.theme);
    const appearanceChanged = lastTheme !== theme;
    if (appearanceChanged) {
      lastTheme = theme;
      applyToolTheme(state.theme);
    }
    if (sending && state.revision === sending.revision) {
      sending = undefined;
      flush();
    }
    let settings = state.settings;
    const trial = state.reference?.preview ? state.reference.generation : undefined;
    for (const request of [sending, ...queued]) {
      if (request?.action === 'edit' && request.trial === trial) settings = { ...settings, ...request.patch };
    }
    snapshot = {
      ...state,
      overview: state.overview ?? (snapshot?.sourceId === state.sourceId ? snapshot.overview : undefined),
      stereoOverview: state.stereoOverview ?? (snapshot?.sourceId === state.sourceId ? snapshot.stereoOverview : undefined),
      cuts: state.cuts ?? (snapshot?.sourceId === state.sourceId ? snapshot.cuts : undefined),
      settings: snapshot && sameTreatment(snapshot.settings, settings) ? snapshot.settings : settings,
      theme: appearanceChanged ? state.theme : snapshot?.theme || state.theme,
    };
  }

  function sync() {
    if (active) void send({ kind, action: 'sync' })?.catch(reason => { connectionError = String(reason); });
  }

  onMount(() => {
    active = true;
    const unlisten: (() => void)[] = [];
    const listeners = [listenToolState(accept), listenToolProbe(acceptProbe), listenToolFlush(fence => {
      requestedFence = fence;
      flush();
    })];
    if (listeners.every(Boolean)) {
      void Promise.all(listeners.map(pending => pending?.then(stop => {
        if (active) unlisten.push(stop);
        else stop();
      }))).then(sync).catch(reason => { connectionError = String(reason); });
    } else connectionError = 'Open audio tools from TAGnetic.';
    return () => {
      active = false;
      unlisten.forEach(stop => stop());
    };
  });
</script>

<svelte:window onfocus={sync}/>
<svelte:document onvisibilitychange={() => { if (document.visibilityState === 'visible') sync(); }}/>
<ControllerTooltip/>
<main class="tool-host">
  {#if connectionError || probeError}
    <p class="problem" role="status">{connectionError || probeError}</p>
  {/if}
  {#key snapshot?.sourceId}
    <AudioSplitToolWindow {kind} {snapshot} {probe} onedit={edit} onaction={action} onviewport={viewport} onanalyzer={analyzer} onlearn={learnThreshold}/>
  {/key}
</main>

<style>
  :global(body) { height:auto; min-height:100vh; overflow:auto; margin:0; background:var(--bg); color:var(--text); }
  .tool-host { min-height:100vh; }
  .problem { margin:0; padding:var(--sp-2) var(--sp-3); color:var(--muted); font-size:var(--text-xs); }
</style>
