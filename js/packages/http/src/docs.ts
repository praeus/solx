/**
 * Pure-fetch `DocManager` — HTTP proxy for `solx-server`, mirroring
 * `solx-client::RemoteDocManager` (solx-core/solx-client/src/docs.rs).
 * Same snake_case<->camelCase conversion pattern as `@solx/docs`'s
 * Neon-backed `native.ts`, but talks `fetch`/JSON directly.
 */
import type {
  DocLink,
  DocManager as DocManagerIface,
  Document,
  DocumentInput,
  FileRef,
  JsonValue,
  ListOptions,
  Page,
  SearchHit,
  SearchQuery,
  SearchResults,
} from '@solx/surface';
import { postJson } from './fetch.js';

interface FileRefSnake {
  name: string;
  rel_path: string;
  content_type?: string;
}

interface DocLinkSnake {
  kind: 'doc_ref' | 'url';
  target: string;
  field?: string;
  title?: string;
  description?: string;
}

interface DocumentSnake {
  id: string;
  path: string;
  name: string;
  title?: string;
  summary?: string;
  type_ref: string;
  contents: JsonValue;
  author?: string;
  pub_date?: string;
  confidence?: number;
  links: DocLinkSnake[];
  files: FileRefSnake[];
  created_at: string;
  updated_at: string;
}

interface DocumentInputSnake {
  title?: string;
  summary?: string;
  type_ref?: string;
  contents: JsonValue;
  author?: string;
  pub_date?: string;
  confidence?: number;
  links?: DocLinkSnake[];
  files?: FileRefSnake[];
}

interface SearchHitSnake {
  id: string;
  path: string;
  name: string;
  title?: string;
  summary?: string;
  type_ref: string;
  score: number;
}

interface SearchResultsSnake {
  hits: SearchHitSnake[];
  total: number;
  limit: number;
  offset: number;
}

interface PageSnake<T> {
  items: T[];
  total: number;
  limit: number;
  offset: number;
}

function fileRefFromSnake(s: FileRefSnake): FileRef {
  return { name: s.name, relPath: s.rel_path, contentType: s.content_type };
}

function fileRefToSnake(f: FileRef): FileRefSnake {
  return { name: f.name, rel_path: f.relPath, content_type: f.contentType };
}

function docLinkFromSnake(s: DocLinkSnake): DocLink {
  return { kind: s.kind, target: s.target, field: s.field, title: s.title, description: s.description };
}

function docLinkToSnake(l: DocLink): DocLinkSnake {
  return { kind: l.kind, target: l.target, field: l.field, title: l.title, description: l.description };
}

function documentFromSnake(s: DocumentSnake): Document {
  return {
    id: s.id,
    path: s.path,
    name: s.name,
    title: s.title,
    summary: s.summary,
    typeRef: s.type_ref,
    contents: s.contents,
    author: s.author,
    pubDate: s.pub_date,
    confidence: s.confidence,
    links: s.links.map(docLinkFromSnake),
    files: s.files.map(fileRefFromSnake),
    createdAt: s.created_at,
    updatedAt: s.updated_at,
  };
}

function documentInputToSnake(i: DocumentInput): DocumentInputSnake {
  const out: DocumentInputSnake = { contents: i.contents, type_ref: i.typeRef };
  if (i.title !== undefined) out.title = i.title;
  if (i.summary !== undefined) out.summary = i.summary;
  if (i.author !== undefined) out.author = i.author;
  if (i.pubDate !== undefined) out.pub_date = i.pubDate;
  if (i.confidence !== undefined) out.confidence = i.confidence;
  if (i.links !== undefined) out.links = i.links.map(docLinkToSnake);
  if (i.files !== undefined) out.files = i.files.map(fileRefToSnake);
  return out;
}

function searchHitFromSnake(s: SearchHitSnake): SearchHit {
  return { id: s.id, path: s.path, name: s.name, title: s.title, summary: s.summary, typeRef: s.type_ref, score: s.score };
}

function searchResultsFromSnake(s: SearchResultsSnake): SearchResults {
  return { hits: s.hits.map(searchHitFromSnake), total: s.total, limit: s.limit, offset: s.offset };
}

function searchQueryToSnake(query: SearchQuery): Record<string, JsonValue> {
  const out: Record<string, JsonValue> = {};
  if (query.q !== undefined) out['q'] = query.q;
  if (query.pathPrefix !== undefined) out['path_prefix'] = query.pathPrefix;
  if (query.typeRef !== undefined) out['type_ref'] = query.typeRef;
  if (query.linkedTo !== undefined) out['linked_to'] = query.linkedTo;
  if (query.limit !== undefined) out['limit'] = query.limit;
  if (query.offset !== undefined) out['offset'] = query.offset;
  return out;
}

function pageFromSnake<T, S>(s: PageSnake<S>, fromSnake: (x: S) => T): Page<T> {
  return { items: s.items.map(fromSnake), total: s.total, limit: s.limit, offset: s.offset };
}

function listOptionsToSnake(opts?: ListOptions): Record<string, JsonValue> {
  if (!opts) return {};
  const out: Record<string, JsonValue> = {};
  if (opts.pathPrefix !== undefined) out['path_prefix'] = opts.pathPrefix;
  if (opts.limit !== undefined) out['limit'] = opts.limit;
  if (opts.offset !== undefined) out['offset'] = opts.offset;
  if (opts.filterField !== undefined) out['filter_field'] = opts.filterField;
  if (opts.filterValue !== undefined) out['filter_value'] = opts.filterValue;
  if (opts.sortBy !== undefined) out['sort_by'] = opts.sortBy;
  if (opts.sortOrder !== undefined) out['sort_order'] = opts.sortOrder;
  if (opts.dateAfter !== undefined) out['date_after'] = opts.dateAfter;
  if (opts.dateBefore !== undefined) out['date_before'] = opts.dateBefore;
  return out;
}

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
    const s = await postJson<unknown, DocumentSnake>(this.baseUrl, this.token, '/docs/save', {
      path,
      name,
      input: documentInputToSnake(input),
    });
    return documentFromSnake(s);
  }

  async get(path: string, name: string): Promise<Document> {
    const s = await postJson<unknown, DocumentSnake>(this.baseUrl, this.token, '/docs/get', {
      path,
      name,
    });
    return documentFromSnake(s);
  }

  async delete(path: string, name: string): Promise<void> {
    await postJson<unknown, unknown>(this.baseUrl, this.token, '/docs/delete', { path, name });
  }

  async list(opts?: ListOptions): Promise<Page<Document>> {
    const s = await postJson<unknown, PageSnake<DocumentSnake>>(
      this.baseUrl,
      this.token,
      '/docs/list',
      listOptionsToSnake(opts),
    );
    return pageFromSnake(s, documentFromSnake);
  }

  async search(query: SearchQuery): Promise<SearchResults> {
    const s = await postJson<unknown, SearchResultsSnake>(
      this.baseUrl,
      this.token,
      '/docs/search',
      searchQueryToSnake(query),
    );
    return searchResultsFromSnake(s);
  }
}
