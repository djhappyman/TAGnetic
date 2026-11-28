<script lang="ts">
  import { onMount } from 'svelte';
  import { ExternalLink } from '@lucide/svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import * as Dialog from '$lib/components/ui/dialog/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import { isDiscogsAuthError, type DiscogsAccessRequest } from '../js/discogs-access';
  import { t } from '../js/i18n.js';

  const id = $props.id();
  let request = $state.raw<DiscogsAccessRequest>();
  let opener: HTMLElement | null = null;
  let token = $state('');
  let error = $state('');
  let busy = $state(false);

  function prompt(event: Event) {
    request = (event as CustomEvent<DiscogsAccessRequest>).detail;
    opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    token = '';
    error = request.rejected ? t('discogs.access.rejected') : '';
    busy = false;
  }

  onMount(() => {
    document.addEventListener('tagnetic:discogs-access', prompt);
    return () => {
      document.removeEventListener('tagnetic:discogs-access', prompt);
      request?.finish(null);
    };
  });

  function finish(value: string | null) {
    const current = request;
    request = undefined;
    token = '';
    current?.finish(value);
  }

  async function openApplications(event: MouseEvent) {
    event.preventDefault();
    try {
      const { invoke } = await import('../js/invoke.js');
      await invoke('open_provider_page', { source: 'discogs', entity: 'applications', id: '' });
    }
    catch { error = t('discogs.access.linkError'); }
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    const current = request;
    const value = token.trim();
    if (!current || !value || busy) return;
    error = '';
    busy = true;
    try {
      const { invoke } = await import('../js/invoke.js');
      await current.verify(value);
      if (request !== current) return;
      await invoke('save_discogs_token', { token: value });
      const field = document.querySelector<HTMLInputElement>('#discogs-token');
      if (field) field.value = value;
      document.dispatchEvent(new Event('tagnetic:discogs-token-saved'));
      if (request === current) finish(value);
    } catch (reason) {
      if (request === current) error = isDiscogsAuthError(reason) ? t('discogs.access.invalid') : t('discogs.access.failed');
    } finally {
      if (request === current) busy = false;
    }
  }
</script>

<Dialog.Root open={!!request} onOpenChange={open => { if (!open) finish(null); }}>
  <Dialog.Content class="discogs-access-dialog" overlayProps={{ class: 'controller-dialog-overlay discogs-access-overlay' }}
    onOpenAutoFocus={event => { event.preventDefault(); document.getElementById(id)?.focus(); }}
    onCloseAutoFocus={event => { event.preventDefault(); opener?.focus({ preventScroll: true }); }}>
    <Dialog.Title>{t('discogs.access.title')}</Dialog.Title>
    <Dialog.Description>{t('discogs.access.instructions')}</Dialog.Description>
    <a class="applications" href="https://www.discogs.com/settings/developers" onclick={openApplications}>
      {t('discogs.access.create')} <ExternalLink size={14} aria-hidden="true"/>
    </a>
    <form onsubmit={submit} aria-busy={busy}>
      <label for={id}>{t('settings.discogs.token')}</label>
      <Input {id} type="password" autocomplete="off" spellcheck={false} bind:value={token} disabled={busy}
        aria-describedby={error ? id + '-status' : undefined}/>
      {#if error}<p class="access-error" id={id + '-status'} role="status">{error}</p>{/if}
      <div class="actions">
        <Button type="button" variant="ghost" onclick={() => finish(null)}>{t('action.cancel')}</Button>
        <Button type="submit" disabled={!token.trim() || busy}>{t(busy ? 'discogs.access.checking' : 'discogs.access.connect')}</Button>
      </div>
    </form>
  </Dialog.Content>
</Dialog.Root>

<style>
  :global(.discogs-access-overlay[data-slot=dialog-overlay]) { z-index: 154; }
  :global(.discogs-access-dialog[data-slot=dialog-content]) {
    z-index: 155; width: min(400px, calc(100vw - 32px)); gap: var(--sp-2);
    max-height: calc(100dvh - 32px); overflow: auto;
    padding: var(--sp-4); border: 1px solid var(--border); border-radius: var(--r-dialog-compact);
    background: var(--panel); color: var(--text);
  }
  .applications { display: inline-flex; gap: var(--sp-1); align-items: center; justify-self: start; color: var(--accent-ink); }
  form { display: grid; gap: var(--sp-2); margin-top: var(--sp-1); }
  label, .access-error { font-size: var(--text-xs); }
  .access-error { margin: 0; color: var(--del); }
  .actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: var(--sp-2); }
</style>
