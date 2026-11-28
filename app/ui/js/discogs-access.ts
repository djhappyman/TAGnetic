export type DiscogsAccessRequest = {
  rejected: boolean;
  verify: (token: string) => Promise<void>;
  finish: (token: string | null) => void;
};

let pending: Promise<string | null> | undefined;

export function isDiscogsAuthError(error: unknown): boolean {
  const failure = (error as { failure?: { message?: { code?: string } } } | null)?.failure;
  return failure?.message?.code === 'error.provider.auth'
    || (error instanceof Error && /authentication failed: HTTP (401|403)\b/i.test(error.message));
}

export async function withDiscogsAccess<Result>(
  use: (token: string) => Promise<Result>,
  isCurrent = () => true,
): Promise<Result | null> {
  const { invoke } = await import('./invoke.js');
  const field = document.querySelector<HTMLInputElement>('#discogs-token');
  const saved = field?.value.trim() || String(await invoke('saved_discogs_token', {}) ?? '').trim();
  if (!isCurrent()) return null;
  if (saved && !pending) {
    try { return await use(saved); }
    catch (error) { if (!isDiscogsAuthError(error)) throw error; }
  }
  if (!isCurrent()) return null;

  if (pending) {
    const replacement = await pending;
    return replacement && isCurrent() ? use(replacement) : null;
  }

  let result: Result | null = null;
  let finish!: DiscogsAccessRequest['finish'];
  pending = new Promise(resolve => { finish = resolve; });
  const request: DiscogsAccessRequest = {
    rejected: Boolean(saved),
    verify: async token => { result = await use(token); },
    finish,
  };
  document.dispatchEvent(new CustomEvent('tagnetic:discogs-access', { detail: request }));
  try { return await pending ? result : null; }
  finally { pending = undefined; }
}
