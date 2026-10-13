/**
 * Pure-fetch `FileStore` — HTTP proxy for `solx-server`, mirroring
 * `solx-client::RemoteFileStore` (solx-core/solx-client/src/files.rs).
 * File content is transferred as raw bytes on the file's own URL, so
 * nothing is base64-encoded and a `get` is a plain download.
 */
import type { FileStore as FileStoreIface } from '@solx/surface';
import { nestedPath, requestBytes, requestJson } from './fetch.js';

interface FilePutResponse {
  relPath: string;
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
    const resp = await requestJson<FilePutResponse>(
      this.baseUrl,
      this.token,
      'PUT',
      nestedPath('files', relPath),
      { bytes },
    );
    return resp.relPath;
  }

  async get(relPath: string): Promise<Uint8Array> {
    return requestBytes(this.baseUrl, this.token, 'GET', nestedPath('files', relPath));
  }

  async delete(relPath: string): Promise<void> {
    await requestJson<void>(this.baseUrl, this.token, 'DELETE', nestedPath('files', relPath));
  }

  async list(prefix: string): Promise<string[]> {
    const resp = await requestJson<FileListResponse>(this.baseUrl, this.token, 'GET', '/files', {
      query: { prefix },
    });
    return resp.paths;
  }
}
