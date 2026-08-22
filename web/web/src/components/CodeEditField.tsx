/**
 * CodeEditField — the app's general-purpose CodeMirror 6 surface.
 *
 * One editor implementation shared by every code-editing context:
 *   - `JsonSchemaEditor` (type schemas, JSON + parse badge)
 *   - `FileEditor` (authoring a new text file in any of the languages in
 *     `codemirror/languages.ts`)
 *   - anywhere else that needs a code input
 *
 * This replaced `@monaco-editor/react`. Monaco has no real touch support —
 * selection handles, scrolling, and IME composition all misbehave on
 * phones — and it shipped every grammar plus web workers regardless of
 * which language was in use. CodeMirror 6 handles touch natively, and the
 * grammars here are dynamically imported, so a plain-text file costs
 * nothing extra.
 *
 * Language, read-only state and theme each live in their own `Compartment`
 * so they reconfigure in place; the view is created once and never torn
 * down for a prop change.
 */

import { useEffect, useMemo, useRef } from "react";
import { Compartment, EditorState, type Extension } from "@codemirror/state";
import {
  EditorView,
  highlightActiveLine,
  keymap,
  lineNumbers,
  placeholder as cmPlaceholder,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import {
  bracketMatching,
  indentOnInput,
  syntaxHighlighting,
} from "@codemirror/language";
import { editorTheme, highlightStyleFor, usePrefersDark } from "./codemirror/theme";
import { loadLanguage, type LanguageId } from "./codemirror/languages";

interface Props {
  /** Current document text. */
  value: string;
  /** Called on every user edit. */
  onChange: (next: string) => void;
  /** Grammar to highlight with. Defaults to plain text. */
  language?: LanguageId;
  /** Read-only mode (still selectable and copyable). */
  readOnly?: boolean;
  /** Fixed height (any CSS length). Omit for auto-height. */
  height?: string;
  /** Minimum height in lines when `height` is not given. */
  minLines?: number;
  /** Maximum height before the editor scrolls internally. */
  maxHeight?: string;
  /** Shown in the empty document. */
  placeholder?: string;
  /** Hide the line-number gutter (useful for short single-value inputs). */
  hideLineNumbers?: boolean;
  /** Extra extensions (linters, gutters) appended after the defaults. */
  extensions?: Extension[];
  /** Accessible name for the editor region. */
  ariaLabel?: string;
}

export function CodeEditField({
  value,
  onChange,
  language = "plaintext",
  readOnly = false,
  height,
  minLines = 6,
  maxHeight,
  placeholder,
  hideLineNumbers = false,
  extensions,
  ariaLabel,
}: Props) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const viewRef = useRef<EditorView | null>(null);
  const dark = usePrefersDark();

  // The view is mounted once, so the update listener would otherwise close
  // over the first render's `onChange`. Route through a ref instead.
  const onChangeRef = useRef(onChange);
  useEffect(() => {
    onChangeRef.current = onChange;
  }, [onChange]);

  const languageCompartment = useMemo(() => new Compartment(), []);
  const readOnlyCompartment = useMemo(() => new Compartment(), []);
  const themeCompartment = useMemo(() => new Compartment(), []);

  // Mount once.
  useEffect(() => {
    if (!containerRef.current) return;

    const base: Extension[] = [
      history(),
      bracketMatching(),
      indentOnInput(),
      highlightActiveLine(),
      keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
      // Wrapping rather than horizontal scrolling: a phone has no room for
      // a second scroll axis inside a dialog that already scrolls.
      EditorView.lineWrapping,
      languageCompartment.of([]),
      readOnlyCompartment.of(EditorState.readOnly.of(readOnly)),
      // Seeded with the current scheme rather than empty, so the first
      // paint is already themed; the effect below only handles changes.
      themeCompartment.of([
        editorTheme(dark, { height, minLines, maxHeight }),
        syntaxHighlighting(highlightStyleFor(dark), { fallback: true }),
      ]),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) onChangeRef.current(update.state.doc.toString());
      }),
    ];

    if (!hideLineNumbers) base.unshift(lineNumbers());
    if (placeholder) base.push(cmPlaceholder(placeholder));
    if (extensions) base.push(...extensions);

    const view = new EditorView({
      state: EditorState.create({ doc: value, extensions: base }),
      parent: containerRef.current,
    });
    viewRef.current = view;

    return () => {
      view.destroy();
      viewRef.current = null;
    };
    // Mount-only: every prop below is reconciled by its own effect.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // External value → editor. Skipped when they already agree, so typing
  // isn't clobbered by the controlled value echoing back.
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    const current = view.state.doc.toString();
    if (current !== value) {
      view.dispatch({ changes: { from: 0, to: current.length, insert: value } });
    }
  }, [value]);

  // Language — resolved asynchronously, so guard against the component
  // unmounting or the language changing again while the import is in flight.
  useEffect(() => {
    let cancelled = false;
    void loadLanguage(language).then((ext) => {
      if (cancelled) return;
      const view = viewRef.current;
      if (!view) return;
      view.dispatch({ effects: languageCompartment.reconfigure(ext ?? []) });
    });
    return () => {
      cancelled = true;
    };
  }, [language, languageCompartment]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: readOnlyCompartment.reconfigure(EditorState.readOnly.of(readOnly)),
    });
  }, [readOnly, readOnlyCompartment]);

  // Theme + syntax palette, repainted live when the OS scheme flips.
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: themeCompartment.reconfigure([
        editorTheme(dark, { height, minLines, maxHeight }),
        syntaxHighlighting(highlightStyleFor(dark), { fallback: true }),
      ]),
    });
  }, [dark, height, minLines, maxHeight, themeCompartment]);

  return (
    <div
      ref={containerRef}
      className="code-edit-field"
      role="group"
      aria-label={ariaLabel}
    />
  );
}
