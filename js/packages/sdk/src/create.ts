/**
 * `createSolx` — the entry point for every consumer.
 *
 * Opens the solx appdata directory and every backing store:
 *   - `solx-config`     — config service (sync, in-memory snapshot; always local)
 *   - `solx-types`      — type registry (libsql, or a remote proxy)
 *   - `solx-files`      — file store (on-disk, or a remote proxy)
 *   - `solx-docs`       — document store (libsql + Tantivy, or a remote proxy)
 *   - `solx-actions`    — action store (libsql, or a remote proxy)
 *   - `solx-scripts`    — shell pipeline runner, scoped to actions
 *
 * **Local vs. remote (client) mode:** every manager except `config`
 * has two constructors, `open(...)` (local) and `connect(serverUrl,
 * token)` (remote/client). `createSolx` decides which to call, once,
 * for all of them — mirroring `solx-manager::App::build_with_config`'s
 * precedence exactly:
 *
 *   1. `opts.serverUrl`/`opts.serverToken` (explicit override)
 *   2. `SOLX_SERVER_URL`/`SOLX_SERVER_TOKEN` env vars
 *   3. `config.snapshot().serverUrl`/`serverToken` (persisted config)
 *
 * `config` itself is **always** local — it's never proxied, even in
 * remote mode, because it's the thing that tells the other managers
 * which mode to use (same as `solx-manager`, where `App`'s config is
 * always the local `ConfigService` regardless of how `types`/`files`/
 * `docs`/`actions` are wired).
 *
 * Cross-store dependencies in local mode (e.g. `DocManager` needs
 * `TypeManager` for content validation, `ActionManager` needs all
 * three) are wired up the same way `solx-manager::App::wire_local`
 * does in Rust.
 */

import { SolxError } from '@solx/surface';
import type { Solx } from './solx.js';
import { TypeManager } from '@solx/types';
import { FileStore } from '@solx/files';
import { ConfigService } from '@solx/config';
import { DocManager } from '@solx/docs';
import { ActionManager } from '@solx/actions';
import { ScriptRunner } from '@solx/scripts';

export interface CreateSolxOptions {
  /**
   * Override the appdata directory. Defaults to:
   *   - `$SOLX_APPDATA_DIR` if set
   *   - `%APPDATA%/praeus/solx` on Windows
   *   - `$HOME/.praeus/solx` elsewhere
   */
  appdata?: string;
  /**
   * Explicit remote connection, taking precedence over the
   * `SOLX_SERVER_URL`/`SOLX_SERVER_TOKEN` env vars and the config
   * file's `serverUrl`/`serverToken`. Set this to force client mode
   * (e.g. in tests) without touching the environment or config.
   */
  serverUrl?: string;
  serverToken?: string;
}

/**
 * Resolve the default appdata directory. Mirrors
 * `solx_config::appdata_dir` in the Rust impl.
 */
function defaultAppdata(): string {
  const env = process.env['SOLX_APPDATA_DIR'];
  if (env) return env;
  if (process.platform === 'win32') {
    const appdata = process.env['APPDATA'];
    if (appdata) return `${appdata}\\praeus\\solx`;
  } else {
    const home = process.env['HOME'];
    if (home) return `${home}/.praeus/solx`;
  }
  // Last-resort fallback (the Rust impl uses a temp dir).
  return `${process.env['TMP'] ?? process.env['TEMP'] ?? '/tmp'}/.praeus/solx`;
}

/**
 * Open every backing store and return the aggregate. Resolves
 * with a `Solx`; rejects with a `SolxError` if opening any store
 * fails.
 */
export async function createSolx(opts: CreateSolxOptions = {}): Promise<Solx> {
  const appdata = opts.appdata ?? defaultAppdata();

  // Config is always local and is the source of truth for
  // derived paths (types DB, files dir, ...) and for the
  // local/remote decision below.
  const config = opts.appdata
    ? ConfigService.openIn(opts.appdata)
    : ConfigService.open();

  const serverUrl =
    opts.serverUrl ?? process.env['SOLX_SERVER_URL'] ?? config.snapshot().serverUrl;

  let types: TypeManager;
  let files: FileStore;
  let docs: DocManager;
  let actions: ActionManager;

  if (serverUrl) {
    const token =
      opts.serverToken ?? process.env['SOLX_SERVER_TOKEN'] ?? config.snapshot().serverToken;
    if (!token) {
      throw new SolxError(
        'Config',
        'server_url is configured but no server_token is available ' +
          '(point SOLX_SERVER_TOKEN at the token solx-server printed on ' +
          'first start, or set config.serverToken).',
      );
    }
    types = await TypeManager.connect(serverUrl, token);
    files = await FileStore.connect(serverUrl, token);
    docs = await DocManager.connect(serverUrl, token);
    actions = await ActionManager.connect(serverUrl, token);
  } else {
    types = await TypeManager.open(config.typesDbPath());
    files = await FileStore.open(config.filesDir());
    docs = await DocManager.open(config.docsDbPath(), `${config.searchIndexDir()}/docs`, types);
    actions = await ActionManager.open(config.actionsDbPath(), config, types, docs, files);
  }

  const scripts = ScriptRunner.for(actions);

  return {
    appdata,
    types,
    files,
    config,
    docs,
    actions,
    scripts,
  };
}
