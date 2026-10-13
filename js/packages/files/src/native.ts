/**
 * TS-side wrapper around the `solx-bindings` files module.
 *
 * The Rust side exposes flat, prefixed function names
 * (`filesNew`, `filesPut`, `filesGet`, etc.) under
 * `require("@solx/<triple>")`. This module:
 *
 *   1. Loads the `.node` binary via the shared loader.
 *   2. Encodes `Uint8Array` to base64 on the way in
 *      (`put`) and decodes base64 to `Uint8Array` on the
 *      way out (`get`).
 *   3. Catches `SolxError`-prefixed `Error` instances and
 *      rethrows them as real `SolxError` values.
 *
 * The public class is just `FileStore` — not `LocalFileStore`
 * — because "local" is an implementation detail. The same
 * class shape will wrap a future client-backed binding; the
 * native binary decides which impl to load via Cargo
 * features (`local` vs `client`). Consumers should not
 * branch on which one they got.
 */

import { Buffer } from 'node:buffer';
import { SolxError } from '@solx/surface';
import { loadNative } from '@solx/types/loader';
import type { NativeFilesModule, FileStoreHandle } from './native-types.js';

export type { NativeFilesModule, FileStoreHandle };

// ----- base64 helpers (browser + node compatible) -----

/**
 * Encode a `Uint8Array` to a base64 string. We use the
 * `Uint8Array.toBase64` static method if available
 * (Node 16+, modern browsers), falling back to a manual
 * `btoa`/`Buffer` path.
 */
function bytesToBase64(bytes: Uint8Array): string {
  // Prefer the built-in static method when present.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const ctor = (Uint8Array as any).toBase64;
  if (typeof ctor === 'function') {
    // toBase64 takes an options bag with alphabet on some impls;
    // we want standard base64.
    return ctor.call(Uint8Array, bytes, { alphabet: 'standard' });
  }
  // Node Buffer fallback.
  if (typeof Buffer !== 'undefined') {
    return Buffer.from(bytes).toString('base64');
  }
  // Browser fallback: chunked btoa.
  let binary = '';
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode.apply(
      null,
      Array.from(bytes.subarray(i, i + chunk)) as number[],
    );
  }
  // eslint-disable-next-line no-undef
  return btoa(binary);
}

function base64ToBytes(b64: string): Uint8Array {
  if (typeof Buffer !== 'undefined') {
    return new Uint8Array(Buffer.from(b64, 'base64'));
  }
  // eslint-disable-next-line no-undef
  const binary = atob(b64);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    out[i] = binary.charCodeAt(i);
  }
  return out;
}

async function withSolxError<T>(fn: () => Promise<T>): Promise<T> {
  try {
    return await fn();
  } catch (e) {
    throw SolxError.fromMessage(e);
  }
}

// ----- FileStore -----
//
// Public name is just `FileStore` (matches the
// `solx_surface::managers::FileStore` trait + the
// `@solx/surface` `FileStore` interface). The class is the
// concrete handle; the "local" / "client" choice lives at
// the native binary level (Cargo feature), not in the TS
// surface. Consumers should treat the class opaquely.

/**
 * Concrete file store. Bytes are addressed by forward-slash
 * relative paths under the store's root
 * (e.g. `files/docs/<doc_id>/diagram.png`). Path traversal
 * (`..`) and backslashes are rejected by the native side.
 *
 * Obtained via {@link FileStore.open} or via
 * {@link createSolx} (the umbrella factory, which wires
 * paths from the config service).
 */
export class FileStore {
  private constructor(
    private readonly handle: FileStoreHandle,
    /** Resolved at construction time. */
    private readonly rootPath: string,
  ) {}

  /**
   * Open a file store rooted at the given directory. The
   * directory is created on first `put` if it does not
   * already exist.
   */
  static async open(root: string): Promise<FileStore> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeFilesModule;
      const handle = native.filesNew(root);
      return new FileStore(handle, root.replace(/\\/g, '/'));
    });
  }

  /**
   * Connect to a remote `solx-server` instead of opening a local
   * directory. {@link root} returns `serverUrl` for a connected
   * instance — there is no meaningful on-disk root.
   */
  static async connect(serverUrl: string, token: string): Promise<FileStore> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeFilesModule;
      const handle = native.filesConnect(serverUrl, token);
      return new FileStore(handle, serverUrl);
    });
  }

  /**
   * @internal Escape hatch for dependent managers' `open()`. Not
   * part of the public API — do not call from application code.
   */
  static handleOf(store: FileStore): FileStoreHandle {
    return store.handle;
  }

  /**
   * Write `bytes` to `relPath`. Returns the normalized rel path
   * (forward-slash separated, no leading slash).
   */
  async put(relPath: string, bytes: Uint8Array): Promise<string> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeFilesModule;
      // The Rust side serializes the returned `String` as JSON,
      // so the wire format is `"the path"` (with surrounding
      // quotes). `JSON.parse` unwraps it.
      const result = await native.filesPut(this.handle, relPath, bytesToBase64(bytes));
      return JSON.parse(result) as string;
    });
  }

  /** Read the bytes at `relPath`. Throws `SolxError('NotFound')` if absent. */
  async get(relPath: string): Promise<Uint8Array> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeFilesModule;
      const b64 = await native.filesGet(this.handle, relPath);
      return base64ToBytes(b64);
    });
  }

  /** Delete the file at `relPath`. Idempotent (no error if missing). */
  async delete(relPath: string): Promise<void> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeFilesModule;
      await native.filesDelete(this.handle, relPath);
    });
  }

  /** List every file under `prefix`, returned as sorted rel paths. */
  async list(prefix: string): Promise<string[]> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeFilesModule;
      const json = await native.filesList(this.handle, prefix);
      return JSON.parse(json) as string[];
    });
  }

  /**
   * The on-disk root this store reads/writes from.
   *
   * Captured at construction time — the binding no longer
   * exposes a `filesRoot` primitive because that would
   * require downcasting the trait object back to the
   * concrete `LocalFileStore` (which a `client` impl
   * wouldn't be). The TS layer remembers the path it was
   * opened with.
   */
  root(): string {
    return this.rootPath;
  }
}

// ----- conventional relative-path builders -----
//
// Mirror `solx_files::doc_file_path` etc. so the TS side can
// produce the same path strings the Rust side expects when
// building file references on `Document` / `Action` rows.

function sanitizeSegment(seg: string): string {
  let out = '';
  for (const c of seg) {
    if (
      (c >= 'A' && c <= 'Z') ||
      (c >= 'a' && c <= 'z') ||
      (c >= '0' && c <= '9') ||
      c === '.' ||
      c === '-' ||
      c === '_'
    ) {
      out += c;
    } else {
      out += '_';
    }
  }
  return out;
}

export const FilePath = {
  docFilePath(docId: string, fileName: string): string {
    return `files/docs/${sanitizeSegment(docId)}/${sanitizeSegment(fileName)}`;
  },
  sharedDocFilePath(fileName: string): string {
    return `files/docs/shared/${sanitizeSegment(fileName)}`;
  },
  actionFilePath(actionId: string, fileName: string): string {
    return `files/actions/${sanitizeSegment(actionId)}/${sanitizeSegment(fileName)}`;
  },
  sharedActionFilePath(fileName: string): string {
    return `files/actions/shared/${sanitizeSegment(fileName)}`;
  },
  sanitizeSegment,
} as const;
