/**
 * Turns a `solx-server` HTTP error body back into a real `SolxError`. The
 * mapping itself lives in `@solx/surface` (`mapWireError`), shared with
 * `@solx/tauri`, whose IPC errors carry the same tagged shape.
 */
import { mapWireError, type SolxError } from '@solx/surface';

export { mapWireError } from '@solx/surface';

/**
 * `mapWireError` plus the HTTP status, which only feeds the fallback message
 * for a non-2xx whose body isn't a `SolxError` (an older server's bare 401,
 * a proxy error page).
 */
export function mapHttpError(body: unknown, status: number, statusText: string): SolxError {
  return mapWireError(
    body,
    body === undefined
      ? `solx-server returned ${status} ${statusText} with no error body`
      : `solx-server returned ${status} ${statusText} with an unrecognized error body: ${JSON.stringify(body)}`,
  );
}
