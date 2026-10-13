/**
 * Native binary interface for the `solx-bindings` files module.
 *
 * Loaded via `../loader.ts` from the per-platform `solx.node`
 * binary. All byte payloads cross the wire as base64 strings
 * inside the JSON envelope; the TS side (`native.ts`) encodes
 * and decodes them so consumers get a real `Uint8Array`.
 */

export interface NativeFilesModule {
  filesNew(root: string): unknown;
  filesConnect(serverUrl: string, token: string): unknown;
  filesPut(mgr: unknown, relPath: string, b64: string): Promise<string>;
  filesGet(mgr: unknown, relPath: string): Promise<string>;
  filesDelete(mgr: unknown, relPath: string): Promise<void>;
  filesList(mgr: unknown, prefix: string): Promise<string>;
}

export type FileStoreHandle = unknown;
