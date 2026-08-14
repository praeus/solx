/**
 * Health endpoint — no solx-js dependency for the basic liveness ping.
 * Reports whether solx-js has successfully connected to solx-server.
 */

import { getConfiguredServerUrl, tryGetSolx } from "../solx-context";

export async function mountHealthRoutes(_req: Request): Promise<Response> {
  const solx = tryGetSolx();
  return json({
    ok: true,
    serverUrl: getConfiguredServerUrl(),
    solxConnected: solx !== null,
  });
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}
