/**
 * JsonEditField — reusable CodeMirror-based JSON editor with live parse badge.
 *
 * Wraps CodeMirror 6 with:
 *   - JSON syntax highlighting
 *   - Live JSON parse badge: ✓ green when valid, ✗ red with error message
 *   - Read-only mode for preview contexts
 *   - Optional line numbers / folding
 *
 * Used by:
 *   - DocumentEditor's JSON-mode contents textarea
 *   - Per-field array/object schema fields
 *   - Any future place that needs an editable JSON input with parse feedback
 */

import { useEffect, useMemo, useRef, useState } from "react";
import { EditorState, Compartment } from "@codemirror/state";
import { EditorView, keymap, highlightActiveLine, lineNumbers } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { json } from "@codemirror/lang-json";
import {
  bracketMatching,
  defaultHighlightStyle,
  indentOnInput,
  syntaxHighlighting,
  HighlightStyle,
} from "@codemirror/language";
import { linter, lintGutter, type Diagnostic } from "@codemirror/lint";
import { tags as t } from "@lezer/highlight";

interface Props {
  /** Current JSON value as a string. */
  value: string;
  /** Called when the user edits. */
  onChange: (next: string) => void;
  /** Placeholder for empty inputs. */
  placeholder?: string;
  /** Minimum height in lines. */
  minLines?: number;
  /** Read-only mode. */
  readOnly?: boolean;
  /** Optional fixed height (CSS length). */
  height?: string;
  /** Custom aria-label for screen readers. */
  ariaLabel?: string;
}

const jsonHighlightStyle = HighlightStyle.define([
  { tag: t.propertyName, color: "var(--accent)" },
  { tag: t.string, color: "#a07f3f" },
  { tag: t.number, color: "#3f7fa0" },
  { tag: t.bool, color: "#a03f7f" },
  { tag: t.null, color: "#a03f7f", fontStyle: "italic" },
  { tag: t.bracket, color: "var(--text-muted)" },
  { tag: t.punctuation, color: "var(--text-muted)" },
]);

export function JsonEditField({
  value,
  onChange,
  placeholder,
  minLines = 6,
  readOnly = false,
  height,
  ariaLabel,
}: Props) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const viewRef = useRef<EditorView | null>(null);
  const [valid, setValid] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  const readOnlyCompartment = useMemo(() => new Compartment(), []);

  // Mount CodeMirror once.
  useEffect(() => {
    if (!containerRef.current) return;
    const updateListener = EditorView.updateListener.of((update) => {
      if (update.docChanged) {
        const next = update.state.doc.toString();
        onChange(next);
      }
    });

    const jsonLinter = linter(() => {
      const text = viewRef.current?.state.doc.toString() ?? "";
      // Empty input: valid by convention; UI shows the placeholder.
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
    });

    const state = EditorState.create({
      doc: value,
      extensions: [
        lineNumbers(),
        history(),
        bracketMatching(),
        indentOnInput(),
        highlightActiveLine(),
        syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
        syntaxHighlighting(jsonHighlightStyle),
        json(),
        lintGutter(),
        jsonLinter,
        keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
        EditorView.lineWrapping,
        readOnlyCompartment.of(EditorState.readOnly.of(readOnly)),
        updateListener,
        EditorView.theme(
          {
            "&": {
              backgroundColor: "var(--bg)",
              color: "var(--text)",
              fontSize: "13px",
              minHeight: `${minLines * 1.4}em`,
              height: height ?? "auto",
            },
            ".cm-content": { fontFamily: "var(--font-mono)" },
            ".cm-gutters": {
              backgroundColor: "var(--bg-sunken)",
              color: "var(--text-faint)",
              border: "none",
            },
            ".cm-activeLine": { backgroundColor: "var(--bg-raised)" },
            ".cm-activeLineGutter": { backgroundColor: "var(--bg-raised)" },
            ".cm-scroller": { fontFamily: "var(--font-mono)" },
          },
          { dark: false },
        ),
      ],
    });

    const view = new EditorView({ state, parent: containerRef.current });
    viewRef.current = view;

    return () => {
      view.destroy();
      viewRef.current = null;
    };
    // Intentionally only mount once; `value` updates flow through `view.dispatch`
    // below to avoid recreating the editor on every keystroke.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Sync external value → editor when it changes (e.g. hydration on dialog open).
  // Skip if the editor already shows this value to avoid clobbering user input.
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    const current = view.state.doc.toString();
    if (current !== value) {
      view.dispatch({
        changes: { from: 0, to: current.length, insert: value },
      });
    }
  }, [value]);

  // Track readOnly.
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: readOnlyCompartment.reconfigure(EditorState.readOnly.of(readOnly)),
    });
  }, [readOnly, readOnlyCompartment]);

  // Compute validity badge.
  useEffect(() => {
    if (value.trim().length === 0) {
      setValid(null);
      setError(null);
      return;
    }
    try {
      JSON.parse(value);
      setValid(true);
      setError(null);
    } catch (err) {
      setValid(false);
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [value]);

  return (
    <div className="json-edit-field">
      <div ref={containerRef} className="json-edit-field__editor" aria-label={ariaLabel} />
      {placeholder && value.trim().length === 0 && (
        <div className="json-edit-field__placeholder" aria-hidden>
          {placeholder}
        </div>
      )}
      <div className="json-edit-field__status">
        {valid === null ? (
          <span className="json-edit-field__status--neutral">empty</span>
        ) : valid ? (
          <span className="json-edit-field__status--ok">✓ valid JSON</span>
        ) : (
          <span className="json-edit-field__status--err" title={error ?? undefined}>
            ✗ {error ?? "invalid JSON"}
          </span>
        )}
      </div>
    </div>
  );
}

/**
 * Best-effort: locate the byte offset in `text` where the JSON.parse error
 * happened. CodeMirror needs a number, not a line/col, so we look for
 * the position mentioned in V8's "Unexpected token ... at position N"
 * message, falling back to the end of the input.
 */
function locateJsonError(text: string, message: string): number {
  const match = /at position (\d+)/.exec(message);
  if (match) {
    const pos = Number(match[1]);
    if (Number.isFinite(pos) && pos >= 0 && pos <= text.length) return pos;
  }
  return text.length;
}
