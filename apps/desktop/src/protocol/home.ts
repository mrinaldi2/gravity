// The projects home and the owner threads (H-128 rev 2, H-130/H-132): typed
// `hermes.home.v1` messages, sent on the JSON protocol as proto3 JSON with
// the proto field names. Replies carry the message as JSON; `decode*` turns
// it into the generated type, so the app reads one shape whatever the wire.

import { fromJson } from "@bufbuild/protobuf";
import type { JsonValue } from "@bufbuild/protobuf";
import { ProjectsOverviewSchema } from "./gen/hermes/home/v1/home_pb";
import type { ProjectsOverview } from "./gen/hermes/home/v1/home_pb";

export type HomeRequestBody =
  | { readonly type: "projects_overview"; readonly project_ids?: readonly string[] }
  | { readonly type: "attention_rows"; readonly project_id: string }
  | { readonly type: "attention_dismiss"; readonly id: string }
  | { readonly type: "project_pin"; readonly project_id: string; readonly pinned: boolean }
  | { readonly type: "owner_threads" }
  | {
      readonly type: "owner_thread_get";
      readonly bot_id: string;
      readonly before_num?: number;
      readonly limit?: number;
    }
  | { readonly type: "owner_thread_read"; readonly bot_id: string; readonly up_to_num: number };

export type HomeReply =
  | { readonly type: "projects_overview"; readonly overview: JsonValue }
  | { readonly type: "attention_rows"; readonly attention_rows: JsonValue }
  | { readonly type: "attention_dismissed"; readonly id: string }
  | { readonly type: "project_pinned"; readonly project_id: string; readonly pinned: boolean }
  | { readonly type: "owner_threads"; readonly owner_threads: JsonValue }
  | { readonly type: "owner_thread"; readonly owner_thread: JsonValue }
  | { readonly type: "owner_thread_marked"; readonly owner_thread_marked: JsonValue };

/** A bot as the home names it: its own computer and its id there. */
interface BotRefJson {
  readonly daemon_id?: string;
  readonly bot_id?: string;
  readonly name?: string;
}

export type HomePush =
  | { readonly type: "projects_overview_changed"; readonly project_ids: readonly string[] }
  | {
      readonly type: "owner_thread_updated";
      readonly bot?: BotRefJson;
      readonly bot_id?: string;
      readonly project_id?: string;
    }
  | { readonly type: "project_pinned"; readonly project_id: string; readonly pinned: boolean };

const LENIENT = { ignoreUnknownFields: true } as const;

export function decodeOverview(json: JsonValue): ProjectsOverview {
  return fromJson(ProjectsOverviewSchema, json, LENIENT);
}
