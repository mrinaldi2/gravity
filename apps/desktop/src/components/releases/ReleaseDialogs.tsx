// The release review's dialogs (H-018 §4A.3). Focus starts on Cancel so a
// stray Enter never rules; Escape cancels.

import { useEffect, useState } from "react";
import type { ReactElement, ReactNode } from "react";
import OverlayShell from "../overlay/OverlayShell";

function Dialog({
  title,
  onCancel,
  children,
  actions,
}: {
  readonly title: string;
  readonly onCancel: () => void;
  readonly children: ReactNode;
  readonly actions: ReactNode;
}): ReactElement {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        onCancel();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onCancel]);
  return (
    <OverlayShell label={title} onClose={onCancel}>
      <div className="confirm-dialog release-dialog">
        <h2 className="confirm-title">{title}</h2>
        {children}
        <div className="confirm-actions">
          <button type="button" className="btn btn-small" autoFocus onClick={onCancel}>
            Cancel
          </button>
          {actions}
        </div>
      </div>
    </OverlayShell>
  );
}

export function ApproveDialog({
  title,
  body,
  warning,
  confirmLabel,
  onConfirm,
  onCancel,
}: {
  readonly title: string;
  readonly body: string;
  /** What should give the owner pause: failing tests, items left out. */
  readonly warning?: string;
  readonly confirmLabel: string;
  readonly onConfirm: () => void;
  readonly onCancel: () => void;
}): ReactElement {
  return (
    <Dialog
      title={title}
      onCancel={onCancel}
      actions={
        <button type="button" className="btn btn-small btn-primary" onClick={onConfirm}>
          {confirmLabel}
        </button>
      }
    >
      {warning ? (
        <p className="release-warning">
          <span aria-hidden="true">⚠ </span>
          {warning}
        </p>
      ) : null}
      <p className="confirm-body">{body}</p>
    </Dialog>
  );
}

/** When a hold reminds the owner: none, or a few fixed offsets. */
const REMIND: readonly { readonly label: string; readonly days: number }[] = [
  { label: "Don't remind me", days: 0 },
  { label: "Tomorrow", days: 1 },
  { label: "In 3 days", days: 3 },
  { label: "Next week", days: 7 },
];

export function HoldDialog({
  title,
  now,
  onConfirm,
  onCancel,
}: {
  readonly title: string;
  /** For tests and stories; the reminder is counted from it. */
  readonly now: () => number;
  readonly onConfirm: (note: string, remindAt: string | null) => void;
  readonly onCancel: () => void;
}): ReactElement {
  const [note, setNote] = useState("");
  const [days, setDays] = useState(0);
  const remindAt = days > 0 ? new Date(now() + days * 86_400_000).toISOString() : null;
  return (
    <Dialog
      title={title}
      onCancel={onCancel}
      actions={
        <button type="button" className="btn btn-small" onClick={() => onConfirm(note, remindAt)}>
          Hold
        </button>
      }
    >
      <p className="confirm-body">
        Nothing is deployed. The package waits as it is, and comes back to you when the reminder is
        due.
      </p>
      <label className="release-field">
        Reason (optional)
        <textarea value={note} rows={2} onChange={(e) => setNote(e.target.value)} />
      </label>
      <label className="release-field">
        Remind me
        <select value={days} onChange={(e) => setDays(Number(e.target.value))}>
          {REMIND.map((r) => (
            <option key={r.days} value={r.days}>
              {r.label}
            </option>
          ))}
        </select>
      </label>
    </Dialog>
  );
}

export type ReturnTo = "rework" | "hold";

export function RejectDialog({
  title,
  items,
  onConfirm,
  onCancel,
}: {
  readonly title: string;
  readonly items: readonly { readonly id: string; readonly title: string }[];
  readonly onConfirm: (reason: string, returnTo: ReadonlyMap<string, ReturnTo>) => void;
  readonly onCancel: () => void;
}): ReactElement {
  const [reason, setReason] = useState("");
  const [returns, setReturns] = useState<ReadonlyMap<string, ReturnTo>>(
    () => new Map(items.map((i) => [i.id, "rework"])),
  );
  const allReady = [...returns.values()].every((r) => r === "hold");
  return (
    <Dialog
      title={title}
      onCancel={onCancel}
      actions={
        <button
          type="button"
          className="btn btn-small btn-danger"
          disabled={!reason.trim()}
          onClick={() => onConfirm(reason.trim(), returns)}
        >
          Reject
        </button>
      }
    >
      <label className="release-field">
        Reason (sent to DevOps and copied to every item)
        <textarea value={reason} rows={2} required onChange={(e) => setReason(e.target.value)} />
      </label>
      <ul className="release-returns" aria-label="Where each item goes">
        {items.map((item) => (
          <li key={item.id}>
            <span className="mono">{item.id}</span> <span>{item.title}</span>
            <select
              aria-label={`Where ${item.id} goes`}
              value={returns.get(item.id)}
              onChange={(e) =>
                setReturns(new Map(returns).set(item.id, e.target.value as ReturnTo))
              }
            >
              <option value="rework">Back to Doing</option>
              <option value="hold">Back to Ready</option>
            </select>
          </li>
        ))}
      </ul>
      {allReady ? (
        <p className="release-hint">
          With every item back to Ready, the package is held rather than rejected.
        </p>
      ) : null}
    </Dialog>
  );
}

export function LeaveOutDialog({
  item,
  onConfirm,
  onCancel,
}: {
  readonly item: { readonly id: string; readonly title: string };
  readonly onConfirm: (verdict: ReturnTo, note: string) => void;
  readonly onCancel: () => void;
}): ReactElement {
  const [verdict, setVerdict] = useState<ReturnTo>("hold");
  const [note, setNote] = useState("");
  const ready = verdict === "hold" || note.trim().length > 0;
  return (
    <Dialog
      title={`Leave out ${item.id}?`}
      onCancel={onCancel}
      actions={
        <button
          type="button"
          className="btn btn-small"
          disabled={!ready}
          onClick={() => onConfirm(verdict, note.trim())}
        >
          Leave out
        </button>
      }
    >
      <p className="confirm-body">{item.title}</p>
      <fieldset className="release-choice">
        <legend>It goes</legend>
        <label>
          <input type="radio" checked={verdict === "hold"} onChange={() => setVerdict("hold")} />
          Hold: it waits for the next release
        </label>
        <label>
          <input
            type="radio"
            checked={verdict === "rework"}
            onChange={() => setVerdict("rework")}
          />
          Rework: back to Doing, with a note
        </label>
      </fieldset>
      <label className="release-field">
        {verdict === "rework" ? "What needs rework" : "Note (optional)"}
        <textarea value={note} rows={2} onChange={(e) => setNote(e.target.value)} />
      </label>
    </Dialog>
  );
}

export function PauseDialog({
  title,
  onConfirm,
  onCancel,
}: {
  readonly title: string;
  readonly onConfirm: (reason: string) => void;
  readonly onCancel: () => void;
}): ReactElement {
  const [reason, setReason] = useState("");
  return (
    <Dialog
      title={title}
      onCancel={onCancel}
      actions={
        <button
          type="button"
          className="btn btn-small"
          disabled={!reason.trim()}
          onClick={() => onConfirm(reason.trim())}
        >
          Pause rollout
        </button>
      }
    >
      <p className="confirm-body">
        Computers already updated stay as they are. Testers holding a deploy task are told not to
        install.
      </p>
      <label className="release-field">
        Reason (the testers read it)
        <textarea value={reason} rows={2} onChange={(e) => setReason(e.target.value)} />
      </label>
    </Dialog>
  );
}
