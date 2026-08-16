/**
 * File routes.
 *
 *   GET    /api/files/list            → solx.files.list(prefix)
 *   GET    /api/files/content         → solx.files.get(relPath) — returns base64
 *   GET    /api/files/raw?relPath=…   → solx.files.get(relPath) — returns bytes with Content-Type
 *                                         (used by <img src> and Tiptap image nodes)
 *   PUT    /api/files                 → solx.files.put(relPath, bytes) — body { relPath, bytesB64 }
 *   DELETE /api/files                 → solx.files.delete(relPath) — body { relPath }
 *
 * Bytes travel as base64-encoded JSON for the dev-time API.  The solx-server
 * route bumps its body limit to 64 MiB to accommodate large files.
 */

import { getSolx } from "../solx-context";
import { errJson, okJson, readJson } from "./_helpers";

export async function mountFilesRoutes(
  req: Request,
  url: URL,
  method: string,
  corsHeaders: Record<string, string>,
): Promise<Response> {
  const { pathname } = url;

  // GET /api/files/list?prefix=...
  if (pathname === "/api/files/list" && method === "GET") {
    return handleList(url, corsHeaders);
  }

  // GET /api/files/content?relPath=...
  if (pathname === "/api/files/content" && method === "GET") {
    return handleGet(url, corsHeaders);
  }

  // GET /api/files/raw?relPath=... (binary, for <img src> etc.)
  if (pathname === "/api/files/raw" && method === "GET") {
    return handleRaw(url, corsHeaders);
  }

  // PUT /api/files (body has relPath + bytesB64)
  if (pathname === "/api/files" && method === "PUT") {
    return handlePut(req, corsHeaders);
  }

  // DELETE /api/files (body has relPath)
  if (pathname === "/api/files" && method === "DELETE") {
    return handleDelete(req, corsHeaders);
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
  const prefix = url.searchParams.get("prefix") ?? "";
  const result = await solx.files.list(prefix);
  // The SDK currently returns the list as a single JSON-encoded string
  // (the wire format from solx-server is `[...]` JSON; the neon binding
  // surfaces that as a JS string instead of parsing it). Decode it
  // server-side so the frontend always receives a real string[].
  let paths: string[];
  if (typeof result === "string") {
    try {
      const parsed = JSON.parse(result);
      paths = Array.isArray(parsed)
        ? parsed.filter((p): p is string => typeof p === "string")
        : [];
    } catch {
      paths = [];
    }
  } else if (Array.isArray(result)) {
    paths = result.filter((p): p is string => typeof p === "string");
  } else {
    paths = [];
  }
  return okJson({ paths, prefix }, 200, corsHeaders);
}

async function handleGet(url: URL, corsHeaders: Record<string, string>): Promise<Response> {
  const solx = await getSolx();
  const relPath = url.searchParams.get("relPath");
  if (!relPath) {
    return errJson("invalid", "relPath query parameter is required", 400, corsHeaders);
  }
  const bytes = await solx.files.get(relPath);
  // The native binding returns a Uint8Array; convert to base64 for JSON.
  const bytesB64 = Buffer.from(bytes).toString("base64");
  return okJson({ relPath, bytesB64, size: bytes.length }, 200, corsHeaders);
}

/**
 * handleRaw — serve the file as binary with the right Content-Type so that
 * `<img src="/api/files/raw?relPath=…">` and Tiptap `<img>` nodes can render
 * the file directly. The content type is inferred from the file extension;
 * anything we don't recognize falls back to application/octet-stream so the
 * browser still offers a download.
 */
async function handleRaw(url: URL, corsHeaders: Record<string, string>): Promise<Response> {
  const solx = await getSolx();
  const relPath = url.searchParams.get("relPath");
  if (!relPath) {
    return errJson("invalid", "relPath query parameter is required", 400, corsHeaders);
  }
  const bytes = await solx.files.get(relPath);
  const body = Buffer.from(bytes);
  const contentType = contentTypeFromName(relPath);
  return new Response(body, {
    status: 200,
    headers: {
      ...corsHeaders,
      "Content-Type": contentType,
      "Content-Length": String(body.length),
      // Cheap cache: images don't change unless the user re-uploads them,
      // and the dev server runs on the same machine. 1h keeps reloads
      // snappy without ignoring edits forever.
      "Cache-Control": "public, max-age=3600",
    },
  });
}

function contentTypeFromName(name: string): string {
  const lower = name.toLowerCase();
  const slash = lower.lastIndexOf("/");
  const dot = lower.lastIndexOf(".");
  if (dot <= slash) return "application/octet-stream";
  const ext = lower.slice(dot + 1);
  switch (ext) {
    case "png": return "image/png";
    case "jpg":
    case "jpeg": return "image/jpeg";
    case "gif": return "image/gif";
    case "webp": return "image/webp";
    case "svg": return "image/svg+xml";
    case "avif": return "image/avif";
    case "bmp": return "image/bmp";
    case "ico": return "image/x-icon";
    case "pdf": return "application/pdf";
    case "json": return "application/json";
    case "wasm": return "application/wasm";
    case "html":
    case "htm": return "text/html; charset=utf-8";
    case "css": return "text/css; charset=utf-8";
    case "js":
    case "mjs": return "text/javascript; charset=utf-8";
    case "md":
    case "txt": return "text/plain; charset=utf-8";
    case "csv": return "text/csv; charset=utf-8";
    case "solx": return "text/x-solx; charset=utf-8";
    default: return "application/octet-stream";
  }
}

async function handlePut(req: Request, corsHeaders: Record<string, string>): Promise<Response> {
  const parsed = await readJson<{ relPath?: string; bytesB64?: string }>(req, corsHeaders);
  if (!parsed.ok) return parsed.response;

  const relPath = String(parsed.value.relPath ?? "").trim();
  const bytesB64 = String(parsed.value.bytesB64 ?? "");

  if (!relPath) {
    return errJson("invalid", "relPath is required", 400, corsHeaders);
  }
  if (!bytesB64) {
    return errJson("invalid", "bytesB64 is required", 400, corsHeaders);
  }

  let bytes: Uint8Array;
  try {
    bytes = Uint8Array.from(Buffer.from(bytesB64, "base64"));
  } catch (e) {
    return errJson("invalid", `bytesB64 is not valid base64: ${(e as Error).message}`, 400, corsHeaders);
  }

  const solx = await getSolx();
  const normalized = await solx.files.put(relPath, bytes);
  return okJson({ relPath: normalized }, 200, corsHeaders);
}

async function handleDelete(req: Request, corsHeaders: Record<string, string>): Promise<Response> {
  const parsed = await readJson<{ relPath?: string }>(req, corsHeaders);
  if (!parsed.ok) return parsed.response;

  const relPath = String(parsed.value.relPath ?? "").trim();
  if (!relPath) {
    return errJson("invalid", "relPath is required", 400, corsHeaders);
  }

  const solx = await getSolx();
  await solx.files.delete(relPath);
  return okJson({ ok: true }, 200, corsHeaders);
}
