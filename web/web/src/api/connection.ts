/**
 * connection.ts — browser-local `solx-server` connection.
 *
 * There is no backend anymore to hold `serverUrl`/`serverToken` for us
 * (previously `server/src/solx-context.ts`, reading env vars). The browser
 * holds them itself, in `localStorage`, entered via `SettingsPanel`. This
 * is the one piece of "config" that has an HTTP-era equivalent — anything
 * else `solx.config.*` used to expose (`dataDirectory`, `filesDirectory`,
 * package registry, …) is local-only to a running `solx-server`/`solx-cli`
 * process and has no browser-side counterpart; that functionality is gone,
 * not moved.
 */
import { connectHttp, type HttpSolx } from "@solx/http";

const STORAGE_KEY_URL = "solx.serverUrl";
const STORAGE_KEY_TOKEN = "solx.serverToken";

const DEFAULT_SERVER_URL = "http://127.0.0.1:8766";

type Listener = () => void;
const listeners = new Set<Listener>();

let cached: HttpSolx | null = null;
let cachedUrl: string | null = null;
let cachedToken: string | null = null;

export function getStoredServerUrl(): string {
  return localStorage.getItem(STORAGE_KEY_URL) ?? DEFAULT_SERVER_URL;
}

export function getStoredServerToken(): string {
  return localStorage.getItem(STORAGE_KEY_TOKEN) ?? "";
}

export function isConfigured(): boolean {
  return getStoredServerToken().trim().length > 0;
}

/** Persist a new server URL/token and invalidate the cached connection. */
export function setConnection(serverUrl: string, serverToken: string): void {
  localStorage.setItem(STORAGE_KEY_URL, serverUrl.trim() || DEFAULT_SERVER_URL);
  localStorage.setItem(STORAGE_KEY_TOKEN, serverToken.trim());
  cached = null;
  cachedUrl = null;
  cachedToken = null;
  for (const cb of listeners) cb();
}

/** Subscribe to connection changes (e.g. to invalidate cached queries). Returns an unsubscribe function. */
export function onConnectionChange(cb: Listener): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

/**
 * Get the current `HttpSolx` handle, building (or rebuilding, if the
 * stored URL/token changed since the last call) as needed. Throws a
 * `SolxError`-shaped error if no token has been configured yet.
 */
export function getConnection(): HttpSolx {
  const url = getStoredServerUrl();
  const token = getStoredServerToken();
  if (!token.trim()) {
    throw new Error(
      "solx-server is not configured yet — open Settings and enter the server URL and token.",
    );
  }
  if (cached && cachedUrl === url && cachedToken === token) return cached;
  cached = connectHttp(url, token);
  cachedUrl = url;
  cachedToken = token;
  return cached;
}
