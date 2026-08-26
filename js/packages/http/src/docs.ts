/**
 * Pure-fetch `DocManager` — HTTP proxy for `solx-server`, mirroring
 * `solx-client::RemoteDocManager` (solx-core/solx-client/src/docs.rs). The
 * wire format is camelCase end to end (same as `@solx/surface`'s public
 * types), so request/response bodies pass straight through with no
 * conversion layer.
 */
import type {
  DocManager as DocManagerIface,
  Document,
  DocumentInput,
  ListOptions,
  Page,
  SearchQuery,
  SearchResults,
} from '@solx/surface';
import { entityPath, requestJson, toQuery } from './fetch.js';

/**
 * HTTP-proxy `DocManager` talking to a `solx-server`. Public name matches
 * `@solx/docs`'s `DocManager` and the `solx_surface::managers::DocManager`
 * trait.
 */
export class HttpDocManager implements DocManagerIface {
  constructor(
    private readonly baseUrl: string,
    private readonly token: string,
  ) {}

  async save(path: string, name: string, input: DocumentInput): Promise<Document> {
    return requestJson<Document>(this.baseUrl, this.token, 'PUT', entityPath('docs', path, name), {
      body: input,
    });
  }

  async get(path: string, name: string): Promise<Document> {
    return requestJson<Document>(this.baseUrl, this.token, 'GET', entityPath('docs', path, name));
  }

  async delete(path: string, name: string): Promise<void> {
    await requestJson<void>(this.baseUrl, this.token, 'DELETE', entityPath('docs', path, name));
  }

  async list(opts?: ListOptions): Promise<Page<Document>> {
    return requestJson<Page<Document>>(this.baseUrl, this.token, 'GET', '/docs', {
      query: toQuery(opts),
    });
  }

  /**
   * `GET /search` — a top-level route rather than `/docs/search`, which as a
   * static sibling of the `/docs/{*ref}` catch-all would shadow any document
   * named `search` at the root.
   */
  async search(query: SearchQuery): Promise<SearchResults> {
    return requestJson<SearchResults>(this.baseUrl, this.token, 'GET', '/search', {
      query: toQuery(query),
    });
  }
}
