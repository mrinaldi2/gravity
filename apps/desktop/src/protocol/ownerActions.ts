// Owner actions (H-117 R1/R2): an exact command a bot proposes and only the
// owner runs, with one tap, from the app or the phone. The client must say
// it renders them (hello feature `owner_actions`) to get them at all.

export type OwnerActionState =
  | "proposed"
  | "running"
  | "succeeded"
  | "failed"
  | "timed_out"
  | "rejected"
  | "withdrawn"
  | "expired";

export interface OwnerAction {
  readonly id: string;
  readonly project_id: string;
  /** `bot:<id>`, or `daemon` for a template the daemon filled. */
  readonly proposed_by: string;
  readonly item_id: string | null;
  readonly decision_id: string | null;
  /** The target daemon's id. */
  readonly target_machine: string;
  /** "this computer", or the linked computer's name. */
  readonly target_name?: string;
  readonly shell: string;
  readonly cwd: string;
  /** Verbatim: what runs, exactly. */
  readonly content: string;
  readonly pinned_files: readonly { readonly path: string; readonly sha256: string }[];
  readonly reason: string;
  readonly timeout_s: number;
  /** What the client shows and sends back to run it. */
  readonly sha256: string;
  /** Warnings, e.g. a path a bot can change that isn't pinned. */
  readonly flags: readonly string[];
  readonly state: OwnerActionState;
  readonly created_at: string;
  readonly expires_at: string;
  readonly exit_code: number | null;
  /** The redacted end of its output. */
  readonly output_tail: string | null;
  readonly reject_reason: string | null;
}

export type OwnerActionRequestBody =
  | { readonly type: "owner_action_list"; readonly project_id?: string }
  | { readonly type: "owner_action_get"; readonly id: string }
  /** Needs approve; `sha256` is the hash the card showed. */
  | { readonly type: "owner_action_run"; readonly id: string; readonly sha256: string }
  | { readonly type: "owner_action_reject"; readonly id: string; readonly reason?: string };

export type OwnerActionReply =
  | { readonly type: "owner_actions"; readonly actions: readonly OwnerAction[] }
  | { readonly type: "owner_action"; readonly action: OwnerAction };

/** The hello feature that asks for owner actions. */
export const OWNER_ACTIONS_FEATURE = "owner_actions";
