// The projects home's data (H-133 U1): one `projects_overview`, read again
// when the service says a project changed. A daemon from before H-130 has no
// such read, so the home builds plain rows from the project and bot lists and
// marks them "Update needed for full info" (H-128 R2.1). The same plain rows
// stand in when the read fails or doesn't answer in time (H-167), so every
// project, and through it every bot, stays reachable.

import { create } from "@bufbuild/protobuf";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useLoadOnConnect } from "../../hooks/useLoadOnConnect";
import type { DaemonApi } from "../../protocol/api";
import type { Bot, Project } from "../../protocol/entities";
import {
  ProjectRowSchema,
  type ProjectRow,
  type Source,
} from "../../protocol/gen/hermes/home/v1/home_pb";
import { decodeOverview } from "../../protocol/home";
import { errText } from "../../util";

export interface ProjectsOverviewState {
  readonly rows: readonly ProjectRow[];
  readonly sources: readonly Source[];
  /** True when the daemon predates the overview: rows are built here, without counts. */
  readonly legacy: boolean;
  /** True when the overview failed or didn't answer: the rows are the plain list. */
  readonly failed: boolean;
  /** Why the last read failed, while nothing has loaded. */
  readonly error: string | null;
  readonly loaded: boolean;
  readonly pin: (projectId: string, pinned: boolean) => Promise<void>;
}

/** How long the home waits on the overview before it shows the plain list. */
export const OVERVIEW_WAIT_MS = 8000;

/** `reply`, or a failure once `ms` pass without one. */
function inTime<T>(reply: Promise<T>, ms: number): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(new Error("The service didn't answer in time"));
    }, ms);
    reply.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (failure: unknown) => {
        clearTimeout(timer);
        reject(failure instanceof Error ? failure : new Error(String(failure)));
      },
    );
  });
}

/** Rows for a daemon without `projects_overview`: names and bot counts only. */
function legacyRows(projects: readonly Project[], bots: readonly Bot[]): readonly ProjectRow[] {
  return projects.map((project, rank) =>
    create(ProjectRowSchema, {
      projectId: project.id,
      name: project.name,
      bots: bots.filter((bot) => bot.project_id === project.id).length,
      rank,
      legacy: true,
    }),
  );
}

export function useProjectsOverview(
  client: DaemonApi,
  connected: boolean,
  projects: readonly Project[],
  bots: readonly Bot[],
): ProjectsOverviewState {
  const supported = client.capabilities.includes("projects_overview");
  const [rows, setRows] = useState<readonly ProjectRow[]>([]);
  const [sources, setSources] = useState<readonly Source[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);

  const refresh = useCallback(async (): Promise<void> => {
    if (!supported) {
      return;
    }
    try {
      const reply = await inTime(
        client.request({ type: "projects_overview" }, "projects_overview"),
        OVERVIEW_WAIT_MS,
      );
      const overview = decodeOverview(reply.overview);
      setRows(overview.rows);
      setSources(overview.sources);
      setError(null);
      setLoaded(true);
    } catch (failure) {
      setError(errText(failure));
    }
  }, [client, supported]);
  useLoadOnConnect(connected, refresh);
  useOverviewPushes(client, supported, refresh);

  const pin = useCallback(
    async (projectId: string, pinned: boolean): Promise<void> => {
      await client.request(
        { type: "project_pin", project_id: projectId, pinned },
        "project_pinned",
      );
      await refresh();
    },
    [client, refresh],
  );

  const fallback = useMemo(() => legacyRows(projects, bots), [projects, bots]);
  if (!supported) {
    return {
      rows: fallback,
      sources: [],
      legacy: true,
      failed: false,
      error: null,
      loaded: true,
      pin,
    };
  }
  if (error !== null && !loaded) {
    return { rows: fallback, sources: [], legacy: true, failed: true, error, loaded: true, pin };
  }
  return { rows, sources, legacy: false, failed: false, error, loaded, pin };
}

/**
 * One project's overview row, for its window's header, Overview and Team:
 * what needs the owner, its release, who is doing what, the lead's summary.
 * Null on an older service, or until it loads.
 */
export function useProjectRow(
  client: DaemonApi,
  projectId: string,
  connected: boolean,
): ProjectRow | null {
  const supported = client.capabilities.includes("projects_overview");
  const [row, setRow] = useState<ProjectRow | null>(null);
  const refresh = useCallback(async (): Promise<void> => {
    if (!supported) {
      return;
    }
    try {
      const reply = await client.request(
        { type: "projects_overview", project_ids: [projectId] },
        "projects_overview",
      );
      setRow(decodeOverview(reply.overview).rows.find((r) => r.projectId === projectId) ?? null);
    } catch {
      // The header and Team read fine without it.
    }
  }, [client, projectId, supported]);
  useLoadOnConnect(connected, refresh);
  useOverviewPushes(client, supported, refresh, projectId);
  return row;
}

/**
 * Reads again when the service says the overview changed, or a pin moved;
 * with `projectId`, only for a change that names it (or names none).
 */
function useOverviewPushes(
  client: DaemonApi,
  supported: boolean,
  refresh: () => Promise<void>,
  projectId?: string,
): void {
  useEffect(() => {
    if (!supported) {
      return undefined;
    }
    const again = (): void => {
      void refresh();
    };
    const offChanged = client.on("projects_overview_changed", (push) => {
      const named = push.project_ids.length === 0 || projectId === undefined;
      if (named || push.project_ids.includes(projectId)) {
        again();
      }
    });
    const offPinned = client.on("project_pinned", again);
    return () => {
      offChanged();
      offPinned();
    };
  }, [client, projectId, refresh, supported]);
}
