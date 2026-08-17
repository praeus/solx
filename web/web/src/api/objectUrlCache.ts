/**
 * objectUrlCache.ts — ref-counted `blob:` URL cache for solx-files bytes.
 *
 * `solx-server` has no raw-byte-serving route (files travel as base64 JSON
 * only, see `solx-surface::wire::FileGetResponse`) and the token can't be
 * embedded in a plain `<img src>` URL the way the old Bun backend's
 * `/api/files/raw?relPath=…` could. So instead: fetch bytes through
 * `@solx/http`'s `FileStore.get`, wrap them in a `Blob` with a
 * best-guess `Content-Type`, and hand back a `URL.createObjectURL(...)`
 * result. Ref-counted so multiple components referencing the same
 * `relPath` (or repeated renders) share one fetch/blob and the URL is only
 * revoked once nothing references it anymore.
 *
 * Use via `useFileObjectUrl` (`hooks.ts`), not directly, outside of tests.
 */
import { getConnection } from "./connection";
import { contentTypeFromName } from "./contentType";

interface CacheEntry {
  url: string | null; // null while the fetch is still in flight
  refCount: number;
  promise: Promise<string>;
}

const cache = new Map<string, CacheEntry>();

async function buildObjectUrl(relPath: string): Promise<string> {
  const bytes = await getConnection().files.get(relPath);
  const blob = new Blob([bytes as BlobPart], { type: contentTypeFromName(relPath) });
  return URL.createObjectURL(blob);
}

/** Resolve `relPath` to a `blob:` URL, incrementing its reference count. */
export function acquireObjectUrl(relPath: string): Promise<string> {
  const existing = cache.get(relPath);
  if (existing) {
    existing.refCount++;
    return existing.promise;
  }

  const promise = buildObjectUrl(relPath).then((url) => {
    const entry = cache.get(relPath);
    if (entry) entry.url = url;
    return url;
  });
  // Don't cache a failed fetch — the next acquire should retry.
  promise.catch(() => cache.delete(relPath));

  cache.set(relPath, { url: null, refCount: 1, promise });
  return promise;
}

/** Decrement `relPath`'s reference count, revoking the `blob:` URL at zero. */
export function releaseObjectUrl(relPath: string): void {
  const entry = cache.get(relPath);
  if (!entry) return;
  entry.refCount--;
  if (entry.refCount > 0) return;

  cache.delete(relPath);
  if (entry.url) {
    URL.revokeObjectURL(entry.url);
  } else {
    // Still resolving when the last consumer released — revoke once it lands.
    entry.promise.then((url) => URL.revokeObjectURL(url)).catch(() => {});
  }
}
