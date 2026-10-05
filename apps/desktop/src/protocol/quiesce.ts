// The pause for an install (H-117): every project on the daemon's computer
// held still while the daemon is replaced. The app shows it as a banner,
// and the owner can end it early.

export interface Quiesce {
  readonly id: string;
  readonly reason: string;
  readonly release_id: string | null;
  /** `bot:<id>` or the owner who asked for it. */
  readonly started_by: string;
  readonly started_at: string;
  /** When it resumes by itself if the install never finishes. */
  readonly deadline_at: string;
  readonly phase: string;
  /** What the pause did so far; fields appear as it goes. */
  readonly report?: {
    /** `[[quiesce.service]]` changed since the daemon started: the list it
     *  started with was used, the new one never runs unseen (ARCH-R49). */
    readonly services_changed?: boolean;
  };
}

export type QuiesceRequestBody =
  | { readonly type: "quiesce_status" }
  /** Resumes every project now; needs approve. */
  | { readonly type: "quiesce_resume" };

export type QuiesceReply = { readonly type: "quiesce"; readonly quiesce: Quiesce | null };
