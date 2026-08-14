/**
 * Document routes.
 *
 *   GET    /api/documents                     → solx.docs.list(opts)
 *   GET    /api/documents/search              → solx.docs.search(query)
 *   GET    /api/documents/:path/:name         → solx.docs.get(path, name)
 *   POST   /api/documents                     → solx.docs.save(path, name, input)
 *   DELETE /api/documents/:path/:name         → solx.docs.delete(path, name)
 *
 * Path/name URL segments: the path can contain slashes (e.g. "/research/ai")
 * so we strip the "/api/documents" prefix and split off the last segment as
 * the name, treating everything else as the path.
 */

import { getSolx } from "../solx-context";

export async function mountDocumentsRoutes(
  req: Request,
  url: URL,
  method: string,
  corsHeaders: Record<string, string>,
): Promise<Response> {
  const { pathname } = url;
  const rest = pathname.replace(/^\/api\/documents/, "");
  const isSearch = rest === "/search";
  const isItem = rest.startsWith("/") && rest !== "/search" && rest !== "/";

  if (method === "GET" && isSearch) {
    return handleSearch(url, corsHeaders);
  }
  if (method === "GET" && isItem) {
    return handleGet(rest, corsHeaders);
  }
  if (method === "DELETE" && isItem) {
    return handleDelete(rest, corsHeaders);
  }
  if (method === "GET") {
    return handleList(url, corsHeaders);
  }
  if (method === "POST") {
    return handleSave(req, corsHeaders);
  }

  return json({ error: { kind: "invalid", message: `Method ${method} not allowed for ${pathname}` } }, 405, corsHeaders);
}

interface Split {
  path: string;
  name: string;
}

/**
 * Split a path segment like "/research/ai/note" into { path: "/research/ai", name: "note" }.
 * The last "/<segment>" is always the name; the rest is the path.
 */
function splitRef(rest: string): Split | null {
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

async function handleList(url: URL, corsHeaders: Record<string, string>): Promise<Response> {
  const solx = await getSolx();
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

  const result = await solx.docs.list(opts as Parameters<typeof solx.docs.list>[0]);
  return json(result, 200, corsHeaders);
}

async function handleSearch(url: URL, corsHeaders: Record<string, string>): Promise<Response> {
  const solx = await getSolx();
  const query: Record<string, unknown> = {};
  const q = url.searchParams.get("q");
  if (q) query.q = q;
  const pathPrefix = url.searchParams.get("path_prefix");
  if (pathPrefix) query.path_prefix = pathPrefix;
  const typeRef = url.searchParams.get("type_ref");
  if (typeRef) query.type_ref = typeRef;
  const limit = url.searchParams.get("limit");
  if (limit) query.limit = Number(limit);
  const offset = url.searchParams.get("offset");
  if (offset) query.offset = Number(offset);

  const result = await solx.docs.search(query as Parameters<typeof solx.docs.search>[0]);
  return json(result, 200, corsHeaders);
}

async function handleGet(rest: string, corsHeaders: Record<string, string>): Promise<Response> {
  const split = splitRef(rest);
  if (!split) {
    return json({ error: { kind: "invalid", message: "Document ref must include a name" } }, 400, corsHeaders);
  }
  const solx = await getSolx();
  const result = await solx.docs.get(split.path, split.name);
  return json(result, 200, corsHeaders);
}

async function handleSave(req: Request, corsHeaders: Record<string, string>): Promise<Response> {
  let body: { path?: string; name?: string; input?: Record<string, unknown> };
  try {
    body = (await req.json()) as typeof body;
  } catch {
    return json({ error: { kind: "invalid", message: "Request body must be valid JSON" } }, 400, corsHeaders);
  }

  const path = String(body.path ?? "").trim();
  const name = String(body.name ?? "").trim();
  const input = body.input ?? {};

  if (!path) return json({ error: { kind: "invalid", message: "path is required" } }, 400, corsHeaders);
  if (!name) return json({ error: { kind: "invalid", message: "name is required" } }, 400, corsHeaders);
  if (!path.startsWith("/")) {
    return json({ error: { kind: "invalid", message: "path must start with '/'" } }, 400, corsHeaders);
  }

  const solx = await getSolx();
  const result = await solx.docs.save(path, name, input as Parameters<typeof solx.docs.save>[2]);
  return json(result, 201, corsHeaders);
}

async function handleDelete(rest: string, corsHeaders: Record<string, string>): Promise<Response> {
  const split = splitRef(rest);
  if (!split) {
    return json({ error: { kind: "invalid", message: "Document ref must include a name" } }, 400, corsHeaders);
  }
  const solx = await getSolx();
  await solx.docs.delete(split.path, split.name);
  return json({ ok: true }, 200, corsHeaders);
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
