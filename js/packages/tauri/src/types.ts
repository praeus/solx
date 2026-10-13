/**
 * IPC `TypeManager` — `plugin:solx|types_*`, mirroring `@solx/http`'s
 * `HttpTypeManager` method-for-method.
 */
import type {
  EntityOptions,
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

  save(path: string, name: string, input: TypeInput, opts?: EntityOptions): Promise<TypeEntity> {
    return this.t.call('types_save', { path, name, input, excludeMode: opts?.excludeMode });
  }

  get(path: string, name: string, opts?: EntityOptions): Promise<TypeEntity> {
    return this.t.call('types_get', { path, name, excludeMode: opts?.excludeMode });
  }

  async delete(path: string, name: string, opts?: EntityOptions): Promise<void> {
    await this.t.call('types_delete', { path, name, excludeMode: opts?.excludeMode });
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
