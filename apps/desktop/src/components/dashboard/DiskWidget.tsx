// This computer's disk (H-275; H-261 §15.6): free space, and what each bot
// takes in its workspace, worktrees and build cache, biggest first. Read on
// open; the daemon keeps an hourly report and takes a new one when asked.

import { useCallback, useState } from "react";
import type { ReactElement } from "react";
import { useLoadOnConnect } from "../../hooks/useLoadOnConnect";
import type { DaemonApi } from "../../protocol/api";
import { sizeText } from "../../protocol/cleanup";
import type { DiskReport, DiskUse } from "../../protocol/cleanup";
import { Widget } from "./Widgets";

/** Bots listed before "and N more". */
const SHOWN = 4;

function useDiskReport(client: DaemonApi, connected: boolean) {
  const [report, setReport] = useState<DiskReport | null>(null);
  const read = useCallback(
    async (refresh: boolean): Promise<void> => {
      try {
        const reply = await client.request({ type: "disk_report", refresh }, "disk_report");
        setReport(reply.disk_report);
      } catch {
        // A service before 0.18 has no disk report: the widget stays away.
      }
    },
    [client],
  );
  const load = useCallback(() => read(false), [read]);
  useLoadOnConnect(connected, load);
  return { report, read };
}

function total(u: DiskUse): number {
  return u.workspace_bytes + u.worktree_bytes + u.cache_bytes;
}

function usageLine(u: DiskUse): string {
  const parts = [`${sizeText(total(u))}`];
  if (u.cache_bytes > 0) {
    parts.push(`${sizeText(u.cache_bytes)} build cache`);
  }
  if (u.worktree_bytes > 0) {
    parts.push(`${sizeText(u.worktree_bytes)} worktrees`);
  }
  return parts.join(" · ");
}

export default function DiskWidget(props: {
  readonly client: DaemonApi;
  readonly connected: boolean;
}): ReactElement | null {
  const { report, read } = useDiskReport(props.client, props.connected);
  if (report === null) {
    return null;
  }
  const old = report.uses.reduce((n, u) => n + u.reclaimable_bytes, 0);
  const rest = report.uses.length - SHOWN;
  return (
    <Widget
      id="dash-disk"
      title={`Disk · ${report.machine}`}
      more={{ label: "Check now", onClick: () => void read(true) }}
    >
      <p className="dash-disk-free">
        {sizeText(report.free_bytes)} free of {sizeText(report.total_bytes)}
        {old > 0 ? ` · ${sizeText(old)} is old build output` : ""}
      </p>
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
    </Widget>
  );
}
