/**
 * connection.ts — how the UI reaches solx.
 *
 * Two transports, picked once at load:
 *
 * - **Browser**: `solx-server` over HTTP via `@solx/http`. There is no
 *   backend to hold `serverUrl`/`serverToken` for us, so the browser keeps
 *   them itself in `localStorage`, entered via `SettingsPanel`. Anything
 *   else `solx.config.*` used to expose (`dataDirectory`, package registry,
 *   …) is local-only to a running `solx-server`/`solx-cli` process and has
 *   no browser-side counterpart.
 * - **Tauri** (the solx desktop app): IPC to the host via `@solx/tauri`. No
 *   URL or token here — the host either runs solx in-process (server build)
 *   or holds the `solx-server` connection itself (client build, set through
 *   {@link connectHost}). {@link refreshHostInfo} says which.
 *
 * Callers only see `RemoteSolx`, the manager set both transports provide.
 */
import { invoke, isTauri } from "@tauri-apps/api/core";
import { connectHttp } from "@solx/http";
import type { RemoteSolx } from "@solx/surface";
import { connectTauri, type HostInfo, type TauriSolx } from "@solx/tauri";

/** True when running inside the solx desktop app's webview. */
export const IN_TAURI = isTauri();

const STORAGE_KEY_URL = "solx.serverUrl";
const STORAGE_KEY_TOKEN = "solx.serverToken";

const DEFAULT_SERVER_URL = "http://127.0.0.1:8766";

type Listener = () => void;
const listeners = new Set<Listener>();

let cached: RemoteSolx | null = null;
let cachedUrl: string | null = null;
let cachedToken: string | null = null;

let tauri: TauriSolx | null = null;
/** Last `info` from the desktop host; `null` until first fetched (or in a browser). */
let hostInfo: HostInfo | null = null;

function tauriSolx(): TauriSolx {
  return (tauri ??= connectTauri(invoke));
}

function notify(): void {
  for (const cb of listeners) cb();
}

export function getStoredServerUrl(): string {
  return localStorage.getItem(STORAGE_KEY_URL) ?? DEFAULT_SERVER_URL;
}

export function getStoredServerToken(): string {
  return localStorage.getItem(STORAGE_KEY_TOKEN) ?? "";
}

/**
 * Whether there is anything to talk to yet. In Tauri that's every host
 * except a client with no server set; until the host has answered `info`,
 * assume yes rather than flash a "not connected" banner.
 */
export function isConfigured(): boolean {
  if (IN_TAURI) return hostInfo?.mode !== "disconnected";
  return getStoredServerToken().trim().length > 0;
}

/** Human-readable name of what the UI is connected to, for status text. */
export function connectionLabel(): string {
  if (!IN_TAURI) return getStoredServerUrl();
  switch (hostInfo?.mode) {
    case "local":
      return "this app (local)";
    case "remote":
      return hostInfo.serverUrl ?? "solx-server";
    case "disconnected":
      return "no server";
    default:
      return "the solx host";
  }
}

/** The desktop host's last-known state, or `null` in a browser / before the first `refreshHostInfo`. */
export function getHostInfo(): HostInfo | null {
  return hostInfo;
}

/** Ask the desktop host what it is (local, remote, disconnected) and cache the answer. */
export async function refreshHostInfo(): Promise<HostInfo> {
  hostInfo = await tauriSolx().host.info();
  return hostInfo;
}

/** Persist a new server URL/token and invalidate the cached connection (browser only). */
export function setConnection(serverUrl: string, serverToken: string): void {
  localStorage.setItem(STORAGE_KEY_URL, serverUrl.trim() || DEFAULT_SERVER_URL);
  localStorage.setItem(STORAGE_KEY_TOKEN, serverToken.trim());
  cached = null;
  cachedUrl = null;
  cachedToken = null;
  notify();
}

/**
 * Desktop client build: point the host at a `solx-server`. The host checks
 * the URL/token before persisting them, so this rejects (with a
 * `SolxError`) on a bad URL or token.
 */
export async function connectHost(serverUrl: string, serverToken: string): Promise<void> {
  await tauriSolx().host.connect(serverUrl, serverToken);
  await refreshHostInfo();
  notify();
}

/** Desktop client build: forget the server. */
export async function disconnectHost(): Promise<void> {
  await tauriSolx().host.disconnect();
  await refreshHostInfo();
  notify();
}

/** The desktop host's IPC client (managers, host, config). Throws outside Tauri. */
export function getTauri(): TauriSolx {
  if (!IN_TAURI) throw new Error("not running in the solx desktop app");
  return tauriSolx();
}

/**
 * Desktop server build: open (or focus) the Settings window — the only
 * window allowed to edit solx-config.json. See `open_settings` in
 * solx-explorer's `lib.rs`.
 */
export async function openSettingsWindow(): Promise<void> {
  await invoke("open_settings");
}

/** Desktop app: restart, e.g. after changing a startup-only config key. */
export async function restartApp(): Promise<void> {
  await invoke("restart_app");
}

/** Subscribe to connection changes (e.g. to invalidate cached queries). Returns an unsubscribe function. */
export function onConnectionChange(cb: Listener): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

/**
 * Get the current manager set. In Tauri this is always the IPC client (a
 * disconnected host answers each call with a `Config` error). In a browser
 * it's built (or rebuilt, if the stored URL/token changed since the last
 * call) as needed, and throws if no token has been configured yet.
 */
export function getConnection(): RemoteSolx {
  if (IN_TAURI) return tauriSolx();
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
