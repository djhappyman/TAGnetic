<script lang="ts">
  import { onMount } from 'svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import { NativeSelect, NativeSelectOption } from '$lib/components/ui/native-select/index.js';
  import { tracks } from '../js/state.js';
  let invoke: typeof import('../js/invoke.js').invoke;

  type Client = { id?: string; principal_id?: string; label?: string; revoked?: boolean; drafts?: { id: string; description?: string; files?: number }[] };
  type Review = { draft: string; description?: string; files: number; next_offset?: number; changes: { path: string; rename_to?: string; tags?: { field: string; old?: string; new?: string }[]; artwork?: { old_count: number; new_count: number } }[] };
  type Operation = { id: string; owner: string; status: string; processed: number; total: number };
  type RecoveryOperation = { operation: string; phase: string; proof: string; action?: 'accept_completed' | 'undo_correlated' | 'close_not_written'; guidance: string; batches?: { id: number; description: string; paths: string[]; files?: number }[] };
  let enabled = $state(false);
  let available = $state(true);
  let sharedStaging = $state(false);
  let clients = $state.raw<Client[]>([]);
  let busy = $state(false);
  let error = $state('');
  let label = $state('');
  let root = $state('');
  let profile = $state('stage');
  let hours = $state('12');
  let paired = $state.raw<{ principal_id: string; credential_file: string; profile: string; executable: string } | null>(null);
  let review = $state.raw<Review | null>(null);
  let operations = $state.raw<Operation[]>([]);
  let analysis = $state.raw<Operation[]>([]);
  const work = $derived([...operations, ...analysis]);
  let recovery = $state.raw<{ recovery_required: boolean; operations: RecoveryOperation[]; guidance?: string }>({ recovery_required: false, operations: [] });
  let reviewOwner = $state('');
  let config = $derived(paired ? JSON.stringify({
    command: paired.executable,
    args: ['--mcp-stdio', '--mcp-profile', paired.profile, '--mcp-credential', paired.credential_file],
  }, null, 2) : '');
  const native = Boolean((window as Window & { __TAURI__?: { core?: unknown } }).__TAURI__?.core);

  function defaultRoot() {
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
    if (!native) return;
    try {
      const status = await invoke('mcp_status', {});
      enabled = status.listening;
      available = status.available !== false;
      sharedStaging = status.shared_staging === true;
      if (status.error) error = status.error;
      const list = Array.isArray(status.clients) ? status.clients : status.clients?.clients;
      clients = Array.isArray(list) ? list : [];
      operations = Array.isArray(status.operations?.operations) ? status.operations.operations : [];
      analysis = Array.isArray(status.analysis?.jobs) ? status.analysis.jobs : [];
      recovery = status.recovery || { recovery_required: false, operations: [] };
      if (!root) root = defaultRoot();
    } catch (failure) { error = String(failure); }
  }

  async function action(work: () => Promise<unknown>) {
    if (busy) return;
    busy = true; error = '';
    try { await work(); }
    catch (failure) { error = String(failure); }
    finally { await refresh(); busy = false; }
  }

  function toggle() {
    action(() => invoke('mcp_set_enabled', { enabled: !enabled }));
  }

  function pair() {
    const roots = root.split(/\r?\n/).map(value => value.trim()).filter(Boolean);
    const duration = Number(hours);
    if (!label.trim() || !roots.length || !Number.isFinite(duration) || duration < 1 || duration > 168) {
      error = 'Give this agent a name, an approved folder and an expiry from 1 to 168 hours.';
      return;
    }
    const actions = profile === 'inspect' ? ['read'] : profile === 'stage'
      ? ['read', 'stage', 'providers']
      : ['read', 'stage', 'write', 'providers', 'settings', 'outputs', 'history', 'open', 'playback'];
    action(async () => {
      paired = await invoke('mcp_pair_client', { label: label.trim(), grant: {
        roots, actions, fields: [], providers: ['discogs', 'musicbrainz', 'beatport', 'bandcamp', 'soundeo'],
        max_files: 100000, expires_at: Math.floor(Date.now() / 1000 + duration * 3600),
      } });
      label = '';
    });
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
    document.addEventListener('tagrex:settings-opened', settingsOpened);
    const showReview = (event: Event) => {
      const detail = (event as CustomEvent).detail;
      if (typeof invoke !== 'function' || !detail?.draft || !detail?.principal_id) return;
      action(async () => {
        review = await invoke('mcp_review_draft', { principalId: detail.principal_id, draftId: detail.draft });
        reviewOwner = detail.principal_id;
      });
    };
    document.addEventListener('tagrex:review-agent-draft', showReview);
    // Legacy controllers initialize against the mounted document. Loading
    // their bridge during component evaluation would run them before the DOM.
    void import('../js/invoke.js').then(async api => {
      if (disposed) return;
      invoke = api.invoke;
      await refresh();
      unlisten = await events?.listen('tagrex://agent-event', () => {
        if (!timer) timer = setTimeout(() => { timer = undefined; void refresh(); }, 250);
      });
      if (disposed) unlisten?.();
    }).catch(failure => error = String(failure));
    return () => { disposed = true; clearTimeout(timer); unlisten?.(); document.removeEventListener('tagrex:review-agent-draft', showReview); document.removeEventListener('tagrex:settings-opened', settingsOpened); };
  });
</script>

<div class="settings-group agent-connections" data-icon="i-agent">
  <h3>External agents</h3>
  <div class="settings-field">
    <div class="settings-row">
      <span class="settings-label">Local MCP connection</span>
      <Button id="mcp-enable" variant={enabled ? 'secondary' : 'outline'} size="sm" disabled={!native || !available || busy} onclick={toggle} aria-pressed={enabled}>
        {enabled ? 'Enabled' : 'Enable'}
      </Button>
    </div>
    <p class="settings-hint settings-detail">Connect a local agent to the open files and connected sources. Approved edits appear live alongside yours in Staged.</p>
    {#if !native}<p class="settings-hint">Agent connections are available in the desktop app.</p>{/if}
  </div>

  {#if enabled}
    <div class="agent-pairing">
      <h4 class="settings-sub">Pair an agent</h4>
      <div class="agent-fields">
        <div class="settings-field">
          <label for="mcp-agent-label">Agent name</label>
          <Input id="mcp-agent-label" bind:value={label} placeholder="Codex or OpenCode" autocomplete="off" />
        </div>
        <div class="settings-field">
          <label for="mcp-agent-access">Access</label>
          <NativeSelect id="mcp-agent-access" bind:value={profile}>
            <NativeSelectOption value="inspect">Inspect files</NativeSelectOption>
            <NativeSelectOption value="stage">Inspect and stage</NativeSelectOption>
            <NativeSelectOption value="workflow">Complete workflow</NativeSelectOption>
          </NativeSelect>
        </div>
        <div class="settings-field agent-roots">
          <label for="mcp-agent-roots">Approved folders</label>
          <textarea id="mcp-agent-roots" bind:value={root} rows="2" spellcheck="false" placeholder="One absolute folder per line"></textarea>
        </div>
        <div class="settings-field">
          <label for="mcp-agent-expiry">Expires in hours</label>
          <Input id="mcp-agent-expiry" bind:value={hours} type="number" min="1" max="168" />
        </div>
      </div>
      <p class="settings-hint">Complete workflow includes writes, artwork, renaming, reusable presets and Undo inside these folders. It does not grant trash access.</p>
      <Button id="mcp-pair" size="sm" disabled={busy || !label.trim() || !root.trim()} onclick={pair}>Pair agent</Button>
    </div>
  {/if}

  {#if paired}
    <div class="agent-config">
      <h4 class="settings-sub">Connection ready</h4>
      <p class="settings-hint">Use this command and its arguments in the client's MCP configuration. The credential stays in a private local file.</p>
      <textarea id="mcp-connection-config" aria-label="MCP command configuration" value={config} readonly rows="8" spellcheck="false"></textarea>
    </div>
  {/if}

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
        <Button id="mcp-view-staged" size="sm" disabled={busy} onclick={() => document.dispatchEvent(new CustomEvent('tagrex:show-shared-stage'))}>View staged changes</Button>
      {:else}
        <Button id="mcp-stage-draft" size="sm" disabled={busy} onclick={() => document.dispatchEvent(new CustomEvent('tagrex:adopt-agent-draft', { detail: { principalId: reviewOwner, draftId: review?.draft } }))}>Stage in app</Button>
      {/if}
    </section>
  {/if}
  {#if error}<p id="mcp-error" class="agent-error" role="alert">{error}</p>{/if}
</div>

<style>
  .agent-pairing, .agent-config, .agent-list { margin-top: 24px; }
  .agent-fields { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: 12px 16px; }
  .agent-fields .settings-field { display: flex; flex-direction: column; gap: 7px; margin: 0; }
  .agent-roots { grid-column: 1 / -1; }
  textarea { width: 100%; min-width: 0; resize: vertical; padding: 10px 12px; border: 1px solid transparent; border-radius: var(--input-radius); background: var(--bg); color: var(--text); font: inherit; box-sizing: border-box; }
  textarea:focus-visible { outline: 2px solid var(--ring-color); outline-offset: 1px; }
  #mcp-connection-config { font-family: var(--mono); font-size: 12px; }
  .agent-client { padding: 9px 12px; margin-top: 8px; border-radius: var(--input-radius); background: var(--bg); }
  .agent-label { overflow-wrap: anywhere; }
  .agent-review { max-height: 420px; overflow: auto; margin-top: 24px; padding: 12px; border-radius: var(--input-radius); background: var(--bg); overflow-wrap: anywhere; }
  .agent-job { padding-block: 8px; }
  .agent-job small { display: block; color: var(--muted); }
  .agent-file-change { padding-block: 12px; }
  .agent-value-change { display: grid; grid-template-columns: minmax(55px, .7fr) minmax(0, 1fr) minmax(0, 1fr); gap: 8px; margin-top: 6px; font-size: 12px; }
  .agent-value-change del { opacity: .55; }
  .agent-error { color: var(--error); margin-top: 12px; }
  @media (max-width: 640px) { .agent-fields { grid-template-columns: minmax(0, 1fr); } }
</style>
