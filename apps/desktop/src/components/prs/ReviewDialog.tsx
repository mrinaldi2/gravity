// Your review of one pull request (UX-051 "review", decision 5): Approve or
// Ask for changes, each saying what it does. Approve is on only for the
// commit you opened it on while that is still the latest; Ask for changes
// needs a note. One PR at a time: there is no bulk approve (UX-016).

import { useEffect, useId, useRef, useState } from "react";
import type { ReactElement } from "react";
import type { PullRequest } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import OverlayShell from "../overlay/OverlayShell";
import { approveBlocked, approveCopy, changesCopy } from "./ownerText";
import { OWNER_ROLE, reviewChip, reviewRoles, sha7 } from "./prText";
import type { OwnerAct } from "./usePrOwner";

type Choice = "approve" | "changes";

/** "Architect ✓ · UX ✓ · CE approved an older commit": where the others stand. */
function others(pr: PullRequest): string {
  return reviewRoles(pr)
    .filter((role) => role !== OWNER_ROLE)
    .map((role) => reviewChip(pr, role).text)
    .join(" · ");
}

export default function ReviewDialog(props: {
  readonly pr: PullRequest;
  /** The commit you opened the review on; Approve names exactly it. */
  readonly sha: string;
  readonly owner: OwnerAct;
  readonly onClose: () => void;
}): ReactElement {
  const { pr, sha, owner, onClose } = props;
  const blocked = approveBlocked(pr, sha);
  const [choice, setChoice] = useState<Choice>(blocked === null ? "approve" : "changes");
  const [note, setNote] = useState("");
  const heading = useRef<HTMLHeadingElement>(null);
  const approveRadio = useRef<HTMLInputElement>(null);
  const noteId = useId();
  const title = `Your review of #${pr.number}`;

  // Opening focuses Approve when it can be used, else the heading (UX-050).
  useEffect(() => {
    (approveRadio.current?.disabled === false ? approveRadio.current : heading.current)?.focus();
  }, []);
  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const needsNote = choice === "changes" && note.trim() === "";
  const off = owner.busy || needsNote || (choice === "approve" && blocked !== null);
  const submit = async (): Promise<void> => {
    const done = await owner.act({
      type: "pr_review_submit",
      project_id: pr.projectId,
      number: pr.number,
      sha,
      verdict: choice === "approve" ? "approved" : "changes_requested",
      ...(note.trim() === "" ? {} : { summary: note.trim() }),
    });
    if (done) {
      onClose();
    }
  };

  return (
    <OverlayShell label={title} onClose={onClose}>
      <form
        className="pr-review-dialog"
        onSubmit={(event) => {
          event.preventDefault();
          if (!off) {
            void submit();
          }
        }}
      >
        <h2 tabIndex={-1} ref={heading}>
          {title}
        </h2>
        <p className="pr-dim">
          {pr.itemId} {pr.itemTitle || pr.title} · latest commit{" "}
          <span className="pr-sha">{sha7(pr.headSha)}</span>
          {others(pr) ? ` · ${others(pr)}` : ""}
        </p>
        <fieldset className="pr-choices">
          <legend className="visually-hidden">Your verdict</legend>
          <label className={`pr-choice ${choice === "approve" ? "pr-choice-on" : ""}`}>
            <input
              ref={approveRadio}
              type="radio"
              name="verdict"
              checked={choice === "approve"}
              disabled={blocked !== null}
              onChange={() => setChoice("approve")}
            />
            <span>
              Approve
              <span className="pr-choice-hint">{blocked ?? approveCopy(pr)}</span>
            </span>
          </label>
          <label className={`pr-choice ${choice === "changes" ? "pr-choice-on" : ""}`}>
            <input
              type="radio"
              name="verdict"
              checked={choice === "changes"}
              onChange={() => setChoice("changes")}
            />
            <span>
              Ask for changes
              <span className="pr-choice-hint">{changesCopy(pr)}</span>
            </span>
          </label>
        </fieldset>
        <label className="pr-note-field" htmlFor={noteId}>
          {choice === "changes" ? "Note (needed for changes)" : "Note (optional for Approve)"}
        </label>
        <textarea
          id={noteId}
          className="pr-note-input"
          rows={3}
          value={note}
          onChange={(event) => setNote(event.target.value)}
        />
        {owner.error ? (
          <p className="pr-tone-bad" role="alert">
            {owner.error}
          </p>
        ) : null}
        <div className="pr-dialog-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn btn-primary" disabled={off}>
            {choice === "approve" ? `Approve #${pr.number}` : `Ask for changes on #${pr.number}`}
          </button>
        </div>
      </form>
    </OverlayShell>
  );
}
