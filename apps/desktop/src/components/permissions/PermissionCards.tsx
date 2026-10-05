import { useState } from "react";
import type { KeyboardEvent, ReactElement } from "react";
import type { PermissionAnswer, PermissionRequest } from "../../protocol/chat";
import { isTerminal, permissionTitle } from "./permissionTitle";
import { errText, fmtTimestamp } from "../../util";
import CodeBlock from "../chat/CodeBlock";
import type { Permissions } from "./usePermissions";

interface PermissionCardsProps {
  readonly permissions: Permissions;
  /** False on a read-only connection: the cards show, the buttons do not act. */
  readonly canAnswer: boolean;
  /** Set where cards from many bots share a list: each card names its bot. */
  readonly botName?: (botId: string) => string | undefined;
  /** Set with `botName`: opens the bot a card is from. */
  readonly onOpenBot?: (botId: string) => void;
}

/** Tools waiting on the owner, one card each, oldest first. */
export default function PermissionCards({
  permissions,
  canAnswer,
  botName,
  onOpenBot,
}: PermissionCardsProps): ReactElement | null {
  if (permissions.pending.length === 0) {
    return null;
  }
  return (
    <div className="permission-cards" aria-label="Permission requests">
      {permissions.pending.map((request) => (
        <PermissionCard
          key={request.id}
          request={request}
          canAnswer={canAnswer}
          onAnswer={permissions.answer}
          botName={botName?.(request.bot_id) ?? (botName === undefined ? undefined : "A bot")}
          onOpenBot={onOpenBot}
        />
      ))}
    </div>
  );
}

interface PermissionCardProps {
  readonly request: PermissionRequest;
  readonly canAnswer: boolean;
  readonly onAnswer: Permissions["answer"];
  readonly botName?: string;
  readonly onOpenBot?: (botId: string) => void;
}

/** Who wants it, with a way to open that bot, and when the prompt runs out. */
function PermissionHead({
  request,
  botName,
  onOpenBot,
}: {
  readonly request: PermissionRequest;
  readonly botName?: string;
  readonly onOpenBot?: (botId: string) => void;
}): ReactElement {
  return (
    <div className="permission-head">
      <span className="permission-label">{permissionTitle(request, botName)}</span>
      {botName === undefined || onOpenBot === undefined || isTerminal(request) ? null : (
        <button
          type="button"
          className="permission-toggle permission-open"
          onClick={() => {
            onOpenBot(request.bot_id);
          }}
        >
          Open {botName}
        </button>
      )}
      <span className="permission-expiry">
        Denied at {fmtTimestamp(request.expires_at)} if unanswered
      </span>
    </div>
  );
}

const KEYS: Readonly<Record<string, PermissionAnswer>> = {
  a: "allow_once",
  s: "allow_session",
  d: "deny",
};

function PermissionCard({
  request,
  canAnswer,
  onAnswer,
  botName,
  onOpenBot,
}: PermissionCardProps): ReactElement {
  const [details, setDetails] = useState(false);
  const [denying, setDenying] = useState(false);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const answer = async (decision: PermissionAnswer): Promise<void> => {
    if (!canAnswer || busy) {
      return;
    }
    setBusy(true);
    try {
      const why = decision === "deny" && reason.trim() !== "" ? reason.trim() : undefined;
      await onAnswer(request.id, decision, why);
    } catch (failure) {
      setError(errText(failure));
      setBusy(false);
    }
  };

  // With an answer button focused, a / s / d pick the answer.
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>): void => {
    if (event.metaKey || event.ctrlKey || event.altKey) {
      return;
    }
    const decision = KEYS[event.key];
    if (decision === "deny") {
      event.preventDefault();
      setDenying(true);
    } else if (decision !== undefined) {
      event.preventDefault();
      void answer(decision);
    }
  };

  return (
    <div className="permission-card" role="group" aria-label={request.summary}>
      <PermissionHead request={request} botName={botName} onOpenBot={onOpenBot} />
      <div className="permission-summary">{request.summary}</div>
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
      {denying ? (
        <input
          className="permission-reason"
          aria-label="Reason (optional)"
          placeholder="Why not? The bot sees this (optional)"
          value={reason}
          autoFocus
          onChange={(event) => {
            setReason(event.target.value);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              void answer("deny");
            }
          }}
        />
      ) : null}
      {error === null ? null : <div className="chat-note chat-error">{error}</div>}
      <div
        className="permission-actions"
        role="toolbar"
        aria-label="Answer"
        tabIndex={-1}
        onKeyDown={onKeyDown}
      >
        <AnswerButtons
          terminal={isTerminal(request)}
          disabled={!canAnswer || busy}
          denying={denying}
          onAnswer={(decision) => void answer(decision)}
          onDeny={() => {
            setDenying(true);
          }}
        />
        {canAnswer ? null : <span className="permission-readonly">Read-only connection</span>}
      </div>
    </div>
  );
}

/** Allow once, Allow for session (bots only: a terminal command asks each time), Deny. */
function AnswerButtons({
  terminal,
  disabled,
  denying,
  onAnswer,
  onDeny,
}: {
  readonly terminal: boolean;
  readonly disabled: boolean;
  readonly denying: boolean;
  readonly onAnswer: (decision: PermissionAnswer) => void;
  readonly onDeny: () => void;
}): ReactElement {
  return (
    <>
      <button
        type="button"
        className="btn btn-good"
        disabled={disabled}
        onClick={() => onAnswer("allow_once")}
      >
        Allow once <kbd>A</kbd>
      </button>
      {terminal ? null : (
        <button
          type="button"
          className="btn"
          disabled={disabled}
          title="For the rest of this bot's session"
          onClick={() => onAnswer("allow_session")}
        >
          Allow for session <kbd>S</kbd>
        </button>
      )}
      <button
        type="button"
        className="btn btn-danger"
        disabled={disabled}
        onClick={() => (denying ? onAnswer("deny") : onDeny())}
      >
        {denying ? "Deny" : "Deny…"} <kbd>D</kbd>
      </button>
    </>
  );
}
