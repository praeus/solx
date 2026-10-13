/**
 * TS-side wrapper around the `solx-bindings` Neon module.
 *
 * The Rust side exports flat, prefixed function names
 * (`typesOpen`, `typesSave`, `typesGet`, etc.) under
 * `require("@solx/<triple>")` (a per-platform npm package).
 * This module:
 *
 *   1. Loads the `.node` binary via `./loader.ts`.
 *   2. Wraps each primitive in a properly-typed async method. The Neon
 *      boundary marshals the same `solx_surface::entities::TypeEntity`
 *      struct as `solx-server`'s HTTP responses, so the JSON crossing it is
 *      camelCase already — no DTO conversion needed on this side.
 *   3. Catches `SolxError`-prefixed `Error` instances and rethrows
 *      them as real {@link SolxError} values.
 */

import { SolxError } from '@solx/surface';
import type { JsonValue } from '@solx/surface';
import type { ListOptions, Page, TypeEntity, TypeInput } from '@solx/surface';
import { loadNative } from './loader.js';
import type { NativeTypesModule, TypeManagerHandle } from './native-types.js';

export type { NativeTypesModule, TypeManagerHandle };

async function withSolxError<T>(fn: () => Promise<T>): Promise<T> {
  try {
    return await fn();
  } catch (e) {
    throw SolxError.fromMessage(e);
  }
}

// ----- TypeManager -----
//
// Public name is just `TypeManager` (matches the
// `solx_surface::managers::TypeManager` trait + the
// `@solx/surface` `TypeManager` interface). The class is
// the concrete handle; the "local" / "client" choice lives
// at the native binary level (Cargo feature), not in the TS
// surface. Consumers should treat the class opaquely.

export class TypeManager {
  private constructor(private readonly handle: TypeManagerHandle) {}

  static async open(dbPath: string): Promise<TypeManager> {
    return withSolxError(async () => {
      const native = loadNative();
      const handle = native.typesOpen(dbPath);
      return new TypeManager(handle);
    });
  }

  /** Connect to a remote `solx-server` instead of opening a local database. */
  static async connect(serverUrl: string, token: string): Promise<TypeManager> {
    return withSolxError(async () => {
      const native = loadNative();
      const handle = native.typesConnect(serverUrl, token);
      return new TypeManager(handle);
    });
  }

  /**
   * @internal Escape hatch for dependent managers' `open()` (e.g.
   * `DocManager.open` needs this handle to construct its own local
   * `LocalDocManager`). Not part of the public API — do not call
   * from application code.
   */
  static handleOf(mgr: TypeManager): TypeManagerHandle {
    return mgr.handle;
  }

  async save(path: string, name: string, input: TypeInput): Promise<TypeEntity> {
    return withSolxError(async () => {
      const native = loadNative();
      const json = await native.typesSave(this.handle, path, name, JSON.stringify(input));
      return JSON.parse(json) as TypeEntity;
    });
  }

  async get(path: string, name: string): Promise<TypeEntity> {
    return withSolxError(async () => {
      const native = loadNative();
      const json = await native.typesGet(this.handle, path, name);
      return JSON.parse(json) as TypeEntity;
    });
  }

  async delete(path: string, name: string): Promise<void> {
    return withSolxError(async () => {
      const native = loadNative();
      await native.typesDelete(this.handle, path, name);
    });
  }

  async list(opts?: ListOptions): Promise<Page<TypeEntity>> {
    return withSolxError(async () => {
      const native = loadNative();
      const json = await native.typesList(this.handle, JSON.stringify(opts ?? {}));
      return JSON.parse(json) as Page<TypeEntity>;
    });
  }

  async resolve(typeRef: string): Promise<TypeEntity> {
    return withSolxError(async () => {
      const native = loadNative();
      const json = await native.typesResolve(this.handle, typeRef);
      return JSON.parse(json) as TypeEntity;
    });
  }

  async validate(value: JsonValue, typeRef: string): Promise<void> {
    return withSolxError(async () => {
      const native = loadNative();
      await native.typesValidate(
        this.handle,
        JSON.stringify(value),
        typeRef,
      );
    });
  }
}
