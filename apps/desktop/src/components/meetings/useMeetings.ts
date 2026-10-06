// The project's meetings (H-102) for the Meetings tab: the series and recent
// meetings from `meeting_list`, the open one in full from `meeting_get`, both
// read again on a `meeting_event` for the project.

import { useCallback, useEffect, useState } from "react";
import { useLatestRef } from "../../app/useLatestRef";
import { useLoadOnConnect } from "../../hooks/useLoadOnConnect";
import type { DaemonApi } from "../../protocol/api";
import type { ListedSeries, MeetingDetail, MeetingSummary } from "../../protocol/meetings";
import { errText } from "../../util";

export interface MeetingsState {
  readonly series: readonly ListedSeries[];
  readonly meetings: readonly MeetingSummary[];
  readonly loaded: boolean;
  /** Why the list can't be read, e.g. the board lives on another computer. */
  readonly error: string | null;
  readonly openId: string | null;
  readonly open: MeetingDetail | null;
  readonly select: (meetingId: string) => void;
}

/** The words a meeting type goes by; an ad-hoc meeting goes by its own name. */
const TYPE_WORD: Readonly<Record<string, string>> = {
  standup: "Stand-up",
  refinement: "Refinement",
  demo: "Demo",
  retro: "Retro",
};

/** "Stand-up minutes", "Retro minutes", "Pairing on H-117 minutes". */
export function minutesTitle(meeting: Pick<MeetingSummary, "type" | "name">): string {
  return `${TYPE_WORD[meeting.type] ?? meeting.name} minutes`;
}

/**
 * One meeting, by id, for naming where a summary came from. Null until it
 * loads, and when it can't be read (off the board's home).
 */
export function useMeeting(
  client: DaemonApi,
  projectId: string,
  meetingId: string | undefined,
  connected: boolean,
): MeetingSummary | null {
  const [meeting, setMeeting] = useState<MeetingSummary | null>(null);
  useEffect(() => {
    if (meetingId === undefined || meetingId.length === 0 || !connected) {
      return undefined;
    }
    let live = true;
    const load = async (): Promise<void> => {
      try {
        const reply = await client.request(
          { type: "meeting_get", project_id: projectId, meeting_id: meetingId },
          "meeting",
        );
        if (live) {
          setMeeting(reply.meeting);
        }
      } catch {
        // The row says "Meeting minutes" without it.
      }
    };
    void load();
    return () => {
      live = false;
    };
  }, [client, connected, meetingId, projectId]);
  return meeting?.id === meetingId ? meeting : null;
}

/** The meeting to show first: the newest held, else the newest of any kind. */
function firstToOpen(meetings: readonly MeetingSummary[]): string | null {
  return (meetings.find((m) => m.status === "held") ?? meetings[0])?.id ?? null;
}

export function useMeetings(
  client: DaemonApi,
  projectId: string,
  connected: boolean,
): MeetingsState {
  const [series, setSeries] = useState<readonly ListedSeries[]>([]);
  const [meetings, setMeetings] = useState<readonly MeetingSummary[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [openId, setOpenId] = useState<string | null>(null);
  const [open, setOpen] = useState<MeetingDetail | null>(null);

  const refresh = useCallback(async (): Promise<void> => {
    try {
      const reply = await client.request(
        { type: "meeting_list", project_id: projectId },
        "meetings",
      );
      setSeries(reply.series);
      setMeetings(reply.meetings);
      setOpenId((current) => current ?? firstToOpen(reply.meetings));
      setError(null);
    } catch (failure) {
      setError(errText(failure));
    }
    setLoaded(true);
  }, [client, projectId]);
  useLoadOnConnect(connected, refresh);

  const openRef = useLatestRef(openId);
  const loadOpen = useCallback(
    async (meetingId: string): Promise<void> => {
      try {
        const reply = await client.request(
          { type: "meeting_get", project_id: projectId, meeting_id: meetingId },
          "meeting",
        );
        // A later pick wins: drop an answer for a meeting no longer open.
        if (openRef.current === meetingId) {
          setOpen(reply.meeting);
        }
      } catch {
        setOpen(null);
      }
    },
    [client, openRef, projectId],
  );

  useEffect(() => {
    return client.on("meeting_event", (push) => {
      if (push.project_id === projectId) {
        void refresh();
        if (openRef.current !== null) {
          void loadOpen(openRef.current);
        }
      }
    });
  }, [client, loadOpen, openRef, projectId, refresh]);

  useEffect(() => {
    if (openId !== null && connected) {
      // oxlint-disable-next-line react/set-state-in-effect
      void loadOpen(openId);
    }
  }, [connected, loadOpen, openId]);

  return { series, meetings, loaded, error, openId, open, select: setOpenId };
}
