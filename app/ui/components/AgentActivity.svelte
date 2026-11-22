<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { Check, Pause, Play, Plug, Plus, Square, Unplug, X } from '@lucide/svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import * as Popover from '$lib/components/ui/popover/index.js';

  import { agentClientName, hiddenAgentSessions, publishAgentConnections, setAgentVisible, type AgentConnection } from '$lib/agent-visibility';
  type SessionAction = 'pause' | 'resume' | 'disconnect' | 'reconnect';
  type Work = { session?: string; tool?: string; kind: 'staging' | 'saving' | 'searching' };
  type Write = { id: string; session?: string; status: string; processed: number; total: number };
  type Receipt = { id: string; files: number; session?: string };
  type Status = { enabled: boolean; can_reconnect?: boolean; connections: AgentConnection[]; work: Work[]; writes: Write[]; recovery_required?: boolean; staged?: Receipt };
  type AgentEvent = { kind: string; data?: { completed?: boolean; status?: string; processed?: number; operation?: string } };

  const native = (window as Window & { __TAURI__?: { core?: unknown; event?: { listen: (name: string, callback: (event: {payload: AgentEvent}) => void) => Promise<() => void> } } }).__TAURI__;
  let invoke: typeof import('../js/invoke.js').invoke;
  let status = $state.raw<Status>({ enabled: false, connections: [], work: [], writes: [] });
  let finished = $state.raw<{ kind: 'staged' | 'saved'; files: number } | null>(null);
  let finishTimer: ReturnType<typeof setTimeout> | undefined;
  let lastStage = '';
  let lastSave = '';
  let open = $state(false);
  let focusHandoff = false;
  let busy = $state(false);
  let error = $state('');

  function jobText(write?: Write, work?: Work) {
    if (write?.status === 'rolling_back') return 'Restoring files…';
    if (write?.status === 'queued' || write?.status === 'preflight') return 'Preparing to save…';
    if (write) return write.total > 0 ? `Saving ${write.processed} of ${write.total} files` : 'Saving files…';
    if (work?.kind === 'staging') {
      if (work.tool === 'draft.rename') return 'Staging file renames…';
      if (work.tool === 'draft.artwork') return 'Staging artwork…';
      if (work.tool === 'draft.transform') return 'Staging transformations…';
      return 'Staging changes…';
    }
    if (work?.kind === 'saving') return 'Preparing to save…';
    if (work?.kind === 'searching') return 'Searching…';
    return '';
  }

  const visibleAgents = $derived(status.connections.filter(connection => !$hiddenAgentSessions.has(connection.id)));
  const visibleIds = $derived(new Set(visibleAgents.map(connection => connection.id)));
  const rows = $derived(visibleAgents.map(connection => ({
    connection,
    name: agentClientName(connection),
    write: status.writes.find(write => write.session === connection.id),
    work: status.work.find(work => work.session === connection.id),
  })));
  const detachedWrite = $derived(status.writes.find(write => !write.session ||
    (!visibleIds.has(write.session) && !$hiddenAgentSessions.has(write.session))));
  const detachedWork = $derived(status.work.find(work => !work.session ||
    (!visibleIds.has(work.session) && !$hiddenAgentSessions.has(work.session))));
  const active = $derived(!!(detachedWrite || detachedWork || rows.some(row => row.write || row.work)));
  const connectedAgents = $derived(status.connections.filter(connection => connection.state !== 'disconnected'));
  const connected = $derived(connectedAgents.length > 0);
  const latestResult = $derived(finished ? `${finished.files} ${finished.files === 1 ? 'file' : 'files'} ${finished.kind}` : '');
  const detachedProgress = $derived(jobText(detachedWrite, detachedWork));
  const progressText = $derived(detachedProgress || rows.map(row => jobText(row.write, row.work)).find(Boolean) || latestResult);
  const connectionLabel = $derived(
    connectedAgents.length === 1 && visibleIds.has(connectedAgents[0].id)
      ? `${agentClientName(connectedAgents[0])} connected${connectedAgents[0].model ? ` · ${connectedAgents[0].model}` : ''}`
      : connected ? `${connectedAgents.length} ${connectedAgents.length === 1 ? 'agent' : 'agents'} connected` : 'No agent connected',
  );
  const label = $derived(progressText ? `${connectionLabel} · ${progressText}` : connectionLabel);

  function control(connection: AgentConnection, action: SessionAction) {
    return run('mcp_control_agent', { sessionId: connection.id, action });
  }

  async function hide(connection: AgentConnection) {
    setAgentVisible(connection.id, false);
    await tick();
    document.getElementById('agent-add')?.focus({ preventScroll: true });
  }

  function showResult(kind: 'staged' | 'saved', files: number) {
    clearTimeout(finishTimer);
    finished = { kind, files };
    finishTimer = setTimeout(() => finished = null, 2600);
  }

  async function openSetup() {
    focusHandoff = true;
    open = false;
    await tick();
    const { openSettings } = await import('../js/settings.js');
    await openSettings();
    document.querySelector('#settings-nav-list use[href="#i-agent"]')?.closest('button')?.click();
  }

  async function run(command: string, args: Record<string, unknown>) {
    if (!invoke || busy) return;
    busy = true;
    error = '';
    try { await invoke(command, args); }
    catch (failure) { error = String(failure); }
    finally { busy = false; }
  }

  async function showRecovery() {
    focusHandoff = true;
    open = false;
    await tick();
    document.dispatchEvent(new CustomEvent('tagrex:show-agent-recovery'));
  }

  onMount(() => {
    if (!native?.core) return;
    let disposed = false;
    let running = false;
    let queued = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let unlisten: (() => void) | undefined;

    function schedule() {
      queued = true;
      if (!disposed && !running && !timer) timer = setTimeout(() => {
        timer = undefined;
        void refresh();
      }, 80);
    }

    async function refresh() {
      if (disposed || running) return;
      queued = false;
      running = true;
      try {
        const next = await invoke('mcp_agent_status', {}) as Status;
        if (disposed) return;
        status = next;
        publishAgentConnections(next.connections);
        if (next.staged && next.staged.id !== lastStage) {
          lastStage = next.staged.id;
          if ((!next.staged.session || !$hiddenAgentSessions.has(next.staged.session)) && (!finished || finished.kind !== 'saved')) showResult('staged', next.staged.files);
        }
      } catch {
        // Development can serve a newer frontend to an older native shell.
      } finally {
        running = false;
        if (queued) schedule();
      }
    }

    void import('../js/invoke.js').then(async api => {
      if (disposed) return;
      invoke = api.invoke;
      unlisten = await native.event?.listen('tagrex://agent-event', ({ payload }) => {
        if (payload.kind === 'operation_changed' && payload.data?.completed &&
            payload.data.status === 'succeeded' && payload.data.processed && payload.data.operation !== lastSave) {
          lastSave = payload.data.operation || '';
          showResult('saved', payload.data.processed);
        }
        if (['agent_activity_changed', 'operation_changed', 'service_changed'].includes(payload.kind)) schedule();
      });
      if (disposed) { unlisten?.(); return; }
      await refresh();
    });
    return () => {
      disposed = true;
      clearTimeout(timer);
      clearTimeout(finishTimer);
      unlisten?.();
    };
  });
</script>

{#if native?.core}
  <Popover.Root bind:open>
    <Popover.Trigger>
      {#snippet child({ props })}
        <Button {...props} id="agent-activity" variant="ghost" size="icon-sm" class="icon agent-status-trigger"
          data-agent-state={active ? 'working' : finished ? 'finished' : connected ? 'connected' : 'disconnected'}
          title={label} aria-label={label}>
          {#if finished && !active}<Check class="ico" aria-hidden="true" />
          {:else}<svg class="ico" aria-hidden="true"><use href="#i-agent" /></svg>{/if}
          <span class="agent-status-dot" aria-hidden="true"></span>
        </Button>
      {/snippet}
    </Popover.Trigger>
    <Popover.Content class="controller-popover-content agent-status-popover" align="end" role="dialog" aria-label="Agents"
      onCloseAutoFocus={(event) => { if (focusHandoff) event.preventDefault(); focusHandoff = false; }}>
      <div class="agent-status-header">
        <h3>Agents</h3>
        {#if status.connections.length}
          <Button id="agent-add" size="icon-xs" variant="ghost" class="agent-floating-action" title="Connect an agent" aria-label="Connect an agent" onclick={openSetup}><Plus aria-hidden="true" /></Button>
        {/if}
      </div>
      <div class="agent-status-rows">
        {#each rows as { connection, name, write, work } (connection.id)}
          <div class="agent-status-row">
            <div class="agent-status-row-head">
              <div class="agent-status-client">
                <span class="agent-status-client-dot" data-agent-state={connection.state} aria-hidden="true"></span>
                <strong>{name}</strong>
                {#if connection.model}<span class="agent-status-model">{connection.model}</span>{/if}
              </div>
              <div class="agent-row-controls">
                {#if write}
                  <Button size="icon-xs" variant="ghost" class="agent-floating-action" disabled={busy || write.status === 'rolling_back'}
                    title="Stop job" aria-label={`Stop job for ${name}`} onclick={() => run('mcp_cancel_operation', { operationId: write.id })}><Square aria-hidden="true" /></Button>
                {/if}
                {#if connection.controllable}
                  {#if connection.state !== 'disconnected'}
                    <Button size="icon-xs" variant="ghost" class="agent-floating-action" disabled={busy}
                      title={connection.state === 'paused' ? 'Resume edits' : 'Pause edits'}
                      aria-label={`${connection.state === 'paused' ? 'Resume edits' : 'Pause edits'} for ${name}`}
                      onclick={() => control(connection, connection.state === 'paused' ? 'resume' : 'pause')}>
                      {#if connection.state === 'paused'}<Play aria-hidden="true" />{:else}<Pause aria-hidden="true" />{/if}
                    </Button>
                  {/if}
                  <Button size="icon-xs" variant="ghost" class="agent-floating-action" disabled={busy}
                    title={connection.state === 'disconnected' ? 'Reconnect agent' : 'Disconnect agent'}
                    aria-label={`${connection.state === 'disconnected' ? 'Reconnect' : 'Disconnect'} ${name}`}
                    onclick={() => control(connection, connection.state === 'disconnected' ? 'reconnect' : 'disconnect')}>
                    {#if connection.state === 'disconnected'}<Plug aria-hidden="true" />{:else}<Unplug aria-hidden="true" />{/if}
                  </Button>
                {/if}
                <Button size="icon-xs" variant="ghost" class="agent-floating-action"
                  title="Hide from Agents" aria-label={`Hide ${name} from Agents`} onclick={() => hide(connection)}><X aria-hidden="true" /></Button>
              </div>
            </div>
            {#if write || work || connection.state !== 'active'}
              <div class="agent-row-status">
                <span>{jobText(write, work) || (connection.state === 'paused' ? 'Edits paused' : 'Disconnected')}</span>
                {#if write?.status === 'writing' && write.total > 0}
                  <progress max={write.total} value={Math.min(write.processed, write.total)} aria-label="Save progress"></progress>
                {/if}
              </div>
            {:else if !connection.controllable}
              <div class="agent-row-status">Start a new agent session to use edit controls.</div>
            {/if}
          </div>
        {:else}
          <p class="agent-status-empty">{status.connections.length ? 'No agents shown' : 'No agent connected'}</p>
        {/each}
      </div>
      {#if detachedProgress || latestResult}
        <div class="agent-status-work">
          <span>{detachedProgress || latestResult}</span>
          {#if detachedWrite?.status === 'writing' && detachedWrite.total > 0}
            <progress max={detachedWrite.total} value={Math.min(detachedWrite.processed, detachedWrite.total)} aria-label="Save progress"></progress>
          {/if}
          {#if detachedWrite}
            <div class="agent-status-actions">
              <Button size="icon-xs" variant="ghost" class="agent-floating-action" disabled={busy || detachedWrite.status === 'rolling_back'}
                title="Stop job" aria-label="Stop agent job" onclick={() => run('mcp_cancel_operation', { operationId: detachedWrite.id })}><Square aria-hidden="true" /></Button>
            </div>
          {/if}
        </div>
      {/if}
      {#if status.recovery_required}
        <div class="agent-status-actions"><Button size="sm" variant="secondary" onclick={showRecovery}>Check interrupted work</Button></div>
      {/if}
      {#if !status.connections.length}
        <div class="agent-status-empty-actions">
          {#if !status.enabled && status.can_reconnect}
            <Button size="sm" variant="ghost" class="agent-floating-action" disabled={busy} onclick={() => run('mcp_set_enabled', { enabled: true })}><Plug aria-hidden="true" />Reconnect agents</Button>
          {/if}
          <Button size="sm" variant="ghost" class="agent-floating-action" onclick={openSetup}><Plus aria-hidden="true" />Connect an agent</Button>
        </div>
      {/if}
      {#if error}<p class="agent-status-error" role="alert">{error}</p>{/if}
    </Popover.Content>
  </Popover.Root>
{/if}
