/**
 * Shared helpers for entity routes.
 *
 *   splitRef()   — splits a URL path segment into { path, name }
 *   okJson()     — JSON response with CORS headers
 *   errJson()    — JSON error response with CORS headers
 *   readJson()   — parses a JSON request body or returns a 400 response
 *
 * Query-param and entity translation lives in ../wire.ts — every SDK call
 * goes through it so the snake_case ⇄ camelCase conversion happens once.
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
 *
 * Each `/`-separated segment is URL-decoded individually so names that
 * contain percent-encoded characters (e.g. spaces encoded as `%20`) are
 * returned as their decoded form. The path is reconstructed by
 * re-joining the decoded segments with `/`s, preserving the canonical
 * leading-slash form.
 */
export function splitRef(rest: string): Split | null {
  if (!rest.startsWith("/")) return null;
  // Strip leading/trailing slashes, then split on the remaining slashes.
  const trimmed = rest.replace(/^\/+/, "").replace(/\/+$/, "");
  if (!trimmed) return null;
  const segments = trimmed.split("/").map((s) => safeDecode(s));
  if (segments.length === 0) return null;
  const name = segments[segments.length - 1];
  if (!name) return null;
  const path = segments.length === 1 ? "/" : "/" + segments.slice(0, -1).join("/");
  return { path, name };
}

/**
 * Decode a single URL-encoded segment. If the segment contains a
 * malformed escape sequence we return it verbatim rather than 400-ing —
 * the upstream solx lookup will surface the not-found error.
 */
function safeDecode(segment: string): string {
  try {
    return decodeURIComponent(segment);
  } catch {
    return segment;
  }
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
