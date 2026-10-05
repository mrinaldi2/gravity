import { Lock } from "lucide-react";
import { useId, useState } from "react";
import type { ReactElement } from "react";
import type { PermissionAnswer, PermissionRequest } from "../../protocol/chat";
import { errText, fmtTimestamp } from "../../util";
import CodeBlock from "../chat/CodeBlock";
import { TERMINAL_BODY, TERMINAL_TITLE, botWarning, originLine } from "./permissionTitle";
import type { Permissions } from "./usePermissions";

interface TerminalCardProps {
  readonly request: PermissionRequest;
  readonly canAnswer: boolean;
  readonly onAnswer: Permissions["answer"];
}

/**
 * A command run in a terminal asking to act as the owner (H-044 T4, UX-014).
 * Unlike a bot's card it has its own look, no single-key answers and no
 * initial focus, so nothing approves it by a stray keypress; Deny takes no
 * reason.
 */
export default function TerminalCard({
  request,
  canAnswer,
  onAnswer,
}: TerminalCardProps): ReactElement {
  const titleId = useId();
  const [details, setDetails] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const origin = request.origin;
  const command = origin?.command ?? "";

  const answer = async (decision: PermissionAnswer): Promise<void> => {
    if (!canAnswer || busy) {
      return;
    }
    setBusy(true);
    try {
      await onAnswer(request.id, decision);
    } catch (failure) {
      setError(errText(failure));
      setBusy(false);
    }
  };
  const label = (verb: string): string => (command === "" ? verb : `${verb}: ${command}`);

  return (
    <section className="permission-card permission-terminal" aria-labelledby={titleId}>
      <h3 id={titleId} className="permission-terminal-title">
        <Lock size={14} aria-hidden="true" />
        {TERMINAL_TITLE}
      </h3>
      {origin === undefined ? null : (
        <>
          <div className="permission-summary">{command}</div>
          <div className="permission-origin">{originLine(origin)}</div>
          {origin.bot === undefined ? null : (
            <div className="permission-bot-warning">{botWarning(origin.bot)}</div>
          )}
        </>
      )}
      <p className="permission-terminal-body">{TERMINAL_BODY}</p>
      <span className="permission-expiry">
        Denied automatically at {fmtTimestamp(request.expires_at)} if you don't answer.
      </span>
      <button
        type="button"
        className="permission-toggle"
        aria-expanded={details}
        onClick={() => {
          setDetails((open) => !open);
        }}
      >
        {details ? "Hide details" : "Show details"}
      </button>
      {details ? <CodeBlock code={request.input} language="json" label={request.tool} /> : null}
      {error === null ? null : <div className="chat-note chat-error">{error}</div>}
      <div className="permission-actions">
        <button
          type="button"
          className="btn btn-primary"
          aria-label={label("Allow this command")}
          disabled={!canAnswer || busy}
          onClick={() => void answer("allow_once")}
        >
          Allow this command
        </button>
        <button
          type="button"
          className="btn btn-danger"
          aria-label={label("Deny this command")}
          disabled={!canAnswer || busy}
          onClick={() => void answer("deny")}
        >
          Deny
        </button>
        {canAnswer ? null : <span className="permission-readonly">Read-only connection</span>}
      </div>
    </section>
  );
}
