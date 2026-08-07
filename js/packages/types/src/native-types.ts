/**
 * Native binary interface. Loaded via `./loader.ts` from one of:
 *   - `process.env.SOLX_NATIVE_BIN` (dev mode: a `cargo build` output)
 *   - `@solx/<napi-triple>` (published platform package)
 */

export interface NativeTypesModule {
  typesOpen(dbPath: string): unknown;
  typesConnect(serverUrl: string, token: string): unknown;
  typesPost(mgr: unknown, path: string, name: string, inputJson: string): Promise<string>;
  typesGet(mgr: unknown, path: string, name: string): Promise<string>;
  typesDelete(mgr: unknown, path: string, name: string): Promise<void>;
  typesList(mgr: unknown, optionsJson: string): Promise<string>;
  typesResolve(mgr: unknown, typeRef: string): Promise<string>;
  typesValidate(mgr: unknown, valueJson: string, typeRef: string): Promise<void>;
}

export type TypeManagerHandle = unknown;
