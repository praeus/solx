/**
 * Manager trait mirrors for the solx SDK.
 *
 * Each manager is exposed as a TypeScript interface; the concrete
 * implementation (e.g. `LocalTypeManager`) is provided by the
 * corresponding `@solx/<crate>` package and satisfies the interface.
 *
 * Method shapes mirror `solx-surface/src/managers.rs` 1:1, with
 * `Result<T, SolxError>` collapsed to `Promise<T>` (rejection = SolxError).
 */

import type {
  Action,
  ActionExecResult,
  ActionInput,
  Document,
  DocumentInput,
  TypeEntity,
  TypeInput,
} from './entities.js';
import type { ListOptions, Page, SearchQuery, SearchResults } from './query.js';
import type { JsonValue } from './json.js';

// ---------- TypeManager ----------

export interface TypeManager {
  save(path: string, name: string, input: TypeInput): Promise<TypeEntity>;
  get(path: string, name: string): Promise<TypeEntity>;
  delete(path: string, name: string): Promise<void>;
  list(opts?: ListOptions): Promise<Page<TypeEntity>>;
  /** Resolve a `/path/Name` ref to the full {@link TypeEntity}. */
  resolve(typeRef: string): Promise<TypeEntity>;
  /** Validate a value against the schema of the given type ref. */
  validate(value: JsonValue, typeRef: string): Promise<void>;
}

// ---------- FileStore ----------

export interface FileStore {
  /** Returns the normalized relative path. */
  put(relPath: string, bytes: Uint8Array): Promise<string>;
  get(relPath: string): Promise<Uint8Array>;
  delete(relPath: string): Promise<void>;
  list(prefix: string): Promise<string[]>;
}

// ---------- DocManager ----------

export interface DocManager {
  save(path: string, name: string, input: DocumentInput): Promise<Document>;
  get(path: string, name: string): Promise<Document>;
  delete(path: string, name: string): Promise<void>;
  list(opts?: ListOptions): Promise<Page<Document>>;
  search(query: SearchQuery): Promise<SearchResults>;
}

// ---------- ActionManager ----------

export interface ActionManager {
  save(path: string, name: string, input: ActionInput): Promise<Action>;
  get(path: string, name: string): Promise<Action>;
  delete(path: string, name: string): Promise<void>;
  list(opts?: ListOptions): Promise<Page<Action>>;
  exec(path: string, name: string, params: JsonValue): Promise<ActionExecResult>;
}

// ---------- ConfigService ----------

/**
 * Config service. All methods are **synchronous** in the Rust impl
 * (the in-process config is held in memory and synchronised via an
 * OS advisory lock on write). In TS this is expressed as direct calls
 * (no `Promise`).
 */
export interface ConfigService {
  readonly appdata: string;
  readonly configPath: string;
  /** Read the raw `serde_json::Value` of the config file. */
  rawSnapshot(): JsonValue;
  /** Read a typed snapshot. */
  snapshot(): SolxConfig;
  /** Get a single key. */
  get(key: string): JsonValue | undefined;
  /** Mutate the config under the file lock. */
  mutate(fn: (draft: Record<string, JsonValue>) => void | Promise<void>): void;
  /** Set a single key. */
  set(key: string, value: JsonValue): void;
  /** Shallow-merge a JSON object into the config. */
  patch(patch: JsonValue): void;
  // Package registry
  listPackages(): InstalledPackage[];
  registerPackage(pkg: InstalledPackage): void;
  unregisterPackage(name: string): void;
}

export interface SolxConfig {
  dataDirectory?: string;
  filesDirectory?: string;
  modelsDirectory?: string;
  searchIndexDir?: string;
  docsDb?: string;
  actionsDb?: string;
  typesDb?: string;
  installedPackages: InstalledPackage[];
  /**
   * Base URL of a remote `solx-server` to proxy manager calls to (e.g.
   * `"http://127.0.0.1:8766"`). Absent/undefined means local mode.
   */
  serverUrl?: string;
  /** Shared bearer token for `serverUrl`. */
  serverToken?: string;
  /** Port `solx-server` binds on `127.0.0.1`. Server-side only. */
  serverPort?: number;
}

export interface InstalledPackage {
  name: string;
  version: string;
  path: string;
  installedAt: string;
}

// ---------- ScriptRunner ----------

/**
 * The solx shell pipeline language (`;`-separated statements, `|`
 * pipeline stages, `$name = <pipeline>` capture), scoped to action
 * execution. There is no `ScriptRunner` trait in solx-core — the
 * underlying seam is `solx_scripts::CommandRunner`, and the only
 * implementation solx-js binds to supports exactly two stage verbs:
 * `exec <path/name> [--json '<params>']` (dispatches to an
 * `ActionManager`) and `json <literal>`. This mirrors
 * `solx-actions/src/script.rs`'s intentionally narrow grammar — a
 * script cannot bypass the `Internal` dispatch guard by reaching
 * docs/types/files directly, only through `exec`.
 *
 * `params`, if given, is bound to `$params` for the script to
 * reference; the pipeline's final stage result is returned as-is
 * (solx scripts operate on JSON values, not text streams).
 */
export interface ScriptRunner {
  run(source: string, params?: JsonValue): Promise<JsonValue>;
}
