// Thumbnail-to-image continuity, using Interior's gentle home spring (150/27).
// Native frames keep the Svelte canvas, keyboard controls and media read-only.
export function artworkLightboxMotion(stage: HTMLElement, origin: HTMLImageElement) {
  const reduced = matchMedia('(prefers-reduced-motion: reduce)');
  let animation: Animation | undefined;
  let disposed = false;
  const previousVisibility = origin.style.visibility;
  const spring = (time: number) => {
    const a = (-27 + Math.sqrt(27 * 27 - 4 * 150)) / 2;
    const b = (-27 - Math.sqrt(27 * 27 - 4 * 150)) / 2;
    return 1 - (b * Math.exp(a * time) - a * Math.exp(b * time)) / (b - a);
  };
  function landing() {
    const hero = stage.querySelector('img');
    if (!hero || !origin.isConnected) return { x: 0, y: 8, sx: .98, sy: .98 };
    const from = origin.getBoundingClientRect(), to = hero.getBoundingClientRect(), box = stage.getBoundingClientRect();
    if (!from.width || !to.width) return { x: 0, y: 8, sx: .98, sy: .98 };
    const sx = from.width / to.width, sy = from.height / to.height;
    const cx = box.left + box.width / 2, cy = box.top + box.height / 2;
    return { x: from.left + from.width / 2 - cx - sx * (to.left + to.width / 2 - cx), y: from.top + from.height / 2 - cy - sy * (to.top + to.height / 2 - cy), sx, sy };
  }
  type Pose = ReturnType<typeof landing>;
  async function run(from: Pose, to: Pose) {
    if (disposed || reduced.matches || !stage.animate) return;
    const frames = Array.from({ length: 45 }, (_, index) => {
      const p = spring(index / 44 * .7) / spring(.7);
      const value = (key: keyof Pose) => from[key] + (to[key] - from[key]) * p;
      return { transform: `translate(${value('x')}px,${value('y')}px) scale(${value('sx')},${value('sy')})`, offset: index / 44 };
    });
    animation = stage.animate(frames, { duration: 520, easing: 'linear', fill: 'both' });
    const current = animation;
    await current.finished.catch(() => {});
    if (animation === current) { animation.cancel(); animation = undefined; }
  }
  const settle = () => { animation?.cancel(); animation = undefined; };
  reduced.addEventListener('change', settle);
  window.addEventListener('resize', settle);
  return {
    enter() {
      const from = landing();
      origin.style.visibility = 'hidden';
      return run(from, { x: 0, y: 0, sx: 1, sy: 1 });
    },
    exit() {
      const current = new DOMMatrixReadOnly(getComputedStyle(stage).transform);
      settle();
      return run({ x: current.e, y: current.f, sx: current.a, sy: current.d }, landing());
    },
    destroy() {
      disposed = true;
      settle();
      origin.style.visibility = previousVisibility;
      reduced.removeEventListener('change', settle);
      window.removeEventListener('resize', settle);
    },
  };
}
