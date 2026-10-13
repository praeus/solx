/**
 * JSON validation shared by the two JSON surfaces (`JsonEditField` and
 * `JsonSchemaEditor`) — one linter feeding CodeMirror's gutter, and one
 * status helper feeding the "valid / invalid" badge each renders in its
 * own chrome.
 */

import { linter, lintGutter, type Diagnostic } from "@codemirror/lint";
import type { Extension } from "@codemirror/state";

export interface JsonStatus {
  /** `null` for an empty document — neither valid nor invalid, just blank. */
  valid: boolean | null;
  error: string | null;
}

export function parseJsonStatus(text: string): JsonStatus {
  if (text.trim().length === 0) return { valid: null, error: null };
  try {
    JSON.parse(text);
    return { valid: true, error: null };
  } catch (err) {
    return { valid: false, error: err instanceof Error ? err.message : String(err) };
  }
}

/**
 * Best-effort offset for a `JSON.parse` failure. CodeMirror wants a document
 * position, not a line/column, so we lift the number out of V8's
 * "…at position N" message and fall back to the end of the document.
 */
export function locateJsonError(text: string, message: string): number {
  const match = /at position (\d+)/.exec(message);
  if (match) {
    const pos = Number(match[1]);
    if (Number.isFinite(pos) && pos >= 0 && pos <= text.length) return pos;
  }
  return text.length;
}

/** Linter + gutter, ready to hand to `CodeEditField`'s `extensions`. */
export function jsonLintExtensions(): Extension[] {
  return [
    lintGutter(),
    linter((view) => {
      const text = view.state.doc.toString();
      // Empty is valid by convention; the placeholder carries the message.
      if (text.trim().length === 0) return [];
      try {
        JSON.parse(text);
        return [];
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        const pos = locateJsonError(text, message);
        const diagnostic: Diagnostic = {
          from: pos,
          to: Math.min(pos + 1, text.length),
          severity: "error",
          message,
        };
        return [diagnostic];
      }
    }),
  ];
}
