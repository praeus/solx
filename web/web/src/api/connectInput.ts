/**
 * connectInput.ts — the one "connect code, link or token" field, classified.
 *
 * TypeScript twin of `solx_config::credentials::parse_connect_input`, with
 * the same test cases (`connectInput.test.ts`):
 * - a link containing `#code=` is a code, and its origin becomes the server
 *   URL when none was given;
 * - exactly 8 digits once spaces and dashes are removed is a code;
 * - anything else non-empty is a token, trimmed.
 *
 * Tokens are 44-character base64, never exactly 8 digits, so this can't be
 * ambiguous. A malformed code (7 digits, say) comes back as a token and
 * fails verification like any other wrong token.
 */

export type ConnectCredential = { kind: "code"; code: string } | { kind: "token"; token: string };

export interface ConnectInput {
  credential: ConnectCredential;
  /** The server to use, without a trailing slash; `null` when unknown. */
  serverUrl: string | null;
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
  const given = (serverUrl ?? "").trim().replace(/\/+$/, "") || null;
  const text = input.trim();
  if (!text) throw new Error("Enter a connect code, a connect link or a token.");

  const at = text.indexOf("#code=");
  if (at >= 0) {
    const code = connectCodeDigits(text.slice(at + "#code=".length));
    if (!code) throw new Error("The connect link's code is not 8 digits.");
    return { credential: { kind: "code", code }, serverUrl: given ?? connectLinkBase(text.slice(0, at)) };
  }

  const code = connectCodeDigits(text);
  return {
    credential: code ? { kind: "code", code } : { kind: "token", token: text },
    serverUrl: given,
  };
}
