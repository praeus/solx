/**
 * Ported verbatim from the old Bun backend's `server/src/routes/files.ts`
 * (`contentTypeFromName`) — used by `objectUrlCache.ts` to give a fetched
 * `Blob` the right MIME type, since `solx-server` serves file bytes as
 * base64 JSON with no `Content-Type` of its own.
 */
export function contentTypeFromName(name: string): string {
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
