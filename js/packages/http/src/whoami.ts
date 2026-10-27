/**
 * `GET /auth/whoami`: what the token is and what it may do, so a client
 * can show it and grey out what it can't do. Answered for any token.
 */
import { requestJson } from './fetch.js';

export interface Whoami {
  kind: 'root' | 'client' | 'action';
  /** The token's scope strings; `null` for the root token, which no scope limits. */
  scopes: string[] | null;
  /** Whether the scopes are enforced yet: `audit` only logs what they would refuse. */
  scope_enforcement: 'off' | 'audit' | 'enforce';
  /** A client token's id and label. */
  id?: string;
  label?: string | null;
  /** An action token's action and invocation. */
  action?: string;
  invocation_id?: string;
}

export function whoami(baseUrl: string, token: string): Promise<Whoami> {
  return requestJson<Whoami>(baseUrl, token, 'GET', '/auth/whoami');
}
