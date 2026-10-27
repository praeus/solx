/**
 * @solx/tauri — the `@solx/surface` manager interfaces over Tauri IPC, for
 * a webview hosted by a solx desktop app (the solx Tauri plugin,
 * solx-core/solx-tauri-plugin). The IPC counterpart of `@solx/http`: same
 * managers, same errors, `invoke()` in place of `fetch`, and no URL or
 * token — the host process already holds the connection.
 *
 * Which backend answers is the host's business: its own in-process
 * managers (the server build) or a `solx-server` it proxies to (the client
 * build). {@link TauriHost.info} says which.
 *
 * @packageDocumentation
 */
import type { RemoteSolx } from '@solx/surface';
import { TauriActionManager } from './actions.js';
import { TauriAuth } from './auth.js';
import { TauriConfig } from './config.js';
export { TauriPackages } from './packages.js';
export type { CatalogEntry, CatalogEntryState, CatalogView, PackageOpResult } from './packages.js';
import { TauriDocManager } from './docs.js';
import { TauriFileStore } from './files.js';
import { TauriPackages } from './packages.js';
import { TauriTransport, type InvokeFn } from './invoke.js';
import { TauriTypeManager } from './types.js';

export { TauriActionManager } from './actions.js';
export { TauriDocManager } from './docs.js';
export { TauriFileStore } from './files.js';
export { TauriTypeManager } from './types.js';
export type { InvokeArgs, InvokeFn, InvokeOptions } from './invoke.js';
export { TauriConfig } from './config.js';
export { TauriAuth } from './auth.js';
export type {
  ClientInfo,
  ConnectCode,
  CreateConnectCodeOptions,
  IssuedClient,
  ScopePreset,
  TokenAccess,
} from './auth.js';
export type {
  CommandDef,
  ConfigChanges,
  ConfigUpdateResult,
  ConfigView,
  EffectiveConfig,
  ExclusionLists,
  Exclusions,
  InstalledPackage,
  PackageResult,
  SolxConfigFile,
  TlsCa,
  TlsCert,
  TlsResult,
  TlsStatus,
  ToolRule,
} from './config.js';

/** What the host reports about itself — the IPC stand-in for `GET /health`. */
export interface HostInfo {
  /** `local`: managers run in the app. `remote`: proxied to `serverUrl`. `disconnected`: a client with no server set. */
  mode: 'local' | 'remote' | 'disconnected';
  /** True for a client build, which can {@link TauriHost.connect}. */
  client: boolean;
  version: string;
  serverUrl?: string | null;
  /** The embedded `solx-server` a local host also runs, if any. */
  embeddedServer?: { url: string; https: boolean } | null;
}

/** Host-level commands with no manager equivalent. */
export class TauriHost {
  constructor(private readonly t: TauriTransport) {}

  info(): Promise<HostInfo> {
    return this.t.call('info');
  }

  /**
   * Client builds: point the host at a `solx-server` (checked, then
   * persisted by the host). `credential` is a connect code (`1234-5678`),
   * a connect link (which also supplies the server, so `serverUrl` may be
   * empty) or a token; the host redeems a code before connecting.
   */
  async connect(serverUrl: string, credential: string): Promise<void> {
    await this.t.call('connect', { serverUrl, credential });
  }

  /** Client builds: forget the server. */
  async disconnect(): Promise<void> {
    await this.t.call('disconnect');
  }
}

export interface TauriSolx extends RemoteSolx {
  types: TauriTypeManager;
  files: TauriFileStore;
  docs: TauriDocManager;
  actions: TauriActionManager;
  host: TauriHost;
  /**
   * `solx-config.json`. Only works in a window granted `solx:config` (the
   * desktop app's Settings window); elsewhere every call is denied.
   */
  config: TauriConfig;
  /**
   * The official package catalog: install/uninstall by name. Needs
   * `solx:packages` (the main and Settings windows) and a local host.
   */
  packages: TauriPackages;
  /**
   * Connect codes and client tokens. Only works in a window granted
   * `solx:auth` (the desktop app's Settings window) on a host build.
   */
  auth: TauriAuth;
}

/**
 * Connect to the solx host this webview runs in. Pass Tauri's `invoke`:
 *
 * ```ts
 * import { invoke } from '@tauri-apps/api/core';
 * const solx = connectTauri(invoke);
 * ```
 */
export function connectTauri(invoke: InvokeFn): TauriSolx {
  const t = new TauriTransport(invoke);
  return {
    types: new TauriTypeManager(t),
    files: new TauriFileStore(t),
    docs: new TauriDocManager(t),
    actions: new TauriActionManager(t),
    host: new TauriHost(t),
    config: new TauriConfig(t),
    packages: new TauriPackages(t),
    auth: new TauriAuth(t),
  };
}
