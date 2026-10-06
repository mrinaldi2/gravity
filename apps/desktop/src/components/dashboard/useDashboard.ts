// The project dashboard's data (H-076): one `dashboard_get`, read again when
// a decision or a meeting changes and every half minute, so "as of" stays near now.

import { useCallback, useEffect, useState } from "react";
import { useLoadOnConnect } from "../../hooks/useLoadOnConnect";
import type { DaemonApi } from "../../protocol/api";
import type { Dashboard } from "../../protocol/dashboard";
import { errText } from "../../util";

/** How often the dashboard reads itself again while it is open. */
export const REFRESH_MS = 30_000;

export interface DashboardState {
  readonly dashboard: Dashboard | null;
  /** Why the last read failed, while nothing has loaded. */
  readonly error: string | null;
  readonly refresh: () => Promise<void>;
}

export function useDashboard(
  client: DaemonApi,
  projectId: string,
  connected: boolean,
): DashboardState {
  const [dashboard, setDashboard] = useState<Dashboard | null>(null);
  const [error, setError] = useState<string | null>(null);
  const refresh = useCallback(async (): Promise<void> => {
    try {
      const reply = await client.request(
        { type: "dashboard_get", project_id: projectId, all_kinds: true },
        "dashboard",
      );
      setDashboard(reply.dashboard);
      setError(null);
    } catch (failure) {
      setError(errText(failure));
    }
  }, [client, projectId]);
  useLoadOnConnect(connected, refresh);

  useEffect(() => {
    if (!connected) {
      return;
    }
    const again = (): void => {
      void refresh();
    };
    const timer = window.setInterval(again, REFRESH_MS);
    const off = [
      client.on("decision_update", again),
      client.on("decision_deleted", again),
      client.on("workers_updated", again),
      client.on("meeting_event", (push) => {
        if (push.project_id === projectId) {
          again();
        }
      }),
    ];
    return () => {
      window.clearInterval(timer);
      for (const stop of off) {
        stop();
      }
    };
  }, [client, connected, projectId, refresh]);

  return { dashboard, error, refresh };
}
