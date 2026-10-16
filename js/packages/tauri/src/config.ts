/**
 * `solx-config.json` over IPC — `plugin:solx|config_*`. Unlike the
 * managers, config has no HTTP route at all: these commands need the
 * separate `solx:config` permission, which the desktop app grants only to
 * its Settings window (never the main window, where package widgets run).
 *
 * Types are snake_case, exactly as the file and the commands carry them —
 * mirroring `solx-config/src/types.rs`. (`@solx/surface`'s camelCase
 * `SolxConfig` is a partial mirror for the Neon binding, not this.)
 */
import type { TauriTransport } from './invoke.js';

/** An allowlisted shell command a `Command` action may run, by key. */
export interface CommandDef {
  command: string;
  description?: string;
  cwd?: string;
}

/** A path-glob rule for `tool_exclude` / `tool_destructive`. */
export interface ToolRule {
  /** Glob on the action's full path: `*` spans `/`, `?` is one character. */
  path: string;
  /** Only these action names at the path; unset/empty means all of them. */
  actions?: string[];
}

/**
 * Exclusion patterns for one scope, by entity kind. A pattern is a glob
 * (`*` spans `/`, `?` is one character) matched against an entity's path or
 * its full reference: `/a/b` hides entities in `/a/b` and the entity `b` in
 * `/a`; `/a/b*` also hides everything beneath.
 */
export interface ExclusionLists {
  documents?: string[];
  types?: string[];
  actions?: string[];
}

/** `exclusions`: two independent scopes. */
export interface Exclusions {
  /** Hidden from the web UI and REST/Tauri clients. */
  user?: ExclusionLists;
  /** Hidden from models: MCP, the agent harness. Unioned with `tool_exclude` for actions. */
  tooling?: ExclusionLists;
}

export interface InstalledPackage {
  name: string;
  version: string;
  path: string;
  installed_at: string;
  granted_commands: string[];
  granted_base_urls: string[];
}

/** `solx-config.json` as stored. Every key is optional; unknown keys survive edits. */
export interface SolxConfigFile {
  data_directory?: string;
  files_directory?: string;
  packages_directory?: string;
  models_directory?: string;
  search_index_dir?: string;
  docs_db?: string;
  actions_db?: string;
  types_db?: string;
  installed_packages?: InstalledPackage[];
  env_mappings?: Record<string, string>;
  env_vars?: Record<string, Record<string, string>>;
  server_url?: string;
  server_token?: string;
  server_port?: number;
  server_host?: string;
  server_cors_origins?: string[];
  /** The address other machines use to reach this server, for connect links. */
  server_public_url?: string;
  /** Lifetime of a client token, in days (default 90). */
  client_token_ttl_days?: number;
  server_embedded?: boolean;
  console_max_entries?: number;
  console_ttl_days?: number;
  background_timeout_secs?: number;
  stop_grace_secs?: number;
  invocation_ttl_days?: number;
  http_stream_idle_ttl_secs?: number;
  http_stream_max_buffer_bytes?: number;
  command_actions?: Record<string, CommandDef>;
  allowed_base_urls?: string[];
  /** Former name of `allowed_base_urls`; still read, folded away on write. */
  allowed_webhook_base_urls?: string[];
  tool_exclude?: ToolRule[];
  /** Former name of `tool_exclude`; still read, folded away on write. */
  mcp_exclude?: ToolRule[];
  tool_destructive?: ToolRule[];
  exclusions?: Exclusions;
  [key: string]: unknown;
}

/** What the host resolves unset keys to (its defaults), for placeholders. */
export interface EffectiveConfig {
  data_directory: string;
  files_directory: string;
  packages_directory: string;
  docs_db: string | null;
  actions_db: string | null;
  types_db: string | null;
  server_host: string;
  server_port: number;
  server_embedded: boolean;
  client_token_ttl_days: number;
  console_max_entries: number;
  console_ttl_days: number;
  background_timeout_secs: number;
  stop_grace_secs: number;
  invocation_ttl_days: number;
  http_stream_idle_ttl_secs: number;
  http_stream_max_buffer_bytes: number;
}

export interface ConfigView {
  raw: SolxConfigFile;
  effective: EffectiveConfig;
  /** Path of `solx-config.json`. */
  path: string;
  appdata: string;
  /** Keys read only at startup: changing one needs a restart. */
  restartKeys: string[];
}

export interface ConfigUpdateResult {
  config: ConfigView;
  changedKeys: string[];
  /** The changed keys that only take effect after a restart. */
  restartRequired: string[];
}

/** Top-level key changes; `null` removes a key. */
export type ConfigChanges = { [K in keyof SolxConfigFile]?: SolxConfigFile[K] | null };

/** What installing or uninstalling a package leaves behind. */
export interface PackageResult {
  /** The config as it is now: the package registered or gone, grants applied or revoked. */
  config: ConfigView;
  /** Allowlist collisions with other installed packages; never blocking. */
  warnings: string[];
}

export class TauriConfig {
  constructor(private readonly t: TauriTransport) {}

  get(): Promise<ConfigView> {
    return this.t.call('config_get');
  }

  /**
   * One validated write. A rejected change leaves the file untouched and
   * fails with a `Validation` `SolxError` naming the problem.
   */
  update(changes: ConfigChanges): Promise<ConfigUpdateResult> {
    return this.t.call('config_update', { changes });
  }

  /**
   * Install a package from the bytes of its zip: unpacked into
   * `packages_directory`, its `install.solx` run, its declared allowlist
   * entries granted. Only available in a build that owns its managers.
   */
  installPackage(zip: Uint8Array): Promise<PackageResult> {
    return this.t.call('config_install_package', zip);
  }

  /** Uninstall by name: run `uninstall.solx`, revoke the grants, delete unpacked files. */
  uninstallPackage(name: string): Promise<PackageResult> {
    return this.t.call('config_uninstall_package', { name });
  }
}
