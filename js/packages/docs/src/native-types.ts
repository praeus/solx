/**
 * Native binary interface for the `solx-bindings` docs module.
 *
 * Loaded via `@solx/types/loader` from the per-platform `solx.node`
 * binary. Structured payloads (`DocumentInput`, `ListOptions`,
 * `SearchQuery`, results) cross the wire as JSON strings — the TS
 * side (`native.ts`) parses/serializes and converts snake_case to
 * camelCase.
 */

export interface NativeDocsModule {
  /** Local mode: opens (or creates) a docs database + Tantivy index. */
  docsOpen(dbPath: string, indexDir: string, typesHandle: unknown): unknown;
  /** Client mode: proxies every call to a remote `solx-server`. */
  docsConnect(serverUrl: string, token: string): unknown;
  docsPost(mgr: unknown, path: string, name: string, inputJson: string): Promise<string>;
  docsGet(mgr: unknown, path: string, name: string): Promise<string>;
  docsDelete(mgr: unknown, path: string, name: string): Promise<void>;
  docsList(mgr: unknown, optionsJson: string): Promise<string>;
  docsSearch(mgr: unknown, queryJson: string): Promise<string>;
}

export type DocManagerHandle = unknown;
