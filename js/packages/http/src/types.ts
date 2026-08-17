/**
 * Pure-fetch `TypeManager` — HTTP proxy for `solx-server`, mirroring
 * `solx-client::RemoteTypeManager` (solx-core/solx-client/src/types.rs)
 * route-for-route. Same snake_case<->camelCase conversion pattern as
 * `@solx/types`'s Neon-backed `native.ts`, but talks `fetch`/JSON directly
 * instead of going through a native binding.
 */
import type {
  JsonValue,
  ListOptions,
  Page,
  TypeEntity,
  TypeInput,
  TypeManager as TypeManagerIface,
} from '@solx/surface';
import { postJson } from './fetch.js';

interface TypeEntitySnake {
  id: string;
  path: string;
  name: string;
  description?: string;
  schema: Record<string, JsonValue>;
  groups: string[];
  created_at: string;
  updated_at: string;
}

interface TypeInputSnake {
  description?: string;
  schema: Record<string, JsonValue>;
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
  const out: TypeInputSnake = { schema: i.schema };
  if (i.description !== undefined) out.description = i.description;
  if (i.groups !== undefined) out.groups = i.groups;
  return out;
}

function pageFromSnake<T, S>(s: PageSnake<S>, fromSnake: (x: S) => T): Page<T> {
  return { items: s.items.map(fromSnake), total: s.total, limit: s.limit, offset: s.offset };
}

function listOptionsToSnake(opts?: ListOptions): Record<string, JsonValue> {
  if (!opts) return {};
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
  return out;
}

/**
 * HTTP-proxy `TypeManager` talking to a `solx-server`. Public name matches
 * `@solx/types`'s `TypeManager` and the `solx_surface::managers::TypeManager`
 * trait — consumers should treat it opaquely, same convention as the Neon
 * binding.
 */
export class HttpTypeManager implements TypeManagerIface {
  constructor(
    private readonly baseUrl: string,
    private readonly token: string,
  ) {}

  async save(path: string, name: string, input: TypeInput): Promise<TypeEntity> {
    const s = await postJson<unknown, TypeEntitySnake>(this.baseUrl, this.token, '/types/save', {
      path,
      name,
      input: inputToSnake(input),
    });
    return entityFromSnake(s);
  }

  async get(path: string, name: string): Promise<TypeEntity> {
    const s = await postJson<unknown, TypeEntitySnake>(this.baseUrl, this.token, '/types/get', {
      path,
      name,
    });
    return entityFromSnake(s);
  }

  async delete(path: string, name: string): Promise<void> {
    await postJson<unknown, unknown>(this.baseUrl, this.token, '/types/delete', { path, name });
  }

  async list(opts?: ListOptions): Promise<Page<TypeEntity>> {
    const s = await postJson<unknown, PageSnake<TypeEntitySnake>>(
      this.baseUrl,
      this.token,
      '/types/list',
      listOptionsToSnake(opts),
    );
    return pageFromSnake(s, entityFromSnake);
  }

  async resolve(typeRef: string): Promise<TypeEntity> {
    const s = await postJson<unknown, TypeEntitySnake>(
      this.baseUrl,
      this.token,
      '/types/resolve',
      { type_ref: typeRef },
    );
    return entityFromSnake(s);
  }

  async validate(value: JsonValue, typeRef: string): Promise<void> {
    await postJson<unknown, unknown>(this.baseUrl, this.token, '/types/validate', {
      value,
      type_ref: typeRef,
    });
  }
}
