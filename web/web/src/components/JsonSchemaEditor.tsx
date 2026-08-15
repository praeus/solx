/**
 * JsonSchemaEditor — Monaco-based JSON Schema editor with a live parse badge.
 *
 * - Renders the schema as JSON with syntax highlighting and 2-space tabs.
 * - Shows a "valid" / "invalid" badge with the parse error message.
 * - Calls onChange(value) whenever the user edits the textarea.
 * - Auto-detects light/dark theme from the system.
 */

import Editor, { type OnMount } from "@monaco-editor/react";
import { useEffect, useState } from "react";

interface Props {
  value: string;
  onChange: (value: string) => void;
  /** Optional value to merge into the editor when it changes (e.g. from a load). */
  externalResetValue?: string;
  readonly?: boolean;
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
  const [parseError, setParseError] = useState<string | null>(null);
  const [theme, setTheme] = useState<"vs" | "vs-dark">("vs");

  // Detect prefers-color-scheme once on mount.
  useEffect(() => {
    if (typeof window === "undefined" || !window.matchMedia) return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    setTheme(mq.matches ? "vs-dark" : "vs");
    const listener = (e: MediaQueryListEvent) => setTheme(e.matches ? "vs-dark" : "vs");
    mq.addEventListener("change", listener);
    return () => mq.removeEventListener("change", listener);
  }, []);

  // Track parse validity as the user types.
  useEffect(() => {
    if (!value.trim()) {
      setParseError(null);
      return;
    }
    try {
      JSON.parse(value);
      setParseError(null);
    } catch (e) {
      setParseError((e as Error).message);
    }
  }, [value]);

  // Push externalResetValue into the editor only when it changes externally.
  useEffect(() => {
    if (externalResetValue === undefined) return;
    if (externalResetValue !== value) {
      onChange(externalResetValue);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [externalResetValue]);

  const handleMount: OnMount = (editor, monaco) => {
    // 2-space tabs, no line numbers isn't ideal for JSON; keep them visible.
    editor.updateOptions({
      tabSize: 2,
      wordWrap: "on",
      insertSpaces: true,
      minimap: { enabled: false },
      fontSize: 13,
      readOnly: readonly,
      renderLineHighlight: "line",
    });
    // Light-touch JSON validation
    monaco.languages.json.jsonDefaults.setDiagnosticsOptions({
      validate: true,
      allowComments: false,
      schemaValidation: "ignore",
    });
  };

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
        {parseError && (
          <code className="json-schema-editor__error">{parseError}</code>
        )}
      </div>
      <Editor
        height={height}
        defaultLanguage="json"
        theme={theme}
        value={value}
        onChange={(v) => onChange(v ?? "")}
        onMount={handleMount}
        options={{ readOnly: readonly }}
        loading={<div className="json-schema-editor__loading">Loading editor…</div>}
      />
      {placeholder && !value && (
        <div className="json-schema-editor__placeholder">{placeholder}</div>
      )}
    </div>
  );
}
