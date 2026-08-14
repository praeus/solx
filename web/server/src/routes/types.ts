/**
 * Type routes — populates the document-type facet dropdown.
 *
 *   GET /api/types → solx.types.list(opts)
 */

import { getSolx } from "../solx-context";

export async function mountTypesRoutes(
  _req: Request,
  url: URL,
  method: string,
  corsHeaders: Record<string, string>,
): Promise<Response> {
  if (method !== "GET") {
    return json({ error: { kind: "invalid", message: `Method ${method} not allowed for ${url.pathname}` } }, 405, corsHeaders);
  }

  const solx = await getSolx();
  const opts: Record<string, unknown> = {};
  const limit = url.searchParams.get("limit");
  if (limit) opts.limit = Number(limit);
  const offset = url.searchParams.get("offset");
  if (offset) opts.offset = Number(offset);
  const sortBy = url.searchParams.get("sort_by");
  if (sortBy) opts.sort_by = sortBy;
  const sortOrder = url.searchParams.get("sort_order");
  if (sortOrder) opts.sort_order = sortOrder;

  const result = await solx.types.list(opts as Parameters<typeof solx.types.list>[0]);
  return json(result, 200, corsHeaders);
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
