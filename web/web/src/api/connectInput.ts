/**
 * connectInput.ts — the one "connect code, link or token" field, classified.
 *
 * TypeScript twin of `solx_config::credentials::parse_connect_input`, with
 * the same test cases (`connectInput.test.ts`):
 * - a link containing `#code=` is a code, and its origin becomes the server
 *   URL when none was given. Its `&pin=`, if any, is the server's root pin;
 * - exactly 8 digits once spaces and dashes are removed is a code;
 * - anything else non-empty is a token, trimmed.
 *
 * The server URL may carry a pin too (`https://host:8766#pin=…`). A pin in
 * both places must be the same pin. A browser can't enforce a pin (it
 * decides trust itself), so here it is only parsed and checked.
 *
 * Tokens are 44-character base64, never exactly 8 digits, so this can't be
 * ambiguous. A malformed code (7 digits, say) comes back as a token and
 * fails verification like any other wrong token.
 */

export type ConnectCredential = { kind: "code"; code: string } | { kind: "token"; token: string };

export interface ConnectInput {
  credential: ConnectCredential;
  /** The server to use, without a trailing slash or `#pin=`; `null` when unknown. */
  serverUrl: string | null;
  /** The server's root pin, from the link or the server URL. */
  pin: string | null;
}

/** 43 base64url characters: 32 bytes, unpadded. */
const PIN = /^[A-Za-z0-9_-]{42}[AEIMQUYcgkosw048]$/;

/** The `pin` among `&`-separated `key=value` fragment parameters. Throws on a malformed one. */
function pinParam(fragment: string): string | null {
  const param = fragment.split("&").find((p) => p.startsWith("pin="));
  if (param === undefined) return null;
  const pin = param.slice("pin=".length).trim();
  if (!PIN.test(pin)) throw new Error("That is not a server pin (43 base64url characters).");
  return pin;
}

/** A pin over plain HTTP would protect nothing, and a code sent there travels in cleartext. */
function requireHttpsForPin(serverUrl: string, pin: string | null): void {
  if (pin && !/^https:\/\//i.test(serverUrl)) {
    throw new Error(`${serverUrl} carries a server pin, but a pinned server must be reached over https://.`);
  }
}

/** `serverUrl` without its fragment and trailing slash, and the pin in its fragment. */
export function splitServerPin(serverUrl: string): { serverUrl: string; pin: string | null } {
  const text = serverUrl.trim();
  const at = text.indexOf("#");
  const base = (at >= 0 ? text.slice(0, at) : text).replace(/\/+$/, "");
  const pin = at >= 0 ? pinParam(text.slice(at + 1)) : null;
  requireHttpsForPin(base, pin);
  return { serverUrl: base, pin };
}

/** The 8 digits of `12345678`, `1234-5678` or `1234 5678`, else `null`. */
export function connectCodeDigits(input: string): string | null {
  const text = input.trim();
  if (!/^[0-9 -]*$/.test(text)) return null;
  const digits = text.replace(/[^0-9]/g, "");
  return digits.length === 8 ? digits : null;
}

/**
 * The server a connect link (`<public url>/connect`, fragment already cut
 * off) points at: the link minus its `/connect`, so a public URL under a
 * reverse-proxy prefix (`https://example.com/solx`) keeps the prefix.
 */
function connectLinkBase(link: string): string | null {
  let url: URL;
  try {
    url = new URL(link);
  } catch {
    return null;
  }
  if (!url.host) return null;
  const path = url.pathname.replace(/\/+$/, "");
  return url.origin + (path.endsWith("/connect") ? path.slice(0, -"/connect".length) : path);
}

/** Classify the field. Throws on empty input or a link whose code isn't 8 digits. */
export function parseConnectInput(serverUrl: string | null | undefined, input: string): ConnectInput {
  const split = splitServerPin(serverUrl ?? "");
  const given = split.serverUrl || null;
  const text = input.trim();
  if (!text) throw new Error("Enter a connect code, a connect link or a token.");

  const at = text.indexOf("#code=");
  if (at >= 0) {
    const fragment = text.slice(at + "#code=".length);
    const amp = fragment.indexOf("&");
    const code = connectCodeDigits(amp >= 0 ? fragment.slice(0, amp) : fragment);
    if (!code) throw new Error("The connect link's code is not 8 digits.");
    const linkPin = amp >= 0 ? pinParam(fragment.slice(amp + 1)) : null;
    if (split.pin && linkPin && split.pin !== linkPin) {
      throw new Error("The server address and the connect link carry different pins.");
    }
    const url = given ?? connectLinkBase(text.slice(0, at));
    const pin = split.pin ?? linkPin;
    if (url) requireHttpsForPin(url, pin);
    return { credential: { kind: "code", code }, serverUrl: url, pin };
  }

  const code = connectCodeDigits(text);
  return {
    credential: code ? { kind: "code", code } : { kind: "token", token: text },
    serverUrl: given,
    pin: split.pin,
  };
}
