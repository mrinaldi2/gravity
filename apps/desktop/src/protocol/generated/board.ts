// Generated from contract/board.schema.json by scripts/contract.mjs. Do not edit;
// change the Rust types in crates/bus/src/contract and regenerate.

/**
 * Where an item's change lands; decides who must verify it.
 *
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "Platform".
 */
export type Platform = "desktop" | "ios" | "daemon" | "infra";
/**
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "Priority".
 */
export type Priority = "P0" | "P1" | "P2" | "P3";
/**
 * L is allowed only in the inbox: it must be split before it is ready.
 *
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "Size".
 */
export type Size = "S" | "M" | "L";
/**
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "ItemType".
 */
export type ItemType = "epic" | "feature" | "bug" | "spike" | "chore";
/**
 * The canonical category a column maps to. Guards and metrics key on it, so
 * renaming or splitting a column never changes the rules.
 *
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "ColumnCategory".
 */
export type ColumnCategory =
  | "inbox"
  | "ready"
  | "doing"
  | "review"
  | "verify"
  | "approval"
  | "deploying"
  | "done"
  | "cancelled";
/**
 * What a column's WIP limit counts.
 *
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "WipScope".
 */
export type WipScope = "column" | "per_assignee";
/**
 * What an item-history event records. Metrics are computed from these.
 *
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "ItemEventKind".
 */
export type ItemEventKind =
  | "created"
  | "moved"
  | "edited"
  | "commented"
  | "linked"
  | "assigned"
  | "blocked"
  | "ranked";
/**
 * A bot's part on one item, beyond the assignee.
 *
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "PersonRole".
 */
export type PersonRole = "reviewer" | "verifier";
/**
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "VerificationResult".
 */
export type VerificationResult = "pass" | "fail" | "blocked";
/**
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "LinkKind".
 */
export type LinkKind =
  | "task"
  | "decision"
  | "artifact"
  | "branch"
  | "pr"
  | "meeting"
  | "item:blocks"
  | "item:relates"
  | "item:duplicates";
/**
 * A bot's role in a project's way of working.
 *
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "Role".
 */
export type Role = "lead" | "coach" | "devops" | "reviewer.arch" | "reviewer.ux" | "tester" | "dev";
/**
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "TemplateKind".
 */
export type TemplateKind = "item_type" | "meeting_type";

/**
 * Every board entity, so one schema (and the generated TypeScript and
 * Swift) names them all. Not a message on the wire.
 */
export interface BoardContract {
  card: ItemCard;
  column: BoardColumn;
  comment: ItemComment;
  event: ItemEvent;
  item: Item;
  link: ItemLink;
  role: ProjectRole;
  settings: BoardSettings;
  template: Template;
  unmet: Unmet;
}
/**
 * The compact form a board column lists; the drawer fetches the [`Item`].
 *
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "ItemCard".
 */
export interface ItemCard {
  ac_checked: number;
  ac_total: number;
  assignee?: string | null;
  blocked: boolean;
  column_key: string;
  id: string;
  labels: string[];
  platforms: Platform[];
  priority: Priority;
  rank: string;
  size?: Size | null;
  stale: boolean;
  title: string;
  type: ItemType;
  version: number;
}
/**
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "BoardColumn".
 */
export interface BoardColumn {
  category: ColumnCategory;
  key: string;
  name: string;
  ord: number;
  project_id: string;
  visible: boolean;
  wip_limit?: number | null;
  wip_scope: WipScope;
}
/**
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "ItemComment".
 */
export interface ItemComment {
  at: string;
  author: string;
  body: string;
  id: string;
  item_id: string;
  reply_to?: string | null;
}
/**
 * One append-only history entry.
 *
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "ItemEvent".
 */
export interface ItemEvent {
  /**
   * The actor as stored: `user`, `bot:<id>` or `device:<id>`.
   */
  actor: string;
  at: string;
  field?: string | null;
  from?: string | null;
  id: number;
  item_id: string;
  kind: ItemEventKind;
  note?: string | null;
  to?: string | null;
}
/**
 * A work item in full, as the item drawer shows it.
 *
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "Item".
 */
export interface Item {
  acceptance_criteria: AcceptanceCriterion[];
  assignee?: string | null;
  blocked?: Blocked | null;
  category: ColumnCategory;
  column_key: string;
  created_at: string;
  created_by: string;
  /**
   * Markdown, filled from the type's template.
   */
  description: string;
  done_at?: string | null;
  /**
   * "H-017"; immutable.
   */
  id: string;
  labels: string[];
  parent_id?: string | null;
  people: ItemPerson[];
  platforms: Platform[];
  priority: Priority;
  /**
   * Fractional-index key: order within the backlog.
   */
  rank: string;
  release_id?: string | null;
  seq: number;
  size?: Size | null;
  state_entered_at: string;
  title: string;
  type: ItemType;
  updated_at: string;
  verifications: ItemVerification[];
  /**
   * Optimistic concurrency: every mutation names the version it read.
   */
  version: number;
}
/**
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "AcceptanceCriterion".
 */
export interface AcceptanceCriterion {
  checked: boolean;
  checked_at?: string | null;
  checked_by?: string | null;
  idx: number;
  /**
   * Where it was checked, for platform criteria.
   */
  machine?: string | null;
  text: string;
}
/**
 * Set while an item is blocked: a flag, not a column.
 *
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "Blocked".
 */
export interface Blocked {
  /**
   * The blocking item, when it is one.
   */
  by?: string | null;
  reason: string;
  since: string;
}
/**
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "ItemPerson".
 */
export interface ItemPerson {
  bot_id: string;
  role: PersonRole;
}
/**
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "ItemVerification".
 */
export interface ItemVerification {
  at: string;
  by: string;
  machine: string;
  note?: string | null;
  result: VerificationResult;
}
/**
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "ItemLink".
 */
export interface ItemLink {
  at: string;
  created_by: string;
  item_id: string;
  kind: LinkKind;
  label?: string | null;
  /**
   * What the link points at: a task id, a branch name, another item id…
   */
  ref: string;
}
/**
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "ProjectRole".
 */
export interface ProjectRole {
  bot_id: string;
  /**
   * The machine a tester verifies on.
   */
  machine?: string | null;
  project_id: string;
  role: Role;
}
/**
 * One project's board configuration.
 *
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "BoardSettings".
 */
export interface BoardSettings {
  /**
   * The daemon that holds the board; linked computers forward to it.
   */
  home_daemon_id: string;
  /**
   * Item id prefix: "H" gives "H-017".
   */
  key: string;
  /**
   * The next item number; ids are never reused.
   */
  next_seq: number;
  project_id: string;
  /**
   * Machines that must verify each platform, e.g. `{"desktop": ["mac", "win-pc"]}`.
   */
  required_machines: {
    daemon?: string[];
    desktop?: string[];
    infra?: string[];
    ios?: string[];
  };
  stale_after_hours: number;
  version: number;
}
/**
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "Template".
 */
export interface Template {
  /**
   * Sections and required fields; its shape is the template's own.
   */
  body: {
    [k: string]: unknown;
  };
  kind: TemplateKind;
  name: string;
  project_id: string;
  version: number;
}
/**
 * A guard a move does not meet yet, with what would fix it. The UI shows these
 * texts as they are and never re-implements a rule.
 *
 * This interface was referenced by `BoardContract`'s JSON-Schema
 * via the `definition` "Unmet".
 */
export interface Unmet {
  code: string;
  fix?: string | null;
  text: string;
}
