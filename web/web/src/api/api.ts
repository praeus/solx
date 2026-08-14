/**
 * Frontend API client — typed fetch wrappers for the Bun backend.
 *
 * The Vite dev server proxies /api/* to http://localhost:3001.
 * All functions throw on non-2xx with a parsed error message.
 */

// We deliberately don't import @solx/surface types at the network layer because
// the Bun backend serializes them as plain JSON; the frontend re-types the
// responses locally. If we later want strict parity, we can import the DTOs.
export interface DocumentSummary {
  id?: string;
  path: string;
  name: string;
  title?: string | null;
  summary?: string | null;
  type_name?: string | null;
  type_ref?: string | null;
  author?: string | null;
  pub_date?: string | null;
  knowledge_base_name?: string | null;
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
  type_name?: string;
  title?: string;
  summary?: string;
  contents?: Record<string, unknown>;
}): Promise<DocumentSummary> {
  const { path, name, type_name, ...rest } = input;
  // solx expects a `typeRef` (e.g. "/packages/solx-google/BlogPostWithComments")
  // inside the document input. The dropdown already sends the full path/name ref.
  const wireInput: Record<string, unknown> = { ...rest };
  if (type_name) {
    wireInput.typeRef = type_name;
  }
  return request<DocumentSummary>("/api/documents", {
    method: "POST",
    body: JSON.stringify({ path, name, input: wireInput }),
  });
}

export function deleteDocument(path: string, name: string): Promise<void> {
  const encoded = `${encodeURI(path)}/${encodeURIComponent(name)}`;
  return request<void>(`/api/documents${encoded}`, { method: "DELETE" });
}

/** ── Types ───────────────────────────────────────────────────────────── */

export function fetchTypes(opts: { limit?: number; offset?: number } = {}): Promise<Page<TypeEntity>> {
  return request<Page<TypeEntity>>(`/api/types${qs(opts)}`);
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
