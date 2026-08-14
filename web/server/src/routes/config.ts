/**
 * Config routes — read/write the solx-js config.
 *
 *   GET   /api/config → solx.config.snapshot()
 *   PATCH /api/config → solx.config.patch(patch)   (body: partial JSON)
 */

import { getSolx } from "../solx-context";

export async function mountConfigRoutes(
  req: Request,
  url: URL,
  method: string,
  corsHeaders: Record<string, string>,
): Promise<Response> {
  const solx = await getSolx();

  if (method === "GET") {
    const snapshot = solx.config.snapshot();
    return json(snapshot, 200, corsHeaders);
  }

  if (method === "PATCH") {
    let patch: Record<string, unknown>;
    try {
      patch = (await req.json()) as Record<string, unknown>;
    } catch {
      return json({ error: { kind: "invalid", message: "Request body must be valid JSON" } }, 400, corsHeaders);
    }
    if (!patch || typeof patch !== "object" || Array.isArray(patch)) {
      return json({ error: { kind: "invalid", message: "Config patch must be a JSON object" } }, 400, corsHeaders);
    }
    solx.config.patch(patch);
    const snapshot = solx.config.snapshot();
    return json(snapshot, 200, corsHeaders);
  }

  return json({ error: { kind: "invalid", message: `Method ${method} not allowed for ${url.pathname}` } }, 405, corsHeaders);
}

function json(body: unknown, status = 200, extraHeaders: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      "Content-Type": "application/json",
      ...extraHeaders,
    },
  });
}
