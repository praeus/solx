/**
 * Pure-fetch `FileStore` — HTTP proxy for `solx-server`, mirroring
 * `solx-client::RemoteFileStore` (solx-core/solx-client/src/files.rs).
 * Bytes are base64-encoded over the wire (JSON has no native binary type),
 * matching `solx-surface::wire::{FilePutRequest,FileGetResponse}`.
 */
import type { FileStore as FileStoreIface } from '@solx/surface';
import { postJson } from './fetch.js';

// ----- base64 helpers (browser + Node compatible, no `node:buffer` import
// so this package stays bundler-safe for a browser target) -----

function bytesToBase64(bytes: Uint8Array): string {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const toBase64 = (Uint8Array as any).prototype?.toBase64;
  if (typeof toBase64 === 'function') {
    return toBase64.call(bytes, { alphabet: 'base64' });
  }
  let binary = '';
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode.apply(
      null,
      Array.from(bytes.subarray(i, i + chunk)) as number[],
    );
  }
  return btoa(binary);
}

function base64ToBytes(b64: string): Uint8Array {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const fromBase64 = (Uint8Array as any).fromBase64;
  if (typeof fromBase64 === 'function') {
    return fromBase64.call(Uint8Array, b64, { alphabet: 'base64' });
  }
  const binary = atob(b64);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    out[i] = binary.charCodeAt(i);
  }
  return out;
}

interface FilePutResponse {
  rel_path: string;
}

interface FileGetResponse {
  bytes_b64: string;
}

interface FileListResponse {
  paths: string[];
}

/**
 * HTTP-proxy `FileStore` talking to a `solx-server`. Public name matches
 * `@solx/files`'s `FileStore` and the `solx_surface::managers::FileStore`
 * trait.
 */
export class HttpFileStore implements FileStoreIface {
  constructor(
    private readonly baseUrl: string,
    private readonly token: string,
  ) {}

  async put(relPath: string, bytes: Uint8Array): Promise<string> {
    const resp = await postJson<unknown, FilePutResponse>(
      this.baseUrl,
      this.token,
      '/files/put',
      { rel_path: relPath, bytes_b64: bytesToBase64(bytes) },
    );
    return resp.rel_path;
  }

  async get(relPath: string): Promise<Uint8Array> {
    const resp = await postJson<unknown, FileGetResponse>(
      this.baseUrl,
      this.token,
      '/files/get',
      { rel_path: relPath },
    );
    return base64ToBytes(resp.bytes_b64);
  }

  async delete(relPath: string): Promise<void> {
    await postJson<unknown, unknown>(this.baseUrl, this.token, '/files/delete', {
      rel_path: relPath,
    });
  }

  async list(prefix: string): Promise<string[]> {
    const resp = await postJson<unknown, FileListResponse>(
      this.baseUrl,
      this.token,
      '/files/list',
      { prefix },
    );
    return resp.paths;
  }
}
