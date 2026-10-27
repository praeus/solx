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
  /** A PEM file of extra roots a client trusts for an https:// server (relative to the appdata dir). */
  server_ca_file?: string;
  server_token?: string;
  server_port?: number;
  server_host?: string;
  server_cors_origins?: string[];
  /** The address other machines use to reach this server, for connect links. */
  server_public_url?: string;
  /**
   * TLS for the HTTP server: `"off"`, `"auto"` (certificates in
   * `<appdata>/certs`, made by the app), PEM files, or (Windows) a
   * certificate-store thumbprint. Unset, the app turns it on for a network
   * address on Windows.
   */
  server_tls?: 'off' | 'auto' | { cert_file: string; key_file: string } | { store_thumbprint: string };
  /** Lifetime of a client token, in days (default 90). */
  client_token_ttl_days?: number;
  /**
   * How a call outside its scopes is treated: allowed, allowed and logged
   * (the default), or refused with a `Forbidden` error.
   */
  scope_enforcement?: 'off' | 'audit' | 'enforce';
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
  scope_enforcement: 'off' | 'audit' | 'enforce';
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

/** One certificate in the TLS certificates directory. */
export interface TlsCert {
  /** SHA-256 of the certificate, lowercase hex (also its Windows thumbprint). */
  sha256: string;
  subject: string;
  notBefore: string;
  notAfter: string;
  sans: string[];
  /** Where its key is: a key storage provider or a file. */
  keyLocation: string;
  /** Whether the key is held in hardware (a TPM). */
  keyHardware: boolean;
  /** `false` when the platform enforces that the key can't be exported; `null` when unknown. */
  keyExportable: boolean | null;
  keyPresent: boolean;
}

/** The local certificate authority behind `server_tls: "auto"`. */
export interface TlsCa {
  backend: 'cng' | 'file';
  /** What clients pin: base64url SHA-256 of the root's public key. */
  pin: string;
  /** The pin as people compare it, `3F7A-91C2-0B4E-D518`. */
  fingerprint: string;
  root: TlsCert;
  server: TlsCert | null;
  /** Whether this machine trusts the root; `null` where that can't be told. */
  trusted: boolean | null;
  rootCertPath: string;
}

/** The "Secure connections" status. */
export interface TlsStatus {
  /** `server_tls` as configured. */
  mode: 'off' | 'auto' | 'files' | 'store';
  modeError: string | null;
  /** Whether the running embedded server serves TLS; `null` when it isn't running. */
  servingHttps: boolean | null;
  /** Why the embedded server didn't start, if it didn't. */
  serverError: string | null;
  /** The certificates directory. */
  dir: string;
  ca: TlsCa | null;
  caError: string | null;
}

/** What a TLS action leaves behind. */
export interface TlsResult {
  status: TlsStatus;
  /** Whether the app must restart before the embedded server serves the change. */
  restartRequired: boolean;
}

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

  /** The TLS certificates behind `server_tls: "auto"`, and what the server is serving. */
  tlsStatus(): Promise<TlsStatus> {
    return this.t.call('config_tls_status');
  }

  /**
   * Trust the root on this machine, so browsers here accept the server. On
   * Windows this waits for the person to confirm Windows' own dialog.
   */
  tlsTrust(): Promise<TlsResult> {
    return this.t.call('config_tls_trust');
  }

  /** Stop trusting the root on this machine. */
  tlsUntrust(): Promise<TlsResult> {
    return this.t.call('config_tls_untrust');
  }

  /** Issue a new server certificate for the current addresses. Clients don't notice. */
  tlsReissue(): Promise<TlsResult> {
    return this.t.call('config_tls_reissue');
  }

  /** Delete the certificate authority. Every other device has to be set up again. */
  tlsReset(): Promise<TlsResult> {
    return this.t.call('config_tls_reset');
  }
}
