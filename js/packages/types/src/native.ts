/**
 * TS-side wrapper around the `solx-bindings` Neon module.
 *
 * The Rust side exports flat, prefixed function names
 * (`typesOpen`, `typesSave`, `typesGet`, etc.) under
 * `require("@solx/<triple>")` (a per-platform npm package).
 * This module:
 *
 *   1. Loads the `.node` binary via `./loader.ts`.
 *   2. Wraps each primitive in a properly-typed async method.
 *   3. Translates snake_case DTOs from the Rust side to camelCase
 *      {@link TypeEntity} on the way out, and back on the way in.
 *   4. Catches `SolxError`-prefixed `Error` instances and rethrows
 *      them as real {@link SolxError} values.
 */

import { SolxError } from '@solx/surface';
import type { JsonObject, JsonValue } from '@solx/surface';
import type { ListOptions, Page, TypeEntity, TypeInput } from '@solx/surface';
import { loadNative } from './loader.js';
import type { NativeTypesModule, TypeManagerHandle } from './native-types.js';

export type { NativeTypesModule, TypeManagerHandle };

// ----- snake_case DTO mirror (Rust serde shape) -----

interface TypeEntitySnake {
  id: string;
  path: string;
  name: string;
  description?: string;
  schema: JsonObject;
  groups: string[];
  created_at: string;
  updated_at: string;
}

interface TypeInputSnake {
  description?: string;
  schema?: JsonObject;
  groups?: string[];
}

interface PageSnake<T> {
  items: T[];
  total: number;
  limit: number;
  offset: number;
}

function entityFromSnake(s: TypeEntitySnake): TypeEntity {
  return {
    id: s.id,
    path: s.path,
    name: s.name,
    description: s.description,
    schema: s.schema,
    groups: s.groups,
    createdAt: s.created_at,
    updatedAt: s.updated_at,
  };
}

function inputToSnake(i: TypeInput): TypeInputSnake {
  const out: TypeInputSnake = {};
  if (i.description !== undefined) out.description = i.description;
  if (i.schema !== undefined) out.schema = i.schema;
  if (i.groups !== undefined) out.groups = i.groups;
  return out;
}

function pageFromSnake<T, S>(s: PageSnake<S>, fromSnake: (x: S) => T): Page<T> {
  return {
    items: s.items.map(fromSnake),
    total: s.total,
    limit: s.limit,
    offset: s.offset,
  };
}

function listOptionsToJson(opts?: ListOptions): string {
  if (!opts) return '{}';
  const out: Record<string, JsonValue> = {};
  if (opts.pathPrefix !== undefined) out['path_prefix'] = opts.pathPrefix;
  if (opts.limit !== undefined) out['limit'] = opts.limit;
  if (opts.offset !== undefined) out['offset'] = opts.offset;
  if (opts.filterField !== undefined) out['filter_field'] = opts.filterField;
  if (opts.filterValue !== undefined) out['filter_value'] = opts.filterValue;
  if (opts.sortBy !== undefined) out['sort_by'] = opts.sortBy;
  if (opts.sortOrder !== undefined) out['sort_order'] = opts.sortOrder;
  if (opts.dateAfter !== undefined) out['date_after'] = opts.dateAfter;
  if (opts.dateBefore !== undefined) out['date_before'] = opts.dateBefore;
  return JSON.stringify(out);
}

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
      const json = await native.typesSave(
        this.handle,
        path,
        name,
        JSON.stringify(inputToSnake(input)),
      );
      return entityFromSnake(JSON.parse(json) as TypeEntitySnake);
    });
  }

  async get(path: string, name: string): Promise<TypeEntity> {
    return withSolxError(async () => {
      const native = loadNative();
      const json = await native.typesGet(this.handle, path, name);
      return entityFromSnake(JSON.parse(json) as TypeEntitySnake);
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
      const json = await native.typesList(this.handle, listOptionsToJson(opts));
      return pageFromSnake(
        JSON.parse(json) as PageSnake<TypeEntitySnake>,
        entityFromSnake,
      );
    });
  }

  async resolve(typeRef: string): Promise<TypeEntity> {
    return withSolxError(async () => {
      const native = loadNative();
      const json = await native.typesResolve(this.handle, typeRef);
      return entityFromSnake(JSON.parse(json) as TypeEntitySnake);
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
