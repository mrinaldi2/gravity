// Release packages over the JSON WebSocket (H-020 §2, §6; docs/protocol.md
// "Releases and the deploy gate"). The owner reads them, rules on them, holds
// them and pauses their rollout; DevOps and testers use MCP tools instead.

export type ReleaseStatus =
  | "planned"
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
  | "rolled_back";

/**
 * Something that happened to a package, kept after the package itself may
 * be gone: today, a successor DevOps cancelled before submitting it. It is
 * listed on the package it would have replaced.
 */
export interface ReleaseEvent {
  readonly release_id: string;
  readonly release_name: string;
  /** The package the subject succeeded, when it did. */
  readonly related_id: string | null;
  readonly kind: string;
  /** A bot id, `owner` or `device:<id>`. */
  readonly actor: string;
  readonly note: string | null;
  /** What the kind carries; `lead_ticked`: the item and criterion. */
  readonly detail?: {
    readonly item_id?: string;
    readonly text?: string;
    readonly passed?: boolean;
    /** `planned`: its items; `items_changed`: what was added and taken out. */
    readonly items?: readonly string[];
    readonly added?: readonly string[];
    readonly removed?: readonly string[];
  };
  readonly at: string;
}

/** A criterion of a packaged item proven only after install (H-116). */
interface PostInstallCriterion {
  readonly item_id: string;
  readonly index: number;
  readonly text: string;
  readonly checked: boolean;
  readonly checked_by: string | null;
}

type Verdict = "pending" | "ship" | "hold" | "rework";

interface ReleaseItem {
  readonly item_id: string;
  readonly verdict: Verdict;
  readonly owner_note: string | null;
}

interface ReleaseBuild {
  readonly platform: string;
  readonly version: string;
  readonly artifact: string;
  readonly url: string | null;
  readonly install_url: string | null;
  readonly sha256: string;
  readonly built_at: string;
  /** The git commit it was built from, when recorded (ARCH-R52 M1). */
  readonly source_commit?: string | null;
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
interface HowToTest {
  readonly item_id?: string | null;
  readonly platform: string;
  readonly steps: readonly string[];
}

/** Where one of a package's items stands on the board now (H-137). */
export interface PlanItem {
  readonly item_id: string;
  readonly title: string;
  readonly column_key: string;
  /** The column's category: `doing`, `verify`, `done`… */
  readonly category: string;
  /** The assignee's bot id. */
  readonly assignee: string | null;
  readonly blocked: boolean;
  readonly ac_checked: number;
  readonly ac_total: number;
  /** In Verify or past it. */
  readonly ready: boolean;
}

/** How far a package is (H-137). */
interface Readiness {
  readonly items_total: number;
  readonly items_ready: number;
  /** The platforms with a build attached. */
  readonly builds: readonly string[];
  /** The computers it must pass on, and those that passed. */
  readonly tests_required: readonly string[];
  readonly tests_passed: readonly string[];
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
  /** Its own events and those of packages that would have succeeded it. */
  readonly events: readonly ReleaseEvent[];
  /** Its items' post-install criteria: what the owner approves unproven. */
  readonly post_install?: readonly PostInstallCriterion[];
  /** The computers it was tested on, frozen at submit (ARCH-R55); empty before. */
  readonly tested_on?: readonly string[];
  /** `owner` or `lead` when they narrowed it; null: every tester's computer. */
  readonly tested_set_by?: string | null;
  /** The computers it must reach before it counts as deployed. */
  readonly deploys_to?: readonly string[];
  /** `owner` when the owner narrowed it; null: every tester's computer. */
  readonly deploys_set_by?: string | null;
  /** Each item's live status, from daemons with planned releases (H-137). */
  readonly plan?: readonly PlanItem[];
  readonly readiness?: Readiness;
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
  | { readonly type: "release_resume"; readonly release_id: string }
  | { readonly type: "release_machines"; readonly project_id: string }
  | {
      readonly type: "release_machines_set";
      readonly project_id: string;
      /** Empty: every tester's computer again. The owner's list narrows deploys too. */
      readonly machines?: readonly string[];
      /** This computer's name for its testers. */
      readonly machine_name?: string;
    };

/** The computers a package must pass on before it is submitted (H-115). */
export interface ReleaseMachines {
  readonly project_id: string;
  /** This computer's name for its testers (ARCH-R55). */
  readonly machine_name?: string;
  /** What submit waits for: `set`, or every tester's computer. */
  readonly required: readonly string[];
  /** Where a package goes: the owner's list, or every tester's computer. */
  readonly deploys_to?: readonly string[];
  /** What the owner or lead chose; empty means every tester's computer. */
  readonly set: readonly string[];
  /** Who chose it: `owner` or `lead`. */
  readonly set_by?: string | null;
  /** Each tester and the computer it tests on. */
  readonly testers: readonly { readonly bot_id: string; readonly machine: string }[];
}

export type ReleaseReply =
  | { readonly type: "releases"; readonly releases: readonly Release[] }
  | { readonly type: "release"; readonly release: Release }
  | { readonly type: "release_machines"; readonly machines: ReleaseMachines };
