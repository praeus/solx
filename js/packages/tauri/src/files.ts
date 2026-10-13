/**
 * IPC `FileStore` — `plugin:solx|files_*`. Content travels as raw bytes
 * both ways (Tauri's raw IPC body and `ipc::Response`), never JSON-encoded,
 * matching `@solx/http`'s raw-body file routes.
 */
import type { FileStore as FileStoreIface } from '@solx/surface';
import type { TauriTransport } from './invoke.js';

/** Must match `FILE_PATH_HEADER` in solx-tauri-plugin's `commands.rs`. */
const FILE_PATH_HEADER = 'x-solx-path';

export class TauriFileStore implements FileStoreIface {
  constructor(private readonly t: TauriTransport) {}

  /**
   * The body is the bytes themselves, so the path rides in a header —
   * percent-encoded, since header values must be ASCII.
   */
  put(relPath: string, bytes: Uint8Array): Promise<string> {
    return this.t.call('files_put', bytes, {
      headers: { [FILE_PATH_HEADER]: encodeURIComponent(relPath) },
    });
  }

  get(relPath: string): Promise<Uint8Array> {
    return this.t.callBytes('files_get', { relPath });
  }

  async delete(relPath: string): Promise<void> {
    await this.t.call('files_delete', { relPath });
  }

  list(prefix: string): Promise<string[]> {
    return this.t.call('files_list', { prefix });
  }
}
