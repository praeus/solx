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
import { TauriConfig } from './config.js';
import { TauriDocManager } from './docs.js';
import { TauriFileStore } from './files.js';
import { TauriTransport, type InvokeFn } from './invoke.js';
import { TauriTypeManager } from './types.js';

export { TauriActionManager } from './actions.js';
export { TauriDocManager } from './docs.js';
export { TauriFileStore } from './files.js';
export { TauriTypeManager } from './types.js';
export type { InvokeArgs, InvokeFn, InvokeOptions } from './invoke.js';
export { TauriConfig } from './config.js';
export type {
  CommandDef,
  ConfigChanges,
  ConfigUpdateResult,
  ConfigView,
  EffectiveConfig,
  InstalledPackage,
  SolxConfigFile,
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
  embeddedServer?: { url: string; token: string } | null;
}

/** Host-level commands with no manager equivalent. */
export class TauriHost {
  constructor(private readonly t: TauriTransport) {}

  info(): Promise<HostInfo> {
    return this.t.call('info');
  }

  /** Client builds: point the host at a `solx-server` (checked, then persisted by the host). */
  async connect(serverUrl: string, token: string): Promise<void> {
    await this.t.call('connect', { serverUrl, token });
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
  };
}
