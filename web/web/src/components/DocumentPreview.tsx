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
 * If no template is provided, we fall back to a small built-in view that
 * dumps the contents map as a definition list. Both the title
 * (`x-sol-template`) and an alias (`x-sol-preview`) are accepted. Templates
 * get `{ contents: <doc.contents>, document: <doc summary> }`, so they can
 * do `<%= contents.title %>` and `<%= document.path %>` etc.
 *
 * ## Templates are untrusted
 *
 * An EJS template is arbitrary JavaScript (`<% … %>`), and a type is not
 * necessarily written by the user: MCP clients, WASM guests, scripts and
 * packages can all save types. Run in this window, a template would hold
 * everything the app holds — Tauri IPC (exec any action, write files) or,
 * in a browser, the server's bearer token. So a template never runs here:
 *
 * - It is *compiled* here, which turns it into a function body (and
 *   reports syntax errors) without running any of the template's code.
 * - That source is posted into an `<iframe sandbox="allow-scripts">`
 *   — an opaque origin with no access to this window, its storage, or IPC —
 *   whose CSP (`default-src 'none'`) also refuses any network request.
 * - The sandbox posts back only its content height and the `data-relpath`
 *   images it wants (below). Messages are checked by `source` on both sides.
 *
 * What a template can still do is navigate its own frame, which could carry
 * the document it was given (or its images) out in a URL. That is no more
 * than an MCP client could already read, and none of the app's authority.
 *
 * Files-store images: `solx-server` requires a bearer token on every route
 * (`/files/*rel_path` included), so a template can't just point `<img src>`
 * at a relPath. Instead, a template marks such an image with
 * `data-relpath="<encodeURIComponent(relPath)>"` (no `src`); this component
 * fetches the bytes through `objectUrlCache` and hands the sandbox a `Blob`.
 * Only files the document itself references (its `files`, or a string in
 * its `contents`) are served, so a template can't read the rest of the store.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import ejs from "ejs";
import type { DocumentSummary, TypeEntity } from "../api/api";
import { acquireObjectUrl, releaseObjectUrl } from "../api/objectUrlCache";

interface Props {
  /** The document being previewed (provides `contents`, `path`, etc.). */
  document: DocumentSummary;
  /** The document's Type (carries the schema with the optional template). */
  type: TypeEntity | null;
}

export function DocumentPreview({ document, type }: Props) {
  const template = useMemo(() => resolveTemplate(type), [type]);
  if (template === null) return <FallbackPreview document={document} />;
  return <SandboxedPreview document={document} template={template} />;
}

/** The built-in view, rendered by React: no template, nothing to sandbox. */
function FallbackPreview({ document }: { document: DocumentSummary }) {
  const contents =
    document.contents && typeof document.contents === "object" && !Array.isArray(document.contents)
      ? Object.entries(document.contents)
      : [];
  return (
    <div className="doc-preview doc-preview--fallback">
      <h1>{document.title || document.name}</h1>
      <p className="doc-preview__meta">
        <code>
          {document.path}/{document.name}
        </code>
        {document.typeRef && (
          <>
            {" · Type: "}
            <code>{document.typeRef}</code>
          </>
        )}
      </p>
      {document.summary && <p className="doc-preview__summary">{document.summary}</p>}
      <h2>Contents</h2>
      <dl className="doc-preview__fields">
        {contents.map(([key, value]) => (
          <div key={key} style={{ display: "contents" }}>
            <dt>{key}</dt>
            <dd>
              <pre>
                <code>{JSON.stringify(value, null, 2)}</code>
              </pre>
            </dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

/** Messages the sandbox posts to this window. */
type FromSandbox =
  | { kind: "height"; height: number }
  | { kind: "images"; generation: number; relPaths: string[] };

function SandboxedPreview({ document, template }: { document: DocumentSummary; template: string }) {
  const frameRef = useRef<HTMLIFrameElement | null>(null);
  const [ready, setReady] = useState(false);
  const [height, setHeight] = useState(0);
  const generation = useRef(0);
  const srcDoc = useMemo(sandboxDocument, []);

  // Compiling builds (and so syntax-checks) the function without calling
  // it: none of the template's code runs here. Only the generated body is
  // kept — the function `compile` returns closes over this window's EJS
  // runtime, which is exactly what must not reach the sandbox.
  const compiled = useMemo(() => {
    try {
      const compiler = new ejs.Template(template, { compileDebug: false });
      compiler.compile();
      return { source: compiler.source };
    } catch (err) {
      return { error: err instanceof Error ? err.message : String(err) };
    }
  }, [template]);

  const allowed = useMemo(() => referencedRelPaths(document), [document]);

  // Everything the sandbox says. `source` is checked so another frame (or
  // the template navigating its own frame elsewhere) can't speak for it.
  useEffect(() => {
    const acquired: string[] = [];
    const onMessage = (event: MessageEvent) => {
      const frame = frameRef.current?.contentWindow;
      if (!frame || event.source !== frame) return;
      const msg = event.data as FromSandbox;
      if (msg?.kind === "height" && Number.isFinite(msg.height)) {
        setHeight(Math.ceil(msg.height));
      } else if (msg?.kind === "images" && Array.isArray(msg.relPaths)) {
        for (const relPath of new Set(msg.relPaths)) {
          if (typeof relPath !== "string" || !allowed.has(relPath)) continue;
          acquired.push(relPath);
          acquireObjectUrl(relPath)
            .then((url) => fetch(url))
            .then((res) => res.blob())
            .then((blob) =>
              frame.postMessage({ kind: "image", generation: msg.generation, relPath, blob }, "*"),
            )
            .catch(() => {});
        }
      }
    };
    window.addEventListener("message", onMessage);
    return () => {
      window.removeEventListener("message", onMessage);
      acquired.forEach(releaseObjectUrl);
    };
  }, [allowed]);

  useEffect(() => {
    if (!ready || !compiled.source) return;
    generation.current += 1;
    frameRef.current?.contentWindow?.postMessage(
      {
        kind: "render",
        generation: generation.current,
        source: compiled.source,
        // Structured-cloned, so the sandbox gets a copy and nothing live.
        data: { document, contents: document.contents ?? {} },
      },
      "*",
    );
  }, [ready, compiled, document]);

  if (compiled.error !== undefined) {
    return (
      <div className="doc-editor-preview-error">
        Template error: {compiled.error}
        <br />
        <small>
          Source: <code>type schema (x-sol-template)</code>
        </small>
      </div>
    );
  }

  return (
    <iframe
      ref={frameRef}
      className="doc-preview"
      title="Document preview"
      // `allow-scripts` without `allow-same-origin`: an opaque origin. Never
      // add `allow-same-origin` here — with `allow-scripts` it lets the
      // frame remove its own sandbox.
      sandbox="allow-scripts"
      srcDoc={srcDoc}
      // Not a message from the sandbox: a `srcdoc` frame can finish
      // loading before an effect could add a listener for one. `load` fires
      // after the frame's script has run and installed its own listener.
      onLoad={() => setReady(true)}
      style={{ width: "100%", height, border: 0, display: "block" }}
    />
  );
}

/**
 * Resolve the template: prefer `x-sol-template`, fall back to
 * `x-sol-preview`, else `null` for the built-in view.
 */
function resolveTemplate(type: TypeEntity | null): string | null {
  const schema = type?.schema;
  if (schema && typeof schema === "object") {
    const s = schema as Record<string, unknown>;
    const candidate = s["x-sol-template"] ?? s["x-sol-preview"];
    if (typeof candidate === "string" && candidate.length > 0) return candidate;
  }
  return null;
}

/** Every file the document references: its `files`, and any string in its
 * `contents` (where image fields keep their relPath). */
function referencedRelPaths(document: DocumentSummary): Set<string> {
  const out = new Set<string>();
  for (const file of document.files ?? []) out.add(file.relPath);
  const walk = (value: unknown) => {
    if (typeof value === "string") out.add(value);
    else if (Array.isArray(value)) value.forEach(walk);
    else if (value && typeof value === "object") Object.values(value).forEach(walk);
  };
  walk(document.contents);
  return out;
}

/** The app's own stylesheets as text, so a template renders in the app's
 * look. A sheet that can't be read (cross-origin) is skipped. */
function appStyles(): string {
  const rules: string[] = [];
  for (const sheet of Array.from(window.document.styleSheets)) {
    try {
      for (const rule of Array.from(sheet.cssRules)) rules.push(rule.cssText);
    } catch {
      // Not readable from here; the preview just goes without it.
    }
  }
  return rules.join("\n");
}

/** The sandbox's CSP. No `connect-src`, `img-src` beyond `blob:`/`data:`,
 * or anything else that reaches the network; scripts only inline, plus
 * `unsafe-eval` for turning the posted source back into a function. */
const SANDBOX_CSP =
  "default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval'; style-src 'unsafe-inline'; img-src blob: data:; font-src data:";

/**
 * The sandbox's whole document. The CSP `<meta>` comes first so it is in
 * force before any script runs; a policy set this way can't be lifted by
 * removing the element later.
 *
 * Inside: `render` turns the posted source back into the EJS template
 * function — supplying EJS's own `<%= %>` escaping, and refusing
 * `include` — and fills `#root`, then asks for its `data-relpath` images;
 * `image` fills them in, ignoring answers for an older render.
 */
function sandboxDocument(): string {
  const bootstrap = `(() => {
  const root = document.getElementById("root");
  let generation = 0;
  let urls = [];
  const post = (msg) => parent.postMessage(msg, "*");
  const ENTITIES = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&#34;", "'": "&#39;" };
  const escapeFn = (v) => (v == null ? "" : String(v).replace(/[&<>'"]/g, (c) => ENTITIES[c]));
  const include = () => { throw new Error("include is not supported in a preview template"); };
  const relPathOf = (img) => {
    try { return decodeURIComponent(img.dataset.relpath || ""); } catch { return ""; }
  };
  const postHeight = () => post({ kind: "height", height: document.documentElement.scrollHeight });
  // Reported after each render and image load as well: an observer
  // callback waits for a rendering frame, which a hidden frame may not get.
  new ResizeObserver(postHeight).observe(root);
  addEventListener("message", (event) => {
    if (event.source !== parent) return;
    const msg = event.data || {};
    if (msg.kind === "render") {
      generation = msg.generation;
      urls.forEach((u) => URL.revokeObjectURL(u));
      urls = [];
      try {
        const render = new Function("locals, escapeFn, include, rethrow", msg.source);
        root.innerHTML = render(msg.data, escapeFn, include, null);
      } catch (err) {
        root.innerHTML = "";
        const box = document.createElement("div");
        box.className = "doc-editor-preview-error";
        box.textContent = "Template error: " + (err && err.message ? err.message : String(err));
        root.appendChild(box);
      }
      for (const img of root.querySelectorAll("img")) img.addEventListener("load", postHeight);
      postHeight();
      const relPaths = Array.from(root.querySelectorAll("img[data-relpath]"), relPathOf).filter(Boolean);
      if (relPaths.length) post({ kind: "images", generation, relPaths });
    } else if (msg.kind === "image" && msg.generation === generation && msg.blob instanceof Blob) {
      const url = URL.createObjectURL(msg.blob);
      urls.push(url);
      for (const img of root.querySelectorAll("img[data-relpath]")) {
        if (relPathOf(img) === msg.relPath) img.src = url;
      }
    }
  });
})();`;
  // `<\/` keeps a stray `</style>` or `</script>` in the app's CSS from
  // closing the element early.
  const css = appStyles().replace(/<\//g, "<\\/");
  return `<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${SANDBOX_CSP}">
<style>${css}
html, body { margin: 0; padding: 0; background: transparent; }
</style>
</head>
<body>
<div id="root" class="doc-preview"></div>
<script>${bootstrap}</script>
</body>
</html>`;
}
