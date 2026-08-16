/**
 * wire.ts — the single snake_case ⇄ camelCase translation boundary.
 *
 * Three layers disagree about field casing:
 *
 *   Rust core / HTTP wire   snake_case   (`type_ref`, `path_prefix`)
 *   solx-js `@solx/surface` camelCase    (`typeRef`, `pathPrefix`)
 *   this HTTP API + React   snake_case   (matches Rust)
 *
 * solx-js already converts camelCase → the Rust snake_case wire internally
 * (see `listOptionsToJson` / `documentInputToSnake` in each package's
 * `native.ts`). Handing it snake_case directly therefore drops every
 * multi-word field **silently**: `listOptionsToJson` reads `opts.pathPrefix`,
 * finds `undefined`, and emits nothing. Only the fields whose two spellings
 * coincide — `q`, `limit`, `offset` — ever survived.
 *
 * Every SDK call is funnelled through the mappers below so the translation
 * happens exactly once in each direction, in one place.
 *
 * ── Opaque payloads ──────────────────────────────────────────────────────
 * `contents`, `schema`, `action_config` and exec `result` are arbitrary user
 * JSON and are passed through **verbatim**. A blanket recursive key rewrite
 * would corrupt user data — a document field genuinely named `my_field`, or
 * the property names inside a JSON Schema's `properties` map.
 */

/** Non-empty string, or undefined. Keeps `JSON.stringify` from emitting nulls. */
function str(value: unknown): string | undefined {
  if (value === undefined || value === null) return undefined;
  const s = String(value);
  return s === "" ? undefined : s;
}

/** Finite number, or undefined. */
function num(value: unknown): number | undefined {
  if (value === undefined || value === null || value === "") return undefined;
  const n = Number(value);
  return Number.isFinite(n) ? n : undefined;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function asArray(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}

/* ── Inbound: query params → SDK options ─────────────────────────────── */

export interface ListOptionsCamel {
  pathPrefix?: string;
  limit?: number;
  offset?: number;
  filterField?: string;
  filterValue?: string;
  sortBy?: string;
  sortOrder?: "asc" | "desc";
  dateAfter?: string;
  dateBefore?: string;
}

/**
 * Parse the common entity-list query params into a camelCase `ListOptions`
 * the SDK will actually read.
 *
 * Note: only `solx-docs` implements `sort_by`/`sort_order`/`filter_field` on
 * the Rust side. `solx-actions` and `solx-types` accept the options but
 * hardcode `ORDER BY path,name` and ignore filters — passing them is
 * harmless, but callers should not present those controls for those tabs.
 */
export function listOptionsFromUrl(url: URL): ListOptionsCamel {
  const p = url.searchParams;
  const opts: ListOptionsCamel = {};

  const pathPrefix = str(p.get("path_prefix"));
  if (pathPrefix) opts.pathPrefix = pathPrefix;

  const limit = num(p.get("limit"));
  if (limit !== undefined) opts.limit = limit;

  const offset = num(p.get("offset"));
  if (offset !== undefined) opts.offset = offset;

  const filterField = str(p.get("filter_field"));
  if (filterField) opts.filterField = filterField;

  const filterValue = str(p.get("filter_value"));
  if (filterValue) opts.filterValue = filterValue;

  const sortBy = str(p.get("sort_by"));
  if (sortBy) opts.sortBy = sortBy;

  const sortOrder = str(p.get("sort_order"))?.toLowerCase();
  if (sortOrder === "asc" || sortOrder === "desc") opts.sortOrder = sortOrder;

  const dateAfter = str(p.get("date_after"));
  if (dateAfter) opts.dateAfter = dateAfter;

  const dateBefore = str(p.get("date_before"));
  if (dateBefore) opts.dateBefore = dateBefore;

  return opts;
}

export interface SearchQueryCamel {
  q?: string;
  pathPrefix?: string;
  typeRef?: string;
  linkedTo?: string;
  limit?: number;
  offset?: number;
}

/** Parse the document-search query params into a camelCase `SearchQuery`. */
export function searchQueryFromUrl(url: URL): SearchQueryCamel {
  const p = url.searchParams;
  const query: SearchQueryCamel = {};

  const q = str(p.get("q"));
  if (q) query.q = q;

  const pathPrefix = str(p.get("path_prefix"));
  if (pathPrefix) query.pathPrefix = pathPrefix;

  const typeRef = str(p.get("type_ref"));
  if (typeRef) query.typeRef = typeRef;

  const linkedTo = str(p.get("linked_to"));
  if (linkedTo) query.linkedTo = linkedTo;

  const limit = num(p.get("limit"));
  if (limit !== undefined) query.limit = limit;

  const offset = num(p.get("offset"));
  if (offset !== undefined) query.offset = offset;

  return query;
}

/* ── Shared value objects ────────────────────────────────────────────── */

function fileRefToWire(raw: unknown): Record<string, unknown> {
  const f = asRecord(raw);
  return {
    name: str(f.name),
    rel_path: str(f.relPath),
    content_type: str(f.contentType),
  };
}

function fileRefFromWire(raw: unknown): Record<string, unknown> {
  const f = asRecord(raw);
  return {
    name: str(f.name),
    relPath: str(f.rel_path),
    contentType: str(f.content_type),
  };
}

/** `DocLink` uses single-word keys throughout — copied, not renamed. */
function docLinkToWire(raw: unknown): Record<string, unknown> {
  const l = asRecord(raw);
  return {
    kind: str(l.kind),
    target: str(l.target),
    field: str(l.field),
    title: str(l.title),
    description: str(l.description),
  };
}

const docLinkFromWire = docLinkToWire;

/* ── Outbound: SDK entities → HTTP JSON ──────────────────────────────── */

export function documentToWire(raw: unknown): Record<string, unknown> {
  const d = asRecord(raw);
  return {
    id: str(d.id),
    path: str(d.path),
    name: str(d.name),
    title: str(d.title),
    summary: str(d.summary),
    type_ref: str(d.typeRef),
    contents: d.contents, // opaque — user JSON
    author: str(d.author),
    pub_date: str(d.pubDate),
    confidence: num(d.confidence),
    links: asArray(d.links).map(docLinkToWire),
    files: asArray(d.files).map(fileRefToWire),
    created_at: str(d.createdAt),
    updated_at: str(d.updatedAt),
  };
}

export function typeToWire(raw: unknown): Record<string, unknown> {
  const t = asRecord(raw);
  return {
    id: str(t.id),
    path: str(t.path),
    name: str(t.name),
    description: str(t.description),
    schema: t.schema, // opaque — JSON Schema, user-defined property names
    groups: asArray(t.groups).map(String),
    created_at: str(t.createdAt),
    updated_at: str(t.updatedAt),
  };
}

export function actionToWire(raw: unknown): Record<string, unknown> {
  const a = asRecord(raw);
  return {
    id: str(a.id),
    path: str(a.path),
    name: str(a.name),
    caption: str(a.caption),
    description: str(a.description),
    capabilities: asArray(a.capabilities).map(String),
    phrases: asArray(a.phrases).map(String),
    category: str(a.category),
    param_type_ref: str(a.paramTypeRef),
    result_type_ref: str(a.resultTypeRef),
    action_type: str(a.actionType),
    fn_name: str(a.fnName),
    bin_name: str(a.binName),
    action_config: a.actionConfig, // opaque — free-form config blob
    files: asArray(a.files).map(fileRefToWire),
    trusted: Boolean(a.trusted),
    created_at: str(a.createdAt),
    updated_at: str(a.updatedAt),
  };
}

export function searchHitToWire(raw: unknown): Record<string, unknown> {
  const h = asRecord(raw);
  return {
    id: str(h.id),
    path: str(h.path),
    name: str(h.name),
    title: str(h.title),
    summary: str(h.summary),
    type_ref: str(h.typeRef),
    score: num(h.score) ?? 0,
  };
}

export function actionExecResultToWire(raw: unknown): Record<string, unknown> {
  const r = asRecord(raw);
  return {
    action: str(r.action),
    result: r.result, // opaque — whatever the action returned
    success: Boolean(r.success),
    message: str(r.message),
  };
}

/** Map a `Page<T>` envelope, converting each item with `mapItem`. */
export function pageToWire(
  raw: unknown,
  mapItem: (item: unknown) => Record<string, unknown>,
): Record<string, unknown> {
  const p = asRecord(raw);
  return {
    items: asArray(p.items).map(mapItem),
    total: num(p.total) ?? 0,
    limit: num(p.limit) ?? 0,
    offset: num(p.offset) ?? 0,
  };
}

/** Map a `SearchResults` envelope. */
export function searchResultsToWire(raw: unknown): Record<string, unknown> {
  const r = asRecord(raw);
  return {
    hits: asArray(r.hits).map(searchHitToWire),
    total: num(r.total) ?? 0,
    limit: num(r.limit) ?? 0,
    offset: num(r.offset) ?? 0,
  };
}

/* ── Inbound: HTTP JSON → SDK inputs ─────────────────────────────────── */

export function documentInputFromWire(raw: unknown): Record<string, unknown> {
  const i = asRecord(raw);
  const out: Record<string, unknown> = {};

  if (i.title !== undefined) out.title = i.title;
  if (i.summary !== undefined) out.summary = i.summary;
  if (i.type_ref !== undefined) out.typeRef = i.type_ref;
  if (i.contents !== undefined) out.contents = i.contents; // opaque
  if (i.author !== undefined) out.author = i.author;
  if (i.pub_date !== undefined) out.pubDate = i.pub_date;
  if (i.confidence !== undefined) out.confidence = i.confidence;
  if (i.links !== undefined) out.links = asArray(i.links).map(docLinkFromWire);
  if (i.files !== undefined) out.files = asArray(i.files).map(fileRefFromWire);

  return out;
}

export function typeInputFromWire(raw: unknown): Record<string, unknown> {
  const i = asRecord(raw);
  const out: Record<string, unknown> = {};

  if (i.description !== undefined) out.description = i.description;
  if (i.schema !== undefined) out.schema = i.schema; // opaque
  if (i.groups !== undefined) out.groups = asArray(i.groups).map(String);

  return out;
}

export function actionInputFromWire(raw: unknown): Record<string, unknown> {
  const i = asRecord(raw);
  const out: Record<string, unknown> = {};

  if (i.caption !== undefined) out.caption = i.caption;
  if (i.description !== undefined) out.description = i.description;
  if (i.capabilities !== undefined) out.capabilities = asArray(i.capabilities).map(String);
  if (i.phrases !== undefined) out.phrases = asArray(i.phrases).map(String);
  if (i.category !== undefined) out.category = i.category;
  if (i.param_type_ref !== undefined) out.paramTypeRef = i.param_type_ref;
  if (i.result_type_ref !== undefined) out.resultTypeRef = i.result_type_ref;
  if (i.action_type !== undefined) out.actionType = i.action_type;
  if (i.fn_name !== undefined) out.fnName = i.fn_name;
  if (i.bin_name !== undefined) out.binName = i.bin_name;
  if (i.action_config !== undefined) out.actionConfig = i.action_config; // opaque
  if (i.files !== undefined) out.files = asArray(i.files).map(fileRefFromWire);
  if (i.trusted !== undefined) out.trusted = Boolean(i.trusted);

  return out;
}
