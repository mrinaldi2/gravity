// A card id as a link (UX-035 §2–§4): the id as written, a preview on a
// 400 ms hover or on focus, and a click that opens the card in the app's
// drawer. A card that can't be opened (deleted, or on a computer that's
// offline) keeps its note open instead. Esc closes the preview first.

import { useEffect, useId, useRef, useState, useSyncExternalStore } from "react";
import type { MouseEvent, ReactElement } from "react";
import type { Preview } from "./cardText";
import { linkName, noAnswerNote, oldServiceNote, preview } from "./cardText";
import type { CardLinksValue } from "./CardLinks";
import { useCardLinks } from "./CardLinks";

/** How long a hover waits before the preview, and the pointer may leave. */
export const HOVER_MS = 400;
export const LEAVE_MS = 150;
/** How long "Loading…" shows before it says the service didn't answer. */
export const SLOW_MS = 1000;

function PreviewBody({ p }: { readonly p: Preview }): ReactElement {
  if (p.kind === "loading") {
    return <span className="card-preview-dim">{p.text}</span>;
  }
  if (p.kind === "note") {
    return (
      <>
        <span>{p.text}</span>
        {p.lastSeen ? <span className="card-preview-dim">Last seen as: {p.lastSeen}</span> : null}
      </>
    );
  }
  return (
    <>
      <span className="card-preview-head">{p.head}</span>
      <span className="card-preview-title">{p.title}</span>
      <span className="card-preview-dim">{p.where}</span>
      {p.extra ? <span>{p.extra}</span> : null}
      {p.project ? <span className="card-preview-dim">{p.project}</span> : null}
    </>
  );
}

type Timer = { current: ReturnType<typeof setTimeout> | null };

/** Whether the preview shows, and the timers that open and close it. */
function usePreviewState(loaded: boolean) {
  const [shown, setShown] = useState(false);
  const [slow, setSlow] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => () => clear(timer), []);
  // "Loading…" only for so long.
  useEffect(() => {
    if (!shown || loaded) {
      return undefined;
    }
    const wait = setTimeout(() => setSlow(true), SLOW_MS);
    return () => clearTimeout(wait);
  }, [shown, loaded]);
  // Esc closes the preview before anything else (the drawer, a dialog).
  useEffect(() => {
    if (!shown) {
      return undefined;
    }
    const onKey = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        setShown(false);
      }
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () => window.removeEventListener("keydown", onKey, { capture: true });
  }, [shown]);

  const later = (ms: number, then: () => void): void => {
    clear(timer);
    timer.current = setTimeout(then, ms);
  };
  const open = (): void => {
    clear(timer);
    setSlow(false);
    setShown(true);
  };
  return {
    shown,
    slow,
    open,
    openLater: () => later(HOVER_MS, open),
    closeLater: () => later(LEAVE_MS, () => setShown(false)),
    keep: () => clear(timer),
    hide: () => setShown(false),
  };
}

/** What the preview says, from the service's answer so far. */
function previewOf(id: string, links: CardLinksValue, slow: boolean): Preview {
  const { cache } = links;
  const cached = cache.get(id);
  if (!cache.supported) {
    return oldServiceNote("this computer");
  }
  if (cached === undefined && slow) {
    return noAnswerNote(id);
  }
  return preview(id, cached?.entry, {
    botName: links.botName,
    currentProjectId: links.currentProjectId,
    lastTitle: cache.lastTitle(id),
  });
}

export default function CardLink({ id }: { readonly id: string }): ReactElement {
  const links = useCardLinks();
  const cache = links?.cache;
  useSyncExternalStore(cache?.subscribe ?? noSubscribe, cache?.snapshot ?? zero);
  const tip = usePreviewState(cache?.get(id) !== undefined);
  const tipId = useId();
  if (links === null) {
    return <>{id}</>;
  }
  const p = previewOf(id, links, tip.slow);
  const show = (): void => {
    links.cache.want(id);
    tip.open();
  };
  const onClick = (event: MouseEvent): void => {
    event.preventDefault();
    links.cache.want(id);
    const projectId = links.cache.get(id)?.entry.project_id;
    if (p.kind === "card" && projectId) {
      tip.hide();
      links.open(id, projectId);
    } else {
      show();
    }
  };
  return (
    <span className="card-link-wrap">
      <a
        href={`hermes://item/${id}`}
        className={p.kind === "note" ? "card-link card-link--dim" : "card-link"}
        aria-label={linkName(id, p)}
        aria-describedby={tip.shown ? tipId : undefined}
        onMouseEnter={() => {
          links.cache.want(id);
          tip.openLater();
        }}
        onMouseLeave={tip.closeLater}
        onFocus={show}
        onBlur={tip.closeLater}
        onClick={onClick}
      >
        {id}
      </a>
      {tip.shown ? (
        // The pointer may cross onto the preview without closing it (UX-035 §3).
        // oxlint-disable-next-line jsx-a11y/no-noninteractive-element-interactions
        <span
          id={tipId}
          role="tooltip"
          className="card-preview"
          onMouseEnter={tip.keep}
          onMouseLeave={tip.closeLater}
        >
          <PreviewBody p={p} />
        </span>
      ) : null}
    </span>
  );
}

function clear(timer: Timer): void {
  if (timer.current !== null) {
    clearTimeout(timer.current);
    timer.current = null;
  }
}

function noSubscribe(): () => void {
  return () => undefined;
}

function zero(): number {
  return 0;
}
