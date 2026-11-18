import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mcpInstallCommand, mcpCheckCommand } from '../../app/ui/js/mcp-install.js';

const pairing = {
  executable: '/Applications/Tagnetic app/tagnetic',
  profile: '/home/user/Application Data/Tagnetic',
  credential_file: "/home/user/Agent's files/client.token",
};

for (const client of ['Codex', 'OpenCode']) {
  test(`${client} registration uses exact executable, profile and credential paths`, { skip: process.platform === 'win32' }, () => {
    const command = mcpInstallCommand(client, pairing);
    const args = command.slice(command.indexOf(' -- ') + 4);
    // Check actual shell parsing, including spaces and an apostrophe. Tokens
    // remain single arguments and no credential contents enter the command.
    const parsed = spawnSync('sh', ['-c', `printf '%s\\0' ${args}`], { encoding: 'utf8' });
    assert.equal(parsed.status, 0);
    assert.deepEqual(parsed.stdout.split('\0').slice(0, -1), [pairing.executable, '--mcp-stdio', '--mcp-profile', pairing.profile, '--mcp-credential', pairing.credential_file]);
    assert.ok(command.startsWith(client === 'Codex' ? 'codex mcp add tagnetic -- ' : 'opencode mcp add tagnetic --global -- '));
  });
}

test('shell metacharacters in a path stay literal', { skip: process.platform === 'win32' }, () => {
  const data = { ...pairing, executable: '/tmp/$(printf injected)`printf injected`/tagnetic' };
  const args = mcpInstallCommand('Codex', data).split(' -- ')[1];
  const result = spawnSync('sh', ['-c', `printf '%s\\0' ${args}`], { encoding: 'utf8' });
  assert.equal(result.stdout.split('\0')[0], data.executable);
});

test('Windows uses PowerShell quoting for paths and apostrophes', () => {
  const command = mcpInstallCommand('Codex', { ...pairing, executable: 'C:\\Program Files\\Tagnetic\\tagnetic.exe', credential_file: "C:\\User's files\\client.token" });
  assert.ok(command.includes("'C:\\Program Files\\Tagnetic\\tagnetic.exe'"));
  assert.ok(command.includes("'C:\\User''s files\\client.token'"));
});

test('only supported clients are registered and verification never installs', () => {
  assert.throws(() => mcpInstallCommand('Unknown', pairing));
  assert.equal(mcpCheckCommand('Codex'), 'codex mcp list');
  assert.equal(mcpCheckCommand('OpenCode'), 'opencode mcp list');
});
