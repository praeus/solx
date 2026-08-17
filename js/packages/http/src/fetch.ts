/**
 * Shared request-sending helper used by every `Http*Manager`. Mirrors
 * `solx-client::http::post_json` (solx-core/solx-client/src/http.rs):
 * every route is a bearer-authenticated `POST` with a JSON body, and a
 * non-2xx response's JSON body is the `SolxError` itself.
 */
import { SolxError } from '@solx/surface';
import { mapWireError } from './error.js';

export async function postJson<Req, Resp>(
  baseUrl: string,
  token: string,
  route: string,
  body: Req,
): Promise<Resp> {
  let res: Response;
  try {
    res = await fetch(`${baseUrl}${route}`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${token}`,
      },
      body: JSON.stringify(body),
    });
  } catch (e) {
    throw new SolxError(
      'Io',
      `solx-server request failed: ${e instanceof Error ? e.message : String(e)}`,
    );
  }

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
