/**
 * solx-web — Bun backend entry point.
 *
 * Loads the solx-js SDK, connects to the running solx-server, and exposes
 * a clean REST API for the React frontend.
 *
 * Endpoints:
 *   GET    /api/health
 *   GET    /api/documents
 *   GET    /api/documents/search
 *   GET    /api/documents/:path/:name
 *   POST   /api/documents
 *   DELETE /api/documents/:path/:name
 *   GET    /api/actions
 *   GET    /api/actions/:path/:name
 *   POST   /api/actions
 *   DELETE /api/actions/:path/:name
 *   POST   /api/actions/:path/:name/exec
 *   GET    /api/types
 *   GET    /api/types/:path/:name
 *   POST   /api/types
 *   DELETE /api/types/:path/:name
 *   GET    /api/files/list
 *   GET    /api/files/content
 *   PUT    /api/files
 *   DELETE /api/files
 *   GET    /api/config
 *   PATCH  /api/config
 */

import { mountDocumentsRoutes } from "./routes/documents";
import { mountActionsRoutes } from "./routes/actions";
import { mountTypesRoutes } from "./routes/types";
import { mountFilesRoutes } from "./routes/files";
import { mountConfigRoutes } from "./routes/config";
import { mountHealthRoutes } from "./routes/health";
import { getSolx, tryGetSolx } from "./solx-context";

const PORT = Number(process.env.PORT ?? 3001);

Bun.serve({
  port: PORT,
  async fetch(req: Request): Promise<Response> {
    const url = new URL(req.url);
    const { pathname } = url;
    const method = req.method.toUpperCase();

    // CORS for development (Vite dev server on a different port)
    const corsHeaders = {
      "Access-Control-Allow-Origin": "*",
      "Access-Control-Allow-Methods": "GET, POST, PUT, PATCH, DELETE, OPTIONS",
      "Access-Control-Allow-Headers": "Content-Type, Authorization",
      "Access-Control-Max-Age": "86400",
    };

    if (method === "OPTIONS") {
      return new Response(null, { status: 204, headers: corsHeaders });
    }

    // Health check is always available (no solx-js required for the liveness ping)
    if (pathname === "/api/health" && method === "GET") {
      return mountHealthRoutes(req);
    }

    // All other routes require a working solx-js connection.
    // Try a non-blocking read so the frontend can surface a connection error
    // instead of the request hanging.
    const solx = tryGetSolx();
    if (!solx) {
      // Attempt to initialize lazily; if it fails, surface as 503.
      try {
        await getSolx();
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        return json(
          { error: { kind: "config", message: `solx-js unavailable: ${message}` } },
          503,
          corsHeaders,
        );
      }
    }

    try {
      const response = await route(req, url, method, corsHeaders);
      if (response) return response;
    } catch (err) {
      return mapError(err, corsHeaders);
    }

    return json({ error: { kind: "not_found", message: `No route for ${method} ${pathname}` } }, 404, corsHeaders);
  },
  error(err: Error): Response {
    console.error("[server] unhandled error:", err);
    return json({ error: { kind: "other", message: err.message } }, 500, {});
  },
});

async function route(
  req: Request,
  url: URL,
  method: string,
  corsHeaders: Record<string, string>,
): Promise<Response | null> {
  if (url.pathname.startsWith("/api/documents")) {
    return mountDocumentsRoutes(req, url, method, corsHeaders);
  }
  if (url.pathname.startsWith("/api/actions")) {
    return mountActionsRoutes(req, url, method, corsHeaders);
  }
  if (url.pathname.startsWith("/api/types")) {
    return mountTypesRoutes(req, url, method, corsHeaders);
  }
  if (url.pathname.startsWith("/api/files")) {
    return mountFilesRoutes(req, url, method, corsHeaders);
  }
  if (url.pathname.startsWith("/api/config")) {
    return mountConfigRoutes(req, url, method, corsHeaders);
  }
  return null;
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

function mapError(err: unknown, corsHeaders: Record<string, string>): Response {
  if (err && typeof err === "object" && "kind" in err) {
    const solxErr = err as { kind?: string; message?: string };
    const kind = String(solxErr.kind ?? "other");
    const message = String(solxErr.message ?? "Unknown error");
    const status = statusForKind(kind);
    return json({ error: { kind, message } }, status, corsHeaders);
  }
  const message = err instanceof Error ? err.message : String(err);
  return json({ error: { kind: "other", message } }, 500, corsHeaders);
}

function statusForKind(kind: string): number {
  switch (kind) {
    case "not_found": return 404;
    case "invalid": return 400;
    case "validation": return 422;
    case "conflict": return 409;
    case "io":
    case "db":
    case "exec":
    case "config":
    case "other":
    default: return 500;
  }
}

console.log(`[solx-web] backend listening on http://localhost:${PORT}`);
console.log(`[solx-web] env: SOLX_SERVER_URL=${process.env.SOLX_SERVER_URL ?? "(default)"}`);
