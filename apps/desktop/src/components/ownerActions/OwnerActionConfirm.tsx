// The last look before an owner action runs (H-117 R2, ARCH-R49): the
// target computer and the whole content, verbatim. Run stays off until the
// owner has scrolled to the end of a long script, so nothing hides below
// the fold; on a touch screen it is press-and-hold, so a stray tap can't
// fire it.

import { useEffect, useRef, useState } from "react";
import type { PointerEvent, ReactElement, UIEvent } from "react";
import type { OwnerAction } from "../../protocol/ownerActions";

/** How long a touch press must last to run. */
export const HOLD_MS = 800;

function coarsePointer(): boolean {
  return typeof window.matchMedia === "function" && window.matchMedia("(pointer: coarse)").matches;
}

function RunButton(props: {
  readonly enabled: boolean;
  readonly hold: boolean;
  readonly onRun: () => void;
}): ReactElement {
  const timer = useRef<number | null>(null);
  const [holding, setHolding] = useState(false);
  const stop = (): void => {
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
    setHolding(false);
  };
  // A press still held when the sheet closes never fires.
  useEffect(
    () => () => {
      if (timer.current !== null) {
        window.clearTimeout(timer.current);
      }
    },
    [],
  );
  if (!props.hold) {
    return (
      <button
        type="button"
        className="btn btn-danger"
        disabled={!props.enabled}
        onClick={props.onRun}
      >
        Run it
      </button>
    );
  }
  const start = (event: PointerEvent<HTMLButtonElement>): void => {
    event.preventDefault();
    setHolding(true);
    timer.current = window.setTimeout(() => {
      stop();
      props.onRun();
    }, HOLD_MS);
  };
  return (
    <button
      type="button"
      className={holding ? "btn btn-danger owner-action-holding" : "btn btn-danger"}
      disabled={!props.enabled}
      onPointerDown={start}
      onPointerUp={stop}
      onPointerLeave={stop}
      onPointerCancel={stop}
    >
      {holding ? "Keep holding…" : "Hold to run"}
    </button>
  );
}

export default function OwnerActionConfirm(props: {
  readonly action: OwnerAction;
  readonly onRun: () => void;
  readonly onCancel: () => void;
  /** Tests force the touch form. */
  readonly hold?: boolean;
}): ReactElement {
  const { action } = props;
  const [seen, setSeen] = useState(false);
  const pre = useRef<HTMLPreElement | null>(null);
  // A script that fits needs no scrolling to be seen whole.
  useEffect(() => {
    const el = pre.current;
    if (el !== null && el.scrollHeight <= el.clientHeight + 1) {
      setSeen(true);
    }
  }, []);
  const onScroll = (event: UIEvent<HTMLPreElement>): void => {
    const el = event.currentTarget;
    if (el.scrollTop + el.clientHeight >= el.scrollHeight - 2) {
      setSeen(true);
    }
  };
  const target = action.target_name ?? "this computer";
  return (
    <div
      className="owner-action-confirm"
      role="dialog"
      aria-modal="true"
      aria-label={`Run on ${target}`}
    >
      <h3>Run on {target}?</h3>
      <p className="owner-action-meta">
        {action.shell} in <code>{action.cwd}</code> · sha {action.sha256.slice(0, 12)}
      </p>
      {/* The script scrolls, so the keyboard must be able to reach it. */}
      <pre
        ref={pre}
        className="owner-action-content owner-action-full"
        // oxlint-disable-next-line jsx-a11y/no-noninteractive-tabindex
        tabIndex={0}
        onScroll={onScroll}
      >
        {action.content}
      </pre>
      {seen ? null : <p className="owner-action-hint">Scroll to the end to run it.</p>}
      <div className="owner-action-buttons">
        <button type="button" className="btn" onClick={props.onCancel}>
          Cancel
        </button>
        <RunButton enabled={seen} hold={props.hold ?? coarsePointer()} onRun={props.onRun} />
      </div>
    </div>
  );
}
