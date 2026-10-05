// The pause for an install (H-117), shown across the whole app while the
// daemon's computer holds every project still. The owner can end it early.

import { useCallback, useEffect, useState } from "react";
import type { ReactElement } from "react";
import type { AddToast } from "../../app/useToasts";
import { useLoadOnConnect } from "../../hooks/useLoadOnConnect";
import type { DaemonApi } from "../../protocol/api";
import type { Quiesce } from "../../protocol/quiesce";
import { errText } from "../../util";

function time(at: string): string {
  return new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/** The open pause, read on connect and kept current by its pushes. */
function useQuiesce(client: DaemonApi, connected: boolean): Quiesce | null {
  const [quiesce, setQuiesce] = useState<Quiesce | null>(null);
  const load = useCallback(async (): Promise<void> => {
    try {
      const reply = await client.request({ type: "quiesce_status" }, "quiesce");
      setQuiesce(reply.quiesce);
    } catch {
      // An older daemon has no pause to show.
      setQuiesce(null);
    }
  }, [client]);
  useLoadOnConnect(connected, load);
  useEffect(() => client.on("quiesce_update", (push) => setQuiesce(push.quiesce)), [client]);
  return quiesce;
}

interface QuiesceBannerProps {
  readonly quiesce: Quiesce;
  /** The owner's approve grant: only it can end the pause early. */
  readonly canResume: boolean;
  readonly onResume: () => void;
}

export function QuiesceBanner({ quiesce, canResume, onResume }: QuiesceBannerProps): ReactElement {
  return (
    <div className="service-recovery quiesce-banner" role="status">
      <div className="service-recovery-text">
        <div className="service-recovery-title">
          <span aria-hidden="true">⏸ </span>Every project here is paused for the {quiesce.reason}
        </div>
        <div className="service-recovery-body">
          Bots, routines, workers and messages wait until the install finishes. Since{" "}
          {time(quiesce.started_at)}; everything resumes by itself at {time(quiesce.deadline_at)} if
          it doesn't.
        </div>
        {quiesce.report?.services_changed === true ? (
          <div className="service-recovery-body">
            <strong>
              <span aria-hidden="true">⚠ </span>The services list in hermesd.toml changed since the
              daemon started.
            </strong>{" "}
            The list it started with was used; check the new one before the next install.
          </div>
        ) : null}
      </div>
      {canResume ? (
        <div className="service-recovery-actions">
          <button type="button" className="btn btn-small btn-primary" onClick={onResume}>
            Resume now
          </button>
        </div>
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
  const quiesce = useQuiesce(client, connected);
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
