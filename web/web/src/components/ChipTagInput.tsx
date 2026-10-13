/**
 * ChipTagInput — free-text + datalist chip input.
 *
 * - Enter / comma → add chip
 * - Backspace on empty input → pop last chip
 * - × on each chip → remove
 * - Optional datalist for autocomplete suggestions
 * - readOnly disables the input and hides the remove buttons
 */

import { useState } from "react";

interface Props {
  chips: string[];
  onChange: (chips: string[]) => void;
  placeholder?: string;
  datalistId?: string;
  datalistOptions?: string[];
  readOnly?: boolean;
}

export function ChipTagInput({
  chips,
  onChange,
  placeholder,
  datalistId,
  datalistOptions,
  readOnly,
}: Props) {
  const [inputVal, setInputVal] = useState("");

  const addChip = (val: string) => {
    const trimmed = val.trim();
    if (!trimmed || chips.includes(trimmed)) return;
    onChange([...chips, trimmed]);
  };

  const removeChip = (chip: string) => onChange(chips.filter((c) => c !== chip));

  return (
    <div className="chip-tag-input">
      {chips.map((chip) => (
        <span key={chip} className="chip-tag-input__chip">
          {chip}
          {!readOnly && (
            <button
              type="button"
              onClick={() => removeChip(chip)}
              title={`Remove "${chip}"`}
              aria-label={`Remove ${chip}`}
            >
              ×
            </button>
          )}
        </span>
      ))}
      {!readOnly && (
        <input
          className="chip-tag-input__input"
          value={inputVal}
          onChange={(e) => setInputVal(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === ",") {
              e.preventDefault();
              addChip(inputVal);
              setInputVal("");
            } else if (e.key === "Backspace" && !inputVal && chips.length) {
              onChange(chips.slice(0, -1));
            }
          }}
          onBlur={() => {
            if (inputVal.trim()) {
              addChip(inputVal);
              setInputVal("");
            }
          }}
          placeholder={chips.length === 0 ? placeholder : ""}
          list={datalistId}
          disabled={readOnly}
        />
      )}
      {datalistId && datalistOptions && (
        <datalist id={datalistId}>
          {datalistOptions.map((o) => (
            <option key={o} value={o} />
          ))}
        </datalist>
      )}
    </div>
  );
}
