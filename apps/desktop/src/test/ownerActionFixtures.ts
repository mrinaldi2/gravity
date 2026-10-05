import type { OwnerAction } from "../protocol/ownerActions";

/** A command a bot proposed for the owner, waiting to run. */
export function ownerAction(over: Partial<OwnerAction> = {}): OwnerAction {
  return {
    id: "oa-1",
    project_id: "p1",
    proposed_by: "bot:ops",
    item_id: "H-117",
    decision_id: null,
    target_machine: "d-mac",
    target_name: "this computer",
    shell: "zsh",
    cwd: "/Users/~/Developer/gravity",
    content: "brew services stop colima",
    pinned_files: [],
    reason: "Colima holds the old daemon binary open",
    timeout_s: 600,
    sha256: "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
    flags: [],
    state: "proposed",
    created_at: "2026-10-05T12:00:00Z",
    expires_at: "2026-10-06T12:00:00Z",
    exit_code: null,
    output_tail: null,
    reject_reason: null,
    ...over,
  };
}
