/**
 * Frontend API client — talks directly to `solx-server` via `@solx/http`,
 * no backend hop in between (there used to be a Bun server here; see
 * `d:\Projects\solx-core\docs\next-steps.md` item 3 for why it's gone).
 *
 * `@solx/http`'s manager methods already return `@solx/surface`'s camelCase
 * entities (`Document`, `TypeEntity`, `Action`, …) — there is no remaining
 * wire hop to justify re-shaping them into snake_case, so this layer just
 * re-exports those types and passes options straight through. The one
 * exception is the async-invocation section below (`startInvocation` /
 * `pollInvocation` / `stopInvocation` / `tailConsole`): those are opaque
 * JSON payloads to/from a seeded Rust action
 * (`solx-actions/src/internal/{invocation,console}.rs`), not mediated by
 * `@solx/surface`, so they're genuinely snake_case on both sides and stay
 * that way.
 */
import {
  SolxError,
  type Action,
  type ActionExecResult,
  type ActionInput,
  type Document,
  type DocumentInput,
  type JsonValue,
  type ListOptions,
  type Page,
  type SearchHit,
  type SearchQuery,
  type SearchResults,
  type TypeEntity,
  type TypeInput,
} from "@solx/surface";
import { getConnection, getStoredServerUrl, isConfigured } from "./connection";

export type DocumentSummary = Document;
export type ActionSummary = Action;
export type { TypeEntity, Page, SearchHit, SearchResults, ActionExecResult };

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

export function fetchDocuments(opts: ListOptions = {}): Promise<Page<Document>> {
  return call(() => getConnection().docs.list(opts));
}

export function searchDocuments(query: SearchQuery): Promise<SearchResults> {
  return call(() => getConnection().docs.search(query));
}

export function fetchDocument(path: string, name: string): Promise<Document> {
  return call(() => getConnection().docs.get(path, name));
}

/**
 * `contents` is arbitrary user JSON (whatever the selected Type's schema
 * describes) — typed `unknown` here rather than `JsonValue` so callers
 * building it from parsed/form state don't need to satisfy that shape
 * exactly; it's passed through to `docs.save` verbatim.
 */
export interface DocumentInputApp extends Omit<DocumentInput, "contents"> {
  contents: unknown;
}

export function createDocument(
  input: DocumentInputApp & {
    path: string;
    name: string;
    /** Bare type name (mapped to typeRef = "/builtin/types/<Name>"). */
    typeName?: string;
  },
): Promise<Document> {
  const { path, name, typeName, ...rest } = input;
  // Map the bare typeName (e.g. "Post") into typeRef. The /builtin/types
  // namespace is where solx stores the core document types; if the caller
  // passes a full typeRef, prefer that.
  const docInput = { ...rest } as DocumentInput;
  if (typeName && !docInput.typeRef) {
    docInput.typeRef = `/builtin/types/${typeName}`;
  }
  return call(() => getConnection().docs.save(path, name, docInput));
}

export function updateDocument(
  path: string,
  name: string,
  input: DocumentInputApp,
): Promise<Document> {
  return call(() => getConnection().docs.save(path, name, input as DocumentInput));
}

export function deleteDocument(path: string, name: string): Promise<void> {
  return call(() => getConnection().docs.delete(path, name));
}

/** ── Types ───────────────────────────────────────────────────────────── */

export function fetchTypes(opts: ListOptions = {}): Promise<Page<TypeEntity>> {
  return call(() => getConnection().types.list(opts));
}

export function fetchType(path: string, name: string): Promise<TypeEntity> {
  return call(() => getConnection().types.get(path, name));
}

/** `schema` is an arbitrary JSON Schema object — see `DocumentInputApp`. */
export interface TypeInputApp extends Omit<TypeInput, "schema"> {
  schema: unknown;
}

export function createType(
  input: TypeInputApp & { path: string; name: string },
): Promise<TypeEntity> {
  const { path, name, ...rest } = input;
  return call(() => getConnection().types.save(path, name, rest as TypeInput));
}

export function deleteType(path: string, name: string): Promise<void> {
  return call(() => getConnection().types.delete(path, name));
}

/** ── Actions ────────────────────────────────────────────────────────── */

export function fetchActions(opts: ListOptions = {}): Promise<Page<Action>> {
  return call(() => getConnection().actions.list(opts));
}

// No searchActions(): solx-actions has no server-side filtering, so the
// Actions tab scopes by path here and narrows in memory (lib/clientView.ts).

export function fetchAction(path: string, name: string): Promise<Action> {
  return call(() => getConnection().actions.get(path, name));
}

/** `actionConfig` is an arbitrary free-form blob — see `DocumentInputApp`. */
export interface ActionInputApp extends Omit<ActionInput, "actionConfig"> {
  actionConfig?: unknown;
}

export function createAction(
  input: ActionInputApp & { path: string; name: string },
): Promise<Action> {
  const { path, name, ...rest } = input;
  const actionInput = { ...rest } as ActionInput;
  // ActionType is a lowercase enum on the Rust side ("wasm", "webhook", …).
  if (typeof actionInput.actionType === "string") {
    actionInput.actionType = actionInput.actionType.toLowerCase() as ActionInput["actionType"];
  }
  return call(() => getConnection().actions.save(path, name, actionInput));
}

export function deleteAction(path: string, name: string): Promise<void> {
  return call(() => getConnection().actions.delete(path, name));
}

export function execAction(
  path: string,
  name: string,
  params: unknown = null,
): Promise<ActionExecResult> {
  return call(() => getConnection().actions.exec(path, name, params as JsonValue));
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
