/**
 * TS-side wrapper around the `solx-bindings` docs module.
 *
 * Same shape as `@solx/types`'s `native.ts`: loads the `.node`
 * binary and rethrows `SolxError`-prefixed errors as real
 * {@link SolxError} values. The Neon boundary marshals the same
 * `solx_surface::entities`/`query` structs as `solx-server`'s HTTP
 * responses, so the JSON crossing it is camelCase already — no DTO
 * conversion needed on this side.
 *
 * The public class is just `DocManager` — not `LocalDocManager` —
 * because "local" is an implementation detail. `open()` (local) and
 * `connect()` (client) both hand back the same class; the choice of
 * backend lives in which constructor you call, not in the class
 * itself. See {@link ScriptRunner}/`ActionManager` for the same
 * pattern.
 */

import { SolxError } from '@solx/surface';
import type { Document, DocumentInput, ListOptions, Page, SearchQuery, SearchResults } from '@solx/surface';
import { TypeManager } from '@solx/types';
import { loadNative } from '@solx/types/loader';
import type { DocManagerHandle, NativeDocsModule } from './native-types.js';

export type { NativeDocsModule, DocManagerHandle };

async function withSolxError<T>(fn: () => Promise<T>): Promise<T> {
  try {
    return await fn();
  } catch (e) {
    throw SolxError.fromMessage(e);
  }
}

// ----- DocManager -----

export class DocManager {
  private constructor(private readonly handle: DocManagerHandle) {}

  /**
   * Open a local document store. `indexDir` is the Tantivy search
   * index directory (conventionally `<searchIndexDir>/docs`).
   * Documents are validated against `types` at write time.
   */
  static async open(dbPath: string, indexDir: string, types: TypeManager): Promise<DocManager> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeDocsModule;
      const handle = native.docsOpen(dbPath, indexDir, TypeManager.handleOf(types));
      return new DocManager(handle);
    });
  }

  /** Connect to a remote `solx-server` instead of opening a local database. */
  static async connect(serverUrl: string, token: string): Promise<DocManager> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeDocsModule;
      const handle = native.docsConnect(serverUrl, token);
      return new DocManager(handle);
    });
  }

  async save(path: string, name: string, input: DocumentInput): Promise<Document> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeDocsModule;
      const json = await native.docsSave(this.handle, path, name, JSON.stringify(input));
      return JSON.parse(json) as Document;
    });
  }

  async get(path: string, name: string): Promise<Document> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeDocsModule;
      const json = await native.docsGet(this.handle, path, name);
      return JSON.parse(json) as Document;
    });
  }

  async delete(path: string, name: string): Promise<void> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeDocsModule;
      await native.docsDelete(this.handle, path, name);
    });
  }

  async list(opts?: ListOptions): Promise<Page<Document>> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeDocsModule;
      const json = await native.docsList(this.handle, JSON.stringify(opts ?? {}));
      return JSON.parse(json) as Page<Document>;
    });
  }

  async search(query: SearchQuery): Promise<SearchResults> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeDocsModule;
      const json = await native.docsSearch(this.handle, JSON.stringify(query));
      return JSON.parse(json) as SearchResults;
    });
  }

  /**
   * @internal Escape hatch for dependent managers' `open()`. Not
   * part of the public API — do not call from application code.
   */
  static handleOf(mgr: DocManager): DocManagerHandle {
    return mgr.handle;
  }
}
