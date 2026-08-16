/**
 * Frontend API client — typed fetch wrappers for the Bun backend.
 *
 * The Vite dev server proxies /api/* to http://localhost:3001.
 * All functions throw on non-2xx with a parsed error message.
 */

// We deliberately don't import @solx/surface types at the network layer because
// the Bun backend serializes them as plain JSON; the frontend re-types the
// responses locally. If we later want strict parity, we can import the DTOs.
// Field casing note: the Bun backend translates the camelCase solx-js SDK
// surface to/from snake_case at its edge (server/src/wire.ts), so everything
// below matches the Rust wire format. Do not mix camelCase in here.
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

export interface SolxConfig {
  dataDirectory?: string;
  filesDirectory?: string;
  modelsDirectory?: string;
  searchIndexDir?: string;
  docsDb?: string;
  actionsDb?: string;
  typesDb?: string;
  serverUrl?: string;
  serverToken?: string;
  serverPort?: number;
  installedPackages?: Array<Record<string, unknown>>;
  [key: string]: unknown;
}

export interface HealthResponse {
  ok: boolean;
  serverUrl: string;
  solxConnected: boolean;
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

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: {
      "Content-Type": "application/json",
      ...(init?.headers ?? {}),
    },
  });

  if (!response.ok) {
    let kind = "other";
    let message = `HTTP ${response.status}`;
    try {
      const body = (await response.json()) as { error?: { kind?: string; message?: string } };
      if (body.error) {
        kind = body.error.kind ?? kind;
        message = body.error.message ?? message;
      }
    } catch {
      // Body wasn't JSON; keep the HTTP status message.
    }
    throw new ApiError(message, response.status, kind);
  }

  // 204 No Content
  if (response.status === 204) return undefined as T;

  return (await response.json()) as T;
}

function qs(params: Record<string, string | number | undefined>): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null || value === "") continue;
    search.set(key, String(value));
  }
  const str = search.toString();
  return str ? `?${str}` : "";
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
  return request<Page<DocumentSummary>>(`/api/documents${qs(opts)}`);
}

export function searchDocuments(query: {
  q?: string;
  path_prefix?: string;
  type_ref?: string;
  limit?: number;
  offset?: number;
}): Promise<SearchResults> {
  return request<SearchResults>(`/api/documents/search${qs(query)}`);
}

export function fetchDocument(path: string, name: string): Promise<DocumentSummary> {
  const encoded = `${encodeURI(path)}/${encodeURIComponent(name)}`;
  return request<DocumentSummary>(`/api/documents${encoded}`);
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
  // Map the bare type_name (e.g. "Post") into the wire-level typeRef. The
  // /builtin/types namespace is where solx stores the core document types;
  // if the caller passes a full type_ref, prefer that.
  if (type_name && !wireInput.typeRef && !wireInput.type_ref) {
    wireInput.typeRef = `/builtin/types/${type_name}`;
  }
  return request<DocumentSummary>("/api/documents", {
    method: "POST",
    body: JSON.stringify({ path, name, input: wireInput }),
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
  return request<DocumentSummary>(`/api/documents/${encodeURI(path)}/${encodeURIComponent(name)}`, {
    method: "POST",
    body: JSON.stringify({ path, name, input }),
  });
}

export function deleteDocument(path: string, name: string): Promise<void> {
  const encoded = `${encodeURI(path)}/${encodeURIComponent(name)}`;
  return request<void>(`/api/documents${encoded}`, { method: "DELETE" });
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
  return request<Page<TypeEntity>>(`/api/types${qs(opts)}`);
}

export function fetchType(path: string, name: string): Promise<TypeEntity> {
  const encoded = `${encodeURI(path)}/${encodeURIComponent(name)}`;
  return request<TypeEntity>(`/api/types${encoded}`);
}

export function createType(input: {
  path: string;
  name: string;
  description?: string;
  schema?: Record<string, unknown>;
  groups?: string[];
}): Promise<TypeEntity> {
  const { path, name, ...rest } = input;
  return request<TypeEntity>("/api/types", {
    method: "POST",
    body: JSON.stringify({ path, name, input: rest }),
  });
}

export function deleteType(path: string, name: string): Promise<void> {
  const encoded = `${encodeURI(path)}/${encodeURIComponent(name)}`;
  return request<void>(`/api/types${encoded}`, { method: "DELETE" });
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
  return request<Page<ActionSummary>>(`/api/actions${qs(opts)}`);
}

// No searchActions(): solx-actions has no server-side filtering, so the
// Actions tab scopes by path here and narrows in memory (lib/clientView.ts).

export function fetchAction(path: string, name: string): Promise<ActionSummary> {
  const encoded = `${encodeURI(path)}/${encodeURIComponent(name)}`;
  return request<ActionSummary>(`/api/actions${encoded}`);
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
  trusted?: boolean;
}): Promise<ActionSummary> {
  const { path, name, ...rest } = input;
  // ActionType is a lowercase enum on the Rust side ("wasm", "webhook", …).
  const wireInput: Record<string, unknown> = { ...rest };
  if (typeof wireInput.action_type === "string") {
    wireInput.action_type = wireInput.action_type.toLowerCase();
  }
  return request<ActionSummary>("/api/actions", {
    method: "POST",
    body: JSON.stringify({ path, name, input: wireInput }),
  });
}

export function deleteAction(path: string, name: string): Promise<void> {
  const encoded = `${encodeURI(path)}/${encodeURIComponent(name)}`;
  return request<void>(`/api/actions${encoded}`, { method: "DELETE" });
}

export function execAction(
  path: string,
  name: string,
  params: unknown = null,
): Promise<ActionExecResult> {
  const encoded = `${encodeURI(path)}/${encodeURIComponent(name)}`;
  return request<ActionExecResult>(`/api/actions${encoded}/exec`, {
    method: "POST",
    body: JSON.stringify({ params }),
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
  return request<FileListResponse>(`/api/files/list${qs({ prefix })}`);
}

export function fetchFileContent(relPath: string): Promise<FileContentResponse> {
  return request<FileContentResponse>(`/api/files/content${qs({ relPath })}`);
}

/**
 * URL of the binary file endpoint. Use as `<img src>` or in Tiptap
 * `<img src>` so the browser loads the file directly with the right
 * Content-Type (vs. the base64-JSON `fetchFileContent`). Returns a
 * path-only URL so the Vite dev server proxy (which forwards `/api/*`
 * to the Bun backend) works in dev, and a relative URL works in prod.
 */
export function fileRawUrl(relPath: string): string {
  return `/api/files/raw${qs({ relPath })}`;
}

export function uploadFile(relPath: string, bytes: Uint8Array): Promise<{ relPath: string }> {
  const bytesB64 = bytesToBase64(bytes);
  return request<{ relPath: string }>("/api/files", {
    method: "PUT",
    body: JSON.stringify({ relPath, bytesB64 }),
  });
}

export function deleteFile(relPath: string): Promise<void> {
  return request<void>("/api/files", {
    method: "DELETE",
    body: JSON.stringify({ relPath }),
  });
}

function bytesToBase64(bytes: Uint8Array): string {
  // Browser-safe base64 encoding.  Chunked to avoid blowing native string
  // length limits on large buffers.
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

/** ── Config ─────────────────────────────────────────────────────────── */

export function fetchConfig(): Promise<SolxConfig> {
  return request<SolxConfig>("/api/config");
}

export function patchConfig(patch: Record<string, unknown>): Promise<SolxConfig> {
  return request<SolxConfig>("/api/config", {
    method: "PATCH",
    body: JSON.stringify(patch),
  });
}

/** ── Health ─────────────────────────────────────────────────────────── */

export function fetchHealth(): Promise<HealthResponse> {
  return request<HealthResponse>("/api/health");
}
