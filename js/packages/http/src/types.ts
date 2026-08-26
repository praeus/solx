/**
 * Pure-fetch `TypeManager` — HTTP proxy for `solx-server`, mirroring
 * `solx-client::RemoteTypeManager` (solx-core/solx-client/src/types.rs)
 * route-for-route. The wire format is camelCase end to end (same as
 * `@solx/surface`'s public types), so request/response bodies pass straight
 * through with no conversion layer.
 */
import type {
  JsonValue,
  ListOptions,
  Page,
  TypeEntity,
  TypeInput,
  TypeManager as TypeManagerIface,
} from '@solx/surface';
import { Ref } from '@solx/surface';
import { entityPath, requestJson, toQuery } from './fetch.js';

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
    return requestJson<TypeEntity>(this.baseUrl, this.token, 'PUT', entityPath('types', path, name), {
      body: input,
    });
  }

  async get(path: string, name: string): Promise<TypeEntity> {
    return requestJson<TypeEntity>(this.baseUrl, this.token, 'GET', entityPath('types', path, name));
  }

  async delete(path: string, name: string): Promise<void> {
    await requestJson<void>(this.baseUrl, this.token, 'DELETE', entityPath('types', path, name));
  }

  async list(opts?: ListOptions): Promise<Page<TypeEntity>> {
    return requestJson<Page<TypeEntity>>(this.baseUrl, this.token, 'GET', '/types', {
      query: toQuery(opts),
    });
  }

  /**
   * Resolved client-side: `resolve` is defined as a ref split plus a `get`,
   * which `GET /types/{*ref}` already is — so there's no route for it.
   */
  async resolve(typeRef: string): Promise<TypeEntity> {
    const { path, name } = Ref.split(typeRef);
    return this.get(path, name);
  }

  /**
   * `POST /validate` — top-level for the same reason document search is; see
   * `HttpDocManager.search`. Answers `204` on success, `422` on failure.
   */
  async validate(value: JsonValue, typeRef: string): Promise<void> {
    await requestJson<void>(this.baseUrl, this.token, 'POST', '/validate', {
      body: { value, typeRef },
    });
  }
}
