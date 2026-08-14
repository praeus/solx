/**
 * solx-context — singleton wrapper around solx-js's createSolx().
 *
 * Reads SOLX_SERVER_URL and SOLX_SERVER_TOKEN from the environment (or the
 * runtime config file) and constructs the Solx client once on first access.
 *
 *  - getSolx()     — returns the Solx handle, initializing lazily.
 *  - tryGetSolx() — returns the handle if already initialized, else null.
 *                    Used by the request router to short-circuit when the
 *                    SDK has not been loaded yet so the frontend can show a
 *                    connection-error banner instead of a hanging request.
 */

import { createSolx, type Solx } from "solx";

const DEFAULT_SERVER_URL = "http://127.0.0.1:8766";

let solxPromise: Promise<Solx> | null = null;
let solxResolved: Solx | null = null;

function resolveServerUrl(): string {
  return process.env.SOLX_SERVER_URL ?? DEFAULT_SERVER_URL;
}

function resolveServerToken(): string | undefined {
  return process.env.SOLX_SERVER_TOKEN ?? undefined;
}

/**
 * Returns the Solx handle, initializing lazily. Throws on connection failure.
 */
export async function getSolx(): Promise<Solx> {
  if (solxResolved) return solxResolved;
  if (!solxPromise) {
    solxPromise = createSolx({
      serverUrl: resolveServerUrl(),
      serverToken: resolveServerToken(),
    }).then((s) => {
      solxResolved = s;
      return s;
    });
  }
  return solxPromise;
}

/**
 * Returns the Solx handle if already initialized, else null. Non-throwing.
 */
export function tryGetSolx(): Solx | null {
  return solxResolved;
}

/**
 * Returns the configured server URL (for the health endpoint).
 */
export function getConfiguredServerUrl(): string {
  return resolveServerUrl();
}
