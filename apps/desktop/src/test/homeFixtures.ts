// A projects overview as the service sends it (proto3 JSON, proto field
// names): three projects on two computers, one of them away.

import type { JsonValue } from "@bufbuild/protobuf";

export const HOME_NOW = Date.parse("2026-10-06T10:30:00Z");

export function overviewJson(): JsonValue {
  return {
    as_of: "2026-10-06T10:30:00Z",
    sources: [
      { daemon_id: "mac", name: "mac", state: "OK", as_of: "2026-10-06T10:30:00Z" },
      { daemon_id: "imac", name: "imac", state: "OFFLINE", as_of: "2026-10-06T10:20:00Z" },
    ],
    total: { count: 5, score: 30 },
    rows: [
      {
        project_id: "p1",
        name: "The Hermes",
        members: [{ daemon_id: "mac", project_id: "p1", computer_name: "mac" }],
        board_home: "mac",
        current_release: {
          release_id: "r1",
          version: "0.17.0",
          state: "awaiting_owner",
          awaiting_owner: true,
          items_total: 3,
        },
        open_tasks: 5,
        doing: [
          {
            item_id: "H-133",
            title: "Project-first desktop UI",
            assignee_name: "Desktop Dev",
            assignee_bot_id: "b1",
          },
        ],
        bots: 8,
        bots_working: 5,
        attention: {
          count: 4,
          score: 25,
          by_kind: { release_awaiting: 1, decision: 2, owner_action: 1 },
        },
        latest_summary: {
          meeting_id: "m1",
          text: "0.17.0 is packaged; installs wait on your ruling.",
          at: "2026-10-06T09:05:00Z",
        },
        last_activity_at: "2026-10-06T10:25:00Z",
        rank: 0,
      },
      {
        project_id: "p2",
        name: "PhD",
        members: [
          { daemon_id: "mac", project_id: "p2", computer_name: "mac" },
          { daemon_id: "imac", project_id: "x2", computer_name: "imac" },
        ],
        current_release: { release_id: "r2", version: "2.3", state: "deployed" },
        open_tasks: 2,
        bots: 4,
        bots_working: 2,
        attention: { count: 1, score: 5, by_kind: { decision: 1 } },
        partial: true,
        stale_sources: ["imac"],
        pinned: true,
        rank: 1,
      },
      {
        project_id: "p3",
        name: "Aurora Notes",
        members: [{ daemon_id: "mac", project_id: "p3", computer_name: "mac" }],
        open_tasks: 0,
        bots: 3,
        bots_working: 0,
        attention: { count: 0, score: 0 },
        last_activity_at: "2026-10-03T10:00:00Z",
        rank: 2,
      },
    ],
  };
}

/** The owner's threads: Desktop Dev asked a question, Team Lead reported. */
export function ownerThreadsJson(): JsonValue {
  return {
    threads: [
      {
        bot: { daemon_id: "mac", bot_id: "b2", name: "Desktop Dev" },
        project_id: "p1",
        last: {
          num: "12",
          from_owner: false,
          text: "Should Resume now also restart the services?",
          at: "2026-10-06T10:20:00Z",
          asks: true,
        },
        unread: 1,
        open_question: true,
      },
      {
        bot: { daemon_id: "mac", bot_id: "b1", name: "Team Lead" },
        project_id: "p1",
        last: {
          num: "9",
          from_owner: false,
          text: "Verify is full; testers clear H-008 first.",
          at: "2026-10-06T09:40:00Z",
        },
      },
    ],
  };
}

/** Desktop Dev's thread with the owner, oldest first. */
export function ownerThreadJson(): JsonValue {
  return {
    bot: { daemon_id: "mac", bot_id: "b2", name: "Desktop Dev" },
    project_id: "p1",
    messages: [
      {
        num: "10",
        id: "m10",
        from_owner: false,
        text: "Pushed 058a4f7.",
        at: "2026-10-06T09:00:00Z",
      },
      { num: "11", id: "m11", from_owner: true, text: "Thanks.", at: "2026-10-06T09:05:00Z" },
      {
        num: "12",
        id: "m12",
        from_owner: false,
        text: "Should Resume now also restart the services?",
        at: "2026-10-06T10:20:00Z",
        asks: true,
        open: true,
      },
    ],
    last_read_num: "11",
  };
}
