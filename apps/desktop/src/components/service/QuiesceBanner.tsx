// The pause for an install (H-117), shown across the whole app while every
// project on this computer is held still. The owner can end it early, after
// a confirm: resuming mid-install can break it. Copy per UX on H-117.

import { useState } from "react";
import type { ReactElement } from "react";
import type { AddToast } from "../../app/useToasts";
import type { DaemonApi } from "../../protocol/api";
import type { Quiesce, QuiesceHolder } from "../../protocol/quiesce";
import { errText } from "../../util";
import ConfirmDialog from "../overlay/ConfirmDialog";
import { installing, useQuiesce } from "./useQuiesce";

function time(at: string): string {
  return new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/** "node (process 4120) · DevOps in Gravity". */
function holderLine(h: QuiesceHolder): string {
  let who = "";
  if (h.bot_name && h.project_name) {
    who = ` · ${h.bot_name} in ${h.project_name}`;
  } else if (h.project_name) {
    who = ` · in ${h.project_name}`;
  }
  return `${h.command} (process ${h.pid})${who}`;
}

/** What still holds the Hermes folder, while the install waits on it. */
function Blocked({ quiesce }: { readonly quiesce: Quiesce }): ReactElement | null {
  const holders = quiesce.report?.unresolved ?? [];
  if (quiesce.phase !== "blocked" || holders.length === 0) {
    return null;
  }
  return (
    <div className="service-recovery-body">
      The install is waiting for these programs to close the Hermes folder:
      <ul className="quiesce-holders">
        {holders.map((h) => (
          <li key={h.pid}>{holderLine(h)}</li>
        ))}
      </ul>
      Quit them to let the install go ahead.
    </div>
  );
}

interface QuiesceBannerProps {
  readonly quiesce: Quiesce;
  /** The owner's approve grant: only it can end the pause early. */
  readonly canResume: boolean;
  readonly onResume: () => void;
}

export function QuiesceBanner({ quiesce, canResume, onResume }: QuiesceBannerProps): ReactElement {
  const [confirming, setConfirming] = useState(false);
  const what = installing(quiesce);
  return (
    <div className="service-recovery quiesce-banner" role="status">
      <div className="service-recovery-text">
        <div className="service-recovery-title">
          <span aria-hidden="true">⏸ </span>Every project on this computer is paused while The
          Hermes {what} installs
        </div>
        <div className="service-recovery-body">
          Bots, routines, workers and messages are on hold. They pick up where they left off when
          the install is done, or at {time(quiesce.deadline_at)} at the latest. Paused since{" "}
          {time(quiesce.started_at)}.
        </div>
        <Blocked quiesce={quiesce} />
        {quiesce.report?.services_changed === true ? (
          <div className="service-recovery-body">
            <strong>
              <span aria-hidden="true">⚠ </span>The list of background services changed after the
              Hermes service started.
            </strong>{" "}
            This install uses the earlier list; restart the Hermes service to use the new one.
          </div>
        ) : null}
      </div>
      {canResume ? (
        <div className="service-recovery-actions">
          <button type="button" className="btn btn-small" onClick={() => setConfirming(true)}>
            Resume now
          </button>
        </div>
      ) : null}
      {confirming ? (
        <ConfirmDialog
          title="Resume everything now?"
          body={`The install of ${what} is still running. If bots start working now, it may fail and this computer may go back to the previous version.`}
          confirmLabel="Resume now"
          onCancel={() => setConfirming(false)}
          onConfirm={() => {
            setConfirming(false);
            onResume();
          }}
        />
      ) : null}
    </div>
  );
}

/** The banner for this daemon, when a pause is open. */
export default function QuiesceLayer(props: {
  readonly client: DaemonApi;
  readonly connected: boolean;
  readonly addToast: AddToast;
}): ReactElement | null {
  const { client, connected, addToast } = props;
  const quiesce = useQuiesce(client, connected, addToast);
  if (quiesce === null) {
    return null;
  }
  const resume = (): void => {
    client.request({ type: "quiesce_resume" }, "quiesce").catch((failure: unknown) => {
      addToast("error", "Couldn't resume the projects", errText(failure));
    });
  };
  return (
    <QuiesceBanner
      quiesce={quiesce}
      canResume={connected && client.hasGrant("approve")}
      onResume={resume}
    />
  );
}
