<script lang="ts">
  import { untrack } from 'svelte';
  import { artworkLightboxMotion } from '$lib/artwork-lightbox-motion';
  import * as Menu from '$lib/components/ui/dropdown-menu/index.js';
  import * as Dialog from '$lib/components/ui/dialog/index.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import ArtworkCanvas from './ArtworkCanvas.svelte';
  import { decodeArtwork } from './artwork';
  import { t, tn } from '../js/i18n.js';
  type Image = { mime: string; data_base64: string; kind?: string; description?: string };
  type Model = { selected: number; loading: boolean; images: Image[]; mixed: boolean; withCover: number; external: boolean; canExport?: boolean; pendingArtwork?: boolean; action: (name: string) => void };
  let { initial }: { initial: Model } = $props();
  let model = $state.raw(untrack(() => initial));
  let active = $state(0);
  let menuOpen = $state(false);
  let lightbox = $state(false);
  let decoded = $state.raw<{ src: string; width: number; height: number } | null>(null);
  let failure = $state('');
  let thumbnail = $state<HTMLButtonElement>();
  let stage = $state<HTMLDivElement>();
  let opening = $state(false);
  let closing = false;
  let motion: ReturnType<typeof artworkLightboxMotion> | undefined;
  $effect(() => {
    if (!lightbox || !decoded || !stage) return;
    const host = stage;
    let secondFrame = 0;
    const frame = requestAnimationFrame(() => {
      secondFrame = requestAnimationFrame(() => {
        const origin = thumbnail?.querySelector('img');
        if (!origin) { opening = false; return; }
        motion = artworkLightboxMotion(host, origin);
        void motion.enter();
        opening = false;
      });
    });
    return () => { cancelAnimationFrame(frame); cancelAnimationFrame(secondFrame); motion?.destroy(); motion = undefined; };
  });
  async function closePreview() {
    if (!lightbox || closing) return;
    closing = true;
    const version = ++generation;
    await motion?.exit();
    if (version === generation) lightbox = false;
    closing = false;
  }
  function outsideImage(event: MouseEvent) {
    if (event.detail === 0 || (event.target as Element).closest('.aw-tools, [data-slot="dialog-close"]')) return;
    const hero = stage?.querySelector('img')?.getBoundingClientRect();
    if (hero && (event.clientX < hero.left || event.clientX > hero.right || event.clientY < hero.top || event.clientY > hero.bottom)) void closePreview();
  }
  let generation = 0;
  let returnFocus: HTMLElement | null = null;
  const image = $derived(model.images[active]);
  const uri = (image: Image) => `data:${image.mime};base64,${image.data_base64}`;
  export function update(next: Model, reset = true) {
    model = next;
    if (reset) { active = 0; menuOpen = false; lightbox = false; decoded = null; generation++; }
  }
  function action(name: string) { menuOpen = false; model.action(name); }
  async function preview() {
    if (!image || model.loading) return;
    const current = image;
    const version = ++generation;
    returnFocus = thumbnail || null;
    opening = true;
    closing = false;
    failure = '';
    lightbox = true;
    decoded = null;
    try {
      const picture = await decodeArtwork(current);
      if (version !== generation || !lightbox) return;
      decoded = { src: uri(current), width: picture.naturalWidth, height: picture.naturalHeight };
    } catch {
      if (version === generation && lightbox) { failure = t('artwork.failed'); opening = false; }
    }
  }
  // Canvas rotates its own inspection view; this preview never stages rotation.
  function previewRotation() {}
  function menuAction(name: string) { if (menuOpen) action(name); }
</script>

<div class="cover-compact" aria-busy={model.loading}>
  <div class="cover-compact-header">
    <h2>{t('editor.coverArt')}</h2>
    <Menu.Root bind:open={menuOpen}>
      <Menu.Trigger class="cover-edit icon" aria-label={t('action.edit')} disabled={!model.selected || model.loading}>
        <svg class="ico" aria-hidden="true" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5"><path d="m13.5 3.5 3 3M4 13l9.5-9.5a2.1 2.1 0 0 1 3 3L7 16l-4 1z"/></svg>
      </Menu.Trigger>
      <Menu.Content class="cover-edit-menu" align="end" sideOffset={5}>
        <Menu.Item data-cover="replace" onSelect={() => menuAction('replace')}>{t('cover.replace')}</Menu.Item>
        <Menu.Item data-cover="export" disabled={!(model.canExport ?? !!model.withCover)} onSelect={() => menuAction('export')}>{t(model.pendingArtwork ? 'cover.exportSaved' : 'cover.export')}</Menu.Item>
        <Menu.Item data-cover="remove" disabled={!model.withCover} onSelect={() => menuAction('remove')}>{model.mixed ? t('cover.removeAll') : t('action.remove')}</Menu.Item>
        {#if model.external}<Menu.Item data-cover="external" onSelect={() => menuAction('external')}>{t('cover.useFolder')}</Menu.Item>{/if}
      </Menu.Content>
    </Menu.Root>
  </div>
  {#if image}
    <button type="button" class="cover-main-image" bind:this={thumbnail} aria-label={t('artwork.title')} disabled={model.loading} onclick={preview}>
      <img src={uri(image)} alt={model.mixed ? t('cover.multiple') : image.description || t('cover.frontCover')} draggable="false" />
    </button>
    {#if model.mixed}<p class="cover-compact-note">{t('cover.multiple')} · {t('cover.withCover', { with_cover: model.withCover, n: model.selected })}</p>
    {:else if model.selected > 1}<p class="cover-compact-note">{t('cover.sharedAcross', { files: tn('unit.file', model.selected) })}</p>{/if}
  {:else}
    <button type="button" class="cover-empty-target" disabled={!model.selected || model.loading} onclick={() => action('replace')}>
      <svg class="ico" aria-hidden="true"><use href="#i-image"/></svg>
      <span>{model.loading ? t('artwork.loading') : model.selected ? t('cover.noCover') : t('cover.noSelection')}</span>
      <small>{model.selected ? t('cover.replace') : t('cover.selectToEdit')}</small>
    </button>
  {/if}
</div>
<Dialog.Root open={lightbox} onOpenChange={(open) => { if (!open) void closePreview(); }}>
  <Dialog.Content class="cover-lightbox" data-artwork-ui overlayProps={{ class: "cover-lightbox-veil" }} onclick={outsideImage} onEscapeKeydown={(event) => { event.preventDefault(); event.stopPropagation(); void closePreview(); }} onCloseAutoFocus={(event) => { event.preventDefault(); if (returnFocus?.isConnected) returnFocus.focus({ preventScroll: true }); }}>
    <Dialog.Title class="sr-only">{t('editor.coverArt')}</Dialog.Title>
    <Dialog.Description class="sr-only">{t('artwork.canvas')}</Dialog.Description>
    <div class="cover-lightbox-stage" bind:this={stage} style:opacity={opening ? 0 : 1}>
      {#if decoded}
        <ArtworkCanvas src={decoded.src} width={decoded.width} height={decoded.height} label={t('editor.coverArt')} onrotate={previewRotation} />
      {:else if failure}<p role="alert">{failure}</p><Button variant="outline" onclick={preview}>{t('artwork.retry')}</Button>
      {:else}<p role="status">{t('artwork.loading')}</p>{/if}
    </div>
  </Dialog.Content>
</Dialog.Root>
