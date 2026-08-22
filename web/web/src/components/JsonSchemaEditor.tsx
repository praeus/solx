/**
 * JsonSchemaEditor — JSON Schema editor with a live parse badge.
 *
 * - Renders the schema as JSON with syntax highlighting and 2-space indent.
 * - Shows a "valid" / "invalid" badge with the parse error message.
 * - Calls onChange(value) whenever the user edits.
 * - Follows the system light/dark preference (handled by `CodeEditField`).
 */

import { useEffect, useMemo } from "react";
import { CodeEditField } from "./CodeEditField";
import { jsonLintExtensions, parseJsonStatus } from "./codemirror/jsonLint";

interface Props {
  value: string;
  onChange: (value: string) => void;
  /** Optional value to merge into the editor when it changes (e.g. from a load). */
  externalResetValue?: string;
  readonly?: boolean;
  /** Editor height in pixels. */
  height?: number;
  placeholder?: string;
}

export function JsonSchemaEditor({
  value,
  onChange,
  externalResetValue,
  readonly,
  height = 320,
  placeholder,
}: Props) {
  const lintExtensions = useMemo(() => jsonLintExtensions(), []);
  const { error: parseError } = parseJsonStatus(value);

  // Push externalResetValue into the editor only when it changes externally.
  useEffect(() => {
    if (externalResetValue === undefined) return;
    if (externalResetValue !== value) {
      onChange(externalResetValue);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [externalResetValue]);

  return (
    <div className="json-schema-editor">
      <div className="json-schema-editor__chrome">
        <span className="json-schema-editor__badge">
          {parseError ? (
            <span className="json-schema-editor__badge--error">⚠ Invalid JSON</span>
          ) : value.trim() ? (
            <span className="json-schema-editor__badge--ok">✓ Valid JSON</span>
          ) : (
            <span className="json-schema-editor__badge--muted">(empty)</span>
          )}
        </span>
        {parseError && <code className="json-schema-editor__error">{parseError}</code>}
      </div>
      <CodeEditField
        value={value}
        onChange={onChange}
        language="json"
        readOnly={readonly}
        height={`${height}px`}
        placeholder={placeholder}
        extensions={lintExtensions}
        ariaLabel="JSON schema editor"
      />
    </div>
  );
}
