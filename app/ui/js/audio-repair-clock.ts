import type { FrameRange } from './audio-split';

export type RepairClock = {
  frame: number;
  observed: number;
  rate: number;
  frames: number;
  running: boolean;
  loop?: FrameRange;
};

// Repair receives authoritative positions at about 8 Hz. Holding after two
// missed updates prevents a stale clock from running through a seek or pause.
export const repairClockHorizon = 250;

export function projectRepairFrame(clock: RepairClock, now: number): number {
  const elapsed = clock.running ? Math.max(0, Math.min(repairClockHorizon, now - clock.observed)) : 0;
  const frame = clock.frame + elapsed * clock.rate / 1000;
  const loop = clock.loop;
  if (loop && loop.end > loop.start && clock.frame >= loop.start && clock.frame < loop.end && frame >= loop.end) {
    return loop.start + (frame - loop.start) % (loop.end - loop.start);
  }
  return Math.max(0, Math.min(clock.frames - 1, frame));
}
