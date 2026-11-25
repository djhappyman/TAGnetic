<script lang="ts">
  import { onMount, tick } from 'svelte';
  import IconSprite from './IconSprite.svelte';
  import Workspace from './Workspace.svelte';
  import UtilityOverlays from './UtilityOverlays.svelte';
  import SettingsPanel from './SettingsPanel.svelte';
  import { recoverAudioSplitSession, type Launch } from '../js/audio-split';

  let launch = $state.raw<Launch | null>(null);
  let Studio = $state<typeof import('./AudioSplitStudio.svelte').default>();
  let loadError = $state('');

  onMount(() => {
    void recoverAudioSplitSession().catch(console.error);
    async function open(event: Event) {
      if (launch) return;
      launch = (event as CustomEvent<Launch>).detail;
      loadError = '';
      try {
        Studio ??= (await import('./AudioSplitStudio.svelte')).default;
      } catch {
        launch = null;
        loadError = 'Audio tools could not open. Try opening them again.';
      }
    }
    document.addEventListener('tagrex:audio-studio', open);
    return () => document.removeEventListener('tagrex:audio-studio', open);
  });

  async function close() {
    if (!launch) return;
    const { opener, mode } = launch;
    launch = null;
    await tick();
    const target = opener?.isConnected && opener.getClientRects().length
      && opener !== document.body && opener.tabIndex >= -1
      ? opener
      : document.getElementById('ribbon-audio-' + mode);
    target?.focus({ preventScroll: true });
  }
</script>

<IconSprite />
<Workspace />
<UtilityOverlays />
<SettingsPanel />

{#if Studio && launch}
  <Studio {launch} onclose={close} />
{/if}
{#if loadError}
  <div role="alert" class="toast">{loadError}</div>
{/if}
