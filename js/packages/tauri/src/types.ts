/**
 * IPC `TypeManager` — `plugin:solx|types_*`, mirroring `@solx/http`'s
 * `HttpTypeManager` method-for-method.
 */
import type {
  JsonValue,
  ListOptions,
  Page,
  PathFacet,
  TypeEntity,
  TypeInput,
  TypeManager as TypeManagerIface,
} from '@solx/surface';
import type { TauriTransport } from './invoke.js';

export class TauriTypeManager implements TypeManagerIface {
  constructor(private readonly t: TauriTransport) {}

  save(path: string, name: string, input: TypeInput): Promise<TypeEntity> {
    return this.t.call('types_save', { path, name, input });
  }

  get(path: string, name: string): Promise<TypeEntity> {
    return this.t.call('types_get', { path, name });
  }

  async delete(path: string, name: string): Promise<void> {
    await this.t.call('types_delete', { path, name });
  }

  list(opts?: ListOptions): Promise<Page<TypeEntity>> {
    return this.t.call('types_list', { opts });
  }

  paths(opts?: ListOptions): Promise<Page<PathFacet>> {
    return this.t.call('types_paths', { opts });
  }

  resolve(typeRef: string): Promise<TypeEntity> {
    return this.t.call('types_resolve', { typeRef });
  }

  async validate(value: JsonValue, typeRef: string): Promise<void> {
    await this.t.call('types_validate', { value, typeRef });
  }
}
