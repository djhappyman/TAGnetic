import { writable } from 'svelte/store';

export type AgentConnection = {
  id: string;
  principal: string;
  name: string;
  model?: string;
  state: 'active' | 'paused' | 'disconnected';
  controllable: boolean;
};

export const liveAgentConnections = writable<AgentConnection[]>([]);
export const hiddenAgentSessions = writable(new Set<string>());

export function publishAgentConnections(connections: AgentConnection[]) {
  liveAgentConnections.set(connections);
  hiddenAgentSessions.update(current => {
    const live = new Set(connections.map(connection => connection.id));
    return new Set([...current].filter(id => live.has(id)));
  });
}

export function setAgentVisible(id: string, visible: boolean) {
  hiddenAgentSessions.update(current => {
    const next = new Set(current);
    if (visible) next.delete(id);
    else next.add(id);
    return next;
  });
}

export function agentClientName(connection: AgentConnection) {
  if (/codex/i.test(connection.name)) return 'Codex';
  if (/open[ -]?code/i.test(connection.name)) return 'OpenCode';
  return connection.name || 'Agent';
}
