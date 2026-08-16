/**
 * Action routes.
 *
 *   GET    /api/actions                    → solx.actions.list(opts)
 *   GET    /api/actions/:path/:name        → solx.actions.get(path, name)
 *   POST   /api/actions                    → solx.actions.save(path, name, input)
 *   DELETE /api/actions/:path/:name        → solx.actions.delete(path, name)
 *   POST   /api/actions/:path/:name/exec   → solx.actions.exec(path, name, params)
 *
 * There is deliberately no `/search` route. `solx-actions::list` honours only
 * `path_prefix`/`limit`/`offset` — it hardcodes `ORDER BY path,name` and
 * ignores `filter_field`/`filter_value` (unlike `solx-docs`, which implements
 * both), so a search endpoint here could only ever return the unfiltered list.
 * The Actions tab scopes by path server-side and narrows the rest in memory
 * (web/src/lib/clientView.ts). Restore a real endpoint here once the Rust
 * manager grows filter support.
 */

import { getSolx } from "../solx-context";
import {
  errJson,
  okJson,
  readJson,
  splitRef,
  validatePathName,
} from "./_helpers";
import {
  actionExecResultToWire,
  actionInputFromWire,
  actionToWire,
  listOptionsFromUrl,
  pageToWire,
} from "../wire";

const POST_SUFFIX = "/exec";

export async function mountActionsRoutes(
  req: Request,
  url: URL,
  method: string,
  corsHeaders: Record<string, string>,
): Promise<Response> {
  const { pathname } = url;
  const rest = pathname.replace(/^\/api\/actions/, "");
  const isExec = rest.endsWith(POST_SUFFIX);
  const execRef = isExec ? rest.slice(0, -POST_SUFFIX.length) : "";
  const isExecItem = isExec && execRef.startsWith("/") && execRef !== "/";
  const isItem = !isExec && rest.startsWith("/") && rest !== "/";

  if (method === "POST" && isExecItem) {
    return handleExec(execRef, req, corsHeaders);
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
  return okJson(pageToWire(result, actionToWire), 200, corsHeaders);
}

async function handleGet(rest: string, corsHeaders: Record<string, string>): Promise<Response> {
  const split = splitRef(rest);
  if (!split) {
    return errJson("invalid", "Action ref must include a name", 400, corsHeaders);
  }
  const solx = await getSolx();
  const result = await solx.actions.get(split.path, split.name);
  return okJson(actionToWire(result), 200, corsHeaders);
}

async function handleSave(req: Request, corsHeaders: Record<string, string>): Promise<Response> {
  const parsed = await readJson<{ path?: string; name?: string; input?: Record<string, unknown> }>(
    req,
    corsHeaders,
  );
  if (!parsed.ok) return parsed.response;

  const validated = validatePathName(parsed.value, corsHeaders);
  if (!validated.ok) return validated.response;
  const input = actionInputFromWire(parsed.value.input ?? {});

  const solx = await getSolx();
  const result = await solx.actions.save(
    validated.path,
    validated.name,
    input as Parameters<typeof solx.actions.save>[2],
  );
  return okJson(actionToWire(result), 201, corsHeaders);
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
  return okJson(actionExecResultToWire(result), 200, corsHeaders);
}
