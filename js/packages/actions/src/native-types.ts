/**
 * Native binary interface for the `solx-bindings` actions module.
 *
 * Loaded via `@solx/types/loader`. Structured payloads
 * (`ActionInput`, `ListOptions`, `params`, `ActionExecResult`) cross
 * the wire as JSON strings.
 */

export interface NativeActionsModule {
  /**
   * Local mode: opens (or creates) an actions database. Mirrors
   * `solx-manager::App::wire_local`'s actions step — needs the
   * config service (env-mapping lookups) plus the three
   * collaborator managers (types/docs/files).
   */
  actionsOpen(
    dbPath: string,
    configHandle: unknown,
    typesHandle: unknown,
    docsHandle: unknown,
    filesHandle: unknown,
  ): unknown;
  /** Client mode: proxies every call to a remote `solx-server`. */
  actionsConnect(serverUrl: string, token: string): unknown;
  actionsSave(mgr: unknown, path: string, name: string, inputJson: string): Promise<string>;
  actionsGet(mgr: unknown, path: string, name: string): Promise<string>;
  actionsDelete(mgr: unknown, path: string, name: string): Promise<void>;
  actionsList(mgr: unknown, optionsJson: string): Promise<string>;
  actionsExec(mgr: unknown, path: string, name: string, paramsJson: string): Promise<string>;
}

export type ActionManagerHandle = unknown;
