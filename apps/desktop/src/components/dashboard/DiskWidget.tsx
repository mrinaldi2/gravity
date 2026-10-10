// This computer's disk (H-275; H-261 §15.6, UX-055): free space, and what
// each bot takes in its workspace, worktrees and build cache, biggest first.
// Read on open; the daemon keeps an hourly report and takes a new one when
// asked. Under 20 GB free it says so, with Clean up.

import { useCallback, useState } from "react";
import type { ReactElement } from "react";
import { useLoadOnConnect } from "../../hooks/useLoadOnConnect";
import type { DaemonApi } from "../../protocol/api";
import { sizeText } from "../../protocol/cleanup";
import type { DiskReport, DiskUse } from "../../protocol/cleanup";
import { fmtTimestamp } from "../../util";

/** Bots listed before "and N more". */
const SHOWN = 4;
/** Under this much free space the widget says the computer is low. */
const LOW = 20_000_000_000;

function useDiskReport(client: DaemonApi, connected: boolean) {
  const [report, setReport] = useState<DiskReport | null>(null);
  const [checking, setChecking] = useState(false);
  const read = useCallback(
    async (refresh: boolean): Promise<void> => {
      setChecking(refresh);
      try {
        const reply = await client.request({ type: "disk_report", refresh }, "disk_report");
        setReport(reply.disk_report);
      } catch {
        // A service before 0.18 has no disk report: the widget stays away.
      }
      setChecking(false);
    },
    [client],
  );
  const load = useCallback(() => read(false), [read]);
  useLoadOnConnect(connected, load);
  return { report, checking, read };
}

function total(u: DiskUse): number {
  return u.workspace_bytes + u.worktree_bytes + u.cache_bytes;
}

function usageLine(u: DiskUse): string {
  const parts = [`${sizeText(total(u))}`];
  if (u.reclaimable_bytes > 0) {
    parts.push(`${sizeText(u.reclaimable_bytes)} old build output`);
  } else if (u.cache_bytes > 0) {
    parts.push(`${sizeText(u.cache_bytes)} build cache`);
  }
  if (u.worktree_bytes > 0) {
    parts.push(`${sizeText(u.worktree_bytes)} worktrees`);
  }
  return parts.join(" · ");
}

function FreeLine(props: {
  readonly report: DiskReport;
  readonly onCleanUp?: (machine: string) => void;
  readonly cleaningUp?: string | null;
}): ReactElement {
  const r = props.report;
  const old = r.uses.reduce((n, u) => n + u.reclaimable_bytes, 0);
  const low = r.free_bytes < LOW;
  return (
    <p className="dash-disk-free">
      {low ? <strong className="dash-disk-low">⚠ Low on disk · </strong> : null}
      {sizeText(r.free_bytes)} free of {sizeText(r.total_bytes)}
      {old > 0 ? ` · ${sizeText(old)} is old build output` : ""}
      {low && props.onCleanUp ? (
        <button
          type="button"
          className="btn btn-small dash-disk-clean"
          aria-label={`Clean up ${r.machine}`}
          disabled={props.cleaningUp === r.machine}
          onClick={() => props.onCleanUp?.(r.machine)}
        >
          {props.cleaningUp === r.machine ? "Cleaning up…" : "Clean up"}
        </button>
      ) : null}
    </p>
  );
}

export default function DiskWidget(props: {
  readonly client: DaemonApi;
  readonly connected: boolean;
  readonly onCleanUp?: (machine: string) => void;
  readonly cleaningUp?: string | null;
}): ReactElement | null {
  const { report, checking, read } = useDiskReport(props.client, props.connected);
  if (report === null) {
    return null;
  }
  const rest = report.uses.length - SHOWN;
  return (
    <section className="dash-widget" aria-labelledby="dash-disk">
      <h2 id="dash-disk">
        Disk · {report.machine}
        <span className="dash-disk-asof">
          {checking ? "Checking…" : `Checked ${fmtTimestamp(report.as_of)}`}
        </span>
        <button
          type="button"
          className="dash-more"
          disabled={checking}
          onClick={() => void read(true)}
        >
          Check now
        </button>
      </h2>
      <FreeLine report={report} onCleanUp={props.onCleanUp} cleaningUp={props.cleaningUp} />
      {report.uses.length === 0 ? (
        <p className="dash-empty">No bot folders on this computer.</p>
      ) : (
        <ul className="dash-disk-uses">
          {report.uses.slice(0, SHOWN).map((u) => (
            <li key={u.bot.id}>
              <span className="dash-disk-bot">{u.bot.name}</span>
              <span className="dash-disk-size">{usageLine(u)}</span>
            </li>
          ))}
          {rest > 0 ? <li className="dash-disk-more">and {rest} more</li> : null}
        </ul>
      )}
    </section>
  );
}
