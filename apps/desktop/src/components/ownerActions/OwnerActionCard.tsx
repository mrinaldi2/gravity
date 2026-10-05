// A command a bot proposes for the owner to run (H-117 R2): where it runs,
// why, the content verbatim, its hash, any warnings, and once it ran, its
// state and (redacted) output. Run opens the confirm sheet; without the
// approve grant the card is read-only.

import { useState } from "react";
import type { ReactElement } from "react";
import type { OwnerAction, OwnerActionState } from "../../protocol/ownerActions";
import OwnerActionConfirm from "./OwnerActionConfirm";

const STATES: Readonly<Record<OwnerActionState, { glyph: string; word: string }>> = {
  proposed: { glyph: "○", word: "Waiting for you" },
  running: { glyph: "◐", word: "Running" },
  succeeded: { glyph: "✓", word: "Done" },
  failed: { glyph: "✕", word: "Failed" },
  timed_out: { glyph: "⏱", word: "Timed out" },
  rejected: { glyph: "⊘", word: "Rejected" },
  withdrawn: { glyph: "⊘", word: "Withdrawn" },
  expired: { glyph: "⊘", word: "Expired" },
};

export interface OwnerActionCardProps {
  readonly action: OwnerAction;
  /** Output streamed while it runs. */
  readonly output?: string;
  readonly proposer: string;
  /** The owner may run or reject it (approve, connected). */
  readonly canRun: boolean;
  readonly onRun: (action: OwnerAction) => void;
  readonly onReject: (action: OwnerAction, reason: string) => void;
}

function Output(props: {
  readonly action: OwnerAction;
  readonly live?: string;
}): ReactElement | null {
  const text =
    props.live && props.action.state === "running" ? props.live : props.action.output_tail;
  if (!text) {
    return null;
  }
  return (
    <details className="owner-action-output" open={props.action.state === "running"}>
      <summary>
        Output{props.action.exit_code === null ? "" : ` · exit ${props.action.exit_code}`}
      </summary>
      <pre>{text}</pre>
    </details>
  );
}

function Actions(props: Pick<OwnerActionCardProps, "action" | "onRun" | "onReject">): ReactElement {
  const [confirming, setConfirming] = useState(false);
  const [rejecting, setRejecting] = useState(false);
  const [reason, setReason] = useState("");
  if (confirming) {
    return (
      <OwnerActionConfirm
        action={props.action}
        onCancel={() => setConfirming(false)}
        onRun={() => {
          setConfirming(false);
          props.onRun(props.action);
        }}
      />
    );
  }
  if (rejecting) {
    return (
      <div className="owner-action-buttons">
        <input
          className="owner-action-reason"
          aria-label="Why not (optional)"
          placeholder="Why not (optional)"
          value={reason}
          onChange={(e) => setReason(e.target.value)}
        />
        <button type="button" className="btn" onClick={() => setRejecting(false)}>
          Cancel
        </button>
        <button type="button" className="btn" onClick={() => props.onReject(props.action, reason)}>
          Reject
        </button>
      </div>
    );
  }
  return (
    <div className="owner-action-buttons">
      <button type="button" className="btn" onClick={() => setRejecting(true)}>
        Reject…
      </button>
      <button type="button" className="btn btn-primary" onClick={() => setConfirming(true)}>
        Run…
      </button>
    </div>
  );
}

export default function OwnerActionCard(props: OwnerActionCardProps): ReactElement {
  const a = props.action;
  const state = STATES[a.state] ?? { glyph: "?", word: a.state };
  return (
    <article className="owner-action" aria-label={`Owner action: ${a.reason}`}>
      <header className="owner-action-head">
        <span className={`owner-action-state owner-action-${a.state}`}>
          <span aria-hidden="true">{state.glyph} </span>
          {state.word}
        </span>
        <span className="owner-action-target">on {a.target_name ?? "this computer"}</span>
      </header>
      <p className="owner-action-reason-text">{a.reason}</p>
      <p className="owner-action-meta">
        {a.shell} in <code>{a.cwd}</code>
      </p>
      <pre className="owner-action-content">{a.content}</pre>
      {a.flags.length > 0 ? (
        <ul className="owner-action-flags">
          {a.flags.map((flag) => (
            <li key={flag}>
              <span aria-hidden="true">⚠ </span>
              {flag}
            </li>
          ))}
        </ul>
      ) : null}
      <p className="owner-action-meta">
        Proposed by {props.proposer} · sha {a.sha256.slice(0, 12)}
        {a.reject_reason ? ` · ${a.reject_reason}` : ""}
      </p>
      <Output action={a} live={props.output} />
      {a.state === "proposed" && props.canRun ? (
        <Actions action={a} onRun={props.onRun} onReject={props.onReject} />
      ) : null}
    </article>
  );
}
