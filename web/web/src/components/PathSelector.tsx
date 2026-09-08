/**
 * PathSelector — a free-text path-scope input with an assistive dropdown of
 * real, distinct paths (each with a count), sourced from the `paths` prop.
 *
 * Unlike `TypeSelector` (which refuses to commit anything but a real type
 * ref), typed text here is never blocked or reverted: `pathPrefix` is, and
 * must remain, a free-text prefix filter — e.g. typing "/packages" to scope
 * every sibling package even though "/packages" itself may never appear as
 * an exact distinct path. Picking a suggestion just fills in that exact
 * path; the dropdown is purely assistive.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { PathFacet } from "../api/api";

interface Props {
  value: string;
  onChange: (path: string) => void;
  paths: PathFacet[];
  loading?: boolean;
  placeholder?: string;
}

export function PathSelector({ value, onChange, paths, loading, placeholder }: Props) {
  const [open, setOpen] = useState(false);
  const [highlight, setHighlight] = useState(-1);
  const comboRef = useRef<HTMLDivElement>(null);

  const sorted = useMemo(
    () => paths.slice().sort((a, b) => a.path.localeCompare(b.path)),
    [paths],
  );

  const shown = useMemo(() => {
    const filter = value.trim().toLowerCase();
    if (!filter) return sorted;
    return sorted.filter((p) => p.path.toLowerCase().includes(filter));
  }, [sorted, value]);

  const closeDropdown = useCallback(() => {
    setOpen(false);
    setHighlight(-1);
  }, []);

  const selectPath = useCallback(
    (p: PathFacet | null) => {
      onChange(p ? p.path : "");
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
        selectPath(shown[highlight]);
      }
    } else if (e.key === "Escape") {
      if (open) {
        e.preventDefault();
        closeDropdown();
      }
    }
  };

  const hasValue = value.trim().length > 0;

  return (
    <div className="path-selector__combo" ref={comboRef}>
      <input
        className="path-selector__input"
        value={value}
        onChange={(e) => {
          onChange(e.target.value);
          setOpen(true);
          setHighlight(-1);
        }}
        onFocus={() => setOpen(true)}
        onClick={() => setOpen(true)}
        onKeyDown={handleKeyDown}
        placeholder={placeholder ?? "/ (everywhere)"}
        autoComplete="off"
        role="combobox"
        aria-expanded={open}
        spellCheck={false}
      />
      {hasValue && (
        <button
          type="button"
          className="path-selector__clear"
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => selectPath(null)}
          title="Clear path scope"
          aria-label="Clear path scope"
        >
          ×
        </button>
      )}
      <button
        type="button"
        className="path-selector__chevron"
        tabIndex={-1}
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => (open ? closeDropdown() : setOpen(true))}
        aria-label="Toggle path list"
      >
        ▾
      </button>
      {open && (
        <ul className="path-selector__dropdown" role="listbox">
          {shown.length === 0 ? (
            <li className="path-selector__dropdown-empty">
              {loading ? "Loading…" : "No matching paths"}
            </li>
          ) : (
            shown.map((p, i) => (
              <li
                key={p.path}
                role="option"
                aria-selected={p.path === value}
                className={
                  "path-selector__dropdown-item" +
                  (i === highlight ? " is-highlighted" : "") +
                  (p.path === value ? " is-selected" : "")
                }
                onMouseDown={(e) => e.preventDefault()}
                onMouseEnter={() => setHighlight(i)}
                onClick={() => selectPath(p)}
              >
                {p.path}
                <span className="path-selector__dropdown-item-count">{p.count}</span>
              </li>
            ))
          )}
        </ul>
      )}
    </div>
  );
}
