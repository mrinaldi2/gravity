// The owner's choice for a held cleanup (H-275; H-261 §15.6): Remove anyway
// deletes the tree after its work is saved again; Keep it leaves it and stops
// the asking. Cancel holds the focus, so a stray Enter decides nothing.

import { useEffect } from "react";
import type { ReactElement } from "react";
import OverlayShell from "../overlay/OverlayShell";

interface CleanupDialogProps {
  readonly title: string;
  readonly busy: boolean;
  readonly onRemove: () => void;
  readonly onKeep: () => void;
  readonly onCancel: () => void;
}

export default function CleanupDialog(props: CleanupDialogProps): ReactElement {
  const { onCancel } = props;
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
    <OverlayShell label="Held cleanup" onClose={onCancel}>
      <div className="confirm-dialog">
        <h2 className="confirm-title">Held cleanup</h2>
        <p className="confirm-body">{props.title}</p>
        <p className="confirm-body">
          Remove anyway saves its changes again to the salvage folder, kept for 30 days, then
          deletes the worktree. If a file can't be saved, nothing is deleted.
        </p>
        <div className="confirm-actions">
          <button type="button" className="btn btn-small" autoFocus onClick={onCancel}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-small"
            disabled={props.busy}
            onClick={props.onKeep}
          >
            Keep it
          </button>
          <button
            type="button"
            className="btn btn-small btn-danger"
            disabled={props.busy}
            onClick={props.onRemove}
          >
            Remove anyway
          </button>
        </div>
      </div>
    </OverlayShell>
  );
}
