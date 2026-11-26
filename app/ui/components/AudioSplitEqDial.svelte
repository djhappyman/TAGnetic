<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import { Button } from '@ui-sdk/svelte/button';
  import { NumberInput, NumberInputControl } from '@ui-sdk/svelte/number-input';

  let { label, value, min, max, step, unit, color, disabled = false, logarithmic = false,
    variant = 'regular', activity = 0, onvalue }: {
    label: string;
    value: number;
    min: number;
    max: number;
    step: number;
    unit: string;
    color: string;
    disabled?: boolean;
    logarithmic?: boolean;
    variant?: 'regular' | 'gain' | 'range';
    activity?: number;
    onvalue: (value: number, gesture?: string) => void;
  } = $props();

  const id = $props.id();
  let serial = 0;
  let frame = 0;
  let element: HTMLDivElement;
  let dragging = $state(false);
  let editing = $state(false);
  let cancelEdit = false;
  let didDrag = false;
  let drag: {
    pointer: number;
    button: HTMLButtonElement;
    y: number;
    fraction: number;
    value: number;
    sent: number;
    moved: boolean;
    gesture: string;
    commit: typeof onvalue;
  } | undefined;

  const clamp = (fraction: number) => Math.max(0, Math.min(1, fraction));
  const fraction = $derived(clamp(max <= min ? 0 : logarithmic
    ? Math.log(Math.max(min, value) / min) / Math.log(max / min) : (value - min) / (max - min)));
  const angle = $derived(fraction * 270);
  const rangeAngle = $derived(variant === 'range' ? clamp(Math.abs(value) / Math.max(Math.abs(min), Math.abs(max))) * Math.sign(value) * 135 : 0);
  const decimals = $derived(String(step).split('.')[1]?.length ?? 0);
  const display = $derived(value.toLocaleString(undefined, { maximumFractionDigits: decimals }));

  function start(event: PointerEvent) {
    if (disabled || editing || event.button !== 0 || drag || max <= min) return;
    const button = event.currentTarget as HTMLButtonElement;
    didDrag = false;
    drag = { pointer: event.pointerId, button, y: event.clientY, fraction, value, sent: value, moved: false,
      gesture: id + '-' + ++serial, commit: onvalue };
    dragging = true;
    button.setPointerCapture(event.pointerId);
  }

  function apply() {
    frame = 0;
    if (!drag) return;
    const raw = logarithmic ? min * (max / min) ** drag.fraction : min + (max - min) * drag.fraction;
    const next = Number(Math.max(min, Math.min(max, min + Math.round((raw - min) / step) * step)).toFixed(decimals));
    if (next === drag.sent) return;
    drag.sent = next;
    drag.commit(next, drag.gesture);
  }

  function accumulate(event: PointerEvent) {
    if (!drag || drag.pointer !== event.pointerId) return false;
    const distance = drag.y - event.clientY;
    if (!drag.moved && Math.abs(distance) < 3) return false;
    drag.moved = true;
    drag.y = event.clientY;
    drag.fraction = clamp(drag.fraction + distance / (event.shiftKey ? 850 : 170));
    return distance !== 0;
  }

  function move(event: PointerEvent) {
    if (accumulate(event) && !frame) frame = requestAnimationFrame(apply);
  }

  function finish(event?: PointerEvent) {
    const current = drag;
    if (!current || (event && current.pointer !== event.pointerId)) return;
    const pending = frame !== 0;
    if (pending) cancelAnimationFrame(frame);
    if (event?.type === 'pointerup') {
      if (accumulate(event) || pending) apply();
    } else if (current.sent !== current.value) current.commit(current.value, current.gesture);
    didDrag = current.moved;
    frame = 0;
    drag = undefined;
    dragging = false;
    if (current.button.hasPointerCapture(current.pointer)) current.button.releasePointerCapture(current.pointer);
  }

  function keys(event: KeyboardEvent) {
    if (event.key !== 'Escape' || !drag) return;
    event.preventDefault();
    event.stopPropagation();
    finish();
  }

  async function openEdit() {
    if (disabled || max <= min || editing) return;
    cancelEdit = false;
    editing = true;
    await tick();
    const input = element.querySelector<HTMLInputElement>('.dial-input');
    input?.focus({ preventScroll: true });
    input?.select();
  }

  function closeEdit(returnFocus = false) {
    if (!editing) return;
    editing = false;
    if (returnFocus) element.querySelector<HTMLButtonElement>('.dial-ring')?.focus({ preventScroll: true });
  }

  function editKeys(event: KeyboardEvent) {
    if (!editing || event.isComposing || !(event.target instanceof HTMLInputElement)) return;
    if (event.key !== 'Enter' && event.key !== 'Escape') return;
    event.preventDefault();
    event.stopPropagation();
    cancelEdit = event.key === 'Escape';
    closeEdit(true);
  }

  onDestroy(() => finish());
</script>

<svelte:window onkeydowncapture={keys} onblur={() => finish()}/>

<div class="eq-dial" class:editing class:gain={variant === 'gain'} class:range={variant === 'range'}
  data-eq-dial={label} bind:this={element} style:--dial-color={color} style:--dial-value={angle + 'deg'}
  style:--range-start={135 + Math.min(0, rangeAngle) + 'deg'}
  style:--range-end={135 + Math.max(0, rangeAngle) + 'deg'}
  style:--live-start={135 + Math.min(0, rangeAngle) * clamp(activity) + 'deg'}
  style:--live-end={135 + Math.max(0, rangeAngle) * clamp(activity) + 'deg'} onkeydowncapture={editKeys}>
  <Button type="button" class="dial-ring" disabled={disabled || max <= min}
    data-dragging={dragging} aria-label={label + ', ' + display + (unit ? ' ' + unit : '')}
    aria-description="Drag to adjust. Double-click or press Enter to type a value."
    title={variant === 'range' ? display + ' dB dynamic range · Drag or double-click to type'
      : 'Drag to adjust · Shift for finer changes · Double-click to type'}
    onpointerdown={start} onpointermove={move} onpointerup={finish}
    onpointercancel={finish} onlostpointercapture={finish}
    onclick={event => { if (event.detail === 0) void openEdit(); }}
    ondblclick={() => { if (!didDrag) void openEdit(); }}>
    {#if variant === 'range'}
      <span class="range-face" aria-hidden="true"></span>
    {:else}
      <span class="label">{label}</span>
      <span class="dial-face" aria-hidden="true"><span class="dial-needle"></span></span>
      <span class="readout">{display}{#if unit}<small>{unit}</small>{/if}</span>
    {/if}
  </Button>
  {#if editing}
    <NumberInput class="dial-value" {value} {min} {max} {step} largeStep={step * 5}
      disabled={disabled || max <= min} onValueCommit={next => { if (!cancelEdit && next !== null) onvalue(next); }}>
      <NumberInputControl class="dial-input" aria-label={unit ? label + ' in ' + unit : label}
        onfocusout={() => closeEdit()}/>
    </NumberInput>
  {/if}
</div>

<style>
  .eq-dial { position:relative; flex:none; inline-size:var(--eq-dial-width); color:var(--text); }
  .eq-dial :global(.dial-ring) {
    display:grid; justify-items:center; gap:3px; inline-size:100%; block-size:auto; min-height:0;
    padding:0; border:0; border-radius:var(--r-compact); background:transparent; color:inherit;
    box-shadow:none; cursor:ns-resize; touch-action:none; user-select:none;
  }
  .label { color:var(--muted); font-size:.625rem; font-weight:500; line-height:1.2; }
  .dial-face {
    position:relative; display:grid; place-items:center; inline-size:var(--eq-dial-size); block-size:var(--eq-dial-size);
    border-radius:var(--r-circle);
    background:conic-gradient(from 225deg, var(--dial-color) 0deg var(--dial-value),
      var(--border) var(--dial-value) 270deg, transparent 270deg 360deg);
  }
  .dial-face::before {
    content:''; position:absolute; inset:3px; border-radius:inherit;
    background:linear-gradient(var(--row-hover), var(--panel));
    box-shadow:0 2px 3px rgb(0 0 0 / 22%);
  }
  .dial-needle { position:absolute; inset:5px; transform:rotate(calc(var(--dial-value) - 135deg)); }
  .dial-needle::before {
    content:''; position:absolute; inset:1px auto auto 50%; transform:translateX(-50%);
    inline-size:2px; block-size:6px; border-radius:var(--r-pill); background:var(--dial-color);
  }
  .eq-dial :global(.dial-ring:hover .dial-face) { filter:brightness(1.13); }
  .eq-dial :global(.dial-ring[data-dragging=true]) { cursor:grabbing; }
  .eq-dial :global(.dial-ring:disabled) { opacity:.55; cursor:default; }
  .eq-dial :global(.dial-ring:focus-visible) { outline:none; }
  .eq-dial :global(.dial-ring:focus-visible .label) { color:var(--dial-color); }
  .eq-dial :global(.dial-ring:focus-visible .readout) { background:var(--sel-tint); }
  .readout, .eq-dial :global(.dial-input) {
    box-sizing:border-box; min-width:0; block-size:16px; padding:0 2px;
    font:10px/16px var(--font-literal); text-align:center; white-space:nowrap;
    border-radius:var(--r-compact); color:inherit;
  }
  .readout small { margin-inline-start:2px; color:var(--muted); font-size:9px; }
  .editing .readout { visibility:hidden; }
  .eq-dial :global(.dial-value) { position:absolute; inset:auto 1px 0; }
  .eq-dial :global(.dial-input) {
    width:100%; border:0; outline:none; box-shadow:none; background:var(--sel-tint);
  }
  .gain { z-index:1; pointer-events:none; }
  .gain .dial-face {
    inline-size:var(--eq-gain-disc); block-size:var(--eq-gain-disc);
    margin-block:calc((var(--eq-range-disc) - var(--eq-gain-disc)) / 2);
  }
  .gain :global(.dial-ring) { pointer-events:none; }
  .gain .dial-face, .gain .label, .gain .readout, .gain :global(.dial-value) { pointer-events:auto; }
  .range { position:absolute; inset:0; inline-size:auto; pointer-events:none; }
  .range :global(.dial-ring) {
    position:absolute; top:calc(.625rem * 1.2 + 3px); left:50%; transform:translateX(-50%);
    inline-size:var(--eq-range-disc); block-size:var(--eq-range-disc);
    place-items:stretch; border-radius:var(--r-circle); pointer-events:auto;
  }
  .range-face {
    --range-tint:color-mix(in srgb, var(--dial-color) 66%, var(--panel));
    border-radius:inherit;
    background:conic-gradient(from 225deg,
      var(--border) 0deg var(--range-start), var(--range-tint) var(--range-start) var(--live-start),
      var(--dial-color) var(--live-start) var(--live-end), var(--range-tint) var(--live-end) var(--range-end),
      var(--border) var(--range-end) 270deg, transparent 270deg 360deg);
    -webkit-mask-image:radial-gradient(farthest-side, transparent calc(100% - 6px), #000 calc(100% - 5px));
    mask-image:radial-gradient(farthest-side, transparent calc(100% - 6px), #000 calc(100% - 5px));
  }
  .range :global(.dial-ring:is(:hover,:focus-visible) .range-face) { filter:brightness(1.3); }
  .range.editing { z-index:2; }
  .range :global(.dial-value) { inset:auto 5px 0; pointer-events:auto; }
</style>
