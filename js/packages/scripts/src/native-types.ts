/**
 * Native binary interface for the `solx-bindings` scripts module.
 *
 * Loaded via `@solx/types/loader`. Unlike the other manager
 * modules, scripts have no `open`/`connect` — a script runs
 * against an already-open `ActionManager` handle, local or
 * remote, transparently.
 */

export interface NativeScriptsModule {
  scriptsRun(actionsHandle: unknown, source: string, paramsJson?: string): Promise<string>;
}
