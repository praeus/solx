/**
 * TypeSelector — searchable combobox for picking a Type by full ref
 * (`${path}/${name}`), with inline "Edit" (opens the existing type) and
 * "+ New" (creates one without leaving the parent form) buttons that both
 * reuse the shared `TypeEditor` dialog.
 *
 * Differs from old sol-browser's `TypeSelector` in one deliberate way: that
 * version let you commit arbitrary free-typed text as the value (sol had no
 * path/name identity to resolve, just a bare type name). Here the value is
 * a `path`+`name` pair, so typing only filters the dropdown — committing a
 * value happens by picking an existing type from the list or via "+ New"
 * (which reports back its own `path`/`name` through `onSaved`). This is
 * safer for `param_type_ref`/`result_type_ref`, which must resolve to a
 * real Type entity.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { TypeEntity } from "../api/api";
import { useDocumentTypes } from "../api/hooks";
import { TypeEditor } from "./TypeEditor";

interface Props {
  label: string;
  /** Full type ref, e.g. "/types/core/Null", or "" for none. */
  value: string;
  onChange: (ref: string) => void;
  readOnly?: boolean;
  /** Default path offered when creating a new type from here. */
  defaultPath?: string;
}

function refOf(t: TypeEntity): string {
  return `${t.path}/${t.name}`;
}

export function TypeSelector({ label, value, onChange, readOnly, defaultPath = "/types" }: Props) {
  const typesQuery = useDocumentTypes();
  const types = useMemo(() => typesQuery.data?.items ?? [], [typesQuery.data]);

  const [open, setOpen] = useState(false);
  // While actively typing, `query` holds the filter text; null means the
  // input just mirrors the committed value, and opening shows every type.
  const [query, setQuery] = useState<string | null>(null);
  const [highlight, setHighlight] = useState(-1);
  const comboRef = useRef<HTMLDivElement>(null);

  const [editor, setEditor] = useState<{
    open: boolean;
    existing: { path: string; name: string } | null;
    existingEntity: TypeEntity | null;
  }>({ open: false, existing: null, existingEntity: null });

  const selected = useMemo(() => types.find((t) => refOf(t) === value) ?? null, [types, value]);

  const sortedNames = useMemo(
    () => types.slice().sort((a, b) => a.name.localeCompare(b.name)),
    [types],
  );

  const shown = useMemo(() => {
    const filter = (query ?? "").trim().toLowerCase();
    if (!filter) return sortedNames;
    return sortedNames.filter((t) => t.name.toLowerCase().includes(filter));
  }, [sortedNames, query]);

  const closeDropdown = useCallback(() => {
    setOpen(false);
    setQuery(null);
    setHighlight(-1);
  }, []);

  const selectType = useCallback(
    (t: TypeEntity | null) => {
      onChange(t ? refOf(t) : "");
      closeDropdown();
    },
    [onChange, closeDropdown],
  );

  // Close when clicking outside the combobox.
  useEffect(() => {
    if (!open) return;
    const onDocMouseDown = (e: MouseEvent) => {
      if (comboRef.current && !comboRef.current.contains(e.target as Node)) closeDropdown();
    };
    document.addEventListener("mousedown", onDocMouseDown);
    return () => document.removeEventListener("mousedown", onDocMouseDown);
  }, [open, closeDropdown]);

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (readOnly) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (!open) {
        setOpen(true);
        return;
      }
      setHighlight((h) => Math.min(h + 1, shown.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setHighlight((h) => Math.max(h - 1, 0));
    } else if (e.key === "Enter") {
      if (open && highlight >= 0 && highlight < shown.length) {
        e.preventDefault();
        selectType(shown[highlight]);
      }
    } else if (e.key === "Escape") {
      if (open) {
        e.preventDefault();
        closeDropdown();
      }
    }
  };

  const handleEdit = () => {
    if (!selected) return;
    setEditor({
      open: true,
      existing: { path: selected.path, name: selected.name },
      existingEntity: selected,
    });
  };

  const handleNew = () => {
    setEditor({ open: true, existing: null, existingEntity: null });
  };

  // displayValue falls back to the raw ref if it doesn't (yet) resolve to a
  // loaded type — e.g. the list is still loading — so nothing looks blank.
  const displayValue = query ?? (selected ? selected.name : value);
  const hasValue = value.trim().length > 0;

  return (
    <div className="form-field">
      <label className="form-label">{label}</label>
      <div className="type-selector__row">
        <div className="type-selector__combo" ref={comboRef}>
          <input
            className="form-input type-selector__input"
            value={displayValue}
            onChange={(e) => {
              setQuery(e.target.value);
              setOpen(true);
              setHighlight(-1);
            }}
            onFocus={() => {
              if (!readOnly) setOpen(true);
            }}
            onClick={() => {
              if (!readOnly) setOpen(true);
            }}
            onKeyDown={handleKeyDown}
            placeholder="(none)"
            readOnly={readOnly}
            disabled={readOnly}
            autoComplete="off"
            role="combobox"
            aria-expanded={open}
            spellCheck={false}
          />
          {!readOnly && hasValue && (
            <button
              type="button"
              className="type-selector__clear"
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => selectType(null)}
              title="Clear type"
              aria-label="Clear type"
            >
              ×
            </button>
          )}
          {!readOnly && (
            <button
              type="button"
              className="type-selector__chevron"
              tabIndex={-1}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => (open ? closeDropdown() : setOpen(true))}
              aria-label="Toggle type list"
            >
              ▾
            </button>
          )}
          {open && !readOnly && (
            <ul className="type-selector__dropdown" role="listbox">
              {shown.length === 0 ? (
                <li className="type-selector__dropdown-empty">
                  {typesQuery.isLoading ? "Loading…" : "No matching types"}
                </li>
              ) : (
                shown.map((t, i) => {
                  const ref = refOf(t);
                  return (
                    <li
                      key={ref}
                      role="option"
                      aria-selected={ref === value}
                      className={
                        "type-selector__dropdown-item" +
                        (i === highlight ? " is-highlighted" : "") +
                        (ref === value ? " is-selected" : "")
                      }
                      onMouseDown={(e) => e.preventDefault()}
                      onMouseEnter={() => setHighlight(i)}
                      onClick={() => selectType(t)}
                    >
                      {t.name}
                      <span className="type-selector__dropdown-item-path">{t.path}</span>
                    </li>
                  );
                })
              )}
            </ul>
          )}
        </div>
        <button
          type="button"
          className="dialog__btn type-selector__btn"
          onClick={handleEdit}
          disabled={readOnly || !selected}
          title={selected ? `Edit type "${selected.name}"` : "Select a type first"}
        >
          Edit
        </button>
        <button
          type="button"
          className="dialog__btn type-selector__btn"
          onClick={handleNew}
          disabled={readOnly}
        >
          + New
        </button>
      </div>

      <TypeEditor
        open={editor.open}
        onClose={() => setEditor({ open: false, existing: null, existingEntity: null })}
        existing={editor.existing}
        existingEntity={editor.existingEntity}
        defaultPath={defaultPath}
        onSaved={(saved) => {
          onChange(`${saved.path}/${saved.name}`);
          setEditor({ open: false, existing: null, existingEntity: null });
        }}
      />
    </div>
  );
}
