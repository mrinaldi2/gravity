// A card id as a link (UX-035 §2–§4): the id as written, a preview on a
// 400 ms hover or on focus, and a click that opens the card in the app's
// drawer. A card that can't be opened (deleted, or on a computer that's
// offline) keeps its note open instead. Esc closes the preview first.

import { useEffect, useId, useRef, useState, useSyncExternalStore } from "react";
import type { MouseEvent, ReactElement } from "react";
import type { Preview } from "./cardText";
import { linkName, noAnswerNote, oldServiceNote, preview } from "./cardText";
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

export default function CardLink({ id }: { readonly id: string }): ReactElement {
  const links = useCardLinks();
  const tipId = useId();
  const [shown, setShown] = useState(false);
  const [slow, setSlow] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const cache = links?.cache;
  useSyncExternalStore(cache?.subscribe ?? noSubscribe, cache?.snapshot ?? zero);
  const cached = cache?.get(id);

  useEffect(() => () => clear(timer), []);
  // "Loading…" only for so long.
  useEffect(() => {
    if (!shown || cached !== undefined) {
      return undefined;
    }
    const wait = setTimeout(() => setSlow(true), SLOW_MS);
    return () => clearTimeout(wait);
  }, [shown, cached]);
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

  if (links === null || cache === undefined) {
    return <>{id}</>;
  }
  const p: Preview = !cache.supported
    ? oldServiceNote("this computer")
    : cached === undefined && slow
      ? noAnswerNote(id)
      : preview(id, cached?.entry, {
          botName: links.botName,
          currentProjectId: links.currentProjectId,
          lastTitle: cache.lastTitle(id),
        });
  const dim = p.kind === "note";
  const show = (): void => {
    clear(timer);
    cache.want(id);
    setSlow(false);
    setShown(true);
  };
  const later = (ms: number, then: () => void): void => {
    clear(timer);
    timer.current = setTimeout(then, ms);
  };
  const onClick = (event: MouseEvent): void => {
    event.preventDefault();
    cache.want(id);
    const projectId = cached?.entry.project_id;
    if (p.kind === "card" && projectId) {
      setShown(false);
      links.open(id, projectId);
    } else {
      show();
    }
  };
  return (
    <span className="card-link-wrap">
      <a
        href={`hermes://item/${id}`}
        className={dim ? "card-link card-link--dim" : "card-link"}
        aria-label={linkName(id, p)}
        aria-describedby={shown ? tipId : undefined}
        onMouseEnter={() => {
          cache.want(id);
          later(HOVER_MS, show);
        }}
        onMouseLeave={() => later(LEAVE_MS, () => setShown(false))}
        onFocus={show}
        onBlur={() => later(LEAVE_MS, () => setShown(false))}
        onClick={onClick}
      >
        {id}
      </a>
      {shown ? (
        // The pointer may cross onto the preview without closing it (UX-035 §3).
        // oxlint-disable-next-line jsx-a11y/no-noninteractive-element-interactions
        <span
          id={tipId}
          role="tooltip"
          className="card-preview"
          onMouseEnter={() => clear(timer)}
          onMouseLeave={() => later(LEAVE_MS, () => setShown(false))}
        >
          <PreviewBody p={p} />
        </span>
      ) : null}
    </span>
  );
}

function clear(timer: { current: ReturnType<typeof setTimeout> | null }): void {
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
