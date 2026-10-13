/**
 * DocumentPreview — render a Document's contents using an EJS template
 * declared in the Type's JSON schema.
 *
 * Solx uses tera for templates on the Rust side, but we don't need to
 * round-trip through the backend for the preview pane — we can render
 * client-side with EJS. The template lives in the type's schema under a
 * well-known meta tag, e.g.:
 *
 *   {
 *     "type": "object",
 *     "properties": { "title": { "type": "string" } },
 *     "x-sol-template": "<h1><%= title %></h1>"
 *   }
 *
 * If no template is provided, we fall back to a small default that dumps
 * the contents map as a definition list. Both the title (`x-sol-template`)
 * and an alias (`x-sol-preview`) are accepted.
 *
 * EJS is run via the `ejs` runtime in the browser. We pass `data` =
 * `{ contents: <doc.contents>, document: <doc summary> }` so templates
 * can do `<%= contents.title %>` and `<%= document.path %>` etc.
 *
 * NOTE: EJS templates come from the user's local type schema. They are
 * trusted (the user wrote the type themselves), so `dangerouslySetInnerHTML`
 * is acceptable here.
 *
 * Files-store images: `solx-server` requires a bearer token on every route
 * (`/files/*rel_path` included), so a template can't just point `<img src>`
 * at a relPath the way the old Bun backend allowed. Instead, a template
 * marks such an image with `data-relpath="<encodeURIComponent(relPath)>"`
 * (no `src`), and the effect below resolves each one to a `blob:` URL via
 * `objectUrlCache` after render — the same mechanism `ResolvedImage.tsx`
 * uses for rich-text images.
 *
 * `dangerouslySetInnerHTML`'s value must stay referentially stable across
 * renders that don't actually change `html` — React's prop diffing for it
 * is a shallow `!==` on the `{ __html }` object itself, not on the string
 * inside. An inline `{ __html: html }` literal is a new object every
 * render, so React would call `innerHTML =` again on every unrelated
 * re-render (parent state changes, query refetches, …), silently
 * discarding the `img.src` the effect below had just set. `useMemo`
 * keyed on `html` keeps that object's identity stable when the string
 * hasn't changed.
 */

import { useEffect, useMemo, useRef } from "react";
import ejs from "ejs";
import type { DocumentSummary, TypeEntity } from "../api/api";
import { acquireObjectUrl, releaseObjectUrl } from "../api/objectUrlCache";

interface Props {
  /** The document being previewed (provides `contents`, `path`, etc.). */
  document: DocumentSummary;
  /** The document's Type (carries the schema with the optional template). */
  type: TypeEntity | null;
}

interface TemplateBundle {
  template: string;
  source: string;
}

const FALLBACK_TEMPLATE = `<div class="doc-preview doc-preview--fallback">
  <h1><%= document.title || document.name %></h1>
  <p class="doc-preview__meta">
    <code><%= document.path %>/<%= document.name %></code>
    <% if (document.type_name) { %> · Type: <code><%= document.type_name %></code><% } %>
  </p>
  <% if (document.summary) { %><p class="doc-preview__summary"><%= document.summary %></p><% } %>
  <h2>Contents</h2>
  <dl class="doc-preview__fields">
    <% for (const [key, value] of Object.entries(contents || {})) { %>
      <dt><%= key %></dt>
      <dd><pre><code><%= JSON.stringify(value, null, 2) %></code></pre></dd>
    <% } %>
  </dl>
</div>`;

export function DocumentPreview({ document, type }: Props) {
  const { template, source } = useMemo<TemplateBundle>(
    () => resolveTemplate(type),
    [type],
  );

  const html = useMemo(() => {
    try {
      return ejs.render(template, {
        document,
        contents: document.contents ?? {},
      }) as string;
    } catch (err) {
      return `<div class="doc-editor-preview-error">Template error: ${escapeHtml(
        err instanceof Error ? err.message : String(err),
      )}<br/><small>Source: <code>${escapeHtml(source)}</code></small></div>`;
    }
  }, [template, document, source]);

  // Stable identity across renders where `html` hasn't changed — see the
  // file docblock for why this matters.
  const innerHtml = useMemo(() => ({ __html: html }), [html]);

  const containerRef = useRef<HTMLDivElement | null>(null);

  // Resolve any `data-relpath` images the template emitted into `blob:`
  // URLs, since a plain `src` can't carry the bearer token `solx-server`
  // requires. Re-runs whenever the rendered HTML changes.
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const imgs = Array.from(
      container.querySelectorAll<HTMLImageElement>("img[data-relpath]"),
    );
    const relPaths = imgs.map((img) => decodeURIComponent(img.dataset.relpath ?? ""));
    let cancelled = false;
    imgs.forEach((img, i) => {
      const relPath = relPaths[i];
      if (!relPath) return;
      acquireObjectUrl(relPath).then((url) => {
        if (!cancelled) img.src = url;
      });
    });
    return () => {
      cancelled = true;
      relPaths.forEach((relPath) => {
        if (relPath) releaseObjectUrl(relPath);
      });
    };
  }, [html]);

  return (
    <div
      ref={containerRef}
      className="doc-preview"
      // `document` and `template` are both from the user's local solx
      // data. Trust boundary is the type schema, written by the user.
      dangerouslySetInnerHTML={innerHtml}
    />
  );
}

/**
 * Resolve the template: prefer `x-sol-template`, fall back to
 * `x-sol-preview`, fall back to the built-in default.
 */
function resolveTemplate(type: TypeEntity | null): TemplateBundle {
  const schema = type?.schema;
  if (schema && typeof schema === "object") {
    const s = schema as Record<string, unknown>;
    const candidate = s["x-sol-template"] ?? s["x-sol-preview"];
    if (typeof candidate === "string" && candidate.length > 0) {
      return { template: candidate, source: "type schema (x-sol-template)" };
    }
  }
  return { template: FALLBACK_TEMPLATE, source: "built-in fallback" };
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}
