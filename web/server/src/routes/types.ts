/**
 * Type routes — populates the document-type facet dropdown and the type-editor.
 *
 *   GET    /api/types                  → solx.types.list(opts)
 *   GET    /api/types/:path/:name      → solx.types.get(path, name)
 *   POST   /api/types                  → solx.types.save(path, name, input)
 *   DELETE /api/types/:path/:name      → solx.types.delete(path, name)
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

export async function mountTypesRoutes(
  req: Request,
  url: URL,
  method: string,
  corsHeaders: Record<string, string>,
): Promise<Response> {
  const { pathname } = url;
  const rest = pathname.replace(/^\/api\/types/, "");
  const isItem = rest.startsWith("/") && rest !== "/";

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
  const result = await solx.types.list(opts as Parameters<typeof solx.types.list>[0]);
  return okJson(result, 200, corsHeaders);
}

async function handleGet(rest: string, corsHeaders: Record<string, string>): Promise<Response> {
  const split = splitRef(rest);
  if (!split) {
    return errJson("invalid", "Type ref must include a name", 400, corsHeaders);
  }
  const solx = await getSolx();
  const result = await solx.types.get(split.path, split.name);
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
  const result = await solx.types.save(
    validated.path,
    validated.name,
    input as Parameters<typeof solx.types.save>[2],
  );
  return okJson(result, 201, corsHeaders);
}

async function handleDelete(rest: string, corsHeaders: Record<string, string>): Promise<Response> {
  const split = splitRef(rest);
  if (!split) {
    return errJson("invalid", "Type ref must include a name", 400, corsHeaders);
  }
  const solx = await getSolx();
  await solx.types.delete(split.path, split.name);
  return okJson({ ok: true }, 200, corsHeaders);
}
