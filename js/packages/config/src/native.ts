/**
 * TS-side wrapper around the `solx-bindings` config module.
 *
 * The Rust side exposes flat, prefixed function names
 * (`configOpen`, `configGet`, `configSet`, etc.) under
 * `require("@solx/<triple>")`. This module:
 *
 *   1. Loads the `.node` binary via the shared loader.
 *   2. Wraps every call in `withSolxError` so consumers
 *      get a typed `SolxError` instead of a raw string-
 *      prefixed `Error`.
 *   3. Encodes/decodes JSON values across the boundary.
 *   4. Returns a `ConfigService` class that conforms
 *      structurally to the `ConfigService` interface
 *      declared in `@solx/surface`.
 *
 * The public class is just `ConfigService` — not
 * `LocalConfigService` — because "local" is an
 * implementation detail. The same class shape will wrap a
 * future client-backed binding; the native binary decides
 * which impl to load via Cargo features (`local` vs
 * `client`). Consumers should not branch on which one
 * they got.
 *
 * The class satisfies the surface `ConfigService`
 * interface (sync, ergonomic `mutate(fn)`, etc.) **and**
 * exposes derived-path helpers (`typesDbPath`, `filesDir`,
 * `docsDbPath`, `actionsDbPath`, `logsDir`, …) that the
 * umbrella `createSolx` uses to wire the other services.
 * A single class — no separate "raw" / "adapter" layer.
 */

import { SolxError } from '@solx/surface';
import type {
  ConfigService as ConfigServiceInterface,
  JsonValue,
  SolxConfig,
  InstalledPackage,
} from '@solx/surface';
import { loadNative } from '@solx/types/loader';
import type { NativeConfigModule, ConfigServiceHandle } from './native-types.js';

export type { NativeConfigModule, ConfigServiceHandle };

function withSolxError<T>(fn: () => T): T {
  try {
    return fn();
  } catch (e) {
    throw SolxError.fromMessage(e);
  }
}

// ----- snake_case DTO mirror (Rust serde shape, see solx-config/src/types.rs) -----

interface SolxConfigSnake {
  data_directory?: string;
  files_directory?: string;
  models_directory?: string;
  search_index_dir?: string;
  docs_db?: string;
  actions_db?: string;
  types_db?: string;
  installed_packages: InstalledPackage[];
  server_url?: string;
  server_token?: string;
  server_port?: number;
}

function snapshotFromSnake(s: SolxConfigSnake): SolxConfig {
  return {
    dataDirectory: s.data_directory,
    filesDirectory: s.files_directory,
    modelsDirectory: s.models_directory,
    searchIndexDir: s.search_index_dir,
    docsDb: s.docs_db,
    actionsDb: s.actions_db,
    typesDb: s.types_db,
    installedPackages: s.installed_packages,
    serverUrl: s.server_url,
    serverToken: s.server_token,
    serverPort: s.server_port,
  };
}

// ----- ConfigService -----
//
// Public name matches the `@solx/surface` `ConfigService`
// interface and the `solx_surface::managers::ConfigService`
// trait (when it lands — see ARCHITECTURE.md §3.1). The
// class is the concrete handle; the "local" / "client"
// choice lives at the native binary level, not in the TS
// surface.

/**
 * On-disk config service backed by `solx-config`.
 *
 * The Rust impl is **synchronous** — there is no event loop
 * hop and no `Promise`. We surface it as direct calls so
 * the consumer-facing API matches `@solx/surface`.
 *
 * Mutations (`set`, `patch`, `registerPackage`,
 * `unregisterPackage`) take an OS-level advisory lock on
 * the config file, so concurrent writers serialize
 * cleanly across processes.
 */
export class ConfigService implements ConfigServiceInterface {
  private constructor(private readonly handle: ConfigServiceHandle) {}

  /** Open a config rooted at the platform-default appdata dir. */
  static open(): ConfigService {
    return withSolxError(() => {
      const native = loadNative() as unknown as NativeConfigModule;
      return new ConfigService(native.configOpen());
    });
  }

  /** Open a config rooted at the given appdata directory. */
  static openIn(appdata: string): ConfigService {
    return withSolxError(() => {
      const native = loadNative() as unknown as NativeConfigModule;
      return new ConfigService(native.configOpen(appdata));
    });
  }

  /**
   * @internal Escape hatch for dependent managers' `open()` (e.g.
   * `ActionManager.open` needs this handle). Not part of the
   * public API — do not call from application code.
   */
  static handleOf(cfg: ConfigService): ConfigServiceHandle {
    return cfg.handle;
  }

  /** The resolved appdata directory (forward-slash). */
  get appdata(): string {
    const native = loadNative() as unknown as NativeConfigModule;
    return native.configAppdata(this.handle);
  }

  /** The resolved `solx-config.json` path (forward-slash). */
  get configPath(): string {
    const native = loadNative() as unknown as NativeConfigModule;
    return native.configConfigPath(this.handle);
  }

  /**
   * Raw `serde_json::Value` snapshot of the config file.
   * Use this when you want to see every key including ones
   * the typed `snapshot()` doesn't expose.
   */
  rawSnapshot(): JsonValue {
    const native = loadNative() as unknown as NativeConfigModule;
    return JSON.parse(native.configRawSnapshot(this.handle)) as JsonValue;
  }

  /** Typed `SolxConfig` snapshot. */
  snapshot(): SolxConfig {
    const native = loadNative() as unknown as NativeConfigModule;
    const raw = JSON.parse(native.configSnapshot(this.handle)) as SolxConfigSnake;
    return snapshotFromSnake(raw);
  }

  /**
   * Read a single key. Returns `undefined` if the key is
   * absent — this is the wire-format marker the Rust side
   * sends when the lookup misses.
   */
  get(key: string): JsonValue | undefined {
    return withSolxError(() => {
      const native = loadNative() as unknown as NativeConfigModule;
      const raw = native.configGet(this.handle, key);
      if (raw === undefined) return undefined;
      // `configGet` serializes the found Value as JSON
      // (so a string key returns `"\"value\""`) and emits
      // `undefined` for a miss.
      return JSON.parse(raw as string) as JsonValue;
    });
  }

  /**
   * Higher-level "checkout → edit → commit" pattern.
   *
   * Reads the raw snapshot, lets the caller mutate it,
   * and applies the diff via `patch`. Note: this is a
   * shallow merge — the caller is responsible for the
   * shape of the draft.
   *
   * Synchronous only (matches the underlying Rust impl).
   * Passing an async function throws a clear error rather
   * than silently dropping the promise.
   */
  mutate(
    fn: (draft: Record<string, JsonValue>) => void | Promise<void>,
  ): void {
    const draft = {
      ...(this.rawSnapshot() as Record<string, JsonValue>),
    };
    const ret = fn(draft);
    if (ret instanceof Promise) {
      throw new Error(
        'ConfigService.mutate received an async function; ' +
          'the config service is synchronous.',
      );
    }
    this.patch(draft as unknown);
  }

  /** Write a single key. The value is JSON-serialized on the way in. */
  set(key: string, value: unknown): void {
    withSolxError(() => {
      const native = loadNative() as unknown as NativeConfigModule;
      native.configSet(this.handle, key, JSON.stringify(value));
    });
  }

  /**
   * Shallow-merge a JSON object into the config. Keys
   * with `undefined` values are skipped (matches the Rust
   * `serde_json::Value::Object` merge semantics).
   */
  patch(patch: Record<string, unknown> | unknown): void {
    if (
      patch === null ||
      patch === undefined ||
      typeof patch !== 'object' ||
      Array.isArray(patch)
    ) {
      throw new Error(
        'ConfigService.patch expects a JSON object; got ' +
          JSON.stringify(patch),
      );
    }
    withSolxError(() => {
      const native = loadNative() as unknown as NativeConfigModule;
      native.configPatch(this.handle, JSON.stringify(patch));
    });
  }

  // ----- derived paths -----
  //
  // These are the canonical on-disk locations the other
  // managers (types/files/docs/actions) expect to find
  // their backing stores in. The umbrella `createSolx`
  // passes them down when wiring the other services.

  /** Database directory (contains the libsql files). */
  dbDir(): string {
    const native = loadNative() as unknown as NativeConfigModule;
    return native.configPath(this.handle, 'dbDir');
  }

  /** Absolute path to the docs database file. */
  docsDbPath(): string {
    const native = loadNative() as unknown as NativeConfigModule;
    return native.configPath(this.handle, 'docsDbPath');
  }

  /** Absolute path to the actions database file. */
  actionsDbPath(): string {
    const native = loadNative() as unknown as NativeConfigModule;
    return native.configPath(this.handle, 'actionsDbPath');
  }

  /** Absolute path to the types database file. */
  typesDbPath(): string {
    const native = loadNative() as unknown as NativeConfigModule;
    return native.configPath(this.handle, 'typesDbPath');
  }

  /** Filesystem root for the byte store. */
  filesDir(): string {
    const native = loadNative() as unknown as NativeConfigModule;
    return native.configPath(this.handle, 'filesDir');
  }

  /** Directory for search-index snapshots. */
  searchIndexDir(): string {
    const native = loadNative() as unknown as NativeConfigModule;
    return native.configPath(this.handle, 'searchIndexDir');
  }

  /** Directory for log files. */
  logsDir(): string {
    const native = loadNative() as unknown as NativeConfigModule;
    return native.configPath(this.handle, 'logsDir');
  }

  // ----- package registry -----

  /** List every registered package. */
  listPackages(): InstalledPackage[] {
    const native = loadNative() as unknown as NativeConfigModule;
    return JSON.parse(
      native.configListPackages(this.handle),
    ) as InstalledPackage[];
  }

  /** Register a package. Replaces an existing entry with the same `name`. */
  registerPackage(pkg: {
    name: string;
    version: string;
    path: string;
    installedAt: string;
  }): void {
    withSolxError(() => {
      const native = loadNative() as unknown as NativeConfigModule;
      native.configRegisterPackage(this.handle, JSON.stringify(pkg));
    });
  }

  /** Remove a package by name. Idempotent — no error if the name is unknown. */
  unregisterPackage(name: string): void {
    withSolxError(() => {
      const native = loadNative() as unknown as NativeConfigModule;
      native.configUnregisterPackage(this.handle, name);
    });
  }
}