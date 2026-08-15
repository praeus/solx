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
import {
  errJson,
  listOptionsFromUrl,
  okJson,
  readJson,
  splitRef,
  validatePathName,
} from "./_helpers";

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

  return errJson(
    "invalid",
    `Method ${method} not allowed for ${pathname}`,
    405,
    corsHeaders,
  );
}

async function handleList(url: URL, corsHeaders: Record<string, string>): Promise<Response> {
  const solx = await getSolx();
  const opts = listOptionsFromUrl(url);
  const result = await solx.docs.list(opts as Parameters<typeof solx.docs.list>[0]);
  return okJson(result, 200, corsHeaders);
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
  return okJson(result, 200, corsHeaders);
}

async function handleGet(rest: string, corsHeaders: Record<string, string>): Promise<Response> {
  const split = splitRef(rest);
  if (!split) {
    return errJson("invalid", "Document ref must include a name", 400, corsHeaders);
  }
  const solx = await getSolx();
  const result = await solx.docs.get(split.path, split.name);
  return okJson(result, 200, corsHeaders);
}

async function handleSave(req: Request, corsHeaders: Record<string, string>): Promise<Response> {
  const parsed = await readJson<{ path?: string; name?: string; input?: Record<string, unknown> }>(
    req,
    corsHeaders,
  );
  if (!parsed.ok) return parsed.response;

  const validated = validatePathName(parsed.value, corsHeaders);
  if (!validated.ok) return validated.response;
  const input = parsed.value.input ?? {};

  const solx = await getSolx();
  const result = await solx.docs.save(
    validated.path,
    validated.name,
    input as Parameters<typeof solx.docs.save>[2],
  );
  return okJson(result, 201, corsHeaders);
}

async function handleDelete(rest: string, corsHeaders: Record<string, string>): Promise<Response> {
  const split = splitRef(rest);
  if (!split) {
    return errJson("invalid", "Document ref must include a name", 400, corsHeaders);
  }
  const solx = await getSolx();
  await solx.docs.delete(split.path, split.name);
  return okJson({ ok: true }, 200, corsHeaders);
}
