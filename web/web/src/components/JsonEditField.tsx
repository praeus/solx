/**
 * JsonEditField — JSON editor with a live parse badge.
 *
 * A thin wrapper over `CodeEditField` that pins the language to JSON, adds
 * the lint gutter, and renders the validity badge underneath. The editor
 * mechanics (theme, dark mode, touch sizing, compartments) all live in
 * `CodeEditField`; this file is just the JSON-flavoured chrome.
 *
 * Used by:
 *   - DocumentEditor's JSON-mode contents editor
 *   - Per-field array/object schema fields
 */

import { useMemo } from "react";
import { CodeEditField } from "./CodeEditField";
import { jsonLintExtensions, parseJsonStatus } from "./codemirror/jsonLint";

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

export function JsonEditField({
  value,
  onChange,
  placeholder,
  minLines = 6,
  readOnly = false,
  height,
  ariaLabel,
}: Props) {
  const lintExtensions = useMemo(() => jsonLintExtensions(), []);
  const { valid, error } = parseJsonStatus(value);

  return (
    <div className="json-edit-field">
      <div className="json-edit-field__editor">
        <CodeEditField
          value={value}
          onChange={onChange}
          language="json"
          readOnly={readOnly}
          height={height}
          minLines={minLines}
          placeholder={placeholder}
          extensions={lintExtensions}
          ariaLabel={ariaLabel}
        />
      </div>
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
