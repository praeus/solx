/**
 * The official package catalog over IPC (solx-tauri-plugin's `packages_*`
 * commands, permission set `solx:packages`). Packages are named, never
 * located: the host resolves each name to a zip in the configured release,
 * so a window holding this can only install packages from the catalog.
 *
 * Keys are snake_case, as `solx_packages::catalog` serializes them.
 */
import type { TauriTransport } from './invoke.js';

/** Where a catalog package stands on this host. */
export type CatalogEntryState =
  | 'installed'
  | 'update_available'
  | 'available'
  | 'not_released'
  | 'unsupported_platform';

export interface CatalogEntry {
  name: string;
  title: string;
  description: string;
  /** Shown first: many other packages build on these. */
  recommended: boolean;
  /** Installed automatically before this one. */
  requires: string[];
  /** Ships a native binary, so its zip is per platform. */
  native: boolean;
  state: CatalogEntryState;
  installed_version: string | null;
  available_version: string | null;
  /** Download size in bytes, when the release has it. */
  size: number | null;
}

export interface CatalogView {
  /** e.g. `praeus/solx-packages (latest release)` or `local folder …`. */
  source: string;
  /** `windows-x64`, `linux-x64` or `macos-arm64`; null where no native zips are built. */
  platform: string | null;
  tag: string | null;
  /** Why nothing is offered, e.g. no published release yet. */
  message: string | null;
  entries: CatalogEntry[];
}

export interface PackageOpResult {
  catalog: CatalogView;
  /** Required packages installed along the way, allowlist collisions, dependents left behind. */
  warnings: string[];
}

export class TauriPackages {
  constructor(private readonly t: TauriTransport) {}

  /** The catalog; `refresh` skips the host's few-minute cache of the release listing. */
  catalog(refresh = false): Promise<CatalogView> {
    return this.t.call('packages_catalog', { refresh });
  }

  /** Install a catalog package (and whatever it requires) from the release. */
  install(name: string): Promise<PackageOpResult> {
    return this.t.call('packages_install', { name });
  }

  uninstall(name: string): Promise<PackageOpResult> {
    return this.t.call('packages_uninstall', { name });
  }
}
