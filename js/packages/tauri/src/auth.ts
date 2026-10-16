/**
 * Connect codes and client tokens over IPC — solx-tauri-plugin's
 * `auth_exec`, which runs the host's `/builtin/auth/*` builtins as the root
 * principal. Needs the `solx:auth` permission, which the desktop app grants
 * only to its Settings window (never the main window, where package widgets
 * run), and a host build: a client's tokens are managed on its server.
 *
 * Keys are snake_case, as the builtins return them.
 */
import type { TauriTransport } from './invoke.js';

/** A single-use code another device redeems for its own client token. */
export interface ConnectCode {
  /** The code as shown to a person: `1234-5678`. */
  display_code: string;
  /** `<public url>/connect#code=12345678`, when the host knows its address. */
  url: string | null;
  /** The address to type next to the code on the other device. */
  server_url: string | null;
  /** RFC 3339. */
  expires_at: string;
}

/** A client token as listed: never the token itself. */
export interface ClientInfo {
  id: string;
  label: string | null;
  created_at: string;
  expires_at: string | null;
  last_used_at: string | null;
}

/** A newly minted client token. Shown once; the host keeps only its hash. */
export interface IssuedClient {
  token: string;
  client_id: string;
  expires_at: string | null;
}

export interface CreateConnectCodeOptions {
  /** Name for the client the code creates, shown in the client list. */
  label?: string;
  /** How long the code stays valid (default 600). */
  codeTtlSecs?: number;
  /** How long the client token stays valid (default `client_token_ttl_days`). */
  tokenTtlSecs?: number;
  /**
   * Address to put in the link instead of `server_public_url` or the bound
   * address, e.g. a Tailscale IP. A URL, or an IP or host name (which gets
   * `http://` and the server's port).
   */
  publicUrl?: string;
}

export class TauriAuth {
  constructor(private readonly t: TauriTransport) {}

  private exec<T>(name: string, params: Record<string, unknown> = {}): Promise<T> {
    return this.t.call('auth_exec', { name, params });
  }

  createConnectCode(options: CreateConnectCodeOptions = {}): Promise<ConnectCode> {
    return this.exec('create-connect-code', {
      label: options.label,
      code_ttl_secs: options.codeTtlSecs,
      token_ttl_secs: options.tokenTtlSecs,
      public_url: options.publicUrl,
    });
  }

  async listClients(): Promise<ClientInfo[]> {
    const { clients } = await this.exec<{ clients: ClientInfo[] }>('list-clients');
    return clients;
  }

  async revokeClient(id: string): Promise<void> {
    await this.exec('revoke-client', { id });
  }

  issueClient(label: string, tokenTtlSecs?: number): Promise<IssuedClient> {
    return this.exec('issue-client', { label, token_ttl_secs: tokenTtlSecs });
  }
}
