/**
 * Frontend API client — talks directly to `solx-server` via `@solx/http`,
 * no backend hop in between (there used to be a Bun server here; see
 * `d:\Projects\solx-core\docs\next-steps.md` item 3 for why it's gone).
 *
 * `wire.ts` is the snake_case(this file)⇄camelCase(`@solx/http`) boundary,
 * moved client-side from the old Bun backend's `server/src/wire.ts`
 * unchanged in spirit. Every function below keeps its original name,
 * signature and snake_case-shaped return type so `hooks.ts` and every
 * consuming component needed no changes for this migration — only the
 * body of each function changed, from a `fetch('/api/...')` call to a
 * `getConnection().<manager>.<method>(...)` call.
 */
import { SolxError, type DocumentInput, type ActionInput, type TypeInput, type JsonValue } from "@solx/surface";
import { getConnection, getStoredServerUrl, isConfigured } from "./connection";
import {
  actionExecResultToWire,
  actionInputFromWire,
  actionToWire,
  documentInputFromWire,
  documentToWire,
  listOptionsFromSnake,
  pageToWire,
  searchQueryFromSnake,
  searchResultsToWire,
  typeInputFromWire,
  typeToWire,
} from "./wire";

export interface DocumentSummary {
  id?: string;
  path: string;
  name: string;
  title?: string | null;
  summary?: string | null;
  type_ref?: string | null;
  author?: string | null;
  pub_date?: string | null;
  confidence?: number | null;
  created_at?: string;
  updated_at?: string;
  [key: string]: unknown;
}

export interface Page<T> {
  items: T[];
  total: number;
  limit: number;
  offset: number;
}

export interface SearchHit {
  id: string;
  path: string;
  name: string;
  title?: string;
  summary?: string;
  type_ref?: string;
  score: number;
}

export interface SearchResults {
  hits: SearchHit[];
  total: number;
  limit: number;
  offset: number;
}

export interface TypeEntity {
  id?: string;
  path: string;
  name: string;
  description?: string;
  schema?: Record<string, unknown>;
  groups?: string[];
  created_at?: string;
  updated_at?: string;
  [key: string]: unknown;
}

export interface HealthResponse {
  reachable: boolean;
}

export class ApiError extends Error {
  readonly status: number;
  readonly kind: string;
  constructor(message: string, status: number, kind: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.kind = kind;
  }
}

const KIND_TO_SNAKE: Record<string, string> = {
  NotFound: "not_found",
  Invalid: "invalid",
  Validation: "validation",
  Conflict: "conflict",
  Io: "io",
  Db: "db",
  Exec: "exec",
  Config: "config",
  Other: "other",
};

/** Mirrors `solx-server::error::status_for` — cosmetic now (no real HTTP hop), kept for shape parity. */
function statusForKind(kind: string): number {
  switch (kind) {
    case "not_found": return 404;
    case "invalid": return 400;
    case "validation": return 422;
    case "conflict": return 409;
    default: return 500;
  }
}

function toApiError(e: unknown): ApiError {
  if (SolxError.is(e)) {
    const err = e as SolxError;
    const kind = KIND_TO_SNAKE[err.kind] ?? "other";
    return new ApiError(err.detail, statusForKind(kind), kind);
  }
  const message = e instanceof Error ? e.message : String(e);
  return new ApiError(message, 500, "other");
}

async function call<T>(fn: () => Promise<T>): Promise<T> {
  try {
    return await fn();
  } catch (e) {
    throw toApiError(e);
  }
}

/** ── Documents ──────────────────────────────────────────────────────── */

export function fetchDocuments(opts: {
  path_prefix?: string;
  limit?: number;
  offset?: number;
  sort_by?: string;
  sort_order?: "asc" | "desc";
  filter_field?: string;
  filter_value?: string;
} = {}): Promise<Page<DocumentSummary>> {
  return call(async () => {
    const page = await getConnection().docs.list(listOptionsFromSnake(opts));
    return pageToWire(page, documentToWire) as unknown as Page<DocumentSummary>;
  });
}

export function searchDocuments(query: {
  q?: string;
  path_prefix?: string;
  type_ref?: string;
  limit?: number;
  offset?: number;
}): Promise<SearchResults> {
  return call(async () => {
    const results = await getConnection().docs.search(searchQueryFromSnake(query));
    return searchResultsToWire(results) as unknown as SearchResults;
  });
}

export function fetchDocument(path: string, name: string): Promise<DocumentSummary> {
  return call(async () => {
    const doc = await getConnection().docs.get(path, name);
    return documentToWire(doc) as unknown as DocumentSummary;
  });
}

export function createDocument(input: {
  path: string;
  name: string;
  /** Full type reference, e.g. "/types/docs/BlogPostWithComments". */
  type_ref?: string;
  /** Bare type name (mapped to typeRef = "/builtin/types/<Name>"). */
  type_name?: string;
  title?: string;
  summary?: string;
  author?: string;
  pub_date?: string;
  confidence?: number;
  contents?: Record<string, unknown>;
  files?: Array<{ name: string; relPath: string; contentType?: string }>;
}): Promise<DocumentSummary> {
  const { path, name, type_name, ...rest } = input;
  const wireInput: Record<string, unknown> = { ...rest };
  // Map the bare type_name (e.g. "Post") into the wire-level type_ref. The
  // /builtin/types namespace is where solx stores the core document types;
  // if the caller passes a full type_ref, prefer that.
  if (type_name && !wireInput.type_ref) {
    wireInput.type_ref = `/builtin/types/${type_name}`;
  }
  return call(async () => {
    const doc = await getConnection().docs.save(
      path,
      name,
      documentInputFromWire(wireInput) as unknown as DocumentInput,
    );
    return documentToWire(doc) as unknown as DocumentSummary;
  });
}

export function updateDocument(
  path: string,
  name: string,
  input: {
    /** Full type reference, e.g. "/types/docs/Post". */
    type_ref?: string;
    title?: string;
    summary?: string;
    contents?: Record<string, unknown>;
    /** Legacy / future-friendly alias accepted alongside type_ref. */
    type_name?: string;
    [key: string]: unknown;
  },
): Promise<DocumentSummary> {
  return call(async () => {
    const doc = await getConnection().docs.save(
      path,
      name,
      documentInputFromWire(input) as unknown as DocumentInput,
    );
    return documentToWire(doc) as unknown as DocumentSummary;
  });
}

export function deleteDocument(path: string, name: string): Promise<void> {
  return call(() => getConnection().docs.delete(path, name));
}

/** ── Types ───────────────────────────────────────────────────────────── */

export function fetchTypes(opts: {
  path_prefix?: string;
  limit?: number;
  offset?: number;
  sort_by?: string;
  sort_order?: "asc" | "desc";
  filter_field?: string;
  filter_value?: string;
} = {}): Promise<Page<TypeEntity>> {
  return call(async () => {
    const page = await getConnection().types.list(listOptionsFromSnake(opts));
    return pageToWire(page, typeToWire) as unknown as Page<TypeEntity>;
  });
}

export function fetchType(path: string, name: string): Promise<TypeEntity> {
  return call(async () => {
    const t = await getConnection().types.get(path, name);
    return typeToWire(t) as unknown as TypeEntity;
  });
}

export function createType(input: {
  path: string;
  name: string;
  description?: string;
  schema?: Record<string, unknown>;
  groups?: string[];
}): Promise<TypeEntity> {
  const { path, name, ...rest } = input;
  return call(async () => {
    const t = await getConnection().types.save(
      path,
      name,
      typeInputFromWire(rest) as unknown as TypeInput,
    );
    return typeToWire(t) as unknown as TypeEntity;
  });
}

export function deleteType(path: string, name: string): Promise<void> {
  return call(() => getConnection().types.delete(path, name));
}

/** ── Actions ────────────────────────────────────────────────────────── */

export interface ActionSummary {
  id?: string;
  path: string;
  name: string;
  caption?: string | null;
  description?: string | null;
  category?: string | null;
  action_type?: string | null;
  /** Full type reference for the action's parameter, e.g. "/types/core/Null". */
  param_type_ref?: string | null;
  /** Full type reference for the action's result. */
  result_type_ref?: string | null;
  capabilities?: string[];
  phrases?: string[];
  bin_name?: string | null;
  fn_name?: string | null;
  action_config?: Record<string, unknown> | null;
  trusted?: boolean;
  created_at?: string;
  updated_at?: string;
  [key: string]: unknown;
}

export interface ActionExecResult {
  action: string;
  result: unknown;
  success: boolean;
  message?: string | null;
}

export function fetchActions(opts: {
  path_prefix?: string;
  limit?: number;
  offset?: number;
  sort_by?: string;
  sort_order?: "asc" | "desc";
  filter_field?: string;
  filter_value?: string;
} = {}): Promise<Page<ActionSummary>> {
  return call(async () => {
    const page = await getConnection().actions.list(listOptionsFromSnake(opts));
    return pageToWire(page, actionToWire) as unknown as Page<ActionSummary>;
  });
}

// No searchActions(): solx-actions has no server-side filtering, so the
// Actions tab scopes by path here and narrows in memory (lib/clientView.ts).

export function fetchAction(path: string, name: string): Promise<ActionSummary> {
  return call(async () => {
    const a = await getConnection().actions.get(path, name);
    return actionToWire(a) as unknown as ActionSummary;
  });
}

export function createAction(input: {
  path: string;
  name: string;
  description?: string;
  caption?: string;
  category?: string;
  capabilities?: string[];
  phrases?: string[];
  param_type_ref?: string;
  result_type_ref?: string;
  action_type?: string;
  bin_name?: string;
  fn_name?: string;
  action_config?: Record<string, unknown>;
  files?: Array<{ name: string; relPath: string; contentType?: string }>;
  trusted?: boolean;
}): Promise<ActionSummary> {
  const { path, name, ...rest } = input;
  // ActionType is a lowercase enum on the Rust side ("wasm", "webhook", …).
  const wireInput: Record<string, unknown> = { ...rest };
  if (typeof wireInput.action_type === "string") {
    wireInput.action_type = wireInput.action_type.toLowerCase();
  }
  return call(async () => {
    const a = await getConnection().actions.save(
      path,
      name,
      actionInputFromWire(wireInput) as ActionInput,
    );
    return actionToWire(a) as unknown as ActionSummary;
  });
}

export function deleteAction(path: string, name: string): Promise<void> {
  return call(() => getConnection().actions.delete(path, name));
}

export function execAction(
  path: string,
  name: string,
  params: unknown = null,
): Promise<ActionExecResult> {
  return call(async () => {
    const result = await getConnection().actions.exec(path, name, params as JsonValue);
    return actionExecResultToWire(result) as unknown as ActionExecResult;
  });
}

/**
 * ── Async invocations (/builtin/action/*, /builtin/console/*) ─────────
 *
 * These are ordinary seeded Internal actions (solx-actions/src/seed.rs),
 * reached through the exact same `ActionManager.exec` call as everything
 * else — no dedicated routes exist or are needed. They exist for detached
 * execution: `start` returns an `invocation_id` immediately while the
 * action runs server-side, `poll`/`tail` long-poll for status/console
 * updates, and `stop` requests cancellation. See
 * `solx-actions/src/invocations.rs` and
 * `solx-actions/src/internal/{invocation,console}.rs` for the source of
 * truth on every field below.
 *
 * Note: unlike the old Bun-backend version of these helpers, cancellation
 * is no longer threaded through an `AbortSignal` on the underlying
 * `fetch` — `@solx/http`'s manager methods don't accept one. A caller
 * that stops watching (e.g. a closed dialog) simply stops awaiting the
 * result; the in-flight long-poll request against solx-server still
 * completes server-side, same as it always did once detached.
 */

export interface Invocation {
  invocation_id: string;
  action_ref: string;
  status: string;
  result: unknown;
  error: string | null;
  created_at: string;
  updated_at: string;
  finished_at: string | null;
  console_seq_start: number;
}

export interface ConsoleEntry {
  seq: number;
  ts: string;
  level: string;
  invocation_id: string | null;
  run_id: string | null;
  source: string;
  message: string | null;
  data: unknown;
}

export interface ConsoleTailResult {
  entries: ConsoleEntry[];
  next_cursor: number | null;
  first_seq: number | null;
  dropped: number;
}

/** `true` once a status can never change again — mirrors `invocations::is_terminal`. */
export function isTerminalInvocationStatus(status: string): boolean {
  return status !== "running" && status !== "cancelling";
}

async function execBuiltin<T>(
  path: string,
  name: string,
  params: Record<string, unknown>,
): Promise<T> {
  return call(async () => {
    const result = await getConnection().actions.exec(path, name, params as JsonValue);
    return (result as unknown as { result: T }).result;
  });
}

export function startInvocation(
  path: string,
  name: string,
  params: unknown,
): Promise<Invocation> {
  return execBuiltin<Invocation>("/builtin/action", "start", { name, path, params: params ?? {} });
}

export function pollInvocation(
  invocationId: string,
  waitSecs?: number,
): Promise<Invocation | null> {
  return execBuiltin<Invocation | null>("/builtin/action", "poll", {
    invocation_id: invocationId,
    wait_secs: waitSecs,
  });
}

export function stopInvocation(
  invocationId: string,
  opts: { force?: boolean; graceSecs?: number } = {},
): Promise<Invocation | null> {
  return execBuiltin<Invocation | null>("/builtin/action", "stop", {
    invocation_id: invocationId,
    force: opts.force,
    grace_secs: opts.graceSecs,
  });
}

export function tailConsole(
  actionRef: string,
  opts: { cursor?: number | null; limit?: number; waitSecs?: number } = {},
): Promise<ConsoleTailResult> {
  return execBuiltin<ConsoleTailResult>("/builtin/console", "tail", {
    action_ref: actionRef,
    cursor: opts.cursor ?? undefined,
    limit: opts.limit,
    wait_secs: opts.waitSecs,
  });
}

/** ── Files ──────────────────────────────────────────────────────────── */

export interface FileEntry {
  relPath: string;
  size?: number;
  contentType?: string | null;
}

export interface FileListResponse {
  paths: string[];
  prefix: string;
}

export interface FileContentResponse {
  relPath: string;
  bytesB64: string;
  size: number;
}

export function listFiles(prefix: string): Promise<FileListResponse> {
  return call(async () => {
    const paths = await getConnection().files.list(prefix);
    return { paths, prefix };
  });
}

export function fetchFileContent(relPath: string): Promise<FileContentResponse> {
  return call(async () => {
    const bytes = await getConnection().files.get(relPath);
    return { relPath, bytesB64: bytesToBase64(bytes), size: bytes.length };
  });
}

export function uploadFile(relPath: string, bytes: Uint8Array): Promise<{ relPath: string }> {
  return call(async () => {
    const normalized = await getConnection().files.put(relPath, bytes);
    return { relPath: normalized };
  });
}

export function deleteFile(relPath: string): Promise<void> {
  return call(() => getConnection().files.delete(relPath));
}

function bytesToBase64(bytes: Uint8Array): string {
  // Browser-safe base64 encoding. Chunked to avoid blowing native string
  // length limits on large buffers.
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

/** ── Health ─────────────────────────────────────────────────────────── */

/**
 * `solx-server`'s `GET /health` is a plain unauthenticated `"ok"` string
 * (no JSON body, no bearer check) — this just probes reachability. Unlike
 * the old Bun backend's `/api/health`, there's no separate "backend up but
 * solx-server down" state to report; there's only "solx-server reachable"
 * or not.
 */
export async function fetchHealth(): Promise<HealthResponse> {
  if (!isConfigured()) return { reachable: false };
  try {
    const res = await fetch(`${getStoredServerUrl()}/health`);
    return { reachable: res.ok };
  } catch {
    return { reachable: false };
  }
}
