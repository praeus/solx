/**
 * Native binary interface for the `solx-bindings` config module.
 *
 * Loaded via `../loader.ts` from the per-platform `solx.node`
 * binary. The Rust side serializes all structured returns
 * (`serde_json::Value`, `SolxConfig`, `Vec<InstalledPackage>`)
 * as JSON strings — the TS side parses them. Inputs that are
 * `serde_json::Value` (`set`, `patch`, `registerPackage`)
 * are serialized with `JSON.stringify` on the way in.
 *
 * Every method is **synchronous** on the Rust side; the
 * `ConfigService` holds the config in memory and uses an
 * OS-level advisory lock only for `set` / `patch` /
 * `registerPackage` / `unregisterPackage`. We still wrap
 * calls in `withSolxError` so consumers get a real
 * `SolxError` instead of a raw string-prefixed `Error`.
 */

export interface NativeConfigModule {
  /**
   * Open a config rooted at the platform-default appdata dir
   * (when called with no argument) or at the given appdata
   * directory (when called with a string).
   */
  configOpen(appdata?: string): unknown;
  /** The resolved appdata directory (forward-slash). */
  configAppdata(mgr: unknown): string;
  /** The resolved config.json path (forward-slash). */
  configConfigPath(mgr: unknown): string;
  /** Raw `serde_json::Value` snapshot (JSON string). */
  configRawSnapshot(mgr: unknown): string;
  /** Typed `SolxConfig` snapshot (JSON string). */
  configSnapshot(mgr: unknown): string;
  /** Read a single key. Returns JSON-encoded `Value` or `undefined`. */
  configGet(mgr: unknown, key: string): unknown;
  /** Write a single key (value is JSON-serialized). */
  configSet(mgr: unknown, key: string, valueJson: string): void;
  /** Shallow-merge a JSON object into the config. */
  configPatch(mgr: unknown, patchJson: string): void;
  /**
   * Resolve a derived on-disk path. `which` is one of:
   *   `dbDir`, `docsDbPath`, `actionsDbPath`, `typesDbPath`,
   *   `filesDir`, `searchIndexDir`, `logsDir`.
   */
  configPath(mgr: unknown, which: string): string;
  /** List installed packages (JSON-encoded array). */
  configListPackages(mgr: unknown): string;
  /** Register a package (pkg is JSON-serialized). */
  configRegisterPackage(mgr: unknown, pkgJson: string): void;
  /** Unregister a package by name. */
  configUnregisterPackage(mgr: unknown, name: string): void;
}

export type ConfigServiceHandle = unknown;