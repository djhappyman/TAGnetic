<script lang="ts">
  import { onMount } from 'svelte';
  import { agentClientName, hiddenAgentSessions, liveAgentConnections, setAgentVisible } from '$lib/agent-visibility';
  import { Button } from '$lib/components/ui/button/index.js';
  import * as Dialog from '$lib/components/ui/dialog/index.js';
  import { sessionRoot, tracks } from '../js/state.js';
  import { mcpInstallCommand, mcpCheckCommand, mcpPairingGrant } from '../js/mcp-install.js';
  let invoke: typeof import('../js/invoke.js').invoke;
  type Review = { draft: string; description?: string; files: number; next_offset?: number; changes: { path: string; rename_to?: string; tags?: { field: string; old?: string; new?: string }[]; artwork?: { old_count: number; new_count: number } }[] };
  type RecoveryOperation = { operation: string; phase: string; proof: string; action?: 'accept_completed' | 'undo_correlated' | 'close_not_written'; guidance: string; batches?: { id: number; description: string; paths: string[]; files?: number }[] };
  type Agent = 'Codex' | 'OpenCode';
  type Pairing = { principal_id: string; credential_file: string; profile: string; executable: string };
  const hiddenAgents = $derived($liveAgentConnections.filter(connection => $hiddenAgentSessions.has(connection.id)));
  let enabled = $state(false);
  let canReconnect = $state(false);
  let ready = $state(false);
  let available = $state(true);
  let sharedStaging = $state(false);
  let recovery = $state.raw<{ recovery_required: boolean; operations: RecoveryOperation[]; guidance?: string }>({ recovery_required: false, operations: [] });
  let busy = $state(false);
  let error = $state('');
  let root = $state('');
  let paired = $state.raw<Pairing | null>(null);
  let agent = $state<Agent>('Codex');
  let copyNotice = $state('');
  let commandInput = $state<HTMLTextAreaElement>();
  let activityOpen = $state(false);
  let review = $state.raw<Review | null>(null);
  let reviewOwner = $state('');
  const command = $derived(paired ? mcpInstallCommand(agent, paired) : '');
  const checkCommand = $derived(mcpCheckCommand(agent));
  const native = Boolean((window as Window & { __TAURI__?: { core?: unknown } }).__TAURI__?.core);

  function defaultRoot() {
    if (/^(?:\/|[A-Za-z]:[\\/]|\\\\)/.test(sessionRoot)) return sessionRoot;
    if (!tracks.length) return '';
    const parts = tracks[0].path.replaceAll('\\', '/').split('/');
    parts.pop();
    for (const track of tracks) {
      const path = track.path.replaceAll('\\', '/');
      while (parts.length && !path.startsWith(parts.join('/') + '/')) parts.pop();
    }
    return parts.join('/');
  }

  async function refresh() {
    root = defaultRoot();
    if (!native) return;
    try {
      const status = await invoke('mcp_status', {});
      enabled = status.listening;
      const clients = Array.isArray(status.clients) ? status.clients : status.clients?.clients;
      canReconnect = Array.isArray(clients) && clients.some((client: { revoked?: boolean }) => !client.revoked);
      available = status.available !== false;
      sharedStaging = status.shared_staging === true;
      if (status.error) error = status.error;
      recovery = status.recovery || { recovery_required: false, operations: [] };
      ready = true;
    } catch (failure) { error = String(failure); }
  }

  async function action(run: () => Promise<unknown>) {
    if (busy) return;
    busy = true; error = '';
    try { await run(); }
    catch (failure) { error = String(failure); }
    finally { await refresh(); busy = false; }
  }

  function pair(client: Agent) {
    if (!native || !ready || !available || !root || busy) return;
    const approvedRoot = root;
    void action(async () => {
      if (!enabled) await invoke('mcp_set_enabled', { enabled: true });
      const result = await invoke('mcp_pair_client', { label: client, grant: mcpPairingGrant(approvedRoot) });
      paired = result;
      agent = client;
      copyNotice = '';
    });
  }

  async function copyCommand() {
    try {
      await navigator.clipboard.writeText(command);
      copyNotice = 'Copied. Paste it into your terminal and run it.';
    } catch {
      commandInput?.focus(); commandInput?.select();
      copyNotice = 'Command selected. Copy it and paste it into your terminal.';
    }
  }

  onMount(() => {
    if (!native) return;
    const events = (window as Window & { __TAURI__?: { event?: { listen: (name: string, handler: () => void) => Promise<() => void> } } }).__TAURI__?.event;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    const settingsOpened = () => { if (typeof invoke === 'function') void refresh(); };
    const showActivity = () => { activityOpen = true; settingsOpened(); };
    const showReview = (event: Event) => {
      const detail = (event as CustomEvent).detail;
      if (typeof invoke !== 'function' || !detail?.draft || !detail?.principal_id) return;
      activityOpen = true;
      void action(async () => {
        review = await invoke('mcp_review_draft', { principalId: detail.principal_id, draftId: detail.draft });
        reviewOwner = detail.principal_id;
      });
    };
    document.addEventListener('tagrex:settings-opened', settingsOpened);
    document.addEventListener('tagrex:show-agent-recovery', showActivity);
    document.addEventListener('tagrex:review-agent-draft', showReview);
    void import('../js/invoke.js').then(async api => {
      if (disposed) return;
      invoke = api.invoke;
      await refresh();
      if (disposed) return;
      unlisten = await events?.listen('tagrex://agent-event', () => {
        if (activityOpen && !timer) timer = setTimeout(() => { timer = undefined; void refresh(); }, 250);
      });
      if (disposed) unlisten?.();
    }).catch(failure => error = String(failure));
    return () => {
      disposed = true; clearTimeout(timer); unlisten?.();
      document.removeEventListener('tagrex:settings-opened', settingsOpened);
      document.removeEventListener('tagrex:show-agent-recovery', showActivity);
      document.removeEventListener('tagrex:review-agent-draft', showReview);
    };
  });
</script>

<div class="settings-group agent-connections" data-icon="i-agent">
  <h3>MCP connection</h3>
  <p class="settings-hint">Your agent can read, stage, write and export within the open music folder.</p>
  {#if ready && available && !enabled && canReconnect}
    <div class="mcp-pair-buttons">
      <Button variant="secondary" size="sm" disabled={busy} onclick={() => action(() => invoke('mcp_set_enabled', { enabled: true }))}>Enable agent connections</Button>
    </div>
  {/if}
  <div class="mcp-setup">
      <div class="mcp-pair-buttons">
        <Button id="mcp-pair-codex" variant="secondary" size="sm" disabled={!native || !ready || !available || !root || busy} onclick={() => pair('Codex')}>Pair Codex</Button>
        <Button id="mcp-pair-opencode" variant="secondary" size="sm" disabled={!native || !ready || !available || !root || busy} onclick={() => pair('OpenCode')}>Pair OpenCode</Button>
      </div>
      {#if !native}<p class="settings-hint">Pair agents from the Tagnetic desktop app.</p>
      {:else if !root}<p class="settings-hint">Open a music folder in Tagnetic before pairing.</p>{/if}
      {#if paired}
        <p class="settings-hint">{agent} is paired. Run this command in your terminal to install the connection.</p>
        <textarea bind:this={commandInput} id="mcp-connection-config" aria-label={`${agent} MCP install command`} value={command} readonly rows="5" spellcheck="false"></textarea>
        <div><Button id="mcp-copy-install" variant="default" size="sm" onclick={copyCommand}>Copy install command</Button></div>
        <p class="settings-hint mcp-copy-notice" role="status">{copyNotice}</p>
        <p class="settings-hint">Check the install with <code>{checkCommand}</code>, then open a new {agent} session.</p>
      {/if}
  </div>
  {#if hiddenAgents.length}
    <div class="settings-field">
      <h4 class="settings-sub">Agents hidden from the popup</h4>
      {#each hiddenAgents as connection (connection.id)}
        <div class="settings-row">
          <span>{agentClientName(connection)}{connection.model ? ` · ${connection.model}` : ''}</span>
          <Button size="sm" variant="ghost" onclick={() => setAgentVisible(connection.id, true)}>Show in Agents</Button>
        </div>
      {/each}
    </div>
  {/if}
  {#if error && !activityOpen}<p id="mcp-error" class="agent-error" role="alert">{error}</p>{/if}
</div>

<Dialog.Root bind:open={activityOpen}>
  <Dialog.Content class="agent-activity-dialog" onCloseAutoFocus={(event) => { event.preventDefault(); document.getElementById('agent-activity')?.focus({ preventScroll: true }); }}>
    <Dialog.Title>Review agent work</Dialog.Title>
    <Dialog.Description>Proposed changes and interrupted work.</Dialog.Description>
    <div class="agent-activity-body">
  {#if recovery.recovery_required}
    <section class="agent-review" aria-label="Interrupted agent work">
      <h4 class="settings-sub">Interrupted work</h4>
      <p class="settings-hint">New agent writes are paused until the interrupted operation is checked. Your other work remains available.</p>
      {#if recovery.guidance}<p>{recovery.guidance}</p>{/if}
      {#each recovery.operations as item (item.operation)}
        <div class="agent-file-change">
          <p>{item.guidance}</p>
          {#each item.batches || [] as batch (batch.id)}
            <strong>{batch.description || 'Affected files'}</strong>
            <ul>{#each (batch.paths || []).slice(0, 20) as path (path)}<li>{path}</li>{/each}</ul>
            {#if (batch.files || batch.paths?.length || 0) > 20}<p class="settings-hint">{batch.files || batch.paths.length} affected files; showing the first 20.</p>{/if}
          {/each}
          {#if item.action}
            <Button size="sm" disabled={busy} onclick={() => action(() => invoke('mcp_reconcile_operation', { operationId: item.operation, proof: item.proof, action: item.action }))}>
              {item.action === 'accept_completed' ? 'Keep completed changes' : item.action === 'undo_correlated' ? 'Restore these files' : 'Close unwritten operation'}
            </Button>
          {/if}
        </div>
      {/each}
    </section>
  {/if}
  {#if review}
    <section class="agent-review" aria-label="Agent draft review">
      <div class="settings-row"><h4 class="settings-sub">{review.description || 'Proposed changes'}</h4><Button variant="ghost" size="sm" onclick={() => review = null}>Close review</Button></div>
      <p class="settings-hint">{review.files} files{sharedStaging ? ' · changes are visible in the shared staging area.' : ' · proposed changes.'}</p>
      {#each review.changes as change (change.path)}
        <div class="agent-file-change">
          <strong>{change.path.split(/[\\/]/).pop()}</strong>
          {#if change.rename_to}<p class="settings-hint">Rename to {change.rename_to.split(/[\\/]/).pop()}</p>{/if}
          {#each change.tags || [] as tag (tag.field)}
            <div class="agent-value-change"><span class="settings-hint">{tag.field}</span><del>{tag.old || 'Empty'}</del><span>{tag.new || 'Empty'}</span></div>
          {/each}
          {#if change.artwork}<p class="settings-hint">Artwork: {change.artwork.old_count} → {change.artwork.new_count} images</p>{/if}
        </div>
      {/each}
      {#if review.next_offset}<p class="settings-hint">Showing the first {review.changes.length} files. The full draft is retained.</p>{/if}
      {#if sharedStaging}
        <Button id="mcp-view-staged" size="sm" disabled={busy} onclick={() => { activityOpen = false; document.dispatchEvent(new CustomEvent('tagrex:show-shared-stage')); }}>View staged changes</Button>
      {:else}
        <Button id="mcp-stage-draft" size="sm" disabled={busy} onclick={() => { activityOpen = false; document.dispatchEvent(new CustomEvent('tagrex:adopt-agent-draft', { detail: { principalId: reviewOwner, draftId: review?.draft } })); }}>Stage in app</Button>
      {/if}
    </section>
  {/if}
    {#if error && activityOpen}<p id="mcp-error" class="agent-error" role="alert">{error}</p>{/if}
    </div>
  </Dialog.Content>
</Dialog.Root>

<style>
  .mcp-setup { display: grid; gap: var(--sp-2); min-width: 0; }
  .mcp-pair-buttons { display: flex; flex-wrap: wrap; gap: var(--sp-2); margin-top: var(--sp-3); }
  .mcp-copy-notice { min-height: 1.4em; }
  .mcp-setup code { font-family: var(--font-literal); overflow-wrap: anywhere; }
  :global(.agent-activity-dialog) { max-width: min(620px, calc(100vw - 24px)); background: var(--panel); color: var(--text); }
  .agent-activity-body { max-height: min(65vh, 580px); overflow: auto; }
  textarea { width: 100%; min-width: 0; resize: vertical; padding: 10px 12px; border: 1px solid transparent; border-radius: var(--input-radius); background: var(--bg); color: var(--text); font: inherit; box-sizing: border-box; }
  textarea:focus-visible { outline: 2px solid var(--ring-color); outline-offset: 1px; }
  #mcp-connection-config { font-family: var(--font-literal); font-size: 12px; margin-block: var(--sp-2); }
  .agent-review { max-height: 420px; overflow: auto; margin-top: 24px; padding: 12px; border-radius: var(--input-radius); background: var(--bg); overflow-wrap: anywhere; }
  .agent-file-change { padding-block: 12px; }
  .agent-value-change { display: grid; grid-template-columns: minmax(55px, .7fr) minmax(0, 1fr) minmax(0, 1fr); gap: 8px; margin-top: 6px; font-size: 12px; }
  .agent-value-change del { opacity: .55; }
  .agent-error { color: var(--error); margin-top: 12px; }
</style>
