/**
 * ConfigApp — the desktop app's Settings window: an editor for
 * solx-config.json.
 *
 * Rendered instead of `App` when the page is opened as `?view=settings`
 * (see main.tsx), in a window of its own. That window is the only one
 * granted the `solx:config` IPC permission: package widgets run in the main
 * window, and must not be able to rewrite the command and URL allowlists.
 * So nothing here mounts widgets or runs actions.
 *
 * Each section keeps its own draft and saves on its own; only keys that
 * differ from the file are sent, and the host validates the whole result
 * before writing. Keys that are only read at startup collect into a
 * "restart to apply" banner.
 */
import { useCallback, useEffect, useState } from "react";
import type { ConfigView } from "@solx/tauri";
import { getTauri, restartApp } from "../api/connection";
import { diffChanges, type SectionDef } from "./model";
import { SECTIONS } from "./sections";
import "../App.css";
import "./config.css";

type Drafts = Record<string, unknown>;

function changesFor(section: SectionDef<unknown>, draft: unknown, view: ConfigView) {
  return diffChanges(view.raw, section.toConfig(draft, view.raw));
}

/** Dirty if anything would be sent, or the draft can't even be expressed. */
function isDirty(section: SectionDef<unknown>, draft: unknown, view: ConfigView): boolean {
  if (section.readOnly) return false;
  try {
    return Object.keys(changesFor(section, draft, view)).length > 0;
  } catch {
    return true;
  }
}

function freshDrafts(view: ConfigView): Drafts {
  return Object.fromEntries(SECTIONS.map((s) => [s.id, s.fromConfig(view.raw)]));
}

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function ConfigApp() {
  const [view, setView] = useState<ConfigView | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Drafts>({});
  const [active, setActive] = useState(SECTIONS[0].id);
  const [errors, setErrors] = useState<Record<string, string | null>>({});
  const [saving, setSaving] = useState(false);
  const [savedFlash, setSavedFlash] = useState<string | null>(null);
  const [pendingRestart, setPendingRestart] = useState<string[]>([]);

  const load = useCallback(async () => {
    try {
      const next = await getTauri().config.get();
      setView(next);
      setDrafts(freshDrafts(next));
      setErrors({});
      setLoadError(null);
    } catch (e) {
      setLoadError(message(e));
    }
  }, []);

  useEffect(() => {
    document.title = "solx settings";
    void load();
  }, [load]);

  if (loadError) {
    return (
      <div className="config-app config-app--center">
        <div className="dialog__error">Couldn't read solx-config.json: {loadError}</div>
      </div>
    );
  }
  if (!view) {
    return <div className="config-app config-app--center dialog__hint">Loading settings…</div>;
  }

  const section = SECTIONS.find((s) => s.id === active) ?? SECTIONS[0];
  const draft = drafts[section.id];
  const dirty = isDirty(section, draft, view);

  const save = async () => {
    setErrors((e) => ({ ...e, [section.id]: null }));
    let changes;
    try {
      changes = changesFor(section, draft, view);
    } catch (e) {
      setErrors((errs) => ({ ...errs, [section.id]: message(e) }));
      return;
    }
    setSaving(true);
    try {
      const result = await getTauri().config.update(changes);
      const next = result.config;
      // Refresh this section, and any other that has no unsaved edits, from
      // the file as written (key order and legacy-key folding may differ).
      setDrafts((prev) =>
        Object.fromEntries(
          SECTIONS.map((s) => [
            s.id,
            s.id === section.id || !isDirty(s, prev[s.id], view) ? s.fromConfig(next.raw) : prev[s.id],
          ]),
        ),
      );
      setView(next);
      if (result.restartRequired.length) {
        setPendingRestart((p) => [...new Set([...p, ...result.restartRequired])]);
      }
      setSavedFlash(section.id);
      window.setTimeout(() => setSavedFlash(null), 2000);
    } catch (e) {
      setErrors((errs) => ({ ...errs, [section.id]: message(e) }));
    } finally {
      setSaving(false);
    }
  };

  const revert = () => {
    setDrafts((d) => ({ ...d, [section.id]: section.fromConfig(view.raw) }));
    setErrors((e) => ({ ...e, [section.id]: null }));
  };

  const Editor = section.Editor;

  return (
    <div className="config-app">
      {pendingRestart.length > 0 && (
        <div className="app-banner app-banner--warning" role="alert">
          <strong>Restart solx to apply</strong>{" "}
          <span>{pendingRestart.join(", ")}.</span>
          <button className="app-banner__btn" onClick={() => void restartApp()}>
            Restart now
          </button>
        </div>
      )}
      <div className="config-app__body">
        <nav className="config-nav" aria-label="Settings sections">
          {SECTIONS.map((s) => (
            <button
              key={s.id}
              className={`config-nav__item${s.id === active ? " is-active" : ""}`}
              onClick={() => setActive(s.id)}
            >
              {s.title}
              {isDirty(s, drafts[s.id], view) && <span className="config-nav__dot" aria-label="unsaved changes" />}
            </button>
          ))}
          <button className="config-nav__reload dialog__btn" onClick={() => void load()} title="Re-read solx-config.json">
            Reload from file
          </button>
        </nav>

        <main className="config-section">
          <header className="config-section__header">
            <h1 className="config-section__title">{section.title}</h1>
            <p className="dialog__hint">{section.description}</p>
          </header>
          <div className="config-section__body">
            <Editor draft={draft} setDraft={(d: unknown) => setDrafts((all) => ({ ...all, [section.id]: d }))} view={view} />
          </div>
          {errors[section.id] && <div className="dialog__error">{errors[section.id]}</div>}
          {!section.readOnly && (
            <footer className="dialog__footer config-section__footer">
              {savedFlash === section.id && <span className="settings-panel__flash">Saved ✓</span>}
              <button className="dialog__btn" onClick={revert} disabled={!dirty || saving}>
                Revert
              </button>
              <button className="dialog__btn dialog__btn--primary" onClick={() => void save()} disabled={!dirty || saving}>
                {saving ? "Saving…" : "Save"}
              </button>
            </footer>
          )}
        </main>
      </div>
    </div>
  );
}
