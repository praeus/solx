/**
 * Shared helpers for entity routes.
 *
 *   splitRef()   — splits a URL path segment into { path, name }
 *   okJson()     — JSON response with CORS headers
 *   errJson()    — JSON error response with CORS headers
 *   readJson()   — parses a JSON request body or returns a 400 response
 *   listOptionsFromUrl() — translates query params into a ListOptions object
 */

/** JSON response with CORS headers and a default 200 status. */
export function okJson(body: unknown, status = 200, corsHeaders: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      "Content-Type": "application/json",
      ...corsHeaders,
    },
  });
}

/** JSON error response. */
export function errJson(
  kind: string,
  message: string,
  status: number,
  corsHeaders: Record<string, string> = {},
): Response {
  return okJson({ error: { kind, message } }, status, corsHeaders);
}

/**
 * Parses a JSON request body. Returns the parsed value, or a 400 response
 * if the body is missing or invalid. Caller should check whether the result
 * is a Response and return it immediately if so.
 */
export async function readJson<T = Record<string, unknown>>(
  req: Request,
  corsHeaders: Record<string, string> = {},
): Promise<{ ok: true; value: T } | { ok: false; response: Response }> {
  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return {
      ok: false,
      response: errJson("invalid", "Request body must be valid JSON", 400, corsHeaders),
    };
  }
  if (!body || typeof body !== "object" || Array.isArray(body)) {
    return {
      ok: false,
      response: errJson("invalid", "Request body must be a JSON object", 400, corsHeaders),
    };
  }
  return { ok: true, value: body as T };
}

export interface Split {
  path: string;
  name: string;
}

/**
 * Split a URL path segment like "/research/ai/note" into
 *   { path: "/research/ai", name: "note" }.
 * The last "/<segment>" is always the name; the rest is the path.
 * Returns null if the segment doesn't contain a name.
 */
export function splitRef(rest: string): Split | null {
  if (!rest.startsWith("/")) return null;
  const trimmed = rest.replace(/^\/+/, "").replace(/\/+$/, "");
  if (!trimmed) return null;
  const lastSlash = trimmed.lastIndexOf("/");
  if (lastSlash === -1) {
    return { path: "/", name: trimmed };
  }
  const name = trimmed.slice(lastSlash + 1);
  const path = "/" + trimmed.slice(0, lastSlash);
  if (!name) return null;
  return { path, name };
}

/**
 * Parse the common entity-list query params (`path_prefix`, `limit`, `offset`,
 * `sort_by`, `sort_order`, `filter_field`, `filter_value`) into an object
 * suitable for the solx-js `list()` call.
 */
export function listOptionsFromUrl(url: URL): Record<string, unknown> {
  const opts: Record<string, unknown> = {};
  const pathPrefix = url.searchParams.get("path_prefix");
  if (pathPrefix) opts.path_prefix = pathPrefix;
  const limit = url.searchParams.get("limit");
  if (limit) opts.limit = Number(limit);
  const offset = url.searchParams.get("offset");
  if (offset) opts.offset = Number(offset);
  const sortBy = url.searchParams.get("sort_by");
  if (sortBy) opts.sort_by = sortBy;
  const sortOrder = url.searchParams.get("sort_order");
  if (sortOrder) opts.sort_order = sortOrder;
  const filterField = url.searchParams.get("filter_field");
  if (filterField) opts.filter_field = filterField;
  const filterValue = url.searchParams.get("filter_value");
  if (filterValue) opts.filter_value = filterValue;
  return opts;
}

/**
 * Validate a document-style { path, name } body. Returns the cleaned values
 * or a 400 response to send back.
 */
export function validatePathName(
  raw: { path?: unknown; name?: unknown },
  corsHeaders: Record<string, string> = {},
): { ok: true; path: string; name: string } | { ok: false; response: Response } {
  const path = String(raw.path ?? "").trim();
  const name = String(raw.name ?? "").trim();
  if (!path) {
    return { ok: false, response: errJson("invalid", "path is required", 400, corsHeaders) };
  }
  if (!name) {
    return { ok: false, response: errJson("invalid", "name is required", 400, corsHeaders) };
  }
  if (!path.startsWith("/")) {
    return { ok: false, response: errJson("invalid", "path must start with '/'", 400, corsHeaders) };
  }
  return { ok: true, path, name };
}
