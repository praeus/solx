/**
 * Shared CodeMirror 6 theme — one syntax palette and one editor chrome for
 * every code surface in the app (JsonEditField, JsonSchemaEditor, FileEditor).
 *
 * Two things this centralises that used to be per-component:
 *
 *   1. Dark mode. The editors previously either hardcoded a light palette
 *      (`JsonEditField` passed `{ dark: false }` with fixed hex colors) or
 *      tracked `prefers-color-scheme` by hand to flip a Monaco theme string.
 *      `usePrefersDark()` is now the single subscription and both palettes
 *      live side by side below.
 *
 *   2. Touch sizing. `.cm-content` is a contenteditable, and iOS Safari
 *      force-zooms the page when one is focused with a font under 16px —
 *      and does not zoom back out. `editorTheme` therefore takes the base
 *      font size from a media query rather than a constant.
 */

import { useEffect, useState } from "react";
import { EditorView } from "@codemirror/view";
import { HighlightStyle } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";

/** Syntax colors. Kept as literals — these are ink on the editor surface,
 *  not app chrome, so they don't track the app's CSS custom properties
 *  beyond the accent and the muted/faint greys. */
const LIGHT = {
  keyword: "#7c3aed",
  string: "#b45309",
  number: "#0369a1",
  atom: "#be185d",
  comment: "#6b7280",
  typeName: "#0f766e",
  tagName: "#b91c1c",
  attrName: "#a16207",
  heading: "#1d4ed8",
};

const DARK = {
  keyword: "#c4b5fd",
  string: "#fcd34d",
  number: "#7dd3fc",
  atom: "#f9a8d4",
  comment: "#6b7280",
  typeName: "#5eead4",
  tagName: "#fca5a5",
  attrName: "#fde68a",
  heading: "#93c5fd",
};

function buildHighlightStyle(dark: boolean): HighlightStyle {
  const c = dark ? DARK : LIGHT;
  return HighlightStyle.define([
    { tag: t.keyword, color: c.keyword },
    { tag: t.controlKeyword, color: c.keyword },
    { tag: t.moduleKeyword, color: c.keyword },
    { tag: t.operatorKeyword, color: c.keyword },
    { tag: t.definitionKeyword, color: c.keyword },
    { tag: t.propertyName, color: "var(--accent)" },
    { tag: t.string, color: c.string },
    { tag: t.special(t.string), color: c.string },
    { tag: t.number, color: c.number },
    { tag: t.bool, color: c.atom },
    { tag: t.null, color: c.atom, fontStyle: "italic" },
    { tag: t.atom, color: c.atom },
    { tag: t.comment, color: c.comment, fontStyle: "italic" },
    { tag: t.lineComment, color: c.comment, fontStyle: "italic" },
    { tag: t.blockComment, color: c.comment, fontStyle: "italic" },
    { tag: t.typeName, color: c.typeName },
    { tag: t.className, color: c.typeName },
    { tag: t.namespace, color: c.typeName },
    { tag: t.tagName, color: c.tagName },
    { tag: t.attributeName, color: c.attrName },
    { tag: t.function(t.variableName), color: c.typeName },
    { tag: t.variableName, color: "var(--text)" },
    { tag: t.bracket, color: "var(--text-muted)" },
    { tag: t.punctuation, color: "var(--text-muted)" },
    { tag: t.heading, color: c.heading, fontWeight: "600" },
    { tag: t.link, color: c.heading, textDecoration: "underline" },
    { tag: t.emphasis, fontStyle: "italic" },
    { tag: t.strong, fontWeight: "600" },
    { tag: t.strikethrough, textDecoration: "line-through" },
    { tag: t.invalid, color: "var(--danger)" },
  ]);
}

const LIGHT_HIGHLIGHT = buildHighlightStyle(false);
const DARK_HIGHLIGHT = buildHighlightStyle(true);

export function highlightStyleFor(dark: boolean): HighlightStyle {
  return dark ? DARK_HIGHLIGHT : LIGHT_HIGHLIGHT;
}

export interface EditorThemeOptions {
  /** Fixed height (any CSS length). Omit for auto-height. */
  height?: string;
  /** Minimum height expressed in lines, when `height` is not set. */
  minLines?: number;
  /** Maximum height (any CSS length) before the editor scrolls internally. */
  maxHeight?: string;
}

/**
 * Editor chrome — backgrounds, gutters, selection, and the touch affordances
 * (16px content on coarse pointers, momentum scrolling, a caret and selection
 * that survive a dark background).
 */
export function editorTheme(dark: boolean, opts: EditorThemeOptions = {}) {
  const { height, minLines = 6, maxHeight } = opts;
  return EditorView.theme(
    {
      "&": {
        backgroundColor: "var(--bg)",
        color: "var(--text)",
        fontSize: "13px",
        height: height ?? "auto",
        ...(height ? {} : { minHeight: `${minLines * 1.5}em` }),
        ...(maxHeight ? { maxHeight } : {}),
      },
      "&.cm-focused": { outline: "2px solid var(--accent)", outlineOffset: "-2px" },
      ".cm-content": {
        fontFamily: "var(--font-mono)",
        caretColor: "var(--accent)",
        // Comfortable touch target for line-tapping without looking airy.
        lineHeight: "1.5",
      },
      ".cm-scroller": {
        fontFamily: "var(--font-mono)",
        lineHeight: "1.5",
        // Momentum scrolling inside the editor on iOS.
        WebkitOverflowScrolling: "touch",
        ...(maxHeight ? { overflow: "auto" } : {}),
      },
      ".cm-gutters": {
        backgroundColor: "var(--bg-sunken)",
        color: "var(--text-faint)",
        border: "none",
      },
      ".cm-activeLine": { backgroundColor: "var(--bg-raised)" },
      ".cm-activeLineGutter": {
        backgroundColor: "var(--bg-raised)",
        color: "var(--text-muted)",
      },
      ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--accent)" },
      "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": {
        backgroundColor: "var(--accent-soft)",
      },
      ".cm-selectionMatch": { backgroundColor: "var(--accent-soft)" },
      ".cm-matchingBracket, &.cm-focused .cm-matchingBracket": {
        backgroundColor: "var(--accent-soft)",
        outline: "1px solid var(--accent)",
      },
      ".cm-placeholder": { color: "var(--text-faint)" },
      ".cm-tooltip": {
        backgroundColor: "var(--bg-raised)",
        border: "1px solid var(--border-strong)",
        borderRadius: "var(--radius)",
        color: "var(--text)",
      },
      ".cm-panels": { backgroundColor: "var(--bg-raised)", color: "var(--text)" },
      // iOS zooms the page when a contenteditable under 16px takes focus.
      "@media (max-width: 700px), (pointer: coarse)": {
        "&": { fontSize: "16px" },
      },
    },
    { dark },
  );
}

/**
 * Subscribe to the OS colour scheme. Returns the current preference and
 * re-renders on change, so a theme switch repaints the editor live.
 */
export function usePrefersDark(): boolean {
  const [dark, setDark] = useState(() => {
    if (typeof window === "undefined" || !window.matchMedia) return false;
    return window.matchMedia("(prefers-color-scheme: dark)").matches;
  });

  useEffect(() => {
    if (typeof window === "undefined" || !window.matchMedia) return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const listener = (e: MediaQueryListEvent) => setDark(e.matches);
    mq.addEventListener("change", listener);
    return () => mq.removeEventListener("change", listener);
  }, []);

  return dark;
}
