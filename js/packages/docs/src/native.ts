/**
 * TS-side wrapper around the `solx-bindings` docs module.
 *
 * Same shape as `@solx/types`'s `native.ts`: loads the `.node`
 * binary, converts snake_case DTOs to camelCase, and rethrows
 * `SolxError`-prefixed errors as real {@link SolxError} values.
 *
 * The public class is just `DocManager` — not `LocalDocManager` —
 * because "local" is an implementation detail. `open()` (local) and
 * `connect()` (client) both hand back the same class; the choice of
 * backend lives in which constructor you call, not in the class
 * itself. See {@link ScriptRunner}/`ActionManager` for the same
 * pattern.
 */

import { SolxError } from '@solx/surface';
import type { JsonValue } from '@solx/surface';
import type {
  Document,
  DocumentInput,
  DocLink,
  FileRef,
  ListOptions,
  Page,
  SearchHit,
  SearchQuery,
  SearchResults,
} from '@solx/surface';
import { TypeManager } from '@solx/types';
import { loadNative } from '@solx/types/loader';
import type { DocManagerHandle, NativeDocsModule } from './native-types.js';

export type { NativeDocsModule, DocManagerHandle };

// ----- snake_case DTO mirror (Rust serde shape) -----

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
  return {
    kind: s.kind,
    target: s.target,
    field: s.field,
    title: s.title,
    description: s.description,
  };
}

function docLinkToSnake(l: DocLink): DocLinkSnake {
  return {
    kind: l.kind,
    target: l.target,
    field: l.field,
    title: l.title,
    description: l.description,
  };
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
  return {
    id: s.id,
    path: s.path,
    name: s.name,
    title: s.title,
    summary: s.summary,
    typeRef: s.type_ref,
    score: s.score,
  };
}

function searchResultsFromSnake(s: SearchResultsSnake): SearchResults {
  return {
    hits: s.hits.map(searchHitFromSnake),
    total: s.total,
    limit: s.limit,
    offset: s.offset,
  };
}

function searchQueryToJson(query: SearchQuery): string {
  const out: Record<string, JsonValue> = {};
  if (query.q !== undefined) out['q'] = query.q;
  if (query.pathPrefix !== undefined) out['path_prefix'] = query.pathPrefix;
  if (query.typeRef !== undefined) out['type_ref'] = query.typeRef;
  if (query.linkedTo !== undefined) out['linked_to'] = query.linkedTo;
  if (query.limit !== undefined) out['limit'] = query.limit;
  if (query.offset !== undefined) out['offset'] = query.offset;
  return JSON.stringify(out);
}

function pageFromSnake<T, S>(s: PageSnake<S>, fromSnake: (x: S) => T): Page<T> {
  return { items: s.items.map(fromSnake), total: s.total, limit: s.limit, offset: s.offset };
}

function listOptionsToJson(opts?: ListOptions): string {
  if (!opts) return '{}';
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
  return JSON.stringify(out);
}

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

  async post(path: string, name: string, input: DocumentInput): Promise<Document> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeDocsModule;
      const json = await native.docsPost(
        this.handle,
        path,
        name,
        JSON.stringify(documentInputToSnake(input)),
      );
      return documentFromSnake(JSON.parse(json) as DocumentSnake);
    });
  }

  async get(path: string, name: string): Promise<Document> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeDocsModule;
      const json = await native.docsGet(this.handle, path, name);
      return documentFromSnake(JSON.parse(json) as DocumentSnake);
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
      const json = await native.docsList(this.handle, listOptionsToJson(opts));
      return pageFromSnake(JSON.parse(json) as PageSnake<DocumentSnake>, documentFromSnake);
    });
  }

  async search(query: SearchQuery): Promise<SearchResults> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeDocsModule;
      const json = await native.docsSearch(this.handle, searchQueryToJson(query));
      return searchResultsFromSnake(JSON.parse(json) as SearchResultsSnake);
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
