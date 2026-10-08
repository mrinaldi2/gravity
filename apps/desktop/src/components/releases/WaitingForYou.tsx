// "Waiting for you" at the top of a release (H-247, UX-048 §2): what only
// the owner can clear, each with its button right there. Three rows, then
// "+n more". Nothing shows when nothing waits (§4).

import { useState } from "react";
import type { ReactElement } from "react";
import type { AddToast } from "../../app/useToasts";
import type { DaemonApi } from "../../protocol/api";
import type { Bot } from "../../protocol/entities";
import type { OwnerBlocker, Release } from "../../protocol/releases";
import CardLink from "../cards/CardLink";
import { useCardLinks } from "../cards/CardLinks";
import OwnerActionList from "../ownerActions/OwnerActionList";
import type { BotName } from "./labels";
import {
  blockerAction,
  blockerBy,
  blockerWords,
  isUnderWay,
  nowLine,
  shortAge,
} from "./waitingText";

/** Rows shown before "+n more". */
const SHOWN = 3;

/** Where the row buttons go; none, the row has no button. */
export interface WaitingActions {
  readonly client: DaemonApi;
  readonly connected: boolean;
  readonly bots: readonly Bot[];
  readonly addToast: AddToast;
  /** Opens a decision where the owner answers it. */
  readonly onDecision: (decisionId: string) => void;
  /** Opens Needs you, where permission prompts are answered. */
  readonly onNeedsYou: () => void;
}

interface RowProps {
  readonly blocker: OwnerBlocker;
  readonly release: Release;
  readonly botName: BotName;
  readonly now: number;
  readonly actions?: WaitingActions;
  readonly onReview: () => void;
}

function Row({ blocker, release, botName, now, actions, onReview }: RowProps): ReactElement {
  const links = useCardLinks();
  const [running, setRunning] = useState(false);
  const { glyph, what } = blockerWords(blocker, botName);
  const by = blockerBy(blocker, botName);
  const item = blocker.item_id;
  const press = (): void => {
    switch (blocker.kind) {
      case "ruling":
        onReview();
        break;
      case "run":
        setRunning((open) => !open);
        break;
      case "decision":
        actions?.onDecision(blocker.id);
        break;
      case "question":
        if (item) {
          links?.open(item, release.project_id);
        }
        break;
      default:
        actions?.onNeedsYou();
    }
  };
  return (
    <li className="waiting-row">
      <span className="waiting-glyph" aria-hidden="true">
        {glyph}
      </span>
      <span className="waiting-text">
        <span className="waiting-what">{what}</span>
        {blocker.kind === "ruling" ? null : (
          <span className="waiting-meta">
            {by ? `${by} · ` : null}
            {item ? (
              <>
                on <CardLink id={item} /> ·{" "}
              </>
            ) : null}
            {shortAge(blocker.created_at, now)}
          </span>
        )}
      </span>
      {actions || blocker.kind === "ruling" ? (
        <button type="button" className="btn btn-small" onClick={press}>
          {blockerAction(blocker)}
        </button>
      ) : null}
      {running && actions && item ? (
        <div className="waiting-run">
          <OwnerActionList
            client={actions.client}
            connected={actions.connected}
            scope={{ projectId: release.project_id, itemId: item }}
            addToast={actions.addToast}
            botName={(id) => actions.bots.find((b) => b.id === id)?.name ?? "a bot"}
          />
        </div>
      ) : null}
    </li>
  );
}

/**
 * The first line of Progress (UX-048 §3): what the release waits on now, and
 * from whom. An older service says nothing of the owner's part, so the line
 * points to Needs you instead (§4).
 */
export function NowLine(props: {
  readonly release: Release;
  readonly botName: BotName;
  readonly onNeedsYou?: () => void;
}): ReactElement | null {
  const { release } = props;
  const line = nowLine(release, props.botName);
  if (line) {
    return <p className="release-now">{line}</p>;
  }
  if (release.owner_blockers !== undefined || !isUnderWay(release)) {
    return null;
  }
  return (
    <p className="release-now">
      Open{" "}
      {props.onNeedsYou ? (
        <button type="button" className="cc-link" onClick={props.onNeedsYou}>
          Needs you
        </button>
      ) : (
        "Needs you"
      )}{" "}
      to see what waits for you.
    </p>
  );
}

export default function WaitingForYou(props: {
  readonly release: Release;
  readonly botName: BotName;
  readonly actions?: WaitingActions;
  /** Takes the owner to the release's own Approve / Hold / Reject. */
  readonly onReview: () => void;
  readonly now?: () => number;
}): ReactElement | null {
  const [all, setAll] = useState(false);
  const blockers = props.release.owner_blockers ?? [];
  if (blockers.length === 0) {
    return null;
  }
  const now = (props.now ?? Date.now)();
  const shown = all ? blockers : blockers.slice(0, SHOWN);
  const more = blockers.length - shown.length;
  return (
    <section className="waiting" aria-label="Waiting for you">
      <h3 className="waiting-title">
        <span aria-hidden="true">▲</span> Waiting for you · {blockers.length}
      </h3>
      <ul className="waiting-rows">
        {shown.map((b) => (
          <Row
            key={`${b.kind}:${b.id}`}
            blocker={b}
            release={props.release}
            botName={props.botName}
            now={now}
            actions={props.actions}
            onReview={props.onReview}
          />
        ))}
      </ul>
      {more > 0 ? (
        <button type="button" className="waiting-more" onClick={() => setAll(true)}>
          +{more} more
        </button>
      ) : null}
    </section>
  );
}
