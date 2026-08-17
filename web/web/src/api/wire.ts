/**
 * wire.ts — the snake_case (this API + React) ⇄ camelCase (`@solx/http`,
 * `@solx/surface`) translation boundary.
 *
 * Ported from the old Bun backend's `server/src/wire.ts` (same boundary,
 * same reasoning — see that file's header comment, preserved below) now
 * that the conversion happens client-side instead of server-side. The only
 * structural change: the "inbound" helpers used to parse a `URL`'s query
 * string; they now read a plain snake_case params object, since `api.ts`'s
 * functions were never backed by real query strings to begin with once the
 * Bun hop is gone — they're just JS function arguments.
 *
 * Three layers disagree about field casing:
 *
 *   Rust core / HTTP wire   snake_case   (`type_ref`, `path_prefix`)
 *   `@solx/surface`         camelCase    (`typeRef`, `pathPrefix`)
 *   this API + React        snake_case   (matches Rust)
 *
 * ── Opaque payloads ──────────────────────────────────────────────────────
 * `contents`, `schema`, `action_config` and exec `result` are arbitrary
 * user JSON and are passed through **verbatim**. A blanket recursive key
 * rewrite would corrupt user data — a document field genuinely named
 * `my_field`, or the property names inside a JSON Schema's `properties`
 * map.
 */
import type { JsonValue } from "@solx/surface";

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

/* ── Inbound: snake_case params → camelCase SDK options ─────────────── */

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
 * Note: only `solx-docs` implements `sort_by`/`sort_order`/`filter_field` on
 * the Rust side. `solx-actions` and `solx-types` accept the options but
 * hardcode `ORDER BY path,name` and ignore filters — passing them is
 * harmless, but callers should not present those controls for those tabs.
 */
export function listOptionsFromSnake(opts: Record<string, unknown> = {}): ListOptionsCamel {
  const out: ListOptionsCamel = {};

  const pathPrefix = str(opts.path_prefix);
  if (pathPrefix) out.pathPrefix = pathPrefix;

  const limit = num(opts.limit);
  if (limit !== undefined) out.limit = limit;

  const offset = num(opts.offset);
  if (offset !== undefined) out.offset = offset;

  const filterField = str(opts.filter_field);
  if (filterField) out.filterField = filterField;

  const filterValue = str(opts.filter_value);
  if (filterValue) out.filterValue = filterValue;

  const sortBy = str(opts.sort_by);
  if (sortBy) out.sortBy = sortBy;

  const sortOrder = str(opts.sort_order)?.toLowerCase();
  if (sortOrder === "asc" || sortOrder === "desc") out.sortOrder = sortOrder;

  const dateAfter = str(opts.date_after);
  if (dateAfter) out.dateAfter = dateAfter;

  const dateBefore = str(opts.date_before);
  if (dateBefore) out.dateBefore = dateBefore;

  return out;
}

export interface SearchQueryCamel {
  q?: string;
  pathPrefix?: string;
  typeRef?: string;
  linkedTo?: string;
  limit?: number;
  offset?: number;
}

export function searchQueryFromSnake(query: Record<string, unknown> = {}): SearchQueryCamel {
  const out: SearchQueryCamel = {};

  const q = str(query.q);
  if (q) out.q = q;

  const pathPrefix = str(query.path_prefix);
  if (pathPrefix) out.pathPrefix = pathPrefix;

  const typeRef = str(query.type_ref);
  if (typeRef) out.typeRef = typeRef;

  const linkedTo = str(query.linked_to);
  if (linkedTo) out.linkedTo = linkedTo;

  const limit = num(query.limit);
  if (limit !== undefined) out.limit = limit;

  const offset = num(query.offset);
  if (offset !== undefined) out.offset = offset;

  return out;
}

/* ── Shared value objects ────────────────────────────────────────────── */

function fileRefToWire(raw: unknown): Record<string, unknown> {
  const f = asRecord(raw);
  return {
    name: str(f.name),
    relPath: str(f.relPath),
    contentType: str(f.contentType),
  };
}

function fileRefFromWire(raw: unknown): Record<string, unknown> {
  const f = asRecord(raw);
  return {
    name: str(f.name),
    relPath: str(f.relPath),
    contentType: str(f.contentType),
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

/* ── Outbound: `@solx/http` (camelCase) entities → snake_case JSON ────── */

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

/* ── Inbound: snake_case JSON → `@solx/http` (camelCase) inputs ───────── */

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

export type { JsonValue };
