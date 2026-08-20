/**
 * Shared request-sending helpers used by every `Http*Manager`. Mirrors
 * `solx-client::http` (solx-core/solx-client/src/http.rs): the `solx-server`
 * surface is RESTful, so an entity's `path`/`name` live in the URL, `list`
 * and `search` options ride in the query string, and a non-2xx response's
 * JSON body is the `SolxError` itself.
 */
import { SolxError } from '@solx/surface';
import { mapWireError } from './error.js';

export type QueryValue = string | number | boolean | undefined;

export interface RequestOptions {
  query?: Record<string, QueryValue>;
  /** JSON-serialized. Mutually exclusive with `bytes`. */
  body?: unknown;
  /** Sent verbatim, for file content. Mutually exclusive with `body`. */
  bytes?: Uint8Array;
}

/**
 * Build `{baseUrl}/{resource}/{path}/{name}`, percent-encoding each segment.
 *
 * Names may legally contain `%`, `#`, `?` and spaces — only `/`, `\` and `:`
 * are forbidden (see `solx_surface::path`) — so every segment has to be
 * escaped individually, keeping the separators as real separators. The root
 * path (`'/'`) contributes no segments, so a root-level entity lands at
 * `/docs/note`, which the server reads back as `('/', 'note')`.
 */
export function entityPath(resource: string, path: string, name: string): string {
  // Guarded rather than merely filtered: an empty name would collapse to a
  // trailing slash, which the server normalizes into a *different, valid*
  // entity rather than an error.
  if (name === '' || name.includes('/')) {
    throw new SolxError('Invalid', `invalid entity name: '${name}'`);
  }
  const segments = [...path.split('/'), name].filter((s) => s.length > 0);
  return `/${resource}/${segments.map(encodeURIComponent).join('/')}`;
}

/** Like {@link entityPath}, for a file's `relPath` (no path/name split). */
export function nestedPath(resource: string, relPath: string): string {
  const segments = relPath.split('/').filter((s) => s.length > 0);
  return `/${resource}/${segments.map(encodeURIComponent).join('/')}`;
}

function buildUrl(baseUrl: string, path: string, query?: Record<string, QueryValue>): string {
  const url = `${baseUrl.replace(/\/$/, '')}${path}`;
  if (!query) return url;
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    if (value !== undefined) params.append(key, String(value));
  }
  const qs = params.toString();
  return qs ? `${url}?${qs}` : url;
}

async function send(
  baseUrl: string,
  token: string,
  method: string,
  path: string,
  opts: RequestOptions = {},
): Promise<Response> {
  const headers: Record<string, string> = { Authorization: `Bearer ${token}` };
  let body: BodyInit | undefined;
  if (opts.bytes !== undefined) {
    // Copy into a fresh ArrayBuffer: a Uint8Array view over a larger or
    // shared buffer is not a valid BodyInit everywhere.
    body = opts.bytes.slice().buffer as ArrayBuffer;
  } else if (opts.body !== undefined) {
    headers['Content-Type'] = 'application/json';
    body = JSON.stringify(opts.body);
  }

  try {
    return await fetch(buildUrl(baseUrl, path, opts.query), { method, headers, body });
  } catch (e) {
    throw new SolxError(
      'Io',
      `solx-server request failed: ${e instanceof Error ? e.message : String(e)}`,
    );
  }
}

/**
 * Send a request and read a JSON response. A `204 No Content` (delete,
 * validate) yields `undefined`.
 */
export async function requestJson<Resp>(
  baseUrl: string,
  token: string,
  method: string,
  path: string,
  opts: RequestOptions = {},
): Promise<Resp> {
  const res = await send(baseUrl, token, method, path, opts);

  const text = await res.text();
  let parsed: unknown;
  try {
    parsed = text ? JSON.parse(text) : undefined;
  } catch (e) {
    throw new SolxError(
      'Other',
      `malformed response body from solx-server: ${e instanceof Error ? e.message : String(e)}`,
    );
  }

  if (!res.ok) {
    throw mapWireError(parsed);
  }
  return parsed as Resp;
}

/**
 * Send a request whose success body is raw bytes (file content). Errors are
 * still JSON, so they map through `mapWireError` the same way.
 */
export async function requestBytes(
  baseUrl: string,
  token: string,
  method: string,
  path: string,
  opts: RequestOptions = {},
): Promise<Uint8Array> {
  const res = await send(baseUrl, token, method, path, opts);

  if (!res.ok) {
    const text = await res.text();
    let parsed: unknown;
    try {
      parsed = text ? JSON.parse(text) : undefined;
    } catch {
      parsed = undefined;
    }
    throw mapWireError(parsed);
  }
  return new Uint8Array(await res.arrayBuffer());
}
