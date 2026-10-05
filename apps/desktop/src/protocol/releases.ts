// Release packages over the JSON WebSocket (H-020 §2, §6; docs/protocol.md
// "Releases and the deploy gate"). The owner reads them, rules on them, holds
// them and pauses their rollout; DevOps and testers use MCP tools instead.

export type ReleaseStatus =
  | "assembling"
  | "built"
  | "awaiting_owner"
  | "held"
  | "repackaging"
  | "superseded"
  | "approved"
  | "deploying"
  | "paused"
  | "partially_deployed"
  | "deployed"
  | "rejected"
  | "rolled_back"
  | "cancelled";

export type Verdict = "pending" | "ship" | "hold" | "rework";

export interface ReleaseItem {
  readonly item_id: string;
  readonly verdict: Verdict;
  readonly owner_note: string | null;
}

export interface ReleaseBuild {
  readonly platform: string;
  readonly version: string;
  readonly artifact: string;
  readonly url: string | null;
  readonly install_url: string | null;
  readonly sha256: string;
  readonly built_at: string;
}

export interface ReleaseTest {
  readonly machine: string;
  /** The tester's bot id. */
  readonly tester: string;
  readonly build_sha256: string;
  readonly result: "pass" | "fail" | "blocked";
}

export interface ReleaseDeployment {
  readonly machine: string;
  readonly action: "deploy" | "rollback";
  /** The bot carrying it out. */
  readonly executor: string;
  readonly task_id: string | null;
  readonly result: "ok" | "failed" | "rolled_back" | null;
  readonly smoke: "pass" | "fail" | null;
  readonly log_artifact: string | null;
  readonly started_at: string;
  readonly at: string | null;
}

/** One how-to-test entry: the steps for one platform, maybe for one item. */
export interface HowToTest {
  readonly item_id?: string | null;
  readonly platform: string;
  readonly steps: readonly string[];
}

export interface Release {
  readonly id: string;
  readonly project_id: string;
  readonly name: string;
  readonly display_version: string | null;
  readonly status: ReleaseStatus;
  readonly decision_id: string | null;
  readonly supersedes: string | null;
  readonly install_mode: string;
  readonly rollback_to: string | null;
  readonly changelog: string;
  readonly how_to_test: readonly HowToTest[];
  readonly frozen_at: string | null;
  readonly frozen_hash: string | null;
  /** The bot id of DevOps, who packaged it. */
  readonly created_by: string;
  readonly version: number;
  readonly paused_reason: string | null;
  readonly held_note: string | null;
  readonly remind_at: string | null;
  readonly items: readonly ReleaseItem[];
  readonly builds: readonly ReleaseBuild[];
  readonly tests: readonly ReleaseTest[];
  readonly deployments: readonly ReleaseDeployment[];
  /** Whether this connection may rule on it (the approve grant, on its home). */
  readonly can_rule?: boolean;
  /** When it can't: the computer where the owner can. */
  readonly rule_on?: string | null;
}

export interface ItemVerdict {
  readonly item_id: string;
  readonly verdict: Exclude<Verdict, "pending">;
  readonly note?: string;
}

export type ReleaseRequestBody =
  | { readonly type: "list_releases"; readonly project_id: string }
  | { readonly type: "get_release"; readonly release_id: string }
  | {
      readonly type: "release_rule";
      readonly release_id: string;
      readonly verdicts: readonly ItemVerdict[];
      readonly expected_version: number;
    }
  | {
      readonly type: "release_hold";
      readonly release_id: string;
      readonly note?: string;
      /** RFC 3339. */
      readonly remind_at?: string;
    }
  | { readonly type: "release_unhold"; readonly release_id: string }
  | { readonly type: "release_pause"; readonly release_id: string; readonly reason: string }
  | { readonly type: "release_resume"; readonly release_id: string };

export type ReleaseReply =
  | { readonly type: "releases"; readonly releases: readonly Release[] }
  | { readonly type: "release"; readonly release: Release };
