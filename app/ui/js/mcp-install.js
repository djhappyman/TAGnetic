// These commands register the existing stdio connector; pairing alone does not
// change an agent's configuration. Credential contents never enter a command.
export function mcpInstallCommand(client, paired) {
  if (!['Codex', 'OpenCode'].includes(client)) throw new Error('Choose Codex or OpenCode.');
  const windows = /^[A-Za-z]:[\\/]|^\\\\/.test(paired.executable);
  const quote = value => windows
    ? `'${String(value).replaceAll("'", "''")}'`
    : `'${String(value).replaceAll("'", "'\"'\"'")}'`;
  const command = client === 'Codex' ? 'codex mcp add tagnetic --' : 'opencode mcp add tagnetic --global --';
  return `${command} ${[
    paired.executable, '--mcp-stdio', '--mcp-profile', paired.profile,
    '--mcp-credential', paired.credential_file,
  ].map(quote).join(' ')}`;
}

export function mcpCheckCommand(client) {
  return client === 'OpenCode' ? 'opencode mcp list' : 'codex mcp list';
}

export function mcpPairingGrant(root) {
  return {
    roots: [root],
    actions: ['read', 'stage', 'write', 'providers', 'settings', 'outputs', 'history', 'open', 'playback', 'ui'],
    fields: [], providers: [], max_files: 100000, expires_at: null,
  };
}
