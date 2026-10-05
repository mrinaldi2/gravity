// A command a bot proposes for the owner to run (H-117 R2, UX-022): who asks
// for what and where, why, the content verbatim, its fingerprint, any
// warnings, and once it ran, one line for how it went with the (redacted)
// output. Run opens the confirm sheet; without the approve grant the card
// says where it can be run.

import { useState } from "react";
import type { ReactElement } from "react";
import type { OwnerAction } from "../../protocol/ownerActions";
import OwnerActionConfirm from "./OwnerActionConfirm";
import { fingerprint, resultLine, titleLine } from "./ownerActionText";

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
  const { state } = props.action;
  const text = props.live && state === "running" ? props.live : props.action.output_tail;
  if (!text) {
    return null;
  }
  // Open while it runs, and when it went wrong.
  const open = state === "running" || state === "failed" || state === "timed_out";
  return (
    <details className="owner-action-output" open={open}>
      <summary>Output</summary>
      <pre>{text}</pre>
    </details>
  );
}

function Actions(
  props: Pick<OwnerActionCardProps, "action" | "proposer" | "onRun" | "onReject">,
): ReactElement {
  const [confirming, setConfirming] = useState(false);
  const [rejecting, setRejecting] = useState(false);
  const [reason, setReason] = useState("");
  if (confirming) {
    return (
      <OwnerActionConfirm
        action={props.action}
        proposer={props.proposer}
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
  const title = titleLine(a, props.proposer);
  const result = resultLine(a, props.proposer);
  return (
    <article className="owner-action" aria-label={`${title}: ${a.reason}`}>
      <p className="owner-action-title">
        <span aria-hidden="true">▶ </span>
        {title}
      </p>
      <p className="owner-action-reason-text">Why: {a.reason}</p>
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
      <p className="owner-action-meta">Fingerprint {fingerprint(a)}</p>
      {result === null ? null : (
        <p className={`owner-action-result owner-action-${a.state}`} role="status">
          {result}
        </p>
      )}
      <Output action={a} live={props.output} />
      {a.state === "proposed" && props.canRun ? (
        <Actions
          action={a}
          proposer={props.proposer}
          onRun={props.onRun}
          onReject={props.onReject}
        />
      ) : null}
      {a.state === "proposed" && !props.canRun ? (
        <p className="owner-action-hint">You can run this from a device with approve access.</p>
      ) : null}
    </article>
  );
}
