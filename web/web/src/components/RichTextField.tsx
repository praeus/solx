/**
 * RichTextField — Tiptap-based rich-text editor that round-trips
 * ProseMirror JSON through the solx wire format.
 *
 * Schema-driven dispatch: the DocumentEditor detects fields whose schema
 * marks them `rich_text` (via `x-sol-editor: "rich_text"` or `$ref` to
 * `RichTextDoc`) and renders this component instead of a raw JSON
 * textarea. The user types rich text; the value stored on the document
 * is the ProseMirror JSON document.
 *
 * Tiptap 3 is used via `@tiptap/react`. We use the StarterKit
 * (`bold`/`italic`/`code`/`heading`/`bulletList`/`orderedList`/`blockquote`/
 * `codeBlock`/`hardBreak`) as a sensible default. Customizing the toolbar
 * per schema would be a future extension.
 *
 * The value is `JSON.stringify(editor.getJSON())` whenever the user
 * edits. On hydration, the parent parses the value and passes it to
 * Tiptap via `content`. Empty input is a `doc` with no content (matching
 * the empty doc the legacy DocumentEditor round-tripped).
 */

import { useEditor, EditorContent } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Image from "@tiptap/extension-image";
import { fileRawUrl } from "../api/api";
import { useUploadFile } from "../api/hooks";
import { useRef, useState } from "react";

interface Props {
  /** ProseMirror JSON as a string. */
  value: string;
  /** Called when the user edits. */
  onChange: (nextJsonString: string) => void;
  /** Placeholder for empty documents. */
  placeholder?: string;
  /** Read-only mode. */
  readOnly?: boolean;
  /** aria-label for screen readers. */
  ariaLabel?: string;
  /**
   * Default destination when uploading an image via the toolbar.
   * Defaults to `files/docs/shared/`.
   */
  uploadPrefix?: string;
}

export function RichTextField({
  value,
  onChange,
  placeholder = "Start writing…",
  readOnly = false,
  ariaLabel,
  uploadPrefix = "files/docs/shared/",
}: Props) {
  const parsedInitial = useMemo(() => parseValue(value), [value]);

  const editor = useEditor({
    extensions: [
      StarterKit,
      // Image extension — `<img src="…">` nodes. The src is whatever
      // URL we hand it (we use the binary `/api/files/raw?relPath=…`
      // route so the browser loads the file directly with the right
      // Content-Type). Inline by default; CSS controls the rendered size.
      Image.configure({
        inline: true,
        allowBase64: false,
        HTMLAttributes: {
          class: "rich-text-field__image",
        },
      }),
    ],
    content: parsedInitial,
    editable: !readOnly,
    editorProps: {
      attributes: {
        class: "rich-text-field__editor",
        "aria-label": ariaLabel ?? "Rich text editor",
        role: "textbox",
      },
    },
    onUpdate({ editor }) {
      const json = editor.getJSON();
      onChange(JSON.stringify(json));
    },
  });

  // If the external value changes (e.g. hydration), sync into the editor.
  useEffect(() => {
    if (!editor) return;
    const current = JSON.stringify(editor.getJSON());
    if (current === value) return;
    const parsed = parseValue(value);
    editor.commands.setContent(parsed, { emitUpdate: false });
  }, [value, editor]);

  // Keep editable in sync with prop.
  useEffect(() => {
    editor?.setEditable(!readOnly);
  }, [editor, readOnly]);

  if (!editor) {
    return (
      <div className="rich-text-field rich-text-field--loading">
        Loading editor…
      </div>
    );
  }

  return (
    <div className={`rich-text-field${readOnly ? " rich-text-field--readonly" : ""}`}>
      <RichTextToolbar editor={editor} uploadPrefix={uploadPrefix} />
      <EditorContent editor={editor} placeholder={placeholder} />
    </div>
  );
}

function RichTextToolbar({
  editor,
  uploadPrefix,
}: {
  editor: NonNullable<ReturnType<typeof useEditor>>;
  uploadPrefix: string;
}) {
  // The toolbar is a flat row of buttons. Disabled state reflects whether
  // the corresponding mark/node is currently active in the selection.
  const btn = (active: boolean) =>
    `rich-text-field__toolbar-btn${active ? " rich-text-field__toolbar-btn--active" : ""}`;

  return (
    <div className="rich-text-field__toolbar" role="toolbar" aria-label="Formatting">
      <button
        type="button"
        className={btn(editor.isActive("bold"))}
        onClick={() => editor.chain().focus().toggleBold().run()}
        title="Bold (⌘B)"
        aria-label="Bold"
      >
        <strong>B</strong>
      </button>
      <button
        type="button"
        className={btn(editor.isActive("italic"))}
        onClick={() => editor.chain().focus().toggleItalic().run()}
        title="Italic (⌘I)"
        aria-label="Italic"
      >
        <em>I</em>
      </button>
      <button
        type="button"
        className={btn(editor.isActive("code"))}
        onClick={() => editor.chain().focus().toggleCode().run()}
        title="Inline code"
        aria-label="Inline code"
      >
        <code>{"</>"}</code>
      </button>
      <span className="rich-text-field__toolbar-sep" aria-hidden />
      <button
        type="button"
        className={btn(editor.isActive("heading", { level: 1 }))}
        onClick={() => editor.chain().focus().toggleHeading({ level: 1 }).run()}
        title="Heading 1"
        aria-label="Heading 1"
      >
        H1
      </button>
      <button
        type="button"
        className={btn(editor.isActive("heading", { level: 2 }))}
        onClick={() => editor.chain().focus().toggleHeading({ level: 2 }).run()}
        title="Heading 2"
        aria-label="Heading 2"
      >
        H2
      </button>
      <button
        type="button"
        className={btn(editor.isActive("heading", { level: 3 }))}
        onClick={() => editor.chain().focus().toggleHeading({ level: 3 }).run()}
        title="Heading 3"
        aria-label="Heading 3"
      >
        H3
      </button>
      <span className="rich-text-field__toolbar-sep" aria-hidden />
      <button
        type="button"
        className={btn(editor.isActive("bulletList"))}
        onClick={() => editor.chain().focus().toggleBulletList().run()}
        title="Bulleted list"
        aria-label="Bulleted list"
      >
        • List
      </button>
      <button
        type="button"
        className={btn(editor.isActive("orderedList"))}
        onClick={() => editor.chain().focus().toggleOrderedList().run()}
        title="Numbered list"
        aria-label="Numbered list"
      >
        1. List
      </button>
      <button
        type="button"
        className={btn(editor.isActive("blockquote"))}
        onClick={() => editor.chain().focus().toggleBlockquote().run()}
        title="Quote"
        aria-label="Quote"
      >
        “
      </button>
      <button
        type="button"
        className={btn(editor.isActive("codeBlock"))}
        onClick={() => editor.chain().focus().toggleCodeBlock().run()}
        title="Code block"
        aria-label="Code block"
      >
        {"{ }"}
      </button>
      <span className="rich-text-field__toolbar-sep" aria-hidden />
      <InsertImageButton editor={editor} uploadPrefix={uploadPrefix} />
    </div>
  );
}

/**
 * InsertImageButton — picks an image file from disk, uploads it via
 * `useUploadFile`, and inserts an `<img>` node whose src points at the
 * binary `/api/files/raw?relPath=…` route. After upload the image is
 * hosted by the backend, so the rich-text doc round-trips correctly:
 * a reload of the document re-renders the same image.
 */
function InsertImageButton({
  editor,
  uploadPrefix,
}: {
  editor: NonNullable<ReturnType<typeof useEditor>>;
  uploadPrefix: string;
}) {
  const inputRef = useRef<HTMLInputElement | null>(null);
  const upload = useUploadFile();
  const [error, setError] = useState<string | null>(null);

  const handleChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
    setError(null);
    const file = e.target.files?.[0];
    if (!file) return;
    if (!file.type.startsWith("image/")) {
      setError(`Not an image: ${file.type || "unknown"}`);
      e.target.value = "";
      return;
    }
    const target = `${uploadPrefix.replace(/\/+$/, "")}/${file.name}`;
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const result = await upload.mutateAsync({ relPath: target, bytes });
      editor
        .chain()
        .focus()
        .setImage({ src: fileRawUrl(result.relPath), alt: file.name })
        .run();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
    e.target.value = "";
  };

  return (
    <span className="rich-text-field__image-btn">
      <button
        type="button"
        className="rich-text-field__toolbar-btn"
        onClick={() => inputRef.current?.click()}
        disabled={upload.isPending}
        title="Insert image"
        aria-label="Insert image"
      >
        {upload.isPending ? "…" : "🖼"}
      </button>
      <input
        ref={inputRef}
        type="file"
        accept="image/*"
        onChange={handleChange}
        className="rich-text-field__image-input"
        aria-hidden
      />
      {error && (
        <span className="rich-text-field__image-error" title={error}>
          !
        </span>
      )}
    </span>
  );
}

// Helpers ──────────────────────────────────────────────────────────────────

import { useEffect, useMemo } from "react";

const EMPTY_DOC = { type: "doc", content: [{ type: "paragraph" }] };

function parseValue(value: string): object {
  if (!value || !value.trim()) return EMPTY_DOC;
  try {
    const parsed = JSON.parse(value);
    if (parsed && typeof parsed === "object" && parsed.type === "doc") return parsed;
  } catch {
    /* fall through to plain-text handling */
  }
  // If the value isn't valid ProseMirror JSON, wrap it as plain text.
  // Tiptap handles this gracefully; the user sees their text in a single
  // paragraph and the wire format becomes valid JSON on the next edit.
  return {
    type: "doc",
    content: [
      {
        type: "paragraph",
        content: value ? [{ type: "text", text: value }] : [],
      },
    ],
  };
}
