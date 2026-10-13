/**
 * Language registry for the CodeMirror editors.
 *
 * Every grammar is behind a dynamic `import()` so none of them land in the
 * initial bundle — the editor mounts with plain-text highlighting and
 * reconfigures its language compartment once the grammar resolves. This is
 * what lets the file editor offer eleven languages without paying for them
 * on first paint (the reason Monaco was heavy here: it shipped all of its
 * grammars plus workers regardless of which one you picked).
 */

import type { Extension } from "@codemirror/state";

export type LanguageId =
  | "plaintext"
  | "markdown"
  | "json"
  | "solx"
  | "html"
  | "css"
  | "javascript"
  | "typescript"
  | "python"
  | "yaml"
  | "xml";

export interface LanguageOption {
  id: LanguageId;
  label: string;
}

/** Presented in the file editor's language picker, in this order. */
export const LANGUAGE_OPTIONS: readonly LanguageOption[] = [
  { id: "plaintext", label: "Plain Text" },
  { id: "markdown", label: "Markdown" },
  { id: "json", label: "JSON" },
  { id: "solx", label: "solx script" },
  { id: "html", label: "HTML" },
  { id: "css", label: "CSS" },
  { id: "javascript", label: "JavaScript" },
  { id: "typescript", label: "TypeScript" },
  { id: "python", label: "Python" },
  { id: "yaml", label: "YAML" },
  { id: "xml", label: "XML" },
] as const;

/**
 * Resolve a language id to its CodeMirror extension.
 *
 * Returns `null` for languages with no grammar (plain text, and solx
 * scripts — which have no CodeMirror mode yet and read fine unhighlighted).
 */
export async function loadLanguage(id: LanguageId): Promise<Extension | null> {
  switch (id) {
    case "markdown": {
      const { markdown } = await import("@codemirror/lang-markdown");
      return markdown();
    }
    case "json": {
      const { json } = await import("@codemirror/lang-json");
      return json();
    }
    case "html": {
      const { html } = await import("@codemirror/lang-html");
      return html();
    }
    case "css": {
      const { css } = await import("@codemirror/lang-css");
      return css();
    }
    case "javascript": {
      const { javascript } = await import("@codemirror/lang-javascript");
      return javascript();
    }
    case "typescript": {
      const { javascript } = await import("@codemirror/lang-javascript");
      return javascript({ typescript: true });
    }
    case "python": {
      const { python } = await import("@codemirror/lang-python");
      return python();
    }
    case "yaml": {
      const { yaml } = await import("@codemirror/lang-yaml");
      return yaml();
    }
    case "xml": {
      const { xml } = await import("@codemirror/lang-xml");
      return xml();
    }
    case "plaintext":
    case "solx":
    default:
      return null;
  }
}
