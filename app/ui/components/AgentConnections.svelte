<script lang="ts">
  import { onMount } from 'svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import * as Dialog from '$lib/components/ui/dialog/index.js';
  import { sessionRoot, tracks } from '../js/state.js';
  import { mcpInstallCommand, mcpCheckCommand, mcpPairingGrant } from '../js/mcp-install.js';
  let invoke: typeof import('../js/invoke.js').invoke;
  type Client = { id?: string; principal_id?: string; label?: string; revoked?: boolean; drafts?: { id: string; description?: string; files?: number }[] };
  type Review = { draft: string; description?: string; files: number; next_offset?: number; changes: { path: string; rename_to?: string; tags?: { field: string; old?: string; new?: string }[]; artwork?: { old_count: number; new_count: number } }[] };
  type Operation = { id: string; owner: string; status: string; processed: number; total: number };
  type RecoveryOperation = { operation: string; phase: string; proof: string; action?: 'accept_completed' | 'undo_correlated' | 'close_not_written'; guidance: string; batches?: { id: number; description: string; paths: string[]; files?: number }[] };
  type Agent = 'Codex' | 'OpenCode';
  type Pairing = { principal_id: string; credential_file: string; profile: string; executable: string };
  let enabled = false;
  let ready = $state(false);
  let available = $state(true);
  let sharedStaging = $state(false);
  let clients = $state.raw<Client[]>([]);
  let operations = $state.raw<Operation[]>([]);
  let analysis = $state.raw<Operation[]>([]);
  const work = $derived([...operations, ...analysis]);
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
      available = status.available !== false;
      sharedStaging = status.shared_staging === true;
      if (status.error) error = status.error;
      const list = Array.isArray(status.clients) ? status.clients : status.clients?.clients;
      clients = Array.isArray(list) ? list : [];
      operations = status.operations?.operations || [];
      analysis = status.analysis?.jobs || [];
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

  function revoke(client: Client) {
    action(() => invoke('mcp_revoke_client', { principalId: client.principal_id || client.id }));
  }

  function inspect(client: Client, id: string) {
    action(async () => {
      review = await invoke('mcp_review_draft', { principalId: client.principal_id || client.id, draftId: id });
      reviewOwner = client.principal_id || client.id || '';
    });
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
    document.addEventListener('tagrex:show-agent-activity', showActivity);
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
      document.removeEventListener('tagrex:show-agent-activity', showActivity);
      document.removeEventListener('tagrex:review-agent-draft', showReview);
    };
  });
</script>

<div class="settings-group agent-connections" data-icon="i-agent">
  <h3>MCP connection</h3>
  <p class="settings-hint">Connect your agent to Tagnetic in three steps.</p>
  <ol class="mcp-setup">
    <li>
      <h4>Pair your agent</h4>
      <p class="settings-hint">Your agent can read, stage, write and export within the open music folder.</p>
      <div class="mcp-pair-buttons">
        <Button id="mcp-pair-codex" variant="secondary" size="sm" disabled={!native || !ready || !available || !root || busy} onclick={() => pair('Codex')}>Pair Codex</Button>
        <Button id="mcp-pair-opencode" variant="secondary" size="sm" disabled={!native || !ready || !available || !root || busy} onclick={() => pair('OpenCode')}>Pair OpenCode</Button>
      </div>
      {#if !native}<p class="settings-hint">Pair agents from the Tagnetic desktop app.</p>
      {:else if !root}<p class="settings-hint">Open a music folder in Tagnetic before pairing.</p>{/if}
    </li>
    <li>
      <h4>Install the connection</h4>
      {#if paired}
        <p class="settings-hint">{agent} is paired. Run this command in your terminal to install the connection.</p>
        <textarea bind:this={commandInput} id="mcp-connection-config" aria-label={`${agent} MCP install command`} value={command} readonly rows="5" spellcheck="false"></textarea>
        <Button id="mcp-copy-install" variant="default" size="sm" onclick={copyCommand}>Copy install command</Button>
        <p class="settings-hint mcp-copy-notice" role="status">{copyNotice}</p>
      {:else}
        <p class="settings-hint">Pair Codex or OpenCode above to get its ready-to-run install command.</p>
      {/if}
    </li>
    <li>
      <h4>Check it is connected</h4>
      {#if paired}
        <p class="settings-hint">Run <code>{checkCommand}</code> and check that <strong>tagnetic</strong> is listed. Start a new {agent} session and ask it to show the music folder open in Tagnetic.</p>
      {:else}
        <p class="settings-hint">After installing, check that Tagnetic is listed in your agent's MCP servers, then start a new agent session.</p>
      {/if}
      <p class="settings-hint">Keep Tagnetic open while your agent is connected.</p>
    </li>
  </ol>
  {#if error && !activityOpen}<p id="mcp-error" class="agent-error" role="alert">{error}</p>{/if}
</div>

<Dialog.Root bind:open={activityOpen}>
  <Dialog.Content class="agent-activity-dialog" onCloseAutoFocus={(event) => { event.preventDefault(); document.getElementById('agent-activity')?.focus({ preventScroll: true }); }}>
    <Dialog.Title>Agent activity</Dialog.Title>
    <Dialog.Description>Connected agents, proposed changes and interrupted work.</Dialog.Description>
    <div class="agent-activity-body">
  {#if clients.length}
    <div class="agent-list">
      <div class="settings-row">
        <h4 class="settings-sub">Paired agents</h4>
        <Button id="mcp-stop-all" variant="outline" size="sm" disabled={busy} onclick={() => action(() => invoke('mcp_stop_all', {}))}>Stop all work</Button>
      </div>
      {#each clients as client (client.principal_id || client.id)}
        <div class="agent-client">
          <div class="settings-row">
            <span class="agent-label">{client.label || 'Agent'}{client.revoked ? ' · revoked' : ''}</span>
            <Button variant="ghost" size="sm" disabled={busy || client.revoked} onclick={() => revoke(client)}>Revoke</Button>
          </div>
          {#each client.drafts || [] as draft (draft.id)}
            <Button variant="ghost" size="sm" disabled={busy} onclick={() => inspect(client, draft.id)}>Review {draft.description || 'draft'} · {draft.files || 0} files</Button>
          {/each}
        </div>
      {/each}
    </div>
  {/if}
  {#if work.length}
    <div class="agent-list" aria-label="Agent work">
      <h4 class="settings-sub">Work history</h4>
      {#each work as operation (operation.id)}
        <div class="settings-row agent-job">
          <span>{clients.find(client => (client.id || client.principal_id) === operation.owner)?.label || 'Agent'} · {operation.status.replaceAll('_', ' ')}<small>{operation.processed} / {operation.total} files</small></span>
          {#if ['queued', 'running', 'preflight', 'writing', 'rolling_back'].includes(operation.status)}
            <Button variant="outline" size="sm" disabled={busy} onclick={() => action(() => invoke('mcp_cancel_operation', { operationId: operation.id }))}>Stop</Button>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
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
  .mcp-setup { list-style: none; counter-reset: setup; padding: 0; margin: var(--sp-5) 0 0; display: grid; gap: var(--sp-5); }
  .mcp-setup > li { counter-increment: setup; min-width: 0; padding-inline-start: 36px; position: relative; }
  .mcp-setup > li::before { content: counter(setup); position: absolute; inset-inline-start: 0; top: 0; width: 24px; height: 24px; display: grid; place-items: center; border-radius: var(--r-circle); background: var(--sel-tint); color: var(--accent-ink); font-weight: var(--fw-strong); }
  .mcp-setup h4 { font-size: var(--type-heading-size); font-weight: var(--fw-strong); color: var(--text); margin: 0 0 var(--sp-2); line-height: 24px; }
  .mcp-pair-buttons { display: flex; flex-wrap: wrap; gap: var(--sp-2); margin-top: var(--sp-3); }
  .mcp-copy-notice { min-height: 1.4em; }
  .mcp-setup code { font-family: var(--font-literal); overflow-wrap: anywhere; }
  :global(.agent-activity-dialog) { max-width: min(620px, calc(100vw - 24px)); background: var(--panel); color: var(--text); }
  .agent-activity-body { max-height: min(65vh, 580px); overflow: auto; }
  .agent-list { margin-top: 24px; }
  textarea { width: 100%; min-width: 0; resize: vertical; padding: 10px 12px; border: 1px solid transparent; border-radius: var(--input-radius); background: var(--bg); color: var(--text); font: inherit; box-sizing: border-box; }
  textarea:focus-visible { outline: 2px solid var(--ring-color); outline-offset: 1px; }
  #mcp-connection-config { font-family: var(--font-literal); font-size: 12px; margin-block: var(--sp-2); }
  .agent-client { padding: 9px 12px; margin-top: 8px; border-radius: var(--input-radius); background: var(--bg); }
  .agent-label { overflow-wrap: anywhere; }
  .agent-review { max-height: 420px; overflow: auto; margin-top: 24px; padding: 12px; border-radius: var(--input-radius); background: var(--bg); overflow-wrap: anywhere; }
  .agent-job { padding-block: 8px; }
  .agent-job small { display: block; color: var(--muted); }
  .agent-file-change { padding-block: 12px; }
  .agent-value-change { display: grid; grid-template-columns: minmax(55px, .7fr) minmax(0, 1fr) minmax(0, 1fr); gap: 8px; margin-top: 6px; font-size: 12px; }
  .agent-value-change del { opacity: .55; }
  .agent-error { color: var(--error); margin-top: 12px; }
</style>
