// What "Review N rulings…" opens (UX-016 §1): every ruling a bot recorded
// for the owner, with the answer it recorded, before one confirm makes them
// the owner's own. It confirms exactly the rulings it lists; when the daemon
// says some changed meanwhile, the list is reread and the dialog stays.

import { useEffect, useState } from "react";
import type { ReactElement } from "react";
import type { RelayedRuling } from "../../protocol/dashboard";
import OverlayShell from "../overlay/OverlayShell";
import { plural } from "../releases/labels";
import { names, when } from "./needsYouText";
import type { ConfirmOutcome } from "./useConfirmRelayed";

interface RelayedDialogProps {
  readonly rulings: readonly RelayedRuling[];
  readonly botName: (id: string) => string;
  readonly confirming: boolean;
  readonly onConfirm: (decisionIds: readonly string[]) => Promise<ConfirmOutcome>;
  readonly onOpen: (decisionId: string) => void;
  readonly onClose: () => void;
}

function firstLine(text: string): string {
  return text.split("\n", 1)[0] ?? "";
}

export default function RelayedDialog(props: RelayedDialogProps): ReactElement {
  const { rulings, botName, onClose } = props;
  const [changed, setChanged] = useState(false);
  // Read before Cancel takes focus: the button that opened the dialog,
  // which gets focus back when it closes.
  const [opener] = useState(() =>
    document.activeElement instanceof HTMLElement ? document.activeElement : null,
  );
  useEffect(
    () => () => {
      if (opener?.isConnected) {
        opener.focus();
      }
    },
    [opener],
  );
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const count = plural(rulings.length, "ruling");
  const by = names([...new Set(rulings.map((r) => botName(r.bot_id)))]);
  const title = `Confirm ${count} recorded for you?`;
  const confirm = async (): Promise<void> => {
    const outcome = await props.onConfirm(rulings.map((r) => r.id));
    setChanged(outcome === "changed");
    if (outcome === "done") {
      onClose();
    }
  };
  return (
    <OverlayShell label={title} onClose={onClose}>
      <div className="confirm-dialog relayed-dialog">
        <h2 className="confirm-title">{title}</h2>
        <p className="confirm-body">
          {by} recorded these answers on your behalf. Confirming makes each one your own ruling;
          bots act on it as yours.
        </p>
        <ul className="relayed-list">
          {rulings.map((r) => (
            <li key={r.id} className="relayed-ruling">
              <div className="relayed-text">
                <strong>{r.title}</strong>
                <span className="relayed-meta">
                  {[
                    `Answer: ${firstLine(r.answer)}`,
                    `by ${botName(r.bot_id)}`,
                    ...(r.at ? [when(r.at)] : []),
                  ].join(" · ")}
                </span>
              </div>
              <button
                type="button"
                className="relayed-open"
                aria-label={`Open ${r.title}`}
                onClick={() => {
                  onClose();
                  props.onOpen(r.id);
                }}
              >
                Open
              </button>
            </li>
          ))}
        </ul>
        {changed ? (
          <p className="relayed-changed" role="status">
            The list changed while it was open. Check it again before confirming.
          </p>
        ) : null}
        <p className="relayed-note">
          To change an answer, open it instead. Rulings you open aren't confirmed here.
        </p>
        <div className="confirm-actions">
          <button type="button" className="btn btn-small" autoFocus onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-small btn-primary"
            disabled={props.confirming || rulings.length === 0}
            onClick={() => void confirm()}
          >
            Confirm {count}
          </button>
        </div>
      </div>
    </OverlayShell>
  );
}
