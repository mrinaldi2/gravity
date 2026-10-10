// The owner's choice for a held or failed cleanup (H-275; UX-055): Remove
// anyway deletes the tree after its changes are saved again; Keep it leaves
// it and stops the asking. Remove anyway is offered only when the changes
// could be saved. Cancel holds the focus, so a stray Enter decides nothing.

import { useEffect } from "react";
import type { ReactElement } from "react";
import type { AttentionRowJson } from "../../protocol/dashboard";
import OverlayShell from "../overlay/OverlayShell";
import { unsaved, whose } from "./cleanupText";

interface CleanupDialogProps {
  readonly row: AttentionRowJson;
  readonly busy: boolean;
  /** A refusal from the service, shown here. */
  readonly error: string | null;
  readonly onRemove: () => void;
  readonly onKeep: () => void;
  readonly onCancel: () => void;
}

interface Lines {
  readonly title: string;
  readonly body: readonly string[];
  readonly removable: boolean;
}

const REMOVE =
  "Remove anyway saves its changes again (kept 30 days), then deletes the worktree. If a file can't be saved, nothing is deleted.";
const KEEP = "Keep it leaves the worktree as it is. You won't be asked about it again.";

function lines(row: AttentionRowJson): Lines {
  const f = row.cleanup;
  if (f === undefined) {
    // An older service: its own words, and the choice it always offered.
    return { title: "Held cleanup", body: [row.title, REMOVE], removable: true };
  }
  if (f.salvaged !== true) {
    const why = f.reason || "nothing was saved";
    return {
      title: `Keep ${whose(f, false)}?`,
      body: [`Remove anyway needs its changes saved first, and they couldn't be: ${why}.`, KEEP],
      removable: false,
    };
  }
  const what = unsaved(f);
  return {
    title: `Remove ${whose(f, false)}?`,
    body: [`${what ? `${what}. ` : ""}Its changes are saved in the salvage folder.`, REMOVE, KEEP],
    removable: true,
  };
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
  const { title, body, removable } = lines(props.row);
  const stopped = props.busy || props.error !== null;
  return (
    <OverlayShell label={title} onClose={onCancel}>
      <div className="confirm-dialog">
        <h2 className="confirm-title">{title}</h2>
        {body.map((line) => (
          <p key={line} className="confirm-body">
            {line}
          </p>
        ))}
        {props.error === null ? null : (
          <p className="confirm-body" role="alert">
            {props.error}
          </p>
        )}
        <div className="confirm-actions">
          <button type="button" className="btn btn-small" autoFocus onClick={onCancel}>
            Cancel
          </button>
          <button type="button" className="btn btn-small" disabled={stopped} onClick={props.onKeep}>
            Keep it
          </button>
          {removable ? (
            <button
              type="button"
              className="btn btn-small btn-danger"
              disabled={stopped}
              onClick={props.onRemove}
            >
              Remove anyway
            </button>
          ) : null}
        </div>
      </div>
    </OverlayShell>
  );
}
