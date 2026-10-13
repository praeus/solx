/**
 * IPC `DocManager` — `plugin:solx|docs_*`, mirroring `@solx/http`'s
 * `HttpDocManager` method-for-method.
 */
import type {
  DocManager as DocManagerIface,
  Document,
  DocumentInput,
  EntityOptions,
  ListOptions,
  Page,
  PathFacet,
  SearchQuery,
} from '@solx/surface';
import type { TauriTransport } from './invoke.js';

export class TauriDocManager implements DocManagerIface {
  constructor(private readonly t: TauriTransport) {}

  save(path: string, name: string, input: DocumentInput, opts?: EntityOptions): Promise<Document> {
    return this.t.call('docs_save', { path, name, input, excludeMode: opts?.excludeMode });
  }

  get(path: string, name: string, opts?: EntityOptions): Promise<Document> {
    return this.t.call('docs_get', { path, name, excludeMode: opts?.excludeMode });
  }

  async delete(path: string, name: string, opts?: EntityOptions): Promise<void> {
    await this.t.call('docs_delete', { path, name, excludeMode: opts?.excludeMode });
  }

  list(opts?: ListOptions): Promise<Page<Document>> {
    return this.t.call('docs_list', { opts });
  }

  paths(opts?: ListOptions): Promise<Page<PathFacet>> {
    return this.t.call('docs_paths', { opts });
  }

  search(query: SearchQuery): Promise<Page<Document>> {
    return this.t.call('docs_search', { query });
  }
}
