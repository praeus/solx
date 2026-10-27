// Run with `bun test` from web/web.
import { describe, expect, test } from "bun:test";
import { describeScopes, presetOf, PRESETS, SCOPE_SUGGESTIONS } from "./scopes";

describe("presetOf", () => {
  test("recognises a preset in any order", () => {
    expect(presetOf(["exec:/*", "read:action"])?.id).toBe("tools");
    expect(presetOf(["all"])?.id).toBe("full");
    expect(presetOf(["read:file", "read:action", "read:type", "read:document"])?.id).toBe("read-only");
  });

  test("anything more or less is not a preset", () => {
    expect(presetOf(["read:action"])).toBeNull();
    expect(presetOf(["read:action", "exec:/*", "read:type"])).toBeNull();
    expect(presetOf([])).toBeNull();
  });
});

describe("describeScopes", () => {
  test("names presets, root and custom sets", () => {
    expect(describeScopes(["all"])).toBe("Full access");
    expect(describeScopes(null)).toBe("Unrestricted (this machine's own token)");
    expect(describeScopes([])).toBe("Nothing");
    expect(describeScopes(["read:document:/notes/*", "exec:/builtin/web/*"])).toBe(
      "read:document:/notes/*, exec:/builtin/web/*",
    );
  });
});

describe("SCOPE_SUGGESTIONS", () => {
  test("offers every scope a preset is made of", () => {
    for (const preset of PRESETS) {
      for (const scope of preset.scopes) expect(SCOPE_SUGGESTIONS).toContain(scope);
    }
  });
});
