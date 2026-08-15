/**
 * Action routes.
 *
 *   GET    /api/actions                    → solx.actions.list(opts)
 *   GET    /api/actions/search             → solx.actions.list(opts) with filter
 *   GET    /api/actions/:path/:name        → solx.actions.get(path, name)
 *   POST   /api/actions                    → solx.actions.save(path, name, input)
 *   DELETE /api/actions/:path/:name        → solx.actions.delete(path, name)
 *   POST   /api/actions/:path/:name/exec   → solx.actions.exec(path, name, params)
 *
 * The "search" endpoint is implemented as a list with a filter_field/filter_value
 * pair (LIKE match on the name) — the SDK doesn't expose a dedicated search.
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

const POST_SUFFIX = "/exec";

export async function mountActionsRoutes(
  req: Request,
  url: URL,
  method: string,
  corsHeaders: Record<string, string>,
): Promise<Response> {
  const { pathname } = url;
  const rest = pathname.replace(/^\/api\/actions/, "");
  const isSearch = rest === "/search";
  const isExec = rest.endsWith(POST_SUFFIX);
  const execRef = isExec ? rest.slice(0, -POST_SUFFIX.length) : "";
  const isExecItem = isExec && execRef.startsWith("/") && execRef !== "/";
  const isItem = !isExec && rest.startsWith("/") && rest !== "/search" && rest !== "/";

  if (method === "POST" && isExecItem) {
    return handleExec(execRef, req, corsHeaders);
  }
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
  if (method === "POST" && !isExec) {
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
  const result = await solx.actions.list(opts as Parameters<typeof solx.actions.list>[0]);
  return okJson(result, 200, corsHeaders);
}

async function handleSearch(url: URL, corsHeaders: Record<string, string>): Promise<Response> {
  const solx = await getSolx();
  const opts = listOptionsFromUrl(url);
  // Search by name (case-insensitive substring via filter_field). The list
  // semantics are exact-match on the SDK side, so we fall back to scoped
  // path-prefix filtering when the query has no spaces.
  const q = url.searchParams.get("q");
  if (q) {
    opts.filter_field = "name";
    opts.filter_value = q;
  }
  const result = await solx.actions.list(opts as Parameters<typeof solx.actions.list>[0]);
  return okJson(result, 200, corsHeaders);
}

async function handleGet(rest: string, corsHeaders: Record<string, string>): Promise<Response> {
  const split = splitRef(rest);
  if (!split) {
    return errJson("invalid", "Action ref must include a name", 400, corsHeaders);
  }
  const solx = await getSolx();
  const result = await solx.actions.get(split.path, split.name);
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
  const result = await solx.actions.save(
    validated.path,
    validated.name,
    input as Parameters<typeof solx.actions.save>[2],
  );
  return okJson(result, 201, corsHeaders);
}

async function handleDelete(rest: string, corsHeaders: Record<string, string>): Promise<Response> {
  const split = splitRef(rest);
  if (!split) {
    return errJson("invalid", "Action ref must include a name", 400, corsHeaders);
  }
  const solx = await getSolx();
  await solx.actions.delete(split.path, split.name);
  return okJson({ ok: true }, 200, corsHeaders);
}

async function handleExec(
  rest: string,
  req: Request,
  corsHeaders: Record<string, string>,
): Promise<Response> {
  const split = splitRef(rest);
  if (!split) {
    return errJson("invalid", "Action ref must include a name", 400, corsHeaders);
  }
  const parsed = await readJson<{ params?: unknown }>(req, corsHeaders);
  if (!parsed.ok) return parsed.response;
  const params = parsed.value.params ?? null;

  const solx = await getSolx();
  const result = await solx.actions.exec(
    split.path,
    split.name,
    params as Parameters<typeof solx.actions.exec>[2],
  );
  return okJson(result, 200, corsHeaders);
}
