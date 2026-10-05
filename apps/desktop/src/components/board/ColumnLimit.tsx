import { Gauge } from "lucide-react";
import { useState } from "react";
import type { FormEvent, ReactElement } from "react";

interface ColumnLimitProps {
  readonly columnName: string;
  readonly limit: number | undefined;
  /** Saves the limit; `undefined` clears it. Rejects with the daemon's refusal. */
  readonly onSave: (limit: number | undefined) => Promise<void>;
}

/** The owner's WIP limit for one column, edited in its header (H-099). */
export default function ColumnLimit(props: ColumnLimitProps): ReactElement {
  const { columnName, limit, onSave } = props;
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const open = (): void => {
    setValue(limit === undefined ? "" : String(limit));
    setError(null);
    setEditing(true);
  };

  const save = async (next: number | undefined): Promise<void> => {
    setSaving(true);
    try {
      await onSave(next);
      setEditing(false);
    } catch (refusal) {
      setError(refusal instanceof Error ? refusal.message : String(refusal));
    } finally {
      setSaving(false);
    }
  };

  const submit = (event: FormEvent): void => {
    event.preventDefault();
    const trimmed = value.trim();
    const parsed = trimmed === "" ? 0 : Number(trimmed);
    if (!Number.isInteger(parsed) || parsed < 0) {
      setError("A whole number, or empty for no limit.");
      return;
    }
    void save(parsed === 0 ? undefined : parsed);
  };

  if (!editing) {
    return (
      <button
        type="button"
        className="board-column-collapse board-column-limit-button"
        aria-label={`WIP limit for ${columnName}`}
        title={`WIP limit for ${columnName}`}
        onClick={open}
      >
        <Gauge size={14} aria-hidden="true" />
      </button>
    );
  }
  return (
    // The field's own message, not the browser's bubble, says what's wrong.
    <form className="board-column-limit" onSubmit={submit} noValidate>
      <label>
        <span className="visually-hidden">WIP limit for {columnName}</span>
        <input
          type="number"
          min={0}
          inputMode="numeric"
          placeholder="No limit"
          value={value}
          // oxlint-disable-next-line jsx-a11y/no-autofocus
          autoFocus
          onChange={(event) => {
            setValue(event.target.value);
          }}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              setEditing(false);
            }
          }}
        />
      </label>
      <button type="submit" className="btn btn-small btn-primary" disabled={saving}>
        Save
      </button>
      <button
        type="button"
        className="btn btn-small"
        onClick={() => {
          setEditing(false);
        }}
      >
        Cancel
      </button>
      {error === null ? null : (
        <p className="board-column-limit-error" role="alert">
          {error}
        </p>
      )}
    </form>
  );
}
