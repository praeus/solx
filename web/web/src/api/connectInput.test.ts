// Same cases as `parse_connect_input`'s tests in solx-config's credentials.rs.
// Run with `bun test` from web/web.
import { describe, expect, test } from "bun:test";
import { parseConnectInput } from "./connectInput";

const code = (c: string) => ({ kind: "code", code: c });

describe("parseConnectInput", () => {
  test("digits with spaces or dashes are codes", () => {
    for (const input of ["12345678", "1234-5678", " 1234 5678 "]) {
      const parsed = parseConnectInput("http://10.0.0.5:8766", input);
      expect(parsed.credential).toEqual(code("12345678"));
      expect(parsed.serverUrl).toBe("http://10.0.0.5:8766");
    }
  });

  test("a connect link is a code and supplies the server", () => {
    const link = "http://10.0.0.5:8766/connect#code=12345678";
    let parsed = parseConnectInput(null, link);
    expect(parsed.credential).toEqual(code("12345678"));
    expect(parsed.serverUrl).toBe("http://10.0.0.5:8766");

    // A reverse-proxy prefix in the public URL survives.
    parsed = parseConnectInput(null, "https://example.com/solx/connect#code=12345678");
    expect(parsed.serverUrl).toBe("https://example.com/solx");

    // An explicit server wins over the link's origin.
    parsed = parseConnectInput("https://solx.example.com/", link);
    expect(parsed.serverUrl).toBe("https://solx.example.com");

    expect(() => parseConnectInput(null, "http://h/connect#code=123")).toThrow();
  });

  const PIN = "q83vASNFZ4mrze8BI0VniavN7wEjRWeJq83vASNFZ4k";

  test("a pin comes from the link or the server address", () => {
    const link = `https://10.0.0.5:8766/connect#code=12345678&pin=${PIN}`;
    let parsed = parseConnectInput(null, link);
    expect(parsed.credential).toEqual(code("12345678"));
    expect(parsed.serverUrl).toBe("https://10.0.0.5:8766");
    expect(parsed.pin).toBe(PIN);

    const address = `https://10.0.0.5:8766/#pin=${PIN}`;
    parsed = parseConnectInput(address, "1234-5678");
    expect(parsed.serverUrl).toBe("https://10.0.0.5:8766");
    expect(parsed.pin).toBe(PIN);

    // The same pin twice is fine; a different one, or a malformed one, is refused.
    expect(() => parseConnectInput(address, link)).not.toThrow();
    expect(() => parseConnectInput(`https://h#pin=${"A".repeat(43)}`, link)).toThrow();
    expect(() => parseConnectInput(null, "https://h/connect#code=12345678&pin=abc")).toThrow();
    expect(() => parseConnectInput("https://h#pin=abc", "12345678")).toThrow();

    expect(parseConnectInput(null, "12345678").pin).toBeNull();
  });

  test("a 44-character base64 token is a token", () => {
    const token = "q2Vq7Jx0bN3m1u5bWb0i8l0S2x4lQe9oZ7mT1aYk3cE=";
    expect(token.length).toBe(44);
    const parsed = parseConnectInput("", ` ${token} `);
    expect(parsed.credential).toEqual({ kind: "token", token });
    expect(parsed.serverUrl).toBeNull();
  });

  test("seven digits are a token, and empty input is refused", () => {
    expect(parseConnectInput(null, "1234567").credential).toEqual({ kind: "token", token: "1234567" });
    expect(() => parseConnectInput(null, "   ")).toThrow();
  });
});
