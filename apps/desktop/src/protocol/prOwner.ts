// The owner's pull request writes (H-269, H-271, H-282; H-261 §4.3, §16):
// the verdict, line comments and their resolve, the 10 s Undo and the Owner
// review setting. They are JSON requests on the app's own connection, which
// the daemon accepts only from a paired device or the app's ticket; the
// owner token and a bot's relay are refused before they run. Replies carry
// the daemon's JSON, not the binary shape, so the views read the PR again
// over the binary channel after a write rather than decode them.

import type { JsonValue } from "@bufbuild/protobuf";
import { fromJson } from "@bufbuild/protobuf";
import { ReviewSettingsSchema } from "./gen/hermes/pr/v1/pr_pb";
import type { ReviewSettings } from "./gen/hermes/pr/v1/pr_pb";

/** The Owner review setting's words on the wire. */
export type OwnerReviewMode = "all" | "areas" | "flagged" | "none";

export type PrOwnerRequestBody =
  | { readonly type: "review_settings_get"; readonly project_id: string }
  | {
      readonly type: "review_settings_set";
      readonly project_id: string;
      readonly owner_review: OwnerReviewMode;
      readonly owner_review_areas: readonly string[];
    }
  | {
      readonly type: "pr_review_submit";
      readonly project_id: string;
      readonly number: number;
      /** Must be the PR's head: an older commit is refused. */
      readonly sha: string;
      readonly verdict: "approved" | "changes_requested";
      /** Required with `changes_requested`: the author's must-fix, as written. */
      readonly summary?: string;
    }
  | {
      readonly type: "pr_comment_add";
      readonly project_id: string;
      readonly number: number;
      readonly sha: string;
      readonly path?: string;
      readonly line?: number;
      readonly side?: "old" | "new";
      readonly body: string;
      readonly severity?: "must" | "should" | "nit";
      /** A reply takes its thread's anchor. */
      readonly reply_to?: string;
    }
  | {
      readonly type: "pr_comment_resolve";
      readonly project_id: string;
      readonly number: number;
      readonly comment_id: string;
    }
  | { readonly type: "pr_merge_undo"; readonly project_id: string; readonly number: number };

export type PrOwnerReply =
  | { readonly type: "review_settings"; readonly review_settings: JsonValue }
  | { readonly type: "pr"; readonly pr: JsonValue; readonly review?: JsonValue }
  | { readonly type: "comment"; readonly comment: JsonValue };

/** The setting as the daemon sends it, read into the generated type. */
export function decodeReviewSettings(json: JsonValue): ReviewSettings {
  const raw = (json ?? {}) as Record<string, JsonValue>;
  const mode = typeof raw.owner_review === "string" ? raw.owner_review : "all";
  return fromJson(
    ReviewSettingsSchema,
    { ...raw, owner_review: `OWNER_REVIEW_${mode.toUpperCase()}` },
    { ignoreUnknownFields: true },
  );
}
