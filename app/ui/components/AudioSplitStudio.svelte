<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { Activity, AudioLines, Check, ChevronDown, ChevronLeft, ChevronRight, CircleStop, Disc3, Ellipsis, FilePlus2, FolderOpen, ImagePlus, Pause, Pencil, Play, Redo2, Repeat, RotateCcw, Scissors, SlidersHorizontal, Undo2, Wrench, X } from '@lucide/svelte';
  import { Button as SdkButton } from '@ui-sdk/svelte/button';
  import { Input } from '@ui-sdk/svelte/input';
  import { Textarea } from '@ui-sdk/svelte/textarea';
  import { createMenuController, Menu, MenuCheckboxItem, MenuContent, MenuItem, MenuItemIndicator, MenuSeparator, MenuShortcut, MenuTrigger } from '@ui-sdk/svelte/menu';
  import { Progress as ProgressBar, ProgressIndicator, ProgressTrack } from '@ui-sdk/svelte/progress';
  import * as Dialog from '$lib/components/ui/dialog/index.js';
  import { buttonVariants } from '$lib/components/ui/button/button.svelte';
  import Button from './AudioSplitButton.svelte';
  import { attachGenrePicker } from '$lib/genre-picker';
  import { savedSettings, tracks as libraryTracks } from '../js/state.js';
  import AudioSplitWaveform from './AudioSplitWaveform.svelte';
  import AudioSplitOutput from './AudioSplitOutput.svelte';
  import AudioSplitImport from './AudioSplitImport.svelte';
  import {
    activeRepairCount, listenToolRequests, listenToolVisibility, requestToolFlush, sameSpectrogramFocus, sendRepairMeter, sendToolProbe, sendToolState,
    toolEffectCount, toolModes, toolNames, toolPatch, toolTheme,
    type EqReference, type PreviewBand, type RepairListenCommand, type RepairListenState, type RepairLoopPreferences,
    type SpectrogramViewOptions, type ToolRequest, type ToolSnapshot, type ToolVisibility,
  } from '../js/audio-tools-window';
  import { maxSpectralRepairs, type EqBand, type SpectralRepair } from '../js/audio-treatment';
  import type { TreatmentProbe } from '../js/audio-treatment-plot';
  import { copyTreatmentSettings, portableTreatment, readCopiedTreatment, readTreatmentRecipes,
    saveTreatmentRecipes, treatmentForRecording, type TreatmentRecipe } from '../js/audio-treatment-recipes';
  import {
    activeEffectLabels, audioCommand, choosePath, closeAudioSplitSession, defaultSplitFilePattern, frameAtTime, hasDraft, hasTreatment, leaf, noTreatment, peakValues, readDraft, readTreatment, sameTreatment,
    recordingCover, recoverAudioSplitSession, rememberAudioSplitSession, saveDraft, stem, timeLabel,
    type AudioTreatment, type TreatmentAnalysis, type Cue, type Draft, type ExportResult, type Format, type FrameRange, type Launch, type Loading, type PeakTile, type Progress, type ReplacementUndo, type Review, type SaveMode, type Source, type SplitMethod, type Status, type Tags, type Track,
  } from '../js/audio-split';

  let { launch, onclose }: { launch: Launch; onclose: () => void } = $props();
  const split = $derived(launch.mode === 'split');
  const title = $derived(split ? 'Split recording' : 'Convert audio');
  const maxFiles = 200;
  const recordingFields = [
    ['album', 'Album'], ['albumartist', 'Album Artist'], ['year', 'Year'], ['genre', 'Genre'], ['comment', 'Comment'],
  ] as const;
  const trackFields = [['artist', 'Artist'], ['title', 'Title']] as const;
  type TrackEdit = { id: string; field: 'tags' | 'start' };
  let editor = $state.raw<TrackEdit | null>(null);
  let trackMenuId = $state('');
  let source = $state.raw<Source | null>(null);
  let tracks = $state.raw<Track[]>([]);
  const toolCuts = $derived(tracks.map(track => track.startFrame));
  const editingTrack = $derived(tracks.find(track => track.id === editor?.id));
  type HistoryStep = { draft: Draft; label: string };
  let previous = $state.raw<HistoryStep[]>([]);
  let next = $state.raw<HistoryStep[]>([]);
  let tags = $state.raw<Tags>({});
  let importedTags = $state.raw<Tags>({});
  let artworkPath = $state('');
  const coverPreview = $derived(source ? recordingCover(source.path, artworkPath).catch(() => '') : '');
  let hasSaved = $state(false);
  let dirty = $state(false);
  let loading = $state(false);
  let loadingInfo = $state.raw<Loading | null>(null);
  let openingRequest = '';
  let openingPoll: ReturnType<typeof setTimeout>;
  let phase = $state<'edit' | 'review' | 'export' | 'finished'>('edit');
  let working = $state(false);
  let previewing = $state(false);
  let reviewRequested = false;
  let stopping = $state(false);
  let closeRequested = $state(false);
  let closeSaveError = $state('');
  let closeTrigger: HTMLElement | null = null;
  let error = $state('');
  let notice = $state('');
  let draftAnnouncement = $state('');
  let showHelp = $state(false);
  let recipesOpen = $state(false);
  let recipes = $state.raw<TreatmentRecipe[]>([]);
  let recipeName = $state('');
  let recipeError = $state('');
  let copiedTreatment = $state.raw<AudioTreatment | null>(null);
  let waveformMenuOpen = $state(false);
  const studioMenus = { waveform: createMenuController(), imports: createMenuController(), tracks: createMenuController(), cover: createMenuController() };
  const audioToolIcons = { eq: SlidersHorizontal, dynamics: Activity, repair: Wrench };
  type Tool = typeof toolModes[number];
  const toolRevisions: Partial<Record<Tool, string>> = {};
  let toolGesture: string | undefined;
  let gestureCheckpoint: { undo: HistoryStep[]; redo: HistoryStep[]; dirty: boolean } | undefined;
  let usedToolWindows = false;
  let toolWindowError = $state('');
  const visibleTools = new Set<Tool>();
  const connectedTools = new Set<Tool>();
  const sentToolStates: Partial<Record<Tool, ToolSnapshot | null>> = {};
  const toolPins: Partial<Record<Tool, boolean>> = {};
  let activeToolTheme: ToolSnapshot['theme'] | undefined;
  let toolTimer: ReturnType<typeof setInterval> | undefined;
  let meterTimer: ReturnType<typeof setInterval> | undefined;
  let repairMeterPending = false;
  let repairMeterRetryAt = 0;
  let repairMeterRead = { sequence: -1, at: 0 };
  const liveMeterRate = 30;
  type ProbeLane = { pending?: string; retryAt: number; latest?: { key: string; sourceId: string; probe: TreatmentProbe } };
  let eqAnalyzerView: 'before' | 'after' | 'both' | 'off' = 'both';
  let reference: EqReference | undefined;
  let referenceCandidate: AudioTreatment | undefined;
  let referenceGeneration = 0;
  let referenceStarted: number | undefined;
  let referenceUndo: AudioTreatment[] = [];
  let referenceRedo: AudioTreatment[] = [];
  let referenceGesture: string | undefined;
  let referenceRedoBeforeGesture: AudioTreatment[] = [];
  const meterLane: ProbeLane = { retryAt: 0 };
  const repairLane: ProbeLane = { retryAt: 0 };
  let repairViewport: FrameRange | null = null;
  type RepairProbeView = Required<Pick<SpectrogramViewOptions, 'treated' | 'difference' | 'channel' | 'detail'>>
    & Pick<SpectrogramViewOptions, 'focus'>;
  const defaultRepairView: Readonly<RepairProbeView> = {
    treated: true, difference: false, channel: 'combined', detail: 'balanced',
  };
  let repairView = defaultRepairView;
  let repairListen: RepairListenState | undefined;
  type RepairTrial = { edit: SpectralRepair; slot?: number };
  let repairTrial: RepairTrial | undefined;
  let repairTrialCache: { treatment: AudioTreatment; trial: RepairTrial; preview: AudioTreatment } | undefined;
  const defaultRepairLoop: RepairLoopPreferences = { smooth: false, returnTo: 'hold' };
  let repairLoopPreferences = defaultRepairLoop;
  let repairAudition: (RepairLoopPreferences & { origin: number; range: FrameRange; selection?: SpectralRepair }) | undefined;
  let repairPick: SpectralRepair | undefined;
  let repeatSelection = false;
  let toolFence: { id: string; remaining: Set<Tool>; finish: (ready: boolean) => void } | undefined;
  let fenceSerial = 0;
  let settlingTools = $state(false);
  let treatment = $state.raw<AudioTreatment>(noTreatment);
  let treatmentAnalysis = $state.raw<TreatmentAnalysis | null>(null);
  let treatmentError = $state('');
  let listeningError = $state('');
  let retunePending = $state(false);
  let hearOriginal = $state(false);
  type Monitor = { owner: Tool; kind: NonNullable<ToolSnapshot['monitor']>; band?: PreviewBand; started?: number };
  let monitor: Monitor | undefined;
  let levelMatch = readLevelMatch();
  type ComparisonLevels = Pick<TreatmentProbe, 'input' | 'output'>;
  const noComparisonGain = { input: 0, output: 0 };
  let comparisonGain = noComparisonGain;
  let comparisonAlternate: Partial<AudioTreatment> | undefined;
  let comparisonCache: { sourceId: string; frame: number;
    readings: { settings: AudioTreatment; levels: ComparisonLevels }[] } | undefined;
  let compareRevision = 0;
  let comparisonReady = false;
  let comparisonInFlight = 0;
  let comparisonUsesProbe = false;
  let comparisonTimer: ReturnType<typeof setTimeout> | undefined;
  let treatmentVersion = 0;
  let retuneVersion = 0;
  let retuneTimer: ReturnType<typeof setTimeout> | undefined;
  let releaseSelectionRange = false;
  let lastRetune = -Infinity;
  let treatmentTimer: ReturnType<typeof setTimeout>;
  let measuringRevision = 0;
  let selected = $state('');
  let rangeAnchor = '';
  let rangeIds = $state.raw<string[]>([]);
  let playhead = $state(0);
  let playing = $state(false);
  let loopRegion = $state.raw<FrameRange | null>(null);
  let loopEnabled = $state(false);
  let buffering = $state(false);
  let follow = $state(false);
  let start = $state(0);
  let end = $state(1);
  let overview = $state.raw(new Float32Array());
  let stereoOverview = $state.raw(new Float32Array());
  let overviewRms = $state.raw(new Float32Array());
  let stereoOverviewRms = $state.raw(new Float32Array());
  let stereoView = $state(false);
  const hasStereo = $derived(stereoOverview.length > 0);
  let tile = $state.raw<PeakTile | null>(null);
  let saveMode = $state<SaveMode>('beside');
  let destination = $state('');
  let backupDirectory = $state('');
  let createFolder = $state(false);
  let folderPattern = $state('');
  let namePattern = $state('');
  let format = $state<Format>('mp3');
  let splitMethod = $state<SplitMethod>('copy');
  let bitrate = $state(192);
  let review = $state.raw<Review | null>(null);
  let lastReview = $state.raw<Review | null>(null);
  let outputAudition = $state<number | null>(null);
  let outputClosing: Promise<void> | undefined;
  let outputPoll: ReturnType<typeof setTimeout>;
  let outputCheck: ReturnType<typeof setTimeout>;
  let checkingOutput = $state(false);
  let progress = $state.raw<Progress | null>(null);
  let finished = $state.raw<string[]>([]);
  let backups = $state.raw<string[]>([]);
  let restored = $state.raw<string[]>([]);
  let exportWarnings = $state.raw<string[]>([]);
  let publishedId = $state('');
  let canUndoReplacement = $state(false);
  let libraryRemoved = $state.raw<string[]>();
  let canRefreshLibrary = $state(false);
  let refreshingLibrary = $state(false);
  let importMenuOpen = $state(false);
  let importMode = $state<'cue' | 'discogs' | null>(null);
  let cue = $state.raw<Cue>();
  let pendingCue = $state('');
  let cuePath = $state('');
  let cueEncoding = $state<'utf8' | 'windows1252'>('utf8');
  let cueLoading = $state(false);
  function focusHeader(node: HTMLHeadingElement) {
    requestAnimationFrame(() => { if (node.isConnected && !node.closest('[data-audio-studio]')?.contains(document.activeElement)) node.focus(); });
  }
  let list = $state<HTMLDivElement>();
  let serial = 0;
  let active = true;
  let frameClock = { frame: 0, observed: 0 };
  let animation = 0;
  let playheadAnimation = 0;
  let playheadTravel = $state<{ from: number; progress: number } | null>(null);
  const visualPlayhead = $derived(playheadTravel
    ? playheadTravel.from + (playhead - playheadTravel.from) * playheadTravel.progress : playhead);
  let viewportAnimation = 0;
  let viewportGoal: { first: number; range: number } | null = null;
  let viewportMotion = { first: 0, range: 1, time: 0 };
  let waveformDragging = false;
  let pollTimer: ReturnType<typeof setTimeout>;
  let peaksTimer: ReturnType<typeof setTimeout>;
  let generation = 0;
  let transportVersion = 0;
  let seekInFlight = false;
  let queuedSeek: { frame: number; sequence: number } | null = null;
  let scrubbing = false;

  const currentIndex = $derived(tracks.findIndex(track => track.id === selected));
  const current = $derived(tracks[currentIndex]);
  const menuTrackIndex = $derived(tracks.findIndex(track => track.id === trackMenuId));
  const canMerge = $derived(rangeIds.length > 1 && rangeIds.includes(trackMenuId));
  const included = $derived(tracks.filter(track => track.include).length);
  const sampleRate = $derived(source?.sampleRate || 44100);
  const total = $derived(source?.frames || 0);
  const duration = $derived(timeLabel(total, sampleRate, 1));
  const readout = $derived(playing && !buffering ? timeLabel(Math.floor(playhead * 10 / sampleRate) * sampleRate / 10, sampleRate, 1) : timeLabel(playhead, sampleRate));
  const canEdit = $derived(!!source && !working && !loading && phase === 'edit');
  const treated = $derived(hasTreatment(treatment));
  const effects = $derived(activeEffectLabels(treatment));
  const canCopy = $derived(split && !treated && source?.originalFormat === format);
  const copying = $derived(canCopy && splitMethod === 'copy');
  const canEnterReview = $derived(split ? !!source && included > 0 : launch.paths.length > 0 && launch.paths.length <= maxFiles);
  const reviewReady = $derived(canEnterReview && (saveMode !== 'folder' || !!destination.trim()) &&
    (!createFolder || !!folderPattern.trim()));
  const reviewingOutput = $derived(previewing || checkingOutput);
  const note = (message: string) => { notice = message; };
  const fail = (message: string) => { error = message.replace(/^Error: /, ''); };
  const newId = () => `segment-${++serial}`;
  const initial = (): Track => ({ id: newId(), startFrame: 0, include: true, tags: {} });

  const draft = (): Draft => ({ tracks, tags, importedTags, artworkPath, treatment });

  function capture(label = 'draft change') {
    toolGesture = undefined;
    gestureCheckpoint = undefined;
    previous = [...previous.slice(-49), { draft: draft(), label }];
    next = [];
    dirty = true;
    invalidate();
  }

  function restore(nextDraft: Draft) {
    toolGesture = undefined;
    gestureCheckpoint = undefined;
    tracks = nextDraft.tracks;
    rangeIds = [];
    rangeAnchor = '';
    tags = nextDraft.tags;
    importedTags = nextDraft.importedTags || {};
    artworkPath = nextDraft.artworkPath;
    const settings = readTreatment(nextDraft.treatment);
    replaceTreatment(hasStereo ? settings : { ...settings, mono: 'off', balance: 0, stereoWidth: 0, swapChannels: false, polarity: 'off' });
    if (!tracks.some(track => track.id === selected)) selected = tracks[0]?.id || '';
    invalidate();
  }

  function history(redo: boolean) {
    const from = redo ? next : previous;
    const change = from.at(-1);
    if (!change || !canEdit) return;
    if (redo) {
      next = next.slice(0, -1);
      previous = [...previous, { draft: draft(), label: change.label }];
    } else {
      previous = previous.slice(0, -1);
      next = [...next, { draft: draft(), label: change.label }];
    }
    restore(change.draft);
    dirty = true;
    draftAnnouncement = (redo ? 'Restored ' : 'Undid ') + change.label;
  }

  function discardReview(plan: Review) {
    void audioCommand('cancel', { id: plan.id }).catch(() => {});
  }

  function invalidate() {
    if (phase === 'export' || phase === 'finished') return;
    generation++;
    stopOutputAudition();
    if (review) discardReview(review);
    review = null;
    error = '';
  }

  onMount(() => {
    const unlisten: (() => void)[] = [];
    for (const listener of [listenToolRequests(handleToolRequest), listenToolVisibility(updateToolVisibility)]) {
      void listener?.then(stop => {
        if (active) unlisten.push(stop);
        else stop();
      }).catch(reason => { toolWindowError = String(reason); });
    }
    document.addEventListener('tagrex:appearance', syncToolAppearance);
    document.addEventListener('tagrex:interfacefont', syncToolAppearance);
    if (split) namePattern = defaultSplitFilePattern;
    if (split && launch.paths.length === 1) void openRecording(launch.paths[0]);
    if (!split && launch.paths.length && launch.paths.every(path => /\.mp3$/i.test(path))) format = 'flac';
    return () => {
      active = false;
      unlisten.forEach(stop => stop());
      document.removeEventListener('tagrex:appearance', syncToolAppearance);
      document.removeEventListener('tagrex:interfacefont', syncToolAppearance);
      hideToolWindows();
      toolFence?.finish(false);
      clearTimeout(outputCheck);
      stopOutputAudition();
      if (review && phase !== 'export') discardReview(review);
      clearTimeout(pollTimer);
      clearTimeout(peaksTimer);
      clearTimeout(openingPoll);
      clearTimeout(treatmentTimer);
      clearTimeout(comparisonTimer);
      if (openingRequest) void audioCommand('cancel_open', { requestId: openingRequest }).catch(() => {});
      cancelAnimationFrame(animation);
      cancelAnimationFrame(viewportAnimation);
      cancelAnimationFrame(playheadAnimation);
      clearTimeout(retuneTimer);
      if (source) void closeAudioSplitSession(source.id).catch(console.error);
    };
  });

  async function openRecording(path: string) {
    const requestId = crypto.getRandomValues(new Uint32Array(4)).join('-');
    if (openingRequest) void audioCommand('cancel_open', { requestId: openingRequest }).catch(() => {});
    clearTimeout(openingPoll);
    openingRequest = requestId;
    loading = true;
    loadingInfo = null;
    error = '';
    async function pollOpening() {
      if (!active || openingRequest !== requestId) return;
      try {
        const state = await audioCommand<Loading | null>('loading', { requestId });
        if (!active || openingRequest !== requestId) return;
        loadingInfo = state;
      } catch { return; }
      if (active && openingRequest === requestId) openingPoll = setTimeout(() => void pollOpening(), 250);
    }
    try {
      await recoverAudioSplitSession();
      if (!active || openingRequest !== requestId) return;
      const opening = audioCommand<Source>('open', { path, requestId });
      openingPoll = setTimeout(() => void pollOpening(), 250);
      const opened = await opening;
      if (!active || openingRequest !== requestId) { await closeAudioSplitSession(opened.id); return; }
      if (!Number.isSafeInteger(opened.frames) || opened.frames < 1 || !opened.sampleRate) {
        await closeAudioSplitSession(opened.id);
        throw new Error('This recording has no readable timeline.');
      }
      if (source) await closeAudioSplitSession(source.id);
      clearReference(false);
      comparisonAlternate = undefined;
      comparisonCache = undefined;
      comparisonGain = noComparisonGain;
      comparisonReady = false;
      source = opened;
      rememberAudioSplitSession(opened.id);
      format = opened.originalFormat || 'mp3';
      splitMethod = 'copy';
      if (opened.sampleRate < 32000 && bitrate > 160) bitrate = 128;
      tracks = [initial()];
      selected = tracks[0].id;
      rangeIds = [];
      rangeAnchor = '';
      loopRegion = null;
      loopEnabled = false;
      repairListen = repairTrial = repairAudition = undefined;
      repairPick = undefined;
      repairMeterRetryAt = 0;
      repairMeterRead = { sequence: -1, at: 0 };
      repairTrialCache = undefined;
      repairLoopPreferences = defaultRepairLoop;
      repeatSelection = false;
      previous = [];
      next = [];
      tags = {};
      importedTags = {};
      artworkPath = '';
      treatment = noTreatment;
      eqLearning = undefined;
      eqLearnGeneration++;
      treatmentAnalysis = null;
      treatmentError = '';
      listeningError = '';
      retunePending = false;
      hearOriginal = false;
      toolGesture = undefined;
      toolWindowError = '';
      repairViewport = { start: 0, end: opened.frames };
      repairView = defaultRepairView;
      monitor = undefined;
      queueComparison();
      for (const lane of [meterLane, repairLane]) {
        lane.latest = undefined;
        lane.retryAt = 0;
      }
      clearTimeout(treatmentTimer);
      clearTimeout(retuneTimer);
      retuneTimer = undefined;
      releaseSelectionRange = false;
      lastRetune = -Infinity;
      measuringRevision = 0;
      treatmentVersion++;
      hasSaved = hasDraft(opened);
      playhead = 0;
      playing = false;
      buffering = false;
      scrubbing = false;
      start = 0;
      end = opened.frames;
      overview = peakValues(opened.overview || []);
      stereoOverview = peakValues(opened.stereoOverview || []);
      overviewRms = peakValues(opened.overviewRms || []);
      stereoOverviewRms = peakValues(opened.stereoOverviewRms || []);
      stereoView = false;
      tile = null;
      if (!overview.length) requestPeaks(0);
      dirty = false;
      invalidate();
      if (pendingCue) {
        const cuePath = pendingCue;
        pendingCue = '';
        await loadCue(cuePath);
      }
    } catch (reason) { if (active && openingRequest === requestId) fail(String(reason)); }
    finally {
      if (openingRequest === requestId) {
        openingRequest = '';
        clearTimeout(openingPoll);
        if (active) { loading = false; loadingInfo = null; }
      }
    }
  }

  async function chooseRecording(allowCue: boolean) {
    try {
      const filters = [{ name: 'Recordings', extensions: allowCue ? ['mp3', 'wav', 'aif', 'aiff', 'flac', 'cue'] : ['mp3', 'wav', 'aif', 'aiff', 'flac'] }];
      const path = await choosePath({ filters });
      if (!path || !active) return;
      if (allowCue && /\.cue$/i.test(path)) {
        pendingCue = path;
        note('Select the recording named in the CUE sheet.');
        await chooseRecording(false);
      } else await openRecording(path);
    } catch (reason) { fail(String(reason)); }
  }

  let viewportVersion = 0;
  function requestPeaks(delay = 42) {
    clearTimeout(peaksTimer);
    const opened = source;
    const revision = ++viewportVersion;
    if (!opened) return;
    if (start === 0 && end === opened.frames && overview.length) return;
    const size = end - start;
    const stereo = hasStereo;
    if (tile && tile.start <= start && tile.end >= end &&
      tile.values.length * size >= (stereo ? 1600 : 800) * (tile.end - tile.start)) return;
    const extra = Math.round(size * 0.30);
    const first = Math.max(0, start - extra);
    const last = Math.min(opened.frames, end + extra);
    peaksTimer = setTimeout(async () => {
      try {
        const energy = !!opened.overviewRms?.length;
        const bins = 1024;
        const data = await audioCommand<ArrayBuffer | number[]>('peaks', {
          id: opened.id, startFrame: first, endFrame: last, bins,
          ...(stereo ? { stereo } : {}), ...(energy ? { energy } : {}),
        });
        if (!active || revision !== viewportVersion || source?.id !== opened.id) return;
        const samples = peakValues(data);
        const bandValues = Math.min(bins, last - first) * (stereo ? 2 : 1);
        const hasEnergy = energy && samples.length === bandValues * 3;
        const values = hasEnergy ? samples.subarray(0, bandValues * 2) : samples;
        const rms = hasEnergy ? samples.subarray(bandValues * 2) : undefined;
        if (first === 0 && last === opened.frames && !overview.length) {
          overview = values;
          if (rms) overviewRms = rms;
        } else tile = { start: first, end: last, values, rms, stereo };
      } catch (reason) {
        if (active && revision === viewportVersion) fail(String(reason));
      }
    }, delay);
  }

  function viewportBounds(first: number, size: number) {
    const range = Math.min(total, Math.max(1, Math.round(size)));
    return { first: Math.round(Math.max(0, Math.min(total - range, first))), range };
  }

  function applyViewport(first: number, size: number, load: boolean) {
    const target = viewportBounds(first, size);
    if (target.first === start && target.first + target.range === end) return false;
    start = target.first;
    end = start + target.range;
    if (load) requestPeaks();
    return true;
  }

  function stopViewportMotion() {
    cancelAnimationFrame(viewportAnimation);
    viewportAnimation = 0;
    viewportGoal = null;
  }

  function viewport(first: number, size: number) {
    const wasMoving = viewportAnimation !== 0;
    stopViewportMotion();
    if (!applyViewport(first, size, true) && wasMoving) requestPeaks();
  }

  function easeViewport(now: number) {
    const target = viewportGoal;
    if (!target) return;
    const motion = viewportMotion;
    const remaining = Math.max(Math.abs(target.first - motion.first), Math.abs(target.range - motion.range));
    if (remaining <= Math.max(1, target.range / 900)) {
      applyViewport(target.first, target.range, false);
      stopViewportMotion();
      requestPeaks(0);
      return;
    }
    const elapsed = Math.min(48, Math.max(1, now - motion.time));
    const progress = 1 - Math.exp(-elapsed / 32);
    const size = motion.range * (target.range / motion.range) ** progress;
    // Move the range and its origin by the same fraction to preserve the anchor.
    const delta = target.range - motion.range;
    const positionProgress = Math.abs(delta) < 0.01 ? progress : (size - motion.range) / delta;
    const first = motion.first + (target.first - motion.first) * positionProgress;
    viewportMotion = { first, range: size, time: now };
    applyViewport(first, size, false);
    viewportAnimation = requestAnimationFrame(easeViewport);
  }

  function animateViewport(first: number, size: number) {
    if (matchMedia('(prefers-reduced-motion: reduce)').matches) {
      viewport(first, size);
      return;
    }
    const target = viewportBounds(first, size);
    const from = viewportGoal ?? { first: start, range: end - start };
    if (target.first === from.first && target.range === from.range) return;
    viewportGoal = target;
    if (viewportAnimation) return;
    clearTimeout(peaksTimer);
    viewportVersion++;
    viewportMotion = { first: start, range: end - start, time: performance.now() };
    viewportAnimation = requestAnimationFrame(easeViewport);
  }

  function wheelZoom(delta: number, fraction: number) {
    const first = viewportGoal?.first ?? start;
    const range = viewportGoal?.range ?? end - start;
    const size = Math.min(total, Math.max(Math.round(sampleRate / 8), range * Math.exp(delta * 0.0021)));
    animateViewport(first + (range - size) * fraction, size);
  }

  function waveformGesture(activeGesture: boolean) {
    waveformDragging = activeGesture;
    if (activeGesture) stopPlayheadTravel();
    if (activeGesture && viewportAnimation) {
      stopViewportMotion();
      requestPeaks();
    }
  }

  function zoom(step: number, around = playhead) {
    const currentLeft = viewportGoal?.first ?? start;
    const currentSize = Math.max(1, viewportGoal?.range ?? end - start);
    const size = step > 0 ? Math.max(sampleRate / 8, currentSize / 2) : Math.min(total, currentSize * 2);
    const fraction = Math.max(0, Math.min(1, (around - currentLeft) / currentSize));
    animateViewport(around - size * fraction, size);
  }

  function zoomToCut() {
    if (!current) return;
    const size = Math.min(total, sampleRate * 8);
    animateViewport(current.startFrame - size / 2, size);
  }

  function movePreview(frame: number) {
    playhead = frame;
    const started = !scrubbing;
    scrubbing = true;
    if (started) publishRepairPlayback();
  }

  function cancelPreview(frame: number) {
    scrubbing = false;
    if (!playing || buffering) playhead = frame;
    else retuneListening();
    publishRepairPlayback();
  }

  function stopPlayheadTravel() {
    cancelAnimationFrame(playheadAnimation);
    playheadTravel = null;
  }

  function seek(frame: number, reveal = true) {
    if (!source) return;
    const listeningRange = repairAudition?.selection ? repairAudition.range : loopEnabled ? loopRegion : undefined;
    const leavingLoop = listeningRange && (frame < listeningRange.start || frame >= listeningRange.end);
    if (leavingLoop) {
      loopEnabled = false;
      repairAudition = undefined;
      if (monitor?.owner === 'repair') stopMonitor(false);
    }
    stopPlayheadTravel();
    scrubbing = false;
    playhead = Math.max(0, Math.min(frame, total - 1));
    queueComparison();
    if (reveal && (playhead < start || playhead >= end)) viewport(playhead - (end - start) / 2, end - start);
    if (leavingLoop && playing) { void transport('play', playhead); return; }
    clearTimeout(pollTimer);
    buffering = playing;
    queuedSeek = { frame: playhead, sequence: ++transportVersion };
    publishRepairPlayback();
    void flushSeek();
  }

  async function flushSeek() {
    const opened = source;
    if (!opened || !active || seekInFlight || !queuedSeek) return;
    const { frame, sequence } = queuedSeek;
    queuedSeek = null;
    seekInFlight = true;
    try {
      await audioCommand('transport', { id: opened.id, action: 'seek', frame, sequence });
      if (active && source?.id === opened.id && sequence === transportVersion) {
        frameClock = { frame, observed: performance.now() };
        if (playing) {
          scheduleStatus(opened.id, sequence, 20);
          retuneListening();
        }
      }
    } catch (reason) {
      if (active && sequence === transportVersion) {
        fail(String(reason));
        playing = false;
        buffering = false;
      }
    } finally { seekInFlight = false; if (queuedSeek) void flushSeek(); }
  }

  function clockFrame(now: number) {
    const frame = frameClock.frame + Math.round((now - frameClock.observed) * sampleRate / 1000);
    if (loopEnabled && loopRegion) {
      const length = loopRegion.end - loopRegion.start;
      const offset = (frame - loopRegion.start) % length;
      return loopRegion.start + (offset < 0 ? offset + length : offset);
    }
    return Math.max(0, Math.min(total - 1, repairAudition?.selection ? repairAudition.range.end : total - 1, frame));
  }

  function paintPlayback(now: number) {
    if (!active || !playing) { animation = 0; return; }
    if (!buffering && !scrubbing) {
      playhead = clockFrame(now);
      if (follow && !waveformDragging && !viewportAnimation && playhead >= end && end < total) viewport(playhead, end - start);
    }
    animation = requestAnimationFrame(paintPlayback);
  }

  function scheduleStatus(id: string, sequence: number, delay: number) {
    clearTimeout(pollTimer);
    pollTimer = setTimeout(() => void pollPlayback(id, sequence), delay);
  }

  async function pollPlayback(id: string, sequence: number) {
    const current = () => active && playing && source?.id === id && sequence === transportVersion;
    if (!current()) return;
    try {
      const status = await audioCommand<Status>('status', { id, sequence });
      if (!current()) return;
      if (!status || status.loading || (status.sequence !== undefined && status.sequence !== sequence)) {
        const changed = !buffering;
        buffering = true;
        if (changed) publishRepairPlayback();
        scheduleStatus(id, sequence, 45);
        return;
      }
      const now = performance.now();
      const predicted = clockFrame(now);
      let drift = status.frame - predicted;
      if (loopEnabled && loopRegion && status.frame >= loopRegion.start && status.frame <= loopRegion.end) {
        const length = loopRegion.end - loopRegion.start;
        // A device status can straddle a loop seam. Correct by the near side
        // of the loop rather than pulling the moving playhead backwards.
        drift -= Math.round(drift / length) * length;
      }
      const reset = buffering || !status.playing || Math.abs(drift) > sampleRate / 4;
      frameClock = { frame: reset ? status.frame : predicted + Math.round(drift * 0.3), observed: now };
      const wasBuffering = buffering;
      const wasRetuning = retunePending;
      const previousError = listeningError;
      const wasPlaying = playing;
      buffering = false;
      retunePending = status.retunePending === retuneVersion;
      if (status.retuneFailed === retuneVersion) listeningError = 'Could not switch the preview. The previous audio is still playing.';
      if (!scrubbing) {
        if (reset) playhead = status.frame;
        playing = status.playing;
      }
      if (wasBuffering || wasRetuning !== retunePending || previousError !== listeningError || wasPlaying !== playing) {
        publishRepairPlayback();
      }
      if (wasBuffering && playing && visibleTools.has('repair')) {
        syncRepairMonitor();
        void refreshRepairMeter();
      }
    } catch (reason) {
      if (!current()) return;
      fail(String(reason));
      playing = false;
      buffering = false;
      publishRepairPlayback();
    }
    if (current()) scheduleStatus(id, sequence, retunePending && visibleTools.has('repair') ? 45 : 170);
  }

  async function transport(action: 'play' | 'pause' | 'audition', frame = playhead, endFrame?: number) {
    if (!source) return;
    stopPlayheadTravel();
    const id = source.id;
    frame = Math.max(0, Math.min(frame, total - 1));
    const sequence = ++transportVersion;
    releaseSelectionRange = false;
    queuedSeek = null;
    scrubbing = false;
    error = '';
    clearTimeout(pollTimer);
    cancelAnimationFrame(animation);
    frameClock = { frame, observed: performance.now() };
    playhead = frame;
    listeningError = '';
    retunePending = false;
    playing = action !== 'pause';
    buffering = playing;
    publishRepairPlayback();
    if (playing) animation = requestAnimationFrame(paintPlayback);
    try {
      const request = playbackTreatment();
      const audition = monitor;
      const repairSignal = repairListen;
      const matchGain = comparisonGain;
      const measured = treatmentAnalysis;
      const repairLoop = action === 'audition' && repairAudition?.range.end === endFrame ? repairAudition : undefined;
      await audioCommand('transport', { id, action, frame, endFrame, sequence,
        ...(repairLoop ? { loopStartFrame: repairLoop.range.start, smoothLoop: repairLoop.smooth, repeat: loopEnabled } : {}),
        ...(action === 'pause' ? {} : { ...request, monitorChannel: repairMonitorChannel() }) });
      if (active && source?.id === id && playing && sequence === transportVersion) {
        scheduleStatus(id, sequence, 20);
        if (!sameTreatment(request.treatment, playbackTreatment().treatment) || measured !== treatmentAnalysis
          || audition !== monitor || repairSignal !== repairListen || matchGain !== comparisonGain) retuneListening();
      }
    } catch (reason) {
      if (active && sequence === transportVersion) {
        fail(String(reason));
        playing = false;
        buffering = false;
        publishRepairPlayback();
      }
    }
  }

  function retuneListening(immediate = false, releaseSelection = false) {
    if (!playing || !source) return;
    releaseSelectionRange ||= releaseSelection;
    if (retuneTimer !== undefined && !immediate) return;
    clearTimeout(retuneTimer);
    retuneTimer = setTimeout(() => {
      retuneTimer = undefined;
      if (!active || !playing || !source || scrubbing || seekInFlight) return;
      lastRetune = performance.now();
      const id = source.id;
      const sequence = transportVersion;
      const revision = ++retuneVersion;
      listeningError = '';
      const selectedAudition = repairAudition?.selection ? repairAudition : undefined;
      const range = selectedAudition
        ? { loopStartFrame: selectedAudition.range.start, endFrame: selectedAudition.range.end, repeat: loopEnabled, smoothLoop: selectedAudition.smooth }
        : releaseSelectionRange ? { loopStartFrame: 0, endFrame: total, repeat: false } : {};
      releaseSelectionRange = false;
      if (repairListen) syncRepairMonitor();
      void audioCommand('transport', { id, action: 'retune', sequence, revision, ...range, ...playbackTreatment() }).catch(reason => {
        if (active && source?.id === id && sequence === transportVersion && revision === retuneVersion) {
          listeningError = String(reason).replace(/^Error: /, '');
          publishRepairPlayback();
        }
      });
    }, immediate ? 0 : Math.max(0, 70 - (performance.now() - lastRetune)));
  }

  function playbackTreatment() {
    if (repairAudition?.selection) {
      const signal = repairListen?.signal ?? (hearOriginal ? 'original' : 'repaired');
      return {
        treatment: signal === 'original' ? noTreatment : repairListeningSettings(),
        previewSelection: repairAudition.selection,
        ...(signal === 'removed' ? { monitor: { kind: 'repair-difference' } } : {}),
      };
    }
    if (monitor?.kind === 'band' && monitor.band) {
      const { lowHz, highHz, channel } = monitor.band;
      return { treatment: noTreatment, monitor: { kind: 'band', lowHz, highHz, channel: hasStereo ? channel : undefined } };
    }
    if (repairListen && !monitor) {
      const { signal } = repairListen;
      const settings = signal === 'original' ? noTreatment : repairListeningSettings();
      if (signal === 'removed') return { treatment: settings, monitor: { kind: 'repair-difference' } };
      return { treatment: settings };
    }
    if (monitor?.kind === 'difference-compressor') {
      return { treatment, monitor: { kind: 'compressor-difference' } };
    }
    if (monitor?.kind === 'bypass-compressor') return { treatment: { ...treatment, compressorDb: 0 } };
    const previewGainDb = hearOriginal ? comparisonGain.input : comparisonGain.output;
    return { treatment: hearOriginal ? noTreatment : listeningSettings(), ...(previewGainDb ? { previewGainDb } : {}) };
  }

  function repairStereoOutput() {
    return hasStereo && (monitor?.kind === 'band'
      || (repairListen ? repairListen.signal === 'original' : hearOriginal)
      || repairListeningSettings().mono === 'off');
  }

  function repairMonitorChannel() {
    return repairStereoOutput() ? repairListen?.channel ?? 'stereo' : 'stereo';
  }

  function syncRepairMonitor() {
    if (!source) return;
    void audioCommand('meter', { id: source.id, sequence: transportVersion, channel: repairMonitorChannel() })
      .catch(console.error);
  }

  function readLevelMatch() {
    try { return localStorage.getItem('tagnetic.audioToolLevelMatch') === 'true'; }
    catch { return false; }
  }

  function setLevelMatch() {
    levelMatch = !levelMatch;
    try { localStorage.setItem('tagnetic.audioToolLevelMatch', String(levelMatch)); } catch { /* Use the session preference. */ }
    queueComparison(0);
    retuneListening(true);
  }

  function queueComparison(delay = 250) {
    clearTimeout(comparisonTimer);
    const revision = ++compareRevision;
    comparisonReady = false;
    comparisonGain = noComparisonGain;
    const opened = source;
    const settings = listeningSettings();
    if (!opened || phase !== 'edit') return;
    const alternate = comparisonAlternate && !reference?.preview
      ? readTreatment({ ...settings, ...comparisonAlternate }) : undefined;
    const compareBoth = alternate && !sameTreatment(settings, alternate);
    if (!compareBoth && (!levelMatch || !hasTreatment(settings))) return;
    const frame = Math.max(0, Math.min(opened.frames - 1, Math.round(playhead)));
    function match(current: ComparisonLevels, other?: ComparisonLevels) {
      comparisonReady = true;
      if (!Number.isFinite(current.output) || current.output <= -72) return;
      const matchOriginal = levelMatch && Number.isFinite(current.input) && current.input > -72;
      let target = matchOriginal ? Math.min(current.input, current.output) : current.output;
      if (other && Number.isFinite(other.output) && other.output > -72) target = Math.min(target, other.output);
      comparisonGain = {
        input: matchOriginal ? Math.max(-48, target - current.input) : 0,
        output: Math.max(-48, target - current.output),
      };
      retuneListening(true);
    }
    const cached = comparisonCache;
    if (compareBoth && cached?.sourceId === opened.id && Math.abs(cached.frame - frame) < opened.sampleRate * 4) {
      const current = cached.readings.find(reading => sameTreatment(reading.settings, settings));
      const other = cached.readings.find(reading => sameTreatment(reading.settings, alternate));
      if (current && other) { match(current.levels, other.levels); return; }
    }
    const stillCurrent = () => active && source?.id === opened.id && revision === compareRevision && phase === 'edit';
    comparisonTimer = setTimeout(async () => {
      if (!stillCurrent()) return;
      comparisonInFlight = revision;
      comparisonUsesProbe = !compareBoth;
      try {
        if (compareBoth) {
          const current = await audioCommand<ComparisonLevels & { alternateOutput: number }>('compare_levels', {
            id: opened.id, frame, treatment: settings, alternate,
          });
          if (!stillCurrent() || !current) return;
          const other = { input: current.input, output: current.alternateOutput };
          comparisonCache = { sourceId: opened.id, frame, readings: [{ settings, levels: current }, { settings: alternate, levels: other }] };
          match(current, other);
        } else {
          const current = await audioCommand<ComparisonLevels>('probe', { id: opened.id, frame, treatment: settings, spectrum: false });
          if (stillCurrent() && current) match(current);
        }
      } catch { /* Keep the preview at its actual level if measurement fails. */ }
      finally {
        if (comparisonInFlight === revision) {
          comparisonInFlight = 0;
          comparisonUsesProbe = false;
        }
      }
    }, delay);
  }

  function stopMonitor(restoreTransport = true) {
    if (!monitor) return;
    const started = monitor.started;
    monitor = undefined;
    if (!restoreTransport) return;
    if (started !== undefined && playing) void transport('pause', started);
    else retuneListening(true);
  }

  function validMonitorBand(band: PreviewBand | undefined): band is PreviewBand {
    return !!band && Number.isInteger(band.lowHz) && Number.isInteger(band.highHz)
      && band.lowHz >= 20 && band.highHz - band.lowHz >= 20 && band.highHz <= Math.min(20_000, sampleRate * .45);
  }

  function startMonitor(owner: Tool, kind: Monitor['kind'], band?: PreviewBand) {
    if (kind === 'band' && !validMonitorBand(band)) return;
    if (kind !== 'band' && !treatment.compressorDb) return;
    monitor = { owner, kind, band, started: monitor?.started ?? (playing ? undefined : playhead) };
    if (!playing) togglePlayback();
    else retuneListening(true);
  }

  function compareOriginal(original: boolean) {
    const repairWasAudible = !!repairListen;
    repairListen = repairTrial = undefined;
    if (original === hearOriginal && !monitor && !repairWasAudible) return;
    hearOriginal = original;
    if (monitor) stopMonitor();
    else retuneListening(true);
    if (levelMatch && !comparisonReady && !comparisonInFlight) queueComparison(0);
  }

  function repairListeningSettings() {
    const settings = listeningSettings();
    const trial = repairTrial;
    if (!trial) return settings;
    if (repairTrialCache?.treatment === settings && repairTrialCache.trial === trial) return repairTrialCache.preview;
    const repairs = [...(settings.spectralRepairs ?? [])];
    const slot = trial.slot ?? repairs.findIndex(edit => !edit);
    if (slot >= 0) repairs[slot] = trial.edit;
    else if (repairs.length < maxSpectralRepairs) repairs.push(trial.edit);
    else return settings;
    const preview = readTreatment({ ...settings, spectralRepairs: repairs });
    repairTrialCache = { treatment: settings, trial, preview };
    return preview;
  }

  function stopRepairListening(returnTo = repairAudition?.returnTo) {
    const action = repairAudition;
    const frame = returnTo === 'loop' && loopRegion ? loopRegion.start : returnTo === 'start' && action ? action.origin : playhead;
    stopMonitor(false);
    void transport('pause', frame);
  }

  function listenToRepair(command: RepairListenCommand) {
    const before: RepairListenState = repairListen ?? { signal: hearOriginal ? 'original' : 'repaired', channel: 'stereo' };
    switch (command.type) {
      case 'set-selection': {
        const pick = readTreatment({ spectralRepairs: [command.selection] }).spectralRepairs?.[0];
        if (!pick || pick.endFrame > total || pick.highHz > Math.ceil(sampleRate / 2)) return;
        repairPick = pick;
        const range = { start: pick.startFrame, end: pick.endFrame };
        if (!command.adjust) {
          repeatSelection = true;
          if (playing) listenToRepair({ type: 'selection', selection: pick, restart: true });
          else {
            repairAudition = undefined;
            loopRegion = null;
            loopEnabled = false;
            stopMonitor(false);
            seek(range.start, false);
          }
        } else if (repairAudition?.selection) {
          repairAudition = { ...repairAudition, range, selection: pick };
          loopRegion = range;
          retuneListening(true);
        }
        break;
      }
      case 'clear-selection': {
        const frame = command.frame;
        if (frame !== undefined && (!Number.isSafeInteger(frame) || frame < 0 || frame >= total)) return;
        repairPick = undefined;
        repeatSelection = false;
        const wasListening = !!repairAudition?.selection;
        if (wasListening) {
          repairAudition = undefined;
          loopRegion = null;
          loopEnabled = false;
        }
        if (frame !== undefined && playing && wasListening) void transport('play', frame);
        else if (frame !== undefined) seek(frame, false);
        else if (wasListening) retuneListening(true, true);
        break;
      }
      case 'selection': {
        const selection = command.selection;
        if (!selection) return;
        listenToRepair({ type: 'loop', range: { start: selection.startFrame, end: selection.endFrame },
          selection, preservePosition: !command.restart && !playing, ...repairLoopPreferences });
        break;
      }
      case 'loop': {
        const { range } = command;
        if (!range || !Number.isSafeInteger(range.start) || !Number.isSafeInteger(range.end)
          || range.start < 0 || range.end > total || range.end <= range.start
          || (command.band !== undefined && !validMonitorBand(command.band))) return;
        const selection = command.selection && readTreatment({ spectralRepairs: [command.selection] }).spectralRepairs?.[0];
        if (command.selection && (!selection || selection.endFrame > total || selection.highHz > Math.ceil(sampleRate / 2))) return;
        const origin = playing ? repairAudition?.origin ?? playhead : playhead;
        const preservePosition = command.preservePosition === true && playhead >= range.start && playhead < range.end;
        repairLoopPreferences = { smooth: command.smooth === true, returnTo: command.returnTo };
        repairAudition = { origin, range, selection: selection ?? undefined, ...repairLoopPreferences };
        monitor = command.band ? { owner: 'repair', kind: 'band', band: command.band } : undefined;
        loopRegion = range;
        loopEnabled = !selection || repeatSelection;
        if (selection && command.preservePosition && playing) retuneListening(true);
        else if (command.play !== false || playing) void transport('audition', preservePosition ? playhead : range.start, range.end);
        break;
      }
      case 'preferences':
        repairLoopPreferences = { smooth: command.smooth === true, returnTo: command.returnTo };
        if (repairAudition) repairAudition = { ...repairAudition, ...repairLoopPreferences };
        break;
      case 'signal':
        if (!['original', 'repaired', 'removed'].includes(command.signal)) return;
        repairListen = { ...before, signal: command.signal };
        hearOriginal = command.signal === 'original';
        stopMonitor(false);
        retuneListening(true);
        break;
      case 'channel':
        if (!['stereo', 'left', 'right'].includes(command.channel) || (!repairStereoOutput() && command.channel !== 'stereo')) return;
        repairListen = { ...before, channel: command.channel };
        syncRepairMonitor();
        break;
      case 'trial': {
        if (!['light', 'full'].includes(command.strength) || !command.candidate) return;
        const amountDb = command.strength === 'light' ? Math.max(1, Math.round(command.candidate.amountDb / 2)) : command.candidate.amountDb;
        const candidate = readTreatment({ spectralRepairs: [{ ...command.candidate, amountDb, bypassed: false }] }).spectralRepairs?.[0];
        if (!candidate || candidate.endFrame > total) return;
        const { slot } = command;
        const saved = listeningSettings().spectralRepairs;
        if (slot === undefined ? saved?.length === maxSpectralRepairs && saved.every(Boolean)
          : !Number.isSafeInteger(slot) || slot < 0 || slot >= maxSpectralRepairs || !saved?.[slot]) return;
        repairTrial = { edit: candidate, slot };
        repairListen = { ...before, signal: 'repaired', trial: command.strength };
        hearOriginal = false;
        stopMonitor(false);
        retuneListening(true);
        break;
      }
      case 'all':
        repairListen = repairTrial = repairAudition = undefined;
        stopMonitor(false);
        hearOriginal = false;
        loopEnabled = false;
        void transport('play', playhead < total - 1 ? playhead : 0);
        break;
      case 'stop':
        stopRepairListening(command.returnTo);
        break;
    }
  }

  function previewEffects() {
    if (!source || total < 2) return;
    const selection = loopRegion ?? previewRange();
    if (selection) { setLoop(selection, true); return; }
    const length = Math.min(total, sampleRate * 8);
    const from = Math.max(0, Math.min(playhead, total - length));
    setLoop({ start: from, end: from + length }, true);
  }

  function previewRange() {
    return loopEnabled && loopRegion ? loopRegion : tracks.length > 1 ? trackRange(selected) : null;
  }

  function hideToolWindows() {
    eqLearnGeneration++;
    eqLearning = undefined;
    clearReference(false);
    stopMonitor(false);
    repairListen = repairTrial = repairAudition = undefined;
    visibleTools.clear();
    clearInterval(toolTimer);
    clearInterval(meterTimer);
    toolTimer = meterTimer = undefined;
    if (usedToolWindows) void audioCommand('tool_window', { action: 'hide_all' }).catch(console.error);
  }

  function updateToolVisibility({ kind, visible }: ToolVisibility) {
    if (!active || phase !== 'edit' || !toolModes.includes(kind)) return;
    usedToolWindows = true;
    if (visible) {
      if (visibleTools.has(kind)) return;
      visibleTools.add(kind);
      if (kind === 'repair' && playing && !buffering) repairMeterRead = { sequence: transportVersion, at: -Infinity };
      if (toolTimer === undefined) toolTimer = setInterval(updateTools, 125);
      if (meterTimer === undefined) {
        meterTimer = setInterval(() => {
          if (!playing) return;
          void refreshToolProbe(false);
          void refreshRepairMeter();
        }, 1000 / liveMeterRate);
      }
      publishToolState(kind, true);
      publishCachedProbe(kind);
      void refreshToolProbe(kind === 'repair');
    } else {
      const wasVisible = visibleTools.delete(kind);
      if (kind === 'eq') clearReference();
      if (kind === 'repair') {
        const releaseSelection = !!repairAudition?.selection;
        repairListen = repairTrial = repairAudition = undefined;
        syncRepairMonitor();
        if (releaseSelection) {
          loopRegion = null;
          loopEnabled = false;
        }
        retuneListening(true, releaseSelection);
      }
      if (monitor?.owner === kind) stopMonitor();
      if (wasVisible) void tick().then(() => document.getElementById('studio-tool-' + kind)?.focus({ preventScroll: true }));
      if (!visibleTools.size) {
        clearInterval(meterTimer);
        meterTimer = undefined;
      }
      if (!visibleTools.size) {
        clearInterval(toolTimer);
        toolTimer = undefined;
      }
    }
  }

  async function toggleToolWindow(kind: Tool) {
    toolWindowError = '';
    usedToolWindows = true;
    try {
      const visible = await audioCommand<boolean>('tool_window', { kind, action: 'toggle' });
      if (!active || phase !== 'edit') hideToolWindows();
      else {
        const range = previewRange();
        if (visible && !playing && range && (playhead < range.start || playhead >= range.end)) seek(range.start, false);
        updateToolVisibility({ kind, visible });
        if (visible) {
          void audioCommand<boolean>('tool_window', { kind, action: 'is_pinned' }).then(pinned => {
            if (typeof pinned === 'boolean') { toolPins[kind] = pinned; publishToolState(kind); }
          }).catch(console.error);
        }
      }
    } catch (reason) {
      toolWindowError = String(reason);
    }
  }

  let eqLearning = $state.raw<ToolSnapshot['eqLearning']>();
  let eqLearnGeneration = 0;

  async function learnEqThreshold(index: number) {
    const opened = source;
    const before = treatment;
    const bands = before.eqBands;
    const band = bands?.[index];
    const range = loopEnabled ? previewRange() : null;
    if (!opened || !bands || !band?.dynamic || band.bypassed || !range || reference?.preview) return;

    const generation = ++eqLearnGeneration;
    const current = () => active && phase === 'edit' && source?.id === opened.id && generation === eqLearnGeneration;
    eqLearning = { index, status: 'learning' };
    publishToolState('eq');
    try {
      const thresholdDb = await audioCommand<number>('eq_learn_threshold', {
        id: opened.id, range, treatment: before, bandIndex: index,
      });
      if (!current()) return;
      const loop = loopEnabled ? previewRange() : null;
      if (treatment !== before || loop?.start !== range.start || loop.end !== range.end || reference?.preview) {
        eqLearning = { index, status: 'error', message: 'The EQ or loop changed. Try learning again.' };
      } else {
        eqLearning = undefined;
        if (Number.isFinite(thresholdDb) && thresholdDb !== band.dynamic.thresholdDb) {
          const eqBands = bands.map((item, position) => position === index
            ? { ...band, dynamic: { ...band.dynamic!, thresholdDb } } : item);
          capture('EQ threshold');
          replaceTreatment(readTreatment({ ...before, eqBands }));
        }
      }
    } catch (reason) {
      if (current()) eqLearning = { index, status: 'error', message: String(reason).replace(/^Error: /, '') };
    } finally {
      if (current()) publishToolState('eq');
    }
  }

  function publishToolState(kind: Tool, force = false) {
    const selectedSettings = kind === 'eq' ? listeningSettings() : treatment;
    const snapshot: ToolSnapshot | null = source && active && phase === 'edit' ? {
      sourceId: source.id, frame: playhead, sampleRate, frames: source.frames, settings: selectedSettings, stereo: hasStereo,
      playing, original: hearOriginal, loop: loopEnabled, revision: toolRevisions[kind], theme: activeToolTheme ??= toolTheme(),
      loopRange: (kind === 'repair' && repairAudition?.selection ? repairAudition.range : previewRange()) ?? undefined,
      pinned: toolPins[kind] ?? false, levelMatch,
      eqLevelComparison: kind === 'eq',
      eqLearning: kind === 'eq' ? eqLearning : undefined,
      monitor: monitor?.owner === kind ? monitor.kind : undefined,
      undoLabel: kind === 'eq' && reference?.preview
        ? (referenceUndo.length ? 'Undo match adjustment' : undefined)
        : previous.length ? 'Undo ' + previous.at(-1)?.label : undefined,
      redoLabel: kind === 'eq' && reference?.preview
        ? (referenceRedo.length ? 'Redo match adjustment' : undefined)
        : next.length ? 'Redo ' + next.at(-1)?.label : undefined,
      reference: kind === 'eq' ? reference : undefined,
      normalisation: kind === 'dynamics' ? treatmentAnalysis ?? undefined : undefined,
      ...(kind === 'repair' ? { overview: source.overview, stereoOverview: source.stereoOverview, cuts: toolCuts,
        repairListen, selectionPlaying: playing && !!repairAudition?.selection, repeatSelection,
        transportSequence: transportVersion, repairStereoOutput: repairStereoOutput(),
        clockRunning: playing && !buffering && !scrubbing,
        repairPlayback: !playing ? undefined : listeningError ? 'failed' : buffering ? 'buffering' : retunePending ? 'updating' : undefined } : {}),
    } : null;
    const sent = sentToolStates[kind];
    if (!force && ((!snapshot && !sent) || (snapshot && sent && snapshot.sourceId === sent.sourceId
      && (kind !== 'repair' || snapshot.frame === sent.frame) && snapshot.settings === sent.settings
      && snapshot.playing === sent.playing && snapshot.stereo === sent.stereo && snapshot.original === sent.original
      && snapshot.loop === sent.loop && snapshot.revision === sent.revision && snapshot.theme === sent.theme
      && snapshot.loopRange?.start === sent.loopRange?.start && snapshot.loopRange?.end === sent.loopRange?.end
      && snapshot.pinned === sent.pinned && snapshot.levelMatch === sent.levelMatch && snapshot.monitor === sent.monitor
      && snapshot.repairListen === sent.repairListen && snapshot.clockRunning === sent.clockRunning
      && snapshot.repeatSelection === sent.repeatSelection && snapshot.selectionPlaying === sent.selectionPlaying
      && snapshot.transportSequence === sent.transportSequence && snapshot.repairStereoOutput === sent.repairStereoOutput
      && snapshot.repairPlayback === sent.repairPlayback
      && snapshot.undoLabel === sent.undoLabel && snapshot.redoLabel === sent.redoLabel
      && snapshot.eqLearning === sent.eqLearning && snapshot.normalisation === sent.normalisation && snapshot.reference === sent.reference && snapshot.cuts === sent.cuts))) return;
    sentToolStates[kind] = snapshot;
    const sameSource = !force && snapshot?.sourceId === sent?.sourceId;
    const message = snapshot && sameSource ? { ...snapshot, overview: undefined, stereoOverview: undefined,
      cuts: snapshot.cuts === sent?.cuts ? undefined : snapshot.cuts,
      settings: kind === 'repair' && snapshot.settings === sent?.settings ? undefined : snapshot.settings } : snapshot;
    return sendToolState(kind, message)?.catch(reason => {
      if (sentToolStates[kind]?.sourceId === snapshot?.sourceId) sentToolStates[kind] = undefined;
      console.error(reason);
    });
  }

  function publishRepairPlayback() {
    if (visibleTools.has('repair')) publishToolState('repair');
  }

  function syncToolAppearance() {
    activeToolTheme = toolTheme();
    for (const kind of toolModes) {
      if (connectedTools.has(kind) || visibleTools.has(kind)) publishToolState(kind);
    }
  }

  function publishCachedProbe(kind: Tool) {
    const update = (kind === 'repair' ? repairLane : meterLane).latest;
    if (update && update.sourceId === source?.id) {
      const { sourceId, probe } = update;
      void sendToolProbe(kind, { sourceId, probe })?.catch(console.error);
    }
  }

  function updateTools() {
    if (visibleTools.size === 0) return;
    for (const kind of visibleTools) publishToolState(kind);
    void refreshToolProbe(false);
    void refreshToolProbe(true);
  }

  async function refreshRepairMeter() {
    const id = source?.id;
    const sequence = transportVersion;
    const now = performance.now();
    const current = () => active && id === source?.id && sequence === transportVersion
      && visibleTools.has('repair') && playing && !buffering && !scrubbing;
    if (!id || repairMeterPending || now < repairMeterRetryAt || !current()) return;
    // A peak read drains the output tap. Drain an expired interval before
    // showing fresh levels when the window is reopened or timers were suspended.
    const expired = repairMeterRead.sequence === sequence && now - repairMeterRead.at > 180;
    repairMeterPending = true;
    try {
      const peaks = await audioCommand<[number, number] | null>('meter', { id, sequence });
      if (peaks && current()) {
        repairMeterRead = { sequence, at: now };
        if (!expired) await sendRepairMeter({ sourceId: id, sequence, peaks });
      }
    } catch {
      repairMeterRetryAt = performance.now() + 500;
    } finally {
      repairMeterPending = false;
    }
  }

  async function refreshToolProbe(spectrogram: boolean, movedViewport = false) {
    const opened = source;
    const lane = spectrogram ? repairLane : meterLane;
    const targets = () => [...visibleTools].filter(kind => spectrogram ? kind === 'repair'
      : kind === 'dynamics' || kind === 'eq');
    const now = performance.now();
    if (!opened || !active || phase !== 'edit' || (!spectrogram && (lane.pending || comparisonUsesProbe))
      || (now < lane.retryAt && !movedViewport) || !targets().length) return;
    const frame = Math.max(0, Math.min(opened.frames - 1, Math.round(playing && !buffering && !scrubbing ? clockFrame(now) : playhead)));
    const version = transportVersion;
    const view = repairView;
    const listening = listeningSettings();
    const trial = repairTrial;
    const settings = spectrogram ? view.treated ? repairListeningSettings() : noTreatment : listening;
    const spectrum = !spectrogram && eqAnalyzerView !== 'off' && visibleTools.has('eq');
    const spectrumView = spectrum ? eqAnalyzerView : undefined;
    const audioIsAudible = () => playing && !buffering && !scrubbing;
    const playback = !spectrogram && audioIsAudible();
    const audibleMode = [hearOriginal, monitor?.kind].join(':');
    const probeRate = spectrogram ? 3 : playback ? liveMeterRate : 8;
    const viewport = spectrogram ? repairViewport : null;
    const viewKey = spectrogram ? view : null;
    const gain = !spectrogram || view.treated ? treatmentAnalysis?.gainDb : null;
    const key = viewport ? JSON.stringify([opened.id, viewport.start, viewport.end, viewKey, settings, gain])
      : JSON.stringify([opened.id, version, Math.floor(frame * probeRate / opened.sampleRate),
        playback ? null : settings, gain, spectrumView, viewKey, audibleMode]);
    if (lane.latest?.key === key || lane.pending === key) return;
    // A new camera can supersede an overview in flight. Follow playback lets
    // each local reading finish before requesting the next transport position.
    if (spectrogram && !viewport && lane.pending && !movedViewport) return;
    lane.pending = key;
    try {
      const probe = await audioCommand<TreatmentProbe | null>('probe', { id: opened.id, frame, treatment: settings, spectrogram, spectrum, spectrumView, viewport,
        playback,
        ...(spectrogram ? { spectrogramChannel: view.channel, spectrogramTreated: view.treated,
          spectrogramDifference: view.difference, spectrogramDetail: view.detail, spectrogramFocus: view.focus } : {}) });
      const currentViewport = viewport ? repairViewport?.start === viewport.start && repairViewport.end === viewport.end
        : (!spectrogram || !repairViewport) && version === transportVersion;
      const currentSettings = listening === listeningSettings();
      const liveAudio = playback && probe?.fromPlayback && audioIsAudible()
        && audibleMode === [hearOriginal, monitor?.kind].join(':');
      // Cached loops may use the offline decoder. Keep that work at the
      // slower cadence while allowing live PCM probes to follow the fast timer.
      if (playback && probe && !probe.fromPlayback) lane.retryAt = performance.now() + 90;
      if (lane.pending === key && probe && active && source?.id === opened.id && phase === 'edit' && currentViewport
        && (spectrogram ? view === repairView && (!view.treated || (currentSettings && trial === repairTrial))
          : (probe.fromPlayback ? liveAudio : currentSettings) && (!spectrum || eqAnalyzerView === spectrumView))) {
        const reading = liveAudio && !currentSettings
          ? { ...probe, dynamicEqMix: undefined, dynamicEqDetectors: undefined, spectralResponse: undefined }
          : probe;
        lane.latest = { key, sourceId: opened.id, probe: reading };
        targets().forEach(publishCachedProbe);
      }
    } catch (reason) {
      if (lane.pending !== key) return;
      lane.retryAt = performance.now() + 600;
      if (source?.id === opened.id && !/superseded|cancel/i.test(String(reason))) {
        for (const kind of targets()) {
          void sendToolProbe(kind, { sourceId: opened.id, error: 'Could not measure the current audio.' })?.catch(console.error);
        }
      }
    } finally {
      if (lane.pending === key) lane.pending = undefined;
    }
  }

  function settleToolEdits() {
    const remaining = new Set(connectedTools);
    if (!remaining.size) return Promise.resolve(true);
    if (toolFence) return Promise.resolve(false);
    return new Promise<boolean>(resolve => {
      const id = String(++fenceSerial);
      const finish = (ready: boolean) => {
        if (toolFence?.id !== id) return;
        clearTimeout(timeout);
        toolFence = undefined;
        settlingTools = false;
        resolve(ready);
      };
      const timeout = setTimeout(() => {
        if (active) toolWindowError = 'The audio tools are still syncing. Try again in a moment.';
        finish(false);
      }, 1400);
      toolFence = { id, remaining, finish };
      settlingTools = true;
      for (const kind of remaining) void requestToolFlush(kind, id)?.catch(() => finish(false));
    });
  }

  function handleToolRequest(request: ToolRequest) {
    if (!request || !toolModes.includes(request.kind)) return;
    if (request.action === 'sync') {
      connectedTools.add(request.kind);
      void publishToolState(request.kind, true)?.then(() => publishCachedProbe(request.kind));
      return;
    }
    if (!active || phase !== 'edit' || !source || request.sourceId !== source.id) return;
    if (request.action === 'flushed') {
      const fence = toolFence;
      if (fence && fence.id === request.fence) {
        fence.remaining.delete(request.kind);
        if (!fence.remaining.size) fence.finish(true);
      }
      return;
    }
    if (request.action === 'analyzer' && request.kind === 'eq') {
      eqAnalyzerView = request.enabled && request.view !== 'off' ? (request.view ?? 'both') : 'off';
      void refreshToolProbe(false);
      return;
    }
    if (request.kind === 'eq') {
      if (request.action === 'learn-threshold' && Number.isSafeInteger(request.index) && request.index >= 0) { void learnEqThreshold(request.index); return; }
      if (request.action === 'reference-start') { void matchReference(); return; }
      if (request.action === 'reference-preview') { previewReference(); return; }
      if (request.action === 'reference-keep') { void keepReference(); return; }
      if (request.action === 'reference-cancel') { clearReference(); return; }
    }
    if (request.action === 'play') {
      if (request.kind === 'repair') {
        if (playing) {
          void transport('pause');
          return;
        }
        if (repairAudition) {
          const { range } = repairAudition;
          void transport('audition', playhead >= range.start && playhead < range.end ? playhead : range.start, range.end);
          return;
        }
        if (repairPick) {
          listenToRepair({ type: 'selection', selection: repairPick });
          return;
        }
      }
      togglePlayback();
    }
    if (request.action === 'hide') {
      void audioCommand('tool_window', { kind: request.kind, action: 'hide' }).then(() =>
        updateToolVisibility({ kind: request.kind, visible: false })).catch(reason => { toolWindowError = String(reason); });
    }
    if (request.action === 'pin') {
      const action = toolPins[request.kind] ? 'unpin' : 'pin';
      void audioCommand<boolean>('tool_window', { kind: request.kind, action }).then(pinned => {
        if (typeof pinned === 'boolean') { toolPins[request.kind] = pinned; publishToolState(request.kind); }
      }).catch(reason => { toolWindowError = String(reason); });
    }
    if (request.action === 'level-match') setLevelMatch();
    if (request.action === 'repair-listen' && request.kind === 'repair' && request.command) {
      listenToRepair(request.command);
      for (const kind of visibleTools) publishToolState(kind);
      return;
    }
    if (request.action === 'seek' && Number.isSafeInteger(request.range?.start)) seek(request.range!.start);
    if (request.action === 'solo-band' && request.kind === 'eq') startMonitor(request.kind, 'band', request.band);
    if (request.action === 'retune-solo-band') {
      if (request.kind === 'eq' && monitor?.owner === 'eq' && monitor.kind === 'band' && validMonitorBand(request.band)) {
        monitor = { ...monitor, band: request.band };
        retuneListening();
      }
      return;
    }
    if (request.action === 'audition-band' && request.kind === 'repair' && request.range && request.band) {
      listenToRepair({ type: 'loop', range: request.range, band: request.band, ...repairLoopPreferences });
    }
    if (request.kind === 'dynamics' && (request.action === 'bypass-compressor' || request.action === 'difference-compressor')) {
      startMonitor(request.kind, request.action);
    }
    if (request.action === 'stop-audition' && monitor?.owner === request.kind) stopMonitor();
    if (request.action === 'compare' && typeof request.original === 'boolean') {
      if (request.kind === 'repair') listenToRepair({ type: 'signal', signal: request.original ? 'original' : 'repaired' });
      else compareOriginal(request.original);
    }
    if (request.action === 'undo' || request.action === 'redo') {
      toolRevisions[request.kind] = request.revision;
      if (request.trial === undefined) history(request.action === 'redo');
      else if (request.kind === 'eq' && reference?.generation === request.trial) referenceHistory(request.action === 'redo');
    }
    if (request.action === 'viewport' && request.kind === 'repair') {
      const bounds = request.range;
      if (!bounds || (Number.isSafeInteger(bounds.start) && Number.isSafeInteger(bounds.end)
        && bounds.start >= 0 && bounds.end <= source.frames && bounds.end > bounds.start)) {
        repairViewport = bounds;
        const requested = request.options?.channel;
        const difference = request.options?.difference === true;
        const treated = difference || request.options?.treated === true;
        const stereo = hasStereo && (!treated || treatment.mono === 'off');
        const channel = stereo && (requested === 'left' || requested === 'right' || requested === 'stereo') ? requested : 'combined';
        const detail = request.options?.detail === 'tones' || request.options?.detail === 'transients' ? request.options.detail : 'balanced';
        const band = request.options?.focus;
        const focus = bounds && band && Number.isSafeInteger(band.lowHz) && Number.isSafeInteger(band.highHz)
          && band.lowHz >= 20 && band.highHz <= Math.min(20_000, Math.floor(source.sampleRate * .45)) && band.highHz > band.lowHz
          ? { lowHz: band.lowHz, highHz: band.highHz } : undefined;
        if (channel !== repairView.channel || treated !== repairView.treated || difference !== repairView.difference || detail !== repairView.detail
          || !sameSpectrogramFocus(focus, repairView.focus)) {
          repairView = { channel, treated, difference, detail, focus };
        }
        void refreshToolProbe(true, true);
      }
      return;
    }
    if (request.action === 'loop') {
      if (request.kind === 'repair' && !request.range && repairPick) {
        repeatSelection = !repeatSelection;
        if (playing && repairAudition?.selection) {
          listenToRepair({ type: 'loop', range: repairAudition.range, selection: repairAudition.selection,
            preservePosition: true, ...repairLoopPreferences });
        }
        publishToolState('repair');
        return;
      }
      stopMonitor(false);
      if (request.range) {
        const { start, end } = request.range;
        if (request.kind === 'repair') listenToRepair({ type: 'loop', range: request.range, ...repairLoopPreferences });
        else if (Number.isSafeInteger(start) && Number.isSafeInteger(end)) setLoop({ start, end }, true);
      } else if (loopEnabled) toggleLoop();
      else previewEffects();
    }
    if (request.action === 'edit' && request.patch) {
      toolRevisions[request.kind] = request.revision;
      const patch = toolPatch(request.kind, request.patch);
      if (request.trial !== undefined) {
        if (request.kind === 'eq' && reference?.generation === request.trial && referenceCandidate) {
          changeReference(readTreatment({ ...referenceCandidate, ...patch }), request.gesture);
        }
        publishToolState(request.kind);
        return;
      }
      const comparisonChanged = request.kind === 'eq' && request.compareEq !== undefined;
      if (comparisonChanged) comparisonAlternate = request.compareEq ? toolPatch('eq', request.compareEq) : undefined;
      const settings = readTreatment({ ...treatment, ...patch });
      if (!sameTreatment(settings, treatment)) {
        const gesture = request.gesture ? `${request.kind}:${request.gesture}` : undefined;
        if (!gesture || gesture !== toolGesture) {
          const before = { undo: previous, redo: next, dirty };
          capture(treatmentChangeLabel(request.kind, settings));
          if (gesture) gestureCheckpoint = before;
        }
        toolGesture = gesture;
        replaceTreatment(settings, comparisonChanged);
        const initial = previous.at(-1)?.draft.treatment;
        if (gesture && gestureCheckpoint && initial && sameTreatment(readTreatment(initial), settings)) {
          previous = gestureCheckpoint.undo;
          next = gestureCheckpoint.redo;
          dirty = gestureCheckpoint.dirty;
          toolGesture = undefined;
          gestureCheckpoint = undefined;
        }
      } else if (comparisonChanged) {
        // Switch both curves together, even when the active curve is unchanged.
        queueComparison(0);
        retuneListening(true);
      }
    }
    publishToolState(request.kind);
  }

  function listeningSettings() {
    return reference?.preview && referenceCandidate ? referenceCandidate : treatment;
  }

  function setReference(next: EqReference | undefined) {
    reference = next;
    publishToolState('eq');
  }

  function stopReferencePreview() {
    const started = referenceStarted;
    referenceStarted = undefined;
    if (levelMatch || comparisonAlternate) queueComparison();
    if (started !== undefined && playing) void transport('pause', started);
    else retuneListening(true);
  }

  function clearReference(restore = true) {
    if (!reference) return;
    const wasPreviewing = reference.preview;
    referenceGeneration++;
    referenceCandidate = undefined;
    referenceUndo = [];
    referenceRedo = [];
    referenceGesture = undefined;
    referenceRedoBeforeGesture = [];
    setReference(undefined);
    if (wasPreviewing && restore) stopReferencePreview();
    else referenceStarted = undefined;
  }

  async function matchReference() {
    const opened = source;
    if (!opened || !canEdit || reference?.status === 'choosing' || reference?.status === 'analysing') return;
    clearReference();
    const generation = ++referenceGeneration;
    const current = () => active && source?.id === opened.id && generation === referenceGeneration && phase === 'edit';
    const selectedRange = previewRange();
    const start = Math.max(0, Math.min(Math.round(playhead), total - Math.min(total, sampleRate * 8)));
    const range = selectedRange ?? { start, end: Math.min(total, start + sampleRate * 8) };
    setReference({ status: 'choosing' });
    try {
      if (!await settleToolEdits() || !current()) { if (current()) clearReference(); return; }
      const path = await choosePath({ filters: [{ name: 'Reference recording', extensions: ['mp3', 'wav', 'aif', 'aiff', 'flac'] }] });
      if (!current()) return;
      if (!path) { clearReference(); return; }
      const name = leaf(path);
      setReference({ status: 'analysing', name });
      const bands = await audioCommand<EqBand[]>('eq_match', { id: opened.id, range, reference: path });
      if (!current()) return;
      if (!bands.length) {
        setReference({ status: 'error', name, message: 'No useful tonal difference found.' });
        return;
      }
      referenceCandidate = readTreatment({ ...treatment, ...toolPatch('eq', noTreatment), eqBands: bands });
      setReference({ status: 'ready', generation, name, count: bands.length });
    } catch (reason) {
      if (current()) setReference({ status: 'error', name: reference?.name, message: String(reason).replace(/^Error: /, '') });
    }
  }

  function previewReference() {
    if (reference?.status !== 'ready' || !referenceCandidate) return;
    const preview = !reference.preview;
    setReference({ ...reference, preview });
    if (!preview) { stopReferencePreview(); return; }
    stopMonitor(false);
    hearOriginal = false;
    if (levelMatch || comparisonAlternate) queueComparison();
    if (!playing) {
      referenceStarted = playhead;
      togglePlayback();
    } else retuneListening(true);
    void refreshToolProbe(false);
  }

  function updateReferenceAudio(settings: AudioTreatment) {
    referenceCandidate = settings;
    if (!reference?.preview) return;
    if (levelMatch || comparisonAlternate) queueComparison();
    retuneListening();
  }

  function changeReference(settings: AudioTreatment, gesture?: string) {
    if (!referenceCandidate || sameTreatment(referenceCandidate, settings)) return;
    if (!gesture || gesture !== referenceGesture) {
      referenceRedoBeforeGesture = referenceRedo;
      referenceUndo = [...referenceUndo, referenceCandidate];
      referenceRedo = [];
    }
    referenceGesture = gesture;
    if (gesture && sameTreatment(referenceUndo.at(-1) ?? noTreatment, settings)) {
      referenceUndo = referenceUndo.slice(0, -1);
      referenceRedo = referenceRedoBeforeGesture;
      referenceGesture = undefined;
    }
    if (referenceUndo.length > 50) referenceUndo = referenceUndo.slice(-50);
    updateReferenceAudio(settings);
  }

  function referenceHistory(redo: boolean) {
    const stack = redo ? referenceRedo : referenceUndo;
    const settings = stack.pop();
    if (!settings || !referenceCandidate) return;
    (redo ? referenceUndo : referenceRedo).push(referenceCandidate);
    referenceGesture = undefined;
    updateReferenceAudio(settings);
  }

  async function keepReference() {
    if (reference?.status !== 'ready') return;
    const generation = reference.generation;
    if (!await settleToolEdits() || !active || phase !== 'edit' || reference?.generation !== generation) return;
    const candidate = referenceCandidate;
    if (!candidate) return;
    clearReference(false);
    capture('EQ reference match');
    replaceTreatment(candidate);
    retuneListening(true);
    publishToolState('eq');
  }

  function treatmentChangeLabel(kind: Tool, settings: AudioTreatment) {
    if (kind === 'repair') {
      const before = treatment.spectralRepairs;
      const after = settings.spectralRepairs;
      const edited = Array.from({ length: Math.max(before?.length ?? 0, after?.length ?? 0) }, (_, index) => {
        const old = before?.[index];
        const fresh = after?.[index];
        return JSON.stringify(old) !== JSON.stringify(fresh) ? fresh || old : null;
      }).find(Boolean);
      return edited ? 'repair at ' + timeLabel(edited.startFrame, sampleRate, 0) : 'repair settings';
    }
    if (kind === 'dynamics') {
      if (settings.compressorInputDb !== treatment.compressorInputDb) return 'compressor input gain change';
      if (settings.compressorDb !== treatment.compressorDb) return 'compressor threshold change';
      if (settings.compressorRatio !== treatment.compressorRatio) return 'compression ratio change';
      if (settings.gateDb !== treatment.gateDb) return 'quiet-gap threshold change';
      if (settings.gainDb !== treatment.gainDb) return 'gain change';
      if (settings.normalize !== treatment.normalize || settings.normalizeTargetDb !== treatment.normalizeTargetDb) return 'loudness change';
      return 'dynamics settings';
    }
    return 'EQ change';
  }

  function toolHint(kind: Tool) {
    const count = toolEffectCount(kind, treatment);
    if (!count) return toolNames[kind];
    const item = kind === 'eq' ? 'active band'
      : kind === 'repair' && count === activeRepairCount(treatment) ? 'area' : 'active effect';
    return `${toolNames[kind]} · ${count} ${item}${count === 1 ? '' : 's'}`;
  }

  function openRecipes() {
    recipes = readTreatmentRecipes();
    recipeName = '';
    recipeError = '';
    recipesOpen = true;
  }

  function restoreRecipeFocus(event: Event) {
    event.preventDefault();
    requestAnimationFrame(() => {
      if (active && !recipesOpen && !closeRequested && phase === 'edit') {
        document.getElementById('studio-waveform-options')?.focus({ preventScroll: true });
      }
    });
  }

  async function copyRecipe() {
    const opened = source;
    if (!opened || !await settleToolEdits() || !active || source !== opened) return;
    try {
      copyTreatmentSettings(treatment);
      copiedTreatment = readCopiedTreatment();
      note('Treatment settings copied');
    } catch (reason) { fail(String(reason)); }
  }

  async function applyRecipe(settings: AudioTreatment, name: string) {
    const opened = source;
    if (!opened || !canEdit || !await settleToolEdits() || !active || source !== opened || phase !== 'edit') return;
    const adapted = treatmentForRecording(settings, treatment, sampleRate, hasStereo);
    if (!sameTreatment(adapted, treatment)) {
      capture('treatment settings');
      replaceTreatment(adapted);
    }
    recipesOpen = false;
    note(name + ' applied');
  }

  async function saveRecipe(event: SubmitEvent) {
    event.preventDefault();
    const name = recipeName.trim();
    const opened = source;
    if (!name || !opened || !await settleToolEdits() || !active || source !== opened || !recipesOpen) return;
    if (recipes.some(recipe => recipe.name.toLowerCase() === name.toLowerCase())) {
      recipeError = 'A recipe with that name already exists.';
      return;
    }
    try {
      const updated = [...recipes, { name, settings: portableTreatment(treatment) }];
      saveTreatmentRecipes(updated);
      recipes = updated;
      recipeName = '';
      recipeError = '';
      draftAnnouncement = 'Recipe saved';
    } catch (reason) { recipeError = String(reason); }
  }

  function removeRecipe(name: string) {
    try {
      const updated = recipes.filter(recipe => recipe.name !== name);
      saveTreatmentRecipes(updated);
      recipes = updated;
      recipeError = '';
      draftAnnouncement = 'Recipe removed';
    } catch (reason) { recipeError = String(reason); }
  }

  function replaceTreatment(settings: AudioTreatment, comparisonChanged = false) {
    if (!source || sameTreatment(settings, treatment)) return;
    clearReference();
    if (repairTrial) {
      repairTrial = repairTrialCache = undefined;
      if (repairListen) repairListen = { ...repairListen, trial: undefined };
    }
    treatment = settings;
    if (!hasTreatment(settings)) hearOriginal = false;
    if (levelMatch || comparisonAlternate || comparisonChanged) queueComparison(comparisonChanged ? 0 : undefined);
    retuneListening(comparisonChanged);
    measureTreatment();
  }

  function stopMeasuringTreatment() {
    clearTimeout(treatmentTimer);
    treatmentVersion++;
    if (!source || !measuringRevision) return;
    measuringRevision = 0;
    // A neutral recipe cancels the previous latest-only job without walking the recording.
    return audioCommand('treatment', { id: source.id, treatment: noTreatment }).catch(() => {});
  }

  function measureTreatment(delay = 600) {
    const opened = source;
    if (!opened) return;
    const settings = treatment;
    const stopped = stopMeasuringTreatment();
    const revision = treatmentVersion;
    treatmentAnalysis = null;
    treatmentError = '';
    // Exact peak protection is prepared when reviewing output. Optimistic
    // playback uses the recording overview and a linked peak ceiling.
    if (!settings.normalize) return;
    treatmentTimer = setTimeout(async () => {
      // Avoid spending the quiet period processing a superseded full-file scan.
      await stopped;
      if (!active || revision !== treatmentVersion || source?.id !== opened.id || phase !== 'edit') return;
      measuringRevision = revision;
      try {
        const result = await audioCommand<TreatmentAnalysis>('treatment', { id: opened.id, treatment: settings });
        if (!active || revision !== treatmentVersion || source?.id !== opened.id) return;
        treatmentAnalysis = result;
        comparisonCache = undefined;
        if (levelMatch || comparisonAlternate) queueComparison();
        if (!hearOriginal) retuneListening(true);
      } catch (reason) {
        if (!active || revision !== treatmentVersion || source?.id !== opened.id) return;
        treatmentError = String(reason).replace(/^Error: /, '');
      } finally {
        if (measuringRevision === revision) measuringRevision = 0;
      }
    }, delay);
  }

  function audition() {
    if (!current || !source) return;
    auditionBoundary(current.startFrame);
  }

  function auditionBoundary(frame: number) {
    if (!source || !Number.isSafeInteger(frame) || frame <= 0 || frame >= total || phase === 'export') return;
    const radius = 2 * sampleRate;
    const region = { start: Math.max(0, frame - radius), end: Math.min(total, frame + radius) };
    if (phase === 'edit') setLoop(region, true);
    else {
      loopEnabled = false;
      void transport('audition', region.start, region.end);
    }
  }

  function togglePlayback() {
    if (playing && repairAudition?.range === loopRegion) stopRepairListening();
    else if (playing) { stopMonitor(false); void transport('pause'); }
    else if (loopEnabled && loopRegion) void transport('audition', loopRegion.start, loopRegion.end);
    else if (playhead < total - 1) void transport('play');
    else {
      const firstCut = tracks[0]?.startFrame ?? 0;
      const restart = firstCut < total - 1 ? firstCut : 0;
      if (restart < start || restart >= end) {
        const span = end - start;
        animateViewport(restart - span / 2, span);
      }
      void transport('play', restart);
    }
  }

  function trackRange(id: string) {
    const index = tracks.findIndex(track => track.id === id);
    return index < 0 ? null : { start: tracks[index].startFrame, end: tracks[index + 1]?.startFrame ?? total };
  }

  function setLoop(region: FrameRange, play = playing) {
    if (!source || region.start < 0 || region.end > total || region.start >= region.end) return;
    repairAudition = undefined;
    loopRegion = region;
    loopEnabled = true;
    if (play) void transport('audition', region.start, region.end);
  }

  function loopTrack(id: string) {
    const region = trackRange(id);
    if (!region) return;
    selection(id);
    if (region.start < start || region.start >= end) animateViewport(region.start - (end - start) / 2, end - start);
    setLoop(region, true);
  }

  function toggleLoop(id = selected) {
    if (loopEnabled) {
      loopEnabled = false;
      if (playing) void transport('play');
    } else {
      const region = loopRegion ?? trackRange(id);
      if (region) setLoop(region);
    }
  }

  function zoomToLoop() {
    if (!loopRegion) return;
    const size = Math.max(sampleRate / 8, (loopRegion.end - loopRegion.start) * 1.1);
    animateViewport((loopRegion.start + loopRegion.end - size) / 2, size);
  }

  function clearLoop() {
    if (loopEnabled) toggleLoop();
    loopRegion = null;
  }

  function selection(id: string, extend = false) {
    if (!tracks.some(track => track.id === id)) return;
    if (extend) {
      const anchor = tracks.findIndex(track => track.id === (rangeAnchor || selected));
      const at = tracks.findIndex(track => track.id === id);
      const origin = anchor < 0 ? at : anchor;
      rangeAnchor = tracks[origin].id;
      rangeIds = tracks.slice(Math.min(origin, at), Math.max(origin, at) + 1).map(track => track.id);
    } else {
      rangeAnchor = id;
      rangeIds = [];
    }
    selected = id;
    void tick().then(() => {
      const control = list?.querySelector<HTMLElement>('.track[data-current="true"]');
      if (!control || !list) return;
      // Keep selection in view without scrolling the entire dialog off the waveform.
      const bounds = list.getBoundingClientRect();
      const row = control.getBoundingClientRect();
      if (row.top < bounds.top) list.scrollTop -= bounds.top - row.top;
      else if (row.bottom > bounds.bottom) list.scrollTop += row.bottom - bounds.bottom;
    });
  }

  function selectTrack(track: Track, extend = false) {
    if (extend) { selection(track.id, true); return; }
    selection(track.id);
    const from = visualPlayhead;
    const frame = track.startFrame;
    const range = end - start;
    seek(frame, false);
    if (frame < start || frame >= end) animateViewport(frame - range / 2, range);
    if (from === frame || matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    const startedAt = performance.now();
    playheadTravel = { from, progress: 0 };
    function travel(now: number) {
      const fraction = Math.min(1, Math.max(0, (now - startedAt) / 200));
      if (fraction === 1) { playheadTravel = null; return; }
      playheadTravel = { from, progress: 1 - (1 - fraction) ** 3 };
      playheadAnimation = requestAnimationFrame(travel);
    }
    playheadAnimation = requestAnimationFrame(travel);
  }

  function addCut() {
    addCutAt(playhead);
  }

  function addCutAt(frame: number) {
    if (!canEdit) return;
    if (tracks.length >= maxFiles) { fail('A draft can contain up to ' + maxFiles + ' tracks.'); return; }
    if (frame <= tracks[0].startFrame || frame >= total) { fail('Choose a position after the start of Track 1.'); return; }
    if (tracks.some(track => track.startFrame === frame)) { fail('A cut already exists at this position.'); return; }
    capture();
    const track = { ...initial(), startFrame: frame };
    tracks = [...tracks, track].sort((left, right) => left.startFrame - right.startFrame);
    selected = track.id;
    error = '';
    draftAnnouncement = 'Cut added at ' + timeLabel(frame, sampleRate);
  }

  function playFromCut(id: string, frame: number) {
    selection(id);
    loopEnabled = false;
    void transport('play', frame);
  }

  function moveCut(id: string, frame: number) {
    const index = tracks.findIndex(track => track.id === id);
    if (!canEdit || index < 0) return;
    const track = tracks[index];
    const lower = index === 0 ? 0 : tracks[index - 1].startFrame + 1;
    const upper = (tracks[index + 1]?.startFrame ?? total) - 1;
    const nextFrame = Math.min(upper, Math.max(lower, frame));
    if (track.startFrame === nextFrame) return;
    capture();
    tracks = tracks.map(item => item === track ? {
      ...item, startFrame: nextFrame,
      ...(item.pregapFrame != null && item.pregapFrame >= nextFrame ? { pregapFrame: undefined, index00: undefined } : {}),
    } : item);
    draftAnnouncement = (index === 0 ? 'Track 1 starts at ' : 'Cut moved to ') + timeLabel(nextFrame, sampleRate);
  }

  function trimStart(frame: number) {
    const first = tracks[0];
    if (!first) return;
    selection(first.id);
    moveCut(first.id, frame);
  }

  function exactTime(id: string, value: string) {
    const index = tracks.findIndex(track => track.id === id);
    if (index < 0) return;
    const frame = frameAtTime(value, sampleRate);
    const lower = index === 0 ? 0 : tracks[index - 1].startFrame + 1;
    const upper = tracks[index + 1]?.startFrame ?? total;
    if (frame === null || frame < lower || frame >= upper) {
      fail('Enter a valid time between the neighboring cuts.');
    } else {
      error = '';
      moveCut(id, frame);
    }
  }

  function removeCutAt(id: string) {
    const index = tracks.findIndex(track => track.id === id);
    if (!canEdit || index < 1) return;
    capture();
    rangeIds = [];
    rangeAnchor = '';
    if (id === selected) selected = tracks[index + 1]?.id ?? tracks[index - 1].id;
    tracks = tracks.filter(track => track.id !== id);
    draftAnnouncement = 'Cut removed. The earlier track keeps its tags.';
  }

  function focusTrack(id: string) {
    void tick().then(() => document.getElementById(`studio-row-${id}`)?.focus());
  }

  function removeTrackSplit(id: string) {
    const index = tracks.findIndex(track => track.id === id);
    if (!canEdit || index < 1) return;
    const next = tracks[index + 1]?.id ?? tracks[index - 1].id;
    removeCutAt(id);
    selection(next);
    focusTrack(next);
  }

  function mergeTracks() {
    if (!canEdit || rangeIds.length < 2) return;
    const [first, ...rest] = rangeIds;
    const discarded = new Set(rest);
    capture();
    tracks = tracks.filter(track => !discarded.has(track.id));
    selection(first);
    focusTrack(first);
    draftAnnouncement = `${rest.length + 1} tracks merged. The first track keeps its tags.`;
  }

  function canMoveToPlayhead(index: number) {
    return index >= 0 && playhead !== tracks[index].startFrame
      && playhead >= (index === 0 ? 0 : tracks[index - 1].startFrame + 1)
      && playhead < (tracks[index + 1]?.startFrame ?? total);
  }

  function moveToPlayhead(id: string) {
    if (!canMoveToPlayhead(tracks.findIndex(track => track.id === id))) return;
    selection(id);
    moveCut(id, playhead);
  }

  function jump(direction: number) {
    const target = direction > 0 ? tracks.find(track => track.startFrame > playhead) : tracks.slice().reverse().find(track => track.startFrame < playhead);
    if (target) selectTrack(target);
  }

  function timelineKeys(event: KeyboardEvent) {
    if (event.isComposing || event.altKey) return;
    const key = event.key.toLowerCase();
    if ((event.metaKey || event.ctrlKey) && key === 'z') { event.preventDefault(); history(event.shiftKey); }
    else if (!event.metaKey && !event.ctrlKey) {
      if (key === ' ') {
        event.preventDefault();
        if (event.shiftKey) audition();
        else togglePlayback();
      }
      else if (key === 'm') { event.preventDefault(); addCut(); }
      else if (key === 'l') { event.preventDefault(); toggleLoop(); }
      else if (key === '[' || key === ']') { event.preventDefault(); jump(key === '[' ? -1 : 1); }
      else if (key === '+' || key === '=') { event.preventDefault(); zoom(1); }
      else if (key === '-') { event.preventDefault(); zoom(-1); }
      else if (key === 'z' && current) { event.preventDefault(); zoomToCut(); }
      else if (key === 'f') { event.preventDefault(); if (loopEnabled && loopRegion) zoomToLoop(); else zoomToCut(); }
      else if (key === 'b') { event.preventDefault(); if (!event.repeat) compareOriginal(!hearOriginal); }
      else if (key === '?') { event.preventDefault(); if (!event.repeat) showHelp = !showHelp; }
      else if (key === '0') { event.preventDefault(); animateViewport(0, total); }
    }
  }

  function updateTrack(id: string, change: Partial<Track>) {
    if (!canEdit || !tracks.some(track => track.id === id)) return;
    capture();
    tracks = tracks.map(track => track.id === id ? { ...track, ...change } : track);
  }

  function changeRecordingTag(field: string, value: string) {
    if (value === (recordingTag(field) ?? '')) return;
    capture();
    tags = { ...tags, [field]: value };
  }

  async function beginEdit(id: string, field: TrackEdit['field']) {
    if (!canEdit || !tracks.some(track => track.id === id)) return;
    const edit = { id, field };
    editor = edit;
    selection(id);
    await tick();
    if (editor !== edit) return;
    const input = document.getElementById(field === 'start' ? 'studio-cut-start' : 'studio-track-artist') as HTMLInputElement | null;
    input?.focus();
    input?.select();
  }

  function finishEdit(edit: TrackEdit, commit: boolean, refocus = false) {
    if (editor !== edit) return;
    editor = null;
    if (commit) {
      const track = tracks.find(track => track.id === edit.id);
      if (track && edit.field === 'start') {
        const input = document.getElementById('studio-cut-start') as HTMLInputElement | null;
        if (input) exactTime(edit.id, input.value);
      } else if (track) {
        const patch: Tags = {};
        for (const [key] of trackFields) {
          const input = document.getElementById(`studio-track-${key}`) as HTMLInputElement | null;
          if (input && input.value !== (trackTag(track, key) ?? '')) patch[key] = input.value;
        }
        if (Object.keys(patch).length) updateTrack(track.id, { tags: { ...track.tags, ...patch } });
      }
    }
    if (refocus) void tick().then(() => {
      if (editor) return;
      const trigger = edit.field === 'start'
        ? document.querySelector<HTMLElement>(`[data-audio-studio] [data-cut-id="${CSS.escape(edit.id)}"]`)
          ?? document.querySelector<HTMLElement>('[data-audio-studio] .scrubber')
        : document.getElementById(`studio-row-${edit.id}`);
      trigger?.focus({ preventScroll: true });
    });
  }

  function editKeys(event: KeyboardEvent, edit: TrackEdit) {
    if (event.isComposing || (event.key !== 'Enter' && event.key !== 'Escape')) return;
    event.preventDefault();
    event.stopPropagation();
    finishEdit(edit, event.key === 'Enter', true);
  }

  function leaveTrackFields(event: FocusEvent, edit: TrackEdit) {
    if (!(event.currentTarget as HTMLElement).contains(event.relatedTarget as Node | null)) finishEdit(edit, true);
  }

  function prepareTrackMenu(event: MouseEvent) {
    event.preventDefault();
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>('button[data-track-id]');
    if (!button?.dataset.trackId || !canEdit) return;
    trackMenuId = button.dataset.trackId;
    void studioMenus.tracks.open({ anchor: { x: event.clientX, y: event.clientY }, event });
  }

  function trackKeys(event: KeyboardEvent, id: string) {
    if (event.isComposing || event.altKey) return;
    if (event.metaKey || event.ctrlKey) { timelineKeys(event); return; }
    if (event.key === 'F2') {
      event.preventDefault();
      void beginEdit(id, 'tags');
    } else if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) {
      event.preventDefault();
      const row = event.currentTarget as HTMLElement;
      const box = row.getBoundingClientRect();
      row.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: box.left + 24, clientY: box.top + box.height / 2 }));
    } else if (event.key === 'Delete') {
      event.preventDefault();
      removeTrackSplit(id);
    } else if (event.key.toLowerCase() === 'l') {
      event.preventDefault();
      toggleLoop(id);
    } else if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
      event.preventDefault();
      const index = tracks.findIndex(track => track.id === id);
      const neighbor = tracks[index + (event.key === 'ArrowUp' ? -1 : 1)];
      if (!neighbor) return;
      if (event.shiftKey) {
        if (!rangeIds.length) rangeAnchor = id;
        selection(neighbor.id, true);
      }
      focusTrack(neighbor.id);
    }
  }

  async function restoreTrackMenuFocus(open: boolean) {
    if (open) return;
    const id = trackMenuId;
    await tick();
    if (!editor) document.getElementById(`studio-row-${id}`)?.focus({ preventScroll: true });
  }

  function popupKeys(event: KeyboardEvent, menu: keyof typeof studioMenus) {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    event.stopPropagation();
    studioMenus[menu].close();
    if (menu !== 'tracks') studioMenus[menu].focus();
  }

  function recordingTagKeys(event: KeyboardEvent, value: string) {
    if (event.defaultPrevented || event.isComposing || (event.key !== 'Enter' && event.key !== 'Escape')) return;
    const input = event.currentTarget as HTMLInputElement | HTMLTextAreaElement;
    if (input instanceof HTMLTextAreaElement && event.key === 'Enter') return;
    event.preventDefault();
    event.stopPropagation();
    if (event.key === 'Escape') input.value = value;
    input.blur();
  }

  function connectGenre(input: HTMLInputElement) {
    return attachGenrePicker(input, () => libraryTracks.map(track => track.tags?.genre ?? ''),
      () => (savedSettings as { multi_value_separator?: string }).multi_value_separator || '; ');
  }

  function resetRecordingTag(field: string) {
    const copy = { ...tags };
    delete copy[field];
    capture();
    tags = copy;
    document.getElementById(`studio-recording-${field}`)?.focus({ preventScroll: true });
  }

  function save() {
    if (!source) return;
    try {
      saveDraft(source, { ...draft(), view: { playhead, start, end, selected, follow, stereo: stereoView, loop: loopRegion, loopEnabled } });
      dirty = false;
      hasSaved = true;
      draftAnnouncement = 'Draft saved';
    }
    catch (reason) { fail(String(reason)); }
  }

  async function saveWithToolEdits() {
    if (!await settleToolEdits() || !active) return false;
    save();
    return !dirty;
  }

  function reopen() {
    if (!source) return;
    try {
      const stored = readDraft(source);
      if (!stored) return;
      if (playing) void transport('pause');
      capture();
      restore(stored);
      serial = Math.max(serial, ...stored.tracks.map(track => Number(track.id.match(/^segment-(\d+)$/)?.[1] || 0)));
      if (stored.view) {
        const saved = stored.view;
        if (tracks.some(track => track.id === saved.selected)) selection(saved.selected);
        follow = saved.follow === true;
        stereoView = hasStereo && saved.stereo === true;
        const loop = saved.loop;
        loopRegion = loop && Number.isSafeInteger(loop.start) && Number.isSafeInteger(loop.end)
          && loop.start >= 0 && loop.end <= total && loop.end > loop.start ? loop : null;
        loopEnabled = false;
        if ([saved.start, saved.end, saved.playhead].every(Number.isSafeInteger)
          && saved.start >= 0 && saved.end > saved.start && saved.end <= total) {
          viewport(saved.start, saved.end - saved.start);
          seek(saved.playhead, false);
        }
        loopEnabled = loopRegion !== null && saved.loopEnabled === true;
      }
      dirty = false;
      draftAnnouncement = 'Saved draft reopened';
    } catch (reason) { fail(String(reason)); }
  }

  function studioKeys(event: KeyboardEvent) {
    if (!event.isComposing && !event.altKey && (event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's' && source && phase === 'edit') {
      event.preventDefault();
      event.stopPropagation();
      if (editor) finishEdit(editor, true);
      else if (document.activeElement instanceof HTMLElement && document.activeElement.id.startsWith('studio-recording-')) {
        document.activeElement.blur();
      }
      void saveWithToolEdits();
    }
  }

  async function focusOutput(id: string) {
    await tick();
    const heading = document.getElementById(id);
    heading?.focus({ preventScroll: true });
    heading?.scrollIntoView({ block: 'nearest' });
  }

  async function loadCue(path?: string, encoding: 'utf8' | 'windows1252' = 'utf8') {
    if (!source || cueLoading) return;
    const opened = source;
    cueLoading = true;
    try {
      const selectedPath = path || await choosePath({ filters: [{ name: 'CUE sheet', extensions: ['cue'] }] });
      if (!selectedPath || !active) return;
      cuePath = selectedPath;
      cueEncoding = encoding;
      cue = undefined;
      importMode = 'cue';
      error = '';
      const result = await audioCommand<Cue>('cue', { id: opened.id, path: selectedPath, encoding });
      if (active && source?.id === opened.id) cue = result;
    } catch (reason) { if (active) fail(String(reason)); }
    finally { if (active) cueLoading = false; }
  }

  const sharedTags = () => ({ ...importedTags, ...tags });
  const recordingTag = (key: string) => tags[key] ?? importedTags[key] ?? source?.tags?.[key];
  const trackTag = (track: Track, key: string) =>
    track.tags[key] ?? tags[key] ?? track.suggestedTags?.[key] ??
    (track.leadIn && key === 'title' ? 'Lead-in' : recordingTag(key));

  function outputTracks() {
    return tracks.map(({ tags: overrides, suggestedTags, leadIn, position: _position, ...track }) => {
      const values: Tags = { ...(leadIn ? { title: 'Lead-in' } : {}), ...suggestedTags };
      // Native per-track overrides win over shared values. Apply an explicit
      // shared edit to any inherited suggestion before sending that override.
      for (const key of Object.keys(values)) if (Object.hasOwn(tags, key)) values[key] = tags[key];
      return { ...track, leadIn, tags: { ...values, ...overrides } };
    });
  }

  function closeImporter() {
    importMode = null;
    void tick().then(() => document.getElementById('studio-import-cuts')?.focus());
  }

  function applyImport(imported: Track[], defaults: Tags, replace: boolean) {
    if (!source) return;
    const frames = source.frames;
    const valid = imported.filter(track => track.startFrame >= 0 && track.startFrame < frames);
    if (!valid.length) return;
    const pristine = tracks.length === 1 && tracks[0].startFrame === 0 && tracks[0].include
      && !Object.keys(tracks[0].tags).length && !tracks[0].suggestedTags;
    const useCueStarts = importMode === 'cue' && (replace || pristine);
    const combined = new Map<number, Track>();
    for (const track of useCueStarts ? [] : replace ? [initial()] : tracks) combined.set(track.startFrame, track);
    for (const track of valid) {
      const existing = combined.get(track.startFrame);
      combined.set(track.startFrame, {
        ...(existing || track), ...track, id: existing?.id || newId(),
        tags: existing?.tags || {},
        suggestedTags: { ...existing?.suggestedTags, ...track.tags },
        leadIn: false,
      });
    }
    const first = combined.get(0);
    if (first && valid.every(track => track.startFrame > 0)
      && !Object.keys(first.tags).length && !first.suggestedTags?.title) {
      combined.set(0, { ...first, leadIn: true });
    }
    if (combined.size > maxFiles) { fail('A draft can contain up to ' + maxFiles + ' tracks.'); return; }
    capture();
    importedTags = replace ? defaults : { ...importedTags, ...defaults };
    tracks = [...combined.values()].sort((a, b) => a.startFrame - b.startFrame);
    selected = tracks[0].id;
    closeImporter();
    draftAnnouncement = 'Imported splits are ready to adjust.';
  }

  async function writeCue() {
    if (!source) return;
    const importedLeadIn = tracks[0]?.leadIn && tracks[1]?.index01 != null;
    if (tracks.some((track, index) => !track.include && !(index === 0 && importedLeadIn))) {
      fail('A CUE sheet cannot omit audio between tracks or at the end. Restore omitted tracks or export audio files.');
      return;
    }
    error = '';
    try {
      const path = await choosePath({ defaultPath: stem(source.path) + '.cue', filters: [{ name: 'CUE sheet', extensions: ['cue'] }] }, true);
      if (!path || !active) return;
      const warnings = await audioCommand<string[]>('write_cue', { id: source.id, path, segments: outputTracks(), tags: sharedTags() });
      note(['CUE sheet saved to ' + path, ...warnings].join('. '));
    } catch (reason) { if (active) fail(String(reason)); }
  }

  function changeOutput(delayed = false) {
    clearTimeout(outputCheck);
    invalidate();
    checkingOutput = delayed;
    if (!delayed) { void preview(); return; }
    outputCheck = setTimeout(() => {
      checkingOutput = false;
      void preview();
    }, 220);
  }

  async function browseOutput(backup = false) {
    try {
      const previous = backup ? backupDirectory : destination.trim();
      const picked = await choosePath({ directory: true, ...(previous ? { defaultPath: previous } : {}) });
      if (!picked || !active || phase !== 'review' || saveMode !== (backup ? 'replace' : 'folder')) return;
      if (picked === previous && !error) return;
      if (backup) backupDirectory = picked;
      else destination = picked;
      changeOutput();
    } catch (reason) { if (active) fail(String(reason)); }
  }

  async function browseArtwork() {
    try {
      const picked = await choosePath({ filters: [{ name: 'Cover artwork', extensions: ['jpg', 'jpeg', 'png'] }] });
      if (picked && active) { capture(); artworkPath = picked; }
    } catch (reason) { if (active) fail(String(reason)); }
  }

  function replacementAllowed() {
    return (launch.canReplace ? launch.canReplace() : !launch.pendingPaths?.length);
  }

  async function reviewAndSave() {
    if (!canEnterReview || loading) return;
    if (!await settleToolEdits() || !active || phase !== 'edit' || !canEnterReview) return;
    phase = 'review';
    hideToolWindows();
    notice = '';
    if (playing) void transport('pause');
    void focusOutput('studio-save-heading');
    await stopMeasuringTreatment();
    void preview();
  }

  function backToEditing() {
    clearTimeout(outputCheck);
    checkingOutput = false;
    invalidate();
    lastReview = null;
    phase = 'edit';
    if (playing) void transport('pause');
    void focusOutput('audio-split-title');
  }

  async function preview() {
    await tick();
    if (!active || phase !== 'review' || !reviewReady) return;
    if (previewing) { reviewRequested = true; return; }
    if (saveMode === 'replace' && !replacementAllowed()) {
      fail('Apply or clear the pending library changes for these files before replacing them.');
      return;
    }
    const revision = generation;
    const pattern = namePattern.trim();
    previewing = true;
    error = '';
    try {
      const result = await audioCommand<Review>('preview', { request: {
        sources: source ? [source.path] : launch.paths,
        sessionId: source?.id,
        segments: source ? outputTracks() : undefined,
        tags: source ? sharedTags() : {},
        artworkPath: artworkPath || undefined,
        saveMode,
        destination: saveMode === 'folder' ? destination.trim() : '',
        backupDirectory: saveMode === 'replace' ? backupDirectory || undefined : undefined,
        folderPattern: createFolder ? folderPattern.trim() : undefined,
        format, splitMethod: treated ? 'exact' : canCopy ? splitMethod : undefined,
        ...(treated ? { treatment } : {}),
        bitrate: format === 'mp3' && !copying ? bitrate : undefined,
        namePattern: split && pattern === defaultSplitFilePattern ? undefined : pattern || undefined,
      } });
      if (active && phase === 'review' && generation === revision) {
        review = result;
        lastReview = result;
      } else discardReview(result);
    } catch (reason) { if (active && generation === revision) fail(String(reason)); }
    finally {
      if (active) {
        previewing = false;
        if (reviewRequested) {
          reviewRequested = false;
          void preview();
        }
      }
    }
  }

  function stopOutputAudition() {
    clearTimeout(outputPoll);
    if (outputAudition === null || !review) return;
    const sequence = ++transportVersion;
    outputClosing = (source
      ? audioCommand<void>('transport', { id: source.id, action: 'pause', sequence })
      : closeAudioSplitSession(review.id)).catch(console.error);
    outputAudition = null;
  }

  async function auditionOutput(index: number) {
    const plan = review;
    if (!plan || phase !== 'review' || reviewingOutput || working) return;
    if (outputAudition === index) { stopOutputAudition(); return; }
    const id = plan.id;
    const owner = source?.id ?? id;
    const sequence = ++transportVersion;
    const current = () => active && review?.id === id && transportVersion === sequence;
    clearTimeout(outputPoll);
    outputAudition = index;

    async function poll() {
      if (!current()) return;
      try {
        const status = await audioCommand<Status>('status', { id: owner, sequence, audition: true });
        if (!current()) return;
        if (!status.loading && status.sequence === sequence && !status.playing) {
          stopOutputAudition();
          return;
        }
        outputPoll = setTimeout(() => void poll(), 120);
      } catch (reason) {
        if (current()) { stopOutputAudition(); fail(String(reason)); }
      }
    }

    try {
      await outputClosing;
      if (!current()) return;
      if (!source) rememberAudioSplitSession(id, true);
      await audioCommand('audition_output', { id, index, sequence });
      if (current()) void poll();
    } catch (reason) {
      if (current()) { stopOutputAudition(); fail(String(reason)); }
    }
  }

  async function runExport() {
    const plan = review;
    if (!plan || previewing || working) return;
    if (saveMode === 'replace' && !replacementAllowed()) {
      fail('Apply or clear the pending library changes for these files before replacing them.');
      return;
    }
    stopOutputAudition();
    phase = 'export';
    working = true;
    stopping = false;
    error = '';
    if (playing) void transport('pause');
    let settled = false;
    async function poll() {
      if (settled || !active || !plan) return;
      try {
        const update = await audioCommand<Progress>('progress', { id: plan.id });
        if (!settled && active && update) progress = update;
      } catch { /* The export response owns the reported error. */ }
      if (!settled && active) setTimeout(() => void poll(), 250);
    }
    progress = { completed: 0, total: plan.outputs.length, phase: 'Preparing audio' };
    try {
      const exporting = audioCommand<ExportResult>('export', { id: plan.id });
      void poll();
      const result = await exporting;
      if (active) {
        finished = result.paths;
        backups = result.backups || [];
        exportWarnings = result.warnings || [];
        publishedId = plan.id;
        phase = 'finished';
        review = null;
        dirty = false;
        canUndoReplacement = !!result.replaced?.length;
        libraryRemoved = result.replaced?.length ? result.replaced : undefined;
        canRefreshLibrary = false;
        void focusOutput('audio-split-done-heading');
        if (launch.refreshLibrary && libraryRemoved) {
          canRefreshLibrary = true;
          await refreshCompleted();
        } else if (launch.refreshLibrary) {
          void audioCommand<boolean>('can_add', { id: plan.id }).then(canAdd => {
            if (active && publishedId === plan.id) canRefreshLibrary = canAdd;
          }).catch(() => {});
        }
      }
    } catch (reason) {
      if (active) { fail(String(reason)); phase = 'review'; review = null; }
    } finally {
      settled = true;
      if (active) { progress = null; working = false; stopping = false; }
    }
  }

  async function stopExport() {
    if (!review || stopping) return;
    stopping = true;
    try { await audioCommand('cancel', { id: review.id }); }
    catch (reason) { fail(String(reason)); stopping = false; }
  }

  async function revealFinished() {
    if (!publishedId) return;
    try { await audioCommand('reveal', { id: publishedId }); }
    catch (reason) { if (active) fail(String(reason)); }
  }

  async function refreshCompleted() {
    if (!launch.refreshLibrary || refreshingLibrary) return;
    refreshingLibrary = true;
    try {
      if (!await launch.refreshLibrary(restored.length ? restored : finished, libraryRemoved)) {
        throw new Error('The files were saved on disk. The library view could not be updated.');
      }
      canRefreshLibrary = false;
      error = '';
    } catch (reason) { if (active) fail(String(reason)); }
    finally { refreshingLibrary = false; }
  }

  async function undoReplacement() {
    if (!canUndoReplacement || working) return;
    working = true;
    error = '';
    try {
      const result = await audioCommand<ReplacementUndo>('undo_replacement', { id: publishedId });
      if (!active) return;
      restored = result.paths;
      exportWarnings = result.warnings || [];
      libraryRemoved = result.removed;
      canUndoReplacement = false;
      if (launch.refreshLibrary) {
        canRefreshLibrary = true;
        await refreshCompleted();
      }
      if (active) void focusOutput('audio-split-done-heading');
    } catch (reason) { if (active) fail(String(reason)); }
    finally { if (active) working = false; }
  }

  function keepEditing() {
    closeRequested = false;
  }

  function restoreCloseFocus(event: Event) {
    event.preventDefault();
    if (!active || closeRequested) return;
    const target = closeTrigger?.isConnected ? closeTrigger : document.getElementById('studio-close');
    target?.focus({ preventScroll: true });
  }

  async function saveAndClose() {
    if (await saveWithToolEdits()) onclose();
    else closeSaveError = error || toolWindowError;
  }

  async function requestClose() {
    if (closeRequested || phase === 'export' || working) return;
    if (phase === 'edit' && (!await settleToolEdits() || !active)) return;
    if (!dirty || !split) { onclose(); return; }
    closeTrigger = document.activeElement instanceof HTMLElement && document.activeElement.closest('[data-audio-studio]')
      ? document.activeElement : null;
    closeSaveError = '';
    closeRequested = true;
  }


</script>

<Dialog.Root open={true} onOpenChange={open => { if (!open) requestClose(); }}>
  <Dialog.Content class="audio-split-dialog" overlayProps={{ class:'controller-dialog-overlay audio-split-overlay' }}
    showCloseButton={false} data-audio-studio aria-labelledby="audio-split-title" aria-describedby="audio-split-summary"
    onkeydown={studioKeys}
    onOpenAutoFocus={event => event.preventDefault()}
    onCloseAutoFocus={event => event.preventDefault()}
    onEscapeKeydown={(event) => {
      event.preventDefault();
      event.stopPropagation();
      if (closeRequested) keepEditing();
      else if (recipesOpen) recipesOpen = false;
      else if (importMenuOpen) studioMenus.imports.close();
      else if (importMode) closeImporter();
      else if (waveformMenuOpen) studioMenus.waveform.close();
      else if (editor) finishEdit(editor, false, true);
      else requestClose();
    }}>
    <div class="studio" data-controller-popup>
      <header class="header">
        <div class="identity">
          <h2 id="audio-split-title" tabindex="-1" {@attach focusHeader}>{title}</h2>
          <p id="audio-split-summary">
            {#if split && source}<span title={source.path}>{leaf(source.path)}</span><span class="duration">{duration}</span>
            {:else if split && launch.paths.length === 1}<span>{leaf(launch.paths[0])}</span>
            {:else if split}Choose a recording or CUE sheet
            {:else}{launch.paths.length} {launch.paths.length === 1 ? 'source file' : 'source files'}{/if}
          </p>
        </div>
      </header>
      <Button id="studio-close" variant="ghost" size="icon-sm" title="Close" aria-label="Close audio studio" onclick={requestClose} disabled={working}><X size={16}/></Button>
      <div class="scroll">
        {#if error}<p class="error" role="alert">{error}</p>{/if}
        <div class="sr-only" aria-live="polite">{draftAnnouncement}</div>
        {#if launch.pendingPaths?.length && phase === 'edit'}<p class="help">These files have pending library edits. Audio tools use the saved tags from disk.</p>{/if}
        {#if loading}
          <div class="opening">
            <p role="status"><AudioLines size={20}/> {loadingInfo ? 'Reading the recording…' : 'Opening the recording…'}</p>
            {#if loadingInfo?.frames}
              <ProgressBar class="recording-scan" max={loadingInfo.frames} value={Math.min(loadingInfo.frames, loadingInfo.scannedFrames)} aria-label="Recording scan progress">
                <ProgressTrack class="recording-scan-track"><ProgressIndicator class="recording-scan-fill"/></ProgressTrack>
              </ProgressBar>
            {:else if loadingInfo?.scannedFrames}
              <span class="help">Read {timeLabel(loadingInfo.scannedFrames, loadingInfo.sampleRate, 0)} so far</span>
            {/if}
            {#if loadingInfo && loadingInfo.overview.length && loadingInfo.scannedFrames}
              {@const interim = peakValues(loadingInfo.overview)}
              {@const extent = Math.max(loadingInfo.frames, loadingInfo.scannedFrames)}
              <AudioSplitWaveform start={0} end={extent} total={extent} sampleRate={loadingInfo.sampleRate}
                overview={interim} overviewRms={peakValues(loadingInfo.overviewRms || [])} coverage={loadingInfo.scannedFrames / extent}
                playhead={0} tracks={[]} selected="" readonly/>
            {/if}
          </div>
        {:else if split && !source}
          <div class="empty">
            <FolderOpen size={26}/>
            {#if launch.paths.length > 1}<p>Splitting takes one recording at a time.</p>{/if}
            <Button onclick={() => chooseRecording(true)}>Choose recording or CUE…</Button>
            {#if pendingCue}<p>{leaf(pendingCue)} · choose its master recording</p>{/if}
          </div>
        {:else if phase === 'edit'}
          {#if split && source}
            <div class="timeline-area">
              <div class="toolbar transport">
                <Button variant="secondary" size="icon-sm" aria-label={playing ? 'Pause audio' : 'Play audio'} title={playing ? 'Pause (Space)' : 'Play (Space)'} aria-busy={buffering} onclick={togglePlayback}>
                  {#if playing}<Pause size={16}/>{:else}<Play size={16}/>{/if}
                </Button>
                <Button class="loop-toggle" variant="ghost" size="icon-sm" aria-label="Loop preview" aria-pressed={loopEnabled}
                  aria-keyshortcuts="L" title={loopEnabled ? 'Disable loop (L)' : 'Enable loop (L)'} onclick={() => toggleLoop()}>
                  <Repeat size={15}/>
                </Button>
                <span class="time">{readout}</span>
                <Button variant="ghost" size="sm" onclick={addCut} disabled={!canEdit}><Scissors size={15}/> Add cut</Button>
                <div class="waveform-options">
                  {#if hasStereo}
                    <Button class="stereo-toggle" variant="ghost" size="sm" aria-label="Show left and right channels"
                      aria-pressed={stereoView}
                      title={stereoView ? 'Switch to combined waveform' : 'Show left and right channels'}
                      onclick={() => stereoView = !stereoView}>
                      {stereoView ? 'Left + Right' : 'Combined'}
                    </Button>
                  {/if}
                  {#each toolModes as kind (kind)}
                    {@const Icon = audioToolIcons[kind]}
                    <Button id={'studio-tool-' + kind} class="audio-tool-trigger" variant="ghost" size="icon-sm" title={toolHint(kind)}
                      aria-label={toolNames[kind]} data-active={toolEffectCount(kind, treatment) > 0}
                      aria-description={toolEffectCount(kind, treatment) ? toolHint(kind) : undefined}
                      onclick={() => toggleToolWindow(kind)}><Icon size={16}/></Button>
                  {/each}
                  <Menu bind:open={waveformMenuOpen} controller={studioMenus.waveform}
                    onOpenChange={open => { if (open) copiedTreatment = readCopiedTreatment(); }}>
                    <MenuTrigger id="studio-waveform-options" data-slot="dropdown-menu-trigger" class={buttonVariants({ variant: 'ghost', size: 'icon-sm' })} aria-label="Waveform options" title="Waveform options"><Ellipsis size={18}/></MenuTrigger>
                    <MenuContent class="app-select-menu studio-waveform-options" align="end" onkeydowncapture={event => popupKeys(event, 'waveform')}>
                      <MenuItem onSelect={() => jump(-1)}><ChevronLeft size={15}/> Previous cut <MenuShortcut>[</MenuShortcut></MenuItem>
                      <MenuItem onSelect={() => jump(1)}><ChevronRight size={15}/> Next cut <MenuShortcut>]</MenuShortcut></MenuItem>
                      <MenuSeparator/>
                      <MenuItem onSelect={() => zoom(1)}>Zoom in <MenuShortcut>+</MenuShortcut></MenuItem>
                      <MenuItem onSelect={() => zoom(-1)}>Zoom out <MenuShortcut>−</MenuShortcut></MenuItem>
                      <MenuItem onSelect={zoomToCut} disabled={!current}>Zoom to cut <MenuShortcut>Z</MenuShortcut></MenuItem>
                      <MenuItem onSelect={zoomToLoop} disabled={!loopRegion}>Zoom to loop</MenuItem>
                      {#if loopRegion}<MenuItem onSelect={clearLoop}>Clear loop</MenuItem>{/if}
                      <MenuItem onSelect={() => animateViewport(0, total)}>Fit recording <MenuShortcut>0</MenuShortcut></MenuItem>
                      <MenuItem onSelect={() => compareOriginal(!hearOriginal)}>{hearOriginal ? 'Listen to treated' : 'Listen to original'} <MenuShortcut>B</MenuShortcut></MenuItem>
                      <MenuSeparator/>
                      <MenuItem onSelect={() => void copyRecipe()}>Copy treatment settings</MenuItem>
                      <MenuItem disabled={!copiedTreatment} onSelect={() => { if (copiedTreatment) void applyRecipe(copiedTreatment, 'Copied treatment'); }}>Paste treatment settings</MenuItem>
                      <MenuItem onSelect={openRecipes}>Treatment recipes…</MenuItem>
                      <MenuSeparator/>
                      <MenuCheckboxItem value="follow" bind:checked={follow} closeOnSelect>
                        <span class="follow-check" aria-hidden="true"><MenuItemIndicator><Check size={14}/></MenuItemIndicator></span> Follow playhead
                      </MenuCheckboxItem>
                      <MenuItem onSelect={() => showHelp = !showHelp}>{showHelp ? 'Hide waveform shortcuts' : 'Waveform shortcuts'}</MenuItem>
                    </MenuContent>
                  </Menu>
                </div>
              </div>
              {#if editor?.field === 'start' && editingTrack}
                {@const edit = editor}
                <label class="cut-time-edit">
                  {editingTrack.id === tracks[0].id ? 'Track 1 start' : 'Cut time'}
                  <Input id="studio-cut-start" aria-label="Track start time" value={timeLabel(editingTrack.startFrame, sampleRate, 6)}
                    onkeydown={event => editKeys(event, edit)} onblur={() => finishEdit(edit, true)}/>
                </label>
              {/if}
              <AudioSplitWaveform {start} {end} {total} {sampleRate} tile={start === 0 && end === total ? null : tile}
                {overview} {stereoOverview} {overviewRms} {stereoOverviewRms} stereo={stereoView} {playhead} {visualPlayhead} {playing} {tracks} {selected}
                {loopRegion} {loopEnabled} onloop={setLoop} ontrack={selectTrack} trackTitle={track => trackTag(track, 'title')} onmove={movePreview} onseek={seek}
                oncancel={cancelPreview} onselect={selection} onmarker={moveCut} onviewport={viewport} onwheelzoom={wheelZoom}
                ongesture={waveformGesture} onkey={timelineKeys} onplaymarker={playFromCut} onadd={addCutAt}
                oneditmarker={id => beginEdit(id, 'start')} onremovemarker={removeCutAt} ontrim={trimStart}
                onrestoremarker={id => updateTrack(id, { include: true })}
                onmovetoplayhead={moveToPlayhead} canmove={id => canMoveToPlayhead(tracks.findIndex(track => track.id === id))}/>
              {#if listeningError}
                <div class="audio-tool-error" role="status"><span class="waveform-hint">{listeningError}</span><Button variant="ghost" size="sm" onclick={() => retuneListening(true)}>Retry preview</Button></div>
              {/if}
              {#if treatmentError}
                <div class="audio-tool-error" role="status"><span class="waveform-hint">{treatmentError}</span><Button variant="ghost" size="sm" onclick={() => measureTreatment(0)}>Retry measurement</Button></div>
              {/if}
              {#if toolWindowError}
                <p class="waveform-hint" role="status">{toolWindowError}</p>
              {/if}
              {#if showHelp}
                <p class="help">Scroll to zoom, middle-drag to pan, and Shift-drag to mark a loop. Space plays or pauses. L toggles the loop, B compares Original and Treated, F fits the cut or loop, and 0 fits the recording. Shift+Space auditions the cut and M adds a cut. In the track list, Shift-click or Shift+Up/Down selects a range, F2 edits tags, and Delete removes a split.</p>
              {/if}
            </div>
            {#if importMode}
              <AudioSplitImport {source} {playhead} mode={importMode} {cue} {cueEncoding} {cueLoading}
                onencoding={(encoding) => loadCue(cuePath, encoding)} onapply={applyImport} oncancel={closeImporter} onerror={fail}/>
            {/if}

            <div class="editor">
              <section class="track-panel" aria-label="Tracks">
                <div class="column-top">
                  <h3>Tracks</h3>
                  <Menu bind:open={importMenuOpen} controller={studioMenus.imports}>
                    <MenuTrigger id="studio-import-cuts" data-slot="dropdown-menu-trigger" class={buttonVariants({ variant: 'secondary', size: 'sm' })} disabled={!canEdit || cueLoading}>Import splits <ChevronDown size={13}/></MenuTrigger>
                    <MenuContent class="app-select-menu studio-import-menu" align="end" onkeydowncapture={event => popupKeys(event, 'imports')}>
                      <MenuItem onSelect={() => { void loadCue(); }}><FilePlus2 size={16}/> Import CUE…</MenuItem>
                      <MenuItem onSelect={() => importMode = 'discogs'}><Disc3 size={16}/> Use Discogs…</MenuItem>
                    </MenuContent>
                  </Menu>
                </div>
                <Menu controller={studioMenus.tracks} onOpenChange={restoreTrackMenuFocus}>
                      <div class="track-list" bind:this={list} role="group" aria-label="Track list" oncontextmenu={prepareTrackMenu}>
                        {#each tracks as track, index (track.id)}
                          {@const trackEnd = tracks[index + 1]?.startFrame ?? total}
                          {@const edit = editor?.id === track.id && editor.field !== 'start' ? editor : null}
                          {#snippet trackNumber()}
                            <span class="track-number" title={track.position}>{String(index + 1).padStart(2, '0')}</span>
                          {/snippet}
                          {#snippet trackTimes()}
                            <span class="track-time"><span class="sr-only">Start </span>{timeLabel(track.startFrame, sampleRate, 1)}</span>
                            <span class="track-time"><span class="sr-only">Length </span>{timeLabel(trackEnd - track.startFrame, sampleRate, 1)}</span>
                          {/snippet}
                          {#if edit}
                            <div class="track editing">
                              {@render trackNumber()}
                              <div class="track-edit-fields" onfocusout={event => leaveTrackFields(event, edit)}>
                                {#each trackFields as [key, label] (key)}
                                  <Input id={`studio-track-${key}`} aria-label={`${label} for track ${index + 1}`} placeholder={label}
                                    value={trackTag(track, key) ?? ''} maxlength={512} spellcheck={false} onkeydown={event => editKeys(event, edit)}/>
                                {/each}
                              </div>
                              {@render trackTimes()}
                            </div>
                          {:else}
                            <SdkButton type="button" id={`studio-row-${track.id}`} data-track-id={track.id}
                              class={['track', { excluded: !track.include }]} aria-pressed={selected === track.id || rangeIds.includes(track.id)}
                              data-current={selected === track.id} aria-haspopup="menu" aria-keyshortcuts="Shift+F10 F2 Delete L"
                              onclick={event => selectTrack(track, event.shiftKey)} onkeydown={event => trackKeys(event, track.id)}>
                              {@render trackNumber()}
                              <span class="track-title">
                                <strong>{trackTag(track, 'title') || `Track ${index + 1}`}</strong>
                                {#if !track.include}<small>Omitted in saved draft</small>
                                {:else if trackTag(track, 'artist')}<small>{trackTag(track, 'artist')}</small>{/if}
                              </span>
                              {@render trackTimes()}
                            </SdkButton>
                          {/if}
                        {/each}
                      </div>
                  <MenuContent class="app-select-menu studio-track-menu" aria-label="Track actions" finalFocus="none" onkeydowncapture={event => popupKeys(event, 'tracks')}>
                    <MenuItem onSelect={() => { void beginEdit(trackMenuId, 'tags'); }}>Edit Artist/Title <MenuShortcut aria-hidden="true">F2</MenuShortcut></MenuItem>
                    <MenuItem onSelect={() => loopTrack(trackMenuId)}>Loop track</MenuItem>
                    <MenuItem onSelect={() => moveToPlayhead(trackMenuId)} disabled={!canMoveToPlayhead(menuTrackIndex)}>Move split to playhead</MenuItem>
                    {#if canMerge}<MenuItem onSelect={mergeTracks}>Merge adjacent tracks</MenuItem>{/if}
                    {#if menuTrackIndex === 0}
                      <MenuItem onSelect={() => trimStart(0)} disabled={!tracks[0]?.startFrame}>Reset start</MenuItem>
                    {:else}
                      <MenuItem onSelect={() => removeTrackSplit(trackMenuId)} disabled={menuTrackIndex < 1}>Remove split <MenuShortcut aria-hidden="true">Del</MenuShortcut></MenuItem>
                    {/if}
                  </MenuContent>
                </Menu>
              </section>
              <section class="tag-panel" aria-label="Recording tags">
                <div class="toolbar draft-actions" role="group" aria-label="Draft actions">
                  <Button variant="ghost" size="icon-sm" onclick={() => history(false)} title={'Undo ' + (previous.at(-1)?.label ?? 'draft change')} aria-label="Undo draft change" disabled={!previous.length || !canEdit}><Undo2 size={16}/></Button>
                  <Button variant="ghost" size="icon-sm" onclick={() => history(true)} title={'Redo ' + (next.at(-1)?.label ?? 'draft change')} aria-label="Redo draft change" disabled={!next.length || !canEdit}><Redo2 size={16}/></Button>
                  <Button variant="ghost" size="sm" onclick={saveWithToolEdits} disabled={!canEdit || settlingTools} aria-keyshortcuts="Control+s Meta+s">Save draft</Button>
                  <Button variant="ghost" size="sm" onclick={reopen} disabled={!hasSaved || !canEdit}>{hasSaved && !dirty && !previous.length ? 'Continue draft' : 'Reopen draft'}</Button>
                </div>
                <div class="recording-editor">
                  {#snippet recordingField(key: string, label: string)}
                    {@const override = Object.hasOwn(tags, key)}
                    {@const value = recordingTag(key) ?? ''}
                    {@const inputId = `studio-recording-${key}`}
                    {@const accessibleLabel = `Recording ${label.toLowerCase()}`}
                    <div class={['tag-field', { edited: override }]}>
                      <label for={inputId}>{label}</label>
                      <div class="tag-input">
                        {#if key === 'comment'}
                          <Textarea id={inputId} class="field-input" data-slot="textarea" aria-label={accessibleLabel} {value}
                            spellcheck={false} placeholder={override && !value ? 'Cleared' : ''} maxlength={512}
                            onkeydown={event => recordingTagKeys(event, value)} onblur={event => changeRecordingTag(key, event.currentTarget.value)}
                            disabled={!canEdit}/>
                        {:else}
                          <Input id={inputId} class="field-input" data-slot="input" aria-label={accessibleLabel} {value}
                            spellcheck={false} placeholder={override && !value ? 'Cleared' : ''}
                            inputmode={key === 'year' || key === 'disc' ? 'numeric' : key === 'bpm' ? 'decimal' : 'text'}
                            maxlength={512} onkeydown={event => recordingTagKeys(event, value)}
                            onblur={event => changeRecordingTag(key, event.currentTarget.value)}
                            {@attach key === 'genre' ? connectGenre : undefined} disabled={!canEdit}/>
                        {/if}
                        {#if override}
                          <SdkButton type="button" class="tag-reset" aria-label={`Reset recording ${label.toLowerCase()}`} title="Use source value"
                            onclick={() => resetRecordingTag(key)} disabled={!canEdit}><RotateCcw size={13}/></SdkButton>
                        {/if}
                      </div>
                    </div>
                  {/snippet}
                  <div class="recording-fields">
                    {#each recordingFields as [key, label] (key)}
                      {@render recordingField(key, label)}
                    {/each}
                    <div class="recording-pair">
                      {@render recordingField('bpm', 'BPM')}
                      {@render recordingField('key', 'Key')}
                    </div>
                  </div>
                  <div class="studio-cover cover-compact">
                    <div class="cover-compact-header">
                      <h3>Cover art</h3>
                      <Menu controller={studioMenus.cover}>
                        <MenuTrigger class="cover-edit" aria-label="Edit cover art" title="Edit cover art" disabled={!canEdit}><Pencil size={14}/></MenuTrigger>
                        <MenuContent class="app-select-menu studio-cover-menu" align="end" onkeydowncapture={event => popupKeys(event, 'cover')}>
                          <MenuItem onSelect={() => { void browseArtwork(); }}>Replace cover…</MenuItem>
                          {#if artworkPath}
                            <MenuItem onSelect={() => { capture(); artworkPath = ''; }}>Use recording cover</MenuItem>
                          {/if}
                        </MenuContent>
                      </Menu>
                    </div>
                    {#snippet coverTarget(image: string)}
                      {#if image}
                        <SdkButton type="button" class="cover-main-image" aria-label="Replace front cover" title={artworkPath ? leaf(artworkPath) : 'Recording cover'} onclick={browseArtwork} disabled={!canEdit}>
                          <img src={image} alt="Front cover" draggable="false"/>
                        </SdkButton>
                      {:else}
                        <SdkButton type="button" class="cover-empty-target" aria-label="Choose front cover…" title={artworkPath ? leaf(artworkPath) : undefined} onclick={browseArtwork} disabled={!canEdit}>
                          <ImagePlus size={20} aria-hidden="true"/>
                          <span>{artworkPath ? leaf(artworkPath) : 'Choose cover…'}</span>
                        </SdkButton>
                      {/if}
                    {/snippet}
                    {#await coverPreview}
                      {@render coverTarget('')}
                    {:then image}
                      {@render coverTarget(image)}
                    {/await}
                  </div>
                </div>
              </section>
            </div>
          {:else if launch.paths.length > maxFiles}
            <p class="help">Choose up to {maxFiles} files at a time for conversion.</p>
          {:else}
            <ul class="conversion-sources" aria-label="Source files">
              {#each launch.paths as path (path)}<li title={path}>{leaf(path)}</li>{/each}
            </ul>
          {/if}
        {:else}
          <AudioSplitOutput {split} {canCopy} {copying} {treated} {effects} blocked={working} {sampleRate} bind:saveMode bind:destination bind:backupDirectory
            sourcePaths={source ? [source.path] : launch.paths} bind:createFolder bind:folderPattern bind:format bind:splitMethod bind:bitrate bind:namePattern
            review={review ?? (reviewReady && reviewingOutput ? lastReview : null)} {progress} {finished} {backups} {restored} {exportWarnings}
            previewing={reviewingOutput} {outputAudition} onbrowse={browseOutput} onchange={changeOutput} onaudition={auditionOutput}/>
        {/if}
      </div>
      <footer class="footer">
        <div class="notice" role="status">{notice}</div>
        <div class="toolbar">
          {#if phase === 'export'}
            <Button variant="secondary" size="sm" onclick={stopExport} disabled={stopping}><CircleStop size={16}/>{stopping ? 'Stopping…' : 'Stop export'}</Button>
          {:else if phase === 'finished'}
            {#if finished.length && !restored.length}<Button variant="secondary" size="sm" onclick={revealFinished} disabled={working}>Open output folder</Button>{/if}
            {#if canRefreshLibrary}<Button variant="secondary" size="sm" onclick={refreshCompleted} disabled={working || refreshingLibrary}>{libraryRemoved ? 'Update library view' : 'Add to open library'}</Button>{/if}
            {#if canUndoReplacement}<Button variant="secondary" size="sm" onclick={undoReplacement} disabled={working || refreshingLibrary}>Undo replacement</Button>{/if}
            <Button size="sm" onclick={onclose} disabled={working || refreshingLibrary}>Done</Button>
          {:else if phase === 'review'}
            <Button variant="secondary" size="sm" onclick={backToEditing}>Back to editing</Button>
            {#if review}
              <Button size="sm" onclick={runExport} disabled={working || previewing || !review.outputs.length}>
                {saveMode === 'replace' ? 'Replace with' : 'Save'} {review.outputs.length} {split ? (review.outputs.length === 1 ? 'track' : 'tracks') : (review.outputs.length === 1 ? 'file' : 'files')}
              </Button>
            {:else if reviewReady}
              <Button size="sm" onclick={preview} disabled={reviewingOutput}>{reviewingOutput ? 'Checking files…' : 'Review files'}</Button>
            {/if}
          {:else}
            {#if source}<Button variant="secondary" size="sm" onclick={writeCue} disabled={working}>Export CUE…</Button>{/if}
            <Button size="sm" onclick={reviewAndSave} disabled={loading || settlingTools || !canEnterReview}>Review &amp; save</Button>
          {/if}
        </div>
      </footer>
    </div>
    <Dialog.Root bind:open={recipesOpen}>
      <Dialog.Content class="split-close-dialog" overlayProps={{ class: 'split-close-overlay' }}
        onCloseAutoFocus={restoreRecipeFocus}
        onEscapeKeydown={event => { event.preventDefault(); event.stopPropagation(); recipesOpen = false; }}>
        <Dialog.Title>Treatment recipes</Dialog.Title>
        <Dialog.Description>Save EQ, dynamics and repair settings for other recordings. Selected repair areas stay with this recording.</Dialog.Description>
        <form class="recipe-save" onsubmit={saveRecipe}>
          <Input aria-label="Recipe name" placeholder="Recipe name" bind:value={recipeName} oninput={() => recipeError = ''}/>
          <Button type="submit" size="sm" disabled={!recipeName.trim() || settlingTools}>Save current settings</Button>
        </form>
        {#if recipeError}<p class="error" role="alert">{recipeError}</p>{/if}
        {#if recipes.length}
          <ul class="recipe-list" aria-label="Saved recipes">
            {#each recipes as recipe (recipe.name)}
              <li>
                <Button variant="ghost" size="sm" disabled={settlingTools} onclick={() => void applyRecipe(recipe.settings, recipe.name)} title={'Apply ' + recipe.name}>{recipe.name}</Button>
                <Button variant="ghost" size="icon-sm" onclick={() => removeRecipe(recipe.name)} aria-label={'Remove ' + recipe.name} title={'Remove ' + recipe.name}><X size={15}/></Button>
              </li>
            {/each}
          </ul>
        {/if}
      </Dialog.Content>
    </Dialog.Root>
    <Dialog.Root open={closeRequested} onOpenChange={open => { if (!open) keepEditing(); }}>
      <Dialog.Content class="split-close-dialog" role="alertdialog" showCloseButton={false}
        overlayProps={{ class: 'split-close-overlay' }}
        onInteractOutside={event => event.preventDefault()}
        onOpenAutoFocus={event => { event.preventDefault(); document.getElementById('keep-split-editing')?.focus(); }}
        onCloseAutoFocus={restoreCloseFocus}
        onEscapeKeydown={event => { event.preventDefault(); event.stopPropagation(); keepEditing(); }}>
        <Dialog.Title>Save split draft?</Dialog.Title>
        <Dialog.Description>Keep your cuts and tag changes for later.</Dialog.Description>
        {#if closeSaveError}<p class="error" role="alert">{closeSaveError}</p>{/if}
        <footer class="close-actions">
          <Button id="keep-split-editing" variant="ghost" size="sm" onclick={keepEditing}>Keep editing</Button>
          <Button variant="destructive" size="sm" onclick={onclose}>Discard changes</Button>
          <Button size="sm" onclick={saveAndClose}>Save &amp; close</Button>
        </footer>
      </Dialog.Content>
    </Dialog.Root>
  </Dialog.Content>
</Dialog.Root>

<style>
  .recipe-save, .recipe-list li { display:flex; align-items:center; gap:var(--sp-2); }
  .recipe-save :global(input) { min-width:0; flex:1; }
  .recipe-list { list-style:none; margin:0; padding:0; max-height:min(260px,35dvh); overflow:auto; }
  .recipe-list li { justify-content:space-between; }
  :global(.audio-split-dialog[data-slot="dialog-content"]) { width:min(1040px,calc(100vw - 24px)); max-width:none; max-height:calc(100dvh - 24px); padding:0; z-index:133; border:1px solid var(--ui-muted-border); border-radius:var(--r-dialog); background:var(--panel); color:var(--text); display:flex; flex-direction:column; box-shadow:0 16px 48px rgb(0 0 0 / .32); }
  :global(.audio-split-overlay[data-slot="dialog-overlay"]) { z-index:132; }
  :global(.split-close-overlay[data-slot="dialog-overlay"]) { z-index:134; }
  :global(.split-close-dialog[data-slot="dialog-content"]) {
    z-index:135; width:min(420px, calc(100vw - 24px)); max-width:none;
    padding:var(--sp-5); gap:var(--sp-2); border:1px solid var(--ui-muted-border);
    background:var(--panel); color:var(--text); border-radius:var(--r-dialog-compact);
    box-shadow:var(--ui-menu-shadow);
  }
  :global(.split-close-dialog [data-slot="dialog-title"]) { font-size:var(--type-dialog-size); font-weight:var(--fw-strong); line-height:var(--lh-tight); }
  :global(.split-close-dialog [data-slot="dialog-description"]) { color:var(--muted); }
  .close-actions { display:flex; justify-content:flex-end; flex-wrap:wrap; gap:var(--sp-1) var(--sp-2); margin-block-start:var(--sp-3); }
  .close-actions :global(button) { border-radius:var(--r-pill); }
  .close-actions :global(button[data-variant="destructive"]:focus-visible) { outline-color:var(--del); }
  :global(:is(.studio-import-menu,.studio-waveform-options,.studio-cover-menu)) { z-index:134; width:max-content; }
  :global(body:has([data-audio-studio]) .genre-options[data-slot="popover-content"]) { z-index:137; }
  :global(.studio-track-menu) { z-index:137; }
  :global(:is(.studio-import-menu,.studio-waveform-options,.studio-cover-menu,.studio-track-menu) :is([data-slot="menu-item"],[data-slot="menu-checkbox-item"])) {
    display:flex; align-items:center; gap:var(--sp-2); min-height:30px; padding:0 var(--sp-2); border-radius:var(--r-control); cursor:default;
  }
  :global(:is(.studio-import-menu,.studio-waveform-options,.studio-cover-menu,.studio-track-menu) :is([data-slot="menu-item"],[data-slot="menu-checkbox-item"]):is(:focus,[data-highlighted],:hover):not([aria-disabled="true"])) {
    outline:none; background:var(--row-hover); color:var(--text);
  }
  :global(:is(.studio-import-menu,.studio-waveform-options,.studio-cover-menu,.studio-track-menu) [aria-disabled="true"]) { opacity:.45; }
  :global(:is(.studio-waveform-options,.studio-track-menu) [data-slot="menu-shortcut"]) { margin-inline-start:auto; color:var(--muted); }
  :global(.studio-waveform-options [data-slot="menu-separator"]) { height:1px; margin:var(--sp-1) 0; background:var(--ui-muted-border); }
  .follow-check { width:14px; color:var(--accent-ink); }
  .studio { --studio-inset:var(--sp-4); position:relative; display:grid; min-height:0; grid-template-rows:auto minmax(0,1fr) auto; }
  .header, .footer { display:flex; align-items:center; gap:var(--sp-3); padding:var(--sp-3) var(--studio-inset); }
  .header { padding-inline-end:calc(var(--studio-inset) + 28px); }
  .studio :global(#studio-close) { position:absolute; inset-block-start:var(--sp-2); inset-inline-end:var(--sp-2); z-index:1; border:0; border-radius:var(--r-circle); background:transparent; color:var(--muted); }
  .studio :global(#studio-close:hover:not(:disabled)) { background:var(--row-hover); color:var(--text); }
  :global(.close-start) .header { padding-inline-start:calc(var(--studio-inset) + 28px); padding-inline-end:var(--studio-inset); }
  :global(.close-start) .studio :global(#studio-close) { inset-inline-start:var(--sp-2); inset-inline-end:auto; }
  .identity { min-width:0; }
  h2 { margin:0; font-size:var(--type-dialog-size); font-weight:var(--fw-strong); }
  .identity p { margin:0; color:var(--muted); display:flex; gap:var(--sp-2); align-items:baseline; }
  .identity p span:first-child { overflow:hidden; white-space:nowrap; text-overflow:ellipsis; }
  .duration { flex-shrink:0; font-family:var(--font-literal); }
  .scroll { overflow:auto; overscroll-behavior:contain; margin-inline:var(--studio-inset); padding:var(--studio-inset); min-height:0; display:flex; flex-direction:column; align-items:stretch; gap:var(--sp-4); background:var(--bg); border-radius:var(--r-inset); }
  .scroll > * { flex-shrink:0; }
  .scroll :global(button:is([data-slot="button"],[data-slot="menu-trigger"])) { background:transparent; border:0; box-shadow:none; }
  .scroll :global(button:is([data-slot="button"],[data-slot="menu-trigger"]):is(:hover,[aria-expanded="true"]):not(:disabled)) { background:var(--row-hover); color:var(--text); }
  .timeline-area { display:grid; gap:var(--sp-1); }
  .waveform-hint { margin:0; color:var(--muted); font-size:var(--text-xs); }
  .waveform-hint:empty { display:none; }
  .audio-tool-error { display:flex; align-items:center; gap:var(--sp-2); flex-wrap:wrap; }
  .toolbar { display:flex; gap:var(--sp-1); align-items:center; flex-wrap:wrap; }
  .transport .time { font-family:var(--font-literal); font-variant-numeric:tabular-nums; min-width:8ch; }
  .transport :global(button.loop-toggle[aria-pressed="true"]) { color:var(--accent-ink); background:var(--sel-tint); }
  .waveform-options { margin-inline-start:auto; display:flex; align-items:center; gap:var(--sp-1); }
  .transport :global(button.audio-tool-trigger[data-active="false"]) { color:var(--muted); }
  .transport :global(button.stereo-toggle[aria-pressed="true"]), .transport :global(button.audio-tool-trigger[data-active="true"]) {
    color:var(--accent-ink); background:var(--sel-tint);
  }
  .editor { display:grid; gap:var(--sp-2); grid-template-columns:minmax(0,1.12fr) minmax(0,1fr); margin-block-start:calc(-1 * var(--sp-2)); }
  .track-panel, .tag-panel { min-width:0; display:grid; align-content:start; gap:var(--sp-2); padding:var(--sp-2); background:var(--panel); border-radius:var(--r-inset); }
  .editor h3 { font-size:var(--text-sm); font-weight:600; margin:0; }
  .column-top, .draft-actions { display:flex; align-items:center; min-height:28px; }
  .column-top { justify-content:space-between; padding-inline-start:var(--sp-2); gap:var(--sp-1); }
  .column-top :global(#studio-import-cuts) { background:var(--bg); border-radius:var(--r-pill); }
  .column-top :global(#studio-import-cuts:is(:hover,[aria-expanded="true"]):not(:disabled)) { background:var(--row-hover); }
  .draft-actions { justify-content:flex-end; }
  .track-list { display:grid; max-height:264px; overflow:auto; padding:2px; gap:2px; }
  .track-list :global(.track) { display:grid; grid-template-columns:2ch minmax(0,1fr) 7ch 7ch; gap:var(--sp-2); align-items:center; width:100%; height:auto; min-height:var(--ctl-h); border-radius:var(--r-compact); border:0; padding:6px var(--sp-2); color:var(--text); background:transparent; text-align:left; }
  .track-list :global(.track:nth-child(even):not(:hover,.editing,[aria-pressed="true"])) { background:var(--row-stripe,color-mix(in srgb,var(--text) 3%,var(--bg))); }
  .track-list :global(.track:hover) { background:var(--row-hover); }
  .track-list :global(.track:is([aria-pressed=true],.editing)) { background:var(--row-sel); }
  .track-list :global(.track[aria-pressed=true]:hover) { background:var(--row-sel-hover); }
  :global(.track:is([aria-pressed=true],.editing)) .track-number { color:var(--accent-ink); font-weight:var(--fw-strong); }
  .track-list :global(.track:focus-visible) { outline:2px solid var(--accent); outline-offset:-2px; }
  .track-number, .track-time { font-family:var(--font-literal); font-variant-numeric:tabular-nums; color:var(--muted); }
  .track-title { display:grid; min-width:0; }
  .track-title strong, .track-title small { white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
  .track-title strong { font-weight:500; }
  .track-title small { color:var(--muted); }
  :global(.track.excluded) .track-title strong { text-decoration:line-through; color:var(--muted); }
  .track-edit-fields { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:var(--sp-1); min-width:0; }
  .track-edit-fields :global(input) { width:100%; height:28px; min-width:0; padding-inline:var(--sp-2); background:var(--bg); color:var(--text); border:0; border-radius:var(--input-radius); box-shadow:none; }
  .track-edit-fields :global(input:focus) { outline:2px solid var(--accent); outline-offset:-1px; }
  .track-edit-fields :global(input::placeholder), .tag-input :global(input::placeholder) { color:var(--field-ghost,var(--muted)); opacity:1; }
  .recording-editor { display:grid; align-items:start; gap:var(--sp-5); }
  .recording-fields { display:grid; gap:var(--sp-2); }
  .recording-pair { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:var(--sp-3); }
  .recording-pair .tag-field + .tag-field { grid-template-columns:3.5ch minmax(0,1fr); }
  .tag-field { display:grid; grid-template-columns:9ch minmax(0,1fr); align-items:center; gap:var(--sp-1); }
  .tag-field > label { color:var(--label); font-size:var(--text-xs); line-height:var(--lh-snug); }
  .tag-field.edited > label { color:var(--accent-ink); }
  .tag-input { position:relative; min-width:0; }
  .tag-input :global(.field-input) { display:block; width:100%; font-family:var(--font-value); background:var(--bg); border:0; box-shadow:none; }
  .tag-input :global(textarea) { min-height:74px; padding:var(--sp-2); border-radius:var(--input-radius); color:var(--text); resize:vertical; }
  .tag-input :global(textarea:focus) { outline:2px solid var(--accent); outline-offset:0; }
  .tag-field.edited :global(.field-input) { padding-inline-end:26px; }
  .tag-input:has(:global(.genre-picker-host)) :global(input) { padding-inline-end:28px; }
  .tag-field.edited .tag-input:has(:global(.genre-picker-host)) :global(input) { padding-inline-end:51px; }
  .tag-input :global(.tag-reset), .tag-input :global(button.genre-toggle) { position:absolute; inset:0 0 0 auto; display:grid; place-items:center; width:23px; height:32px; min-height:0; padding:0; background:transparent; border:0; border-radius:var(--r-control); color:var(--muted); }
  .tag-input:has(:global(.genre-picker-host)) :global(.tag-reset) { right:24px; }
  .tag-input :global(.tag-reset) { z-index:1; opacity:0; pointer-events:none; }
  .tag-input:is(:hover,:focus-within) :global(.tag-reset) { opacity:1; pointer-events:auto; }
  :global(.tag-reset:hover), .tag-input :global(button.genre-toggle:hover) { color:var(--text); background:var(--row-hover); }
  .tag-input :global(.genre-combobox input:focus) { outline:none; box-shadow:none; }
  .studio-cover .cover-compact-header { min-height:28px; margin-bottom:var(--sp-2); gap:var(--sp-1); }
  .studio-cover h3 { white-space:nowrap; }
  .studio-cover :global(button.cover-edit) { flex:0 0 24px; width:24px; min-width:24px; height:24px; min-height:24px; display:grid; place-items:center; padding:0; border:0; background:transparent; color:var(--muted); }
  .studio-cover :global(button.cover-edit:is(:hover,[data-state="open"])) { background:var(--row-hover); color:var(--text); }
  .studio-cover :global(button.cover-empty-target), .studio-cover :global(button.cover-main-image) { display:grid; place-content:center; justify-items:center; gap:var(--sp-1); width:min(100%,200px); height:auto; margin-inline:auto; padding:var(--sp-2); border-radius:var(--r-media); background:transparent; color:var(--muted); text-align:center; font-size:var(--text-xs); line-height:1.3; }
  .studio-cover :global(button.cover-empty-target) { min-height:144px; border:1px dashed var(--border); }
  .studio-cover :global(button.cover-main-image) { border:0; padding:0; overflow:hidden; }
  .studio-cover :global(button.cover-main-image img) { width:auto; max-width:100%; max-height:200px; object-fit:contain; }
  .studio-cover :global(button.cover-empty-target:hover:not(:disabled)) { background:var(--sel-tint); border-color:var(--accent); }
  .studio-cover :global(button span) { width:100%; overflow:hidden; text-overflow:ellipsis; }
  .cut-time-edit { display:flex; align-items:center; gap:var(--sp-2); color:var(--muted); font-size:var(--text-xs); }
  .cut-time-edit :global(input) { width:17ch; font-family:var(--font-literal); background:var(--panel); }
  .cut-time-edit :global(input), .tag-input :global(input) { min-width:0; height:32px; padding:0 var(--sp-2); border-radius:var(--input-radius); border:0; color:var(--text); background:var(--bg); }
  .cut-time-edit :global(input:focus), .tag-input :global(input:focus) { outline:2px solid var(--accent); outline-offset:0; }
  .empty { min-height:190px; display:grid; place-content:center; place-items:center; gap:var(--sp-2); color:var(--muted); text-align:center; }
  .opening { display:grid; gap:var(--sp-2); }
  .opening p { display:flex; gap:var(--sp-2); align-items:center; color:var(--muted); margin:0; }
  .opening :global(.recording-scan-track) { width:100%; height:4px; overflow:hidden; border-radius:var(--r-pill); background:var(--panel); }
  .opening :global(.recording-scan-fill) { height:100%; border-radius:inherit; background:var(--accent); }
  .empty p, .help, .error { margin:0; }
  .help { padding:var(--sp-2) var(--sp-3); background:var(--panel); border-radius:var(--r-control); color:var(--text); }
  .notice:empty { display:none; }
  .notice { color:var(--muted); overflow-wrap:anywhere; min-width:0; flex:1 1 200px; font-size:var(--text-xs); }
  .error { padding:var(--sp-2) var(--sp-3); border-inline-start:2px solid var(--danger); color:var(--text); background:var(--bg); border-radius:var(--r-control); }
  .conversion-sources { margin:0; padding:var(--sp-2) var(--sp-4); max-height:180px; overflow:auto; background:var(--bg); border-radius:var(--r-inset); }
  .conversion-sources li { padding:var(--sp-1); overflow-wrap:anywhere; }
  .footer { justify-content:space-between; flex-wrap:wrap; }
  .footer .toolbar { margin-inline-start:auto; justify-content:flex-end; }
  .footer :global(button[data-slot="button"]) { border-radius:var(--r-pill); }
  .footer :global(button[data-variant="secondary"]) { background:var(--bg); }
  .footer :global(button[data-variant="secondary"]:hover:not(:disabled)) { background:var(--row-hover); }
  @media (max-width:700px) { .studio { --studio-inset:var(--sp-3); } .editor { grid-template-columns:minmax(0,1fr); } }
  @media (max-width:450px) { .track { grid-template-columns:2ch minmax(0,1fr) 6ch 6ch; gap:var(--sp-1); } }
  @media (prefers-reduced-motion: reduce) { :global(.audio-split-dialog[data-slot="dialog-content"]) { animation:none; } }
</style>
