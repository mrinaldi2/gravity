// The projects home's data (H-133 U1): one `projects_overview`, read again
// when the service says a project changed. A daemon from before H-130 has no
// such read, so the home builds plain rows from the project and bot lists and
// marks them "Update needed for full info" (H-128 R2.1).

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
  /** Why the last read failed, while nothing has loaded. */
  readonly error: string | null;
  readonly loaded: boolean;
  readonly pin: (projectId: string, pinned: boolean) => Promise<void>;
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
      const reply = await client.request({ type: "projects_overview" }, "projects_overview");
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

  useEffect(() => {
    if (!supported) {
      return undefined;
    }
    const again = (): void => {
      void refresh();
    };
    const offChanged = client.on("projects_overview_changed", again);
    const offPinned = client.on("project_pinned", again);
    return () => {
      offChanged();
      offPinned();
    };
  }, [client, refresh, supported]);

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
    return { rows: fallback, sources: [], legacy: true, error: null, loaded: true, pin };
  }
  return { rows, sources, legacy: false, error, loaded, pin };
}
