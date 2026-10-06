// The Flow widget's numbers (B11): one `metrics_get` for the chosen range,
// read again when the range changes and on the dashboard's refresh beat.

import { useCallback, useEffect, useState } from "react";
import { useLoadOnConnect } from "../../hooks/useLoadOnConnect";
import type { DaemonApi } from "../../protocol/api";
import type { FlowMetrics, MetricsRange } from "../../protocol/metrics";
import { errText } from "../../util";
import { REFRESH_MS } from "./useDashboard";

export interface MetricsState {
  readonly range: MetricsRange;
  readonly setRange: (range: MetricsRange) => void;
  /** Null until read, without a board, or with the home away. */
  readonly metrics: FlowMetrics | null;
  /** Loaded at least once. */
  readonly loaded: boolean;
  /** Off-home: where the numbers are kept, when they can't be read. */
  readonly note: string | null;
  readonly error: string | null;
}

export function useMetrics(client: DaemonApi, projectId: string, connected: boolean): MetricsState {
  const [range, setRange] = useState<MetricsRange>("week");
  const [metrics, setMetrics] = useState<FlowMetrics | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const refresh = useCallback(async (): Promise<void> => {
    try {
      const reply = await client.request(
        { type: "metrics_get", project_id: projectId, range },
        "metrics",
      );
      setMetrics(reply.metrics);
      setNote(reply.note);
      setError(null);
      setLoaded(true);
    } catch (failure) {
      setError(errText(failure));
    }
  }, [client, projectId, range]);
  useLoadOnConnect(connected, refresh);

  useEffect(() => {
    if (!connected) {
      return;
    }
    const timer = window.setInterval(() => void refresh(), REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [connected, refresh]);

  return { range, setRange, metrics, loaded, note, error };
}
