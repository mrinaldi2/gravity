// A PR's checks (UX-051 "checks", decision 6): always "at <commit>", with the
// computer each ran on. Checks on earlier commits fold away.

import type { ReactElement } from "react";
import type { CheckRun, PullRequest } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { checkResult, checksSummary, checkWhere, splitChecks } from "./checkText";
import { sha7 } from "./prText";

function CheckRow({ check }: { readonly check: CheckRun }): ReactElement {
  const result = checkResult(check);
  const notes = [
    check.required ? "" : "optional",
    check.treeOf ? `same tree as ${sha7(check.treeOf)}` : "",
  ].filter(Boolean);
  return (
    <li className="pr-check">
      <span className={`pr-check-result pr-tone-${result.tone}`}>
        {result.glyph} {result.word}
      </span>
      <span className="pr-check-name">
        {check.name}
        {notes.length > 0 ? <span className="pr-dim"> · {notes.join(" · ")}</span> : null}
      </span>
      <span className="pr-dim">{checkWhere(check)}</span>
    </li>
  );
}

/** Earlier commits, one folded group each. */
function Earlier({ checks }: { readonly checks: readonly CheckRun[] }): ReactElement | null {
  const shas = [...new Set(checks.map((c) => c.sha))];
  if (shas.length === 0) {
    return null;
  }
  return (
    <section className="pr-box" aria-label="Earlier commits">
      <h4>Earlier commits</h4>
      {shas.map((sha) => {
        const runs = checks.filter((c) => c.sha === sha);
        const summary = checksSummary(runs);
        return (
          <details key={sha} className="pr-earlier">
            <summary>
              <span className="pr-sha">{sha7(sha)}</span>{" "}
              <span className={`pr-tone-${summary.tone}`}>{summary.text}</span>
            </summary>
            <ul className="pr-plain">
              {runs.map((check) => (
                <CheckRow key={check.name} check={check} />
              ))}
            </ul>
          </details>
        );
      })}
    </section>
  );
}

export default function PrChecks(props: {
  readonly pr: PullRequest;
  /** The overview's box: the latest commit only. */
  readonly compact?: boolean;
}): ReactElement {
  const { pr } = props;
  const { head, earlier } = splitChecks(pr);
  const summary = checksSummary(head);
  return (
    <>
      <section className="pr-box" aria-label="Checks">
        <h4>
          Checks · at <span className="pr-sha">{sha7(pr.headSha)}</span>
          {props.compact ? null : " · latest"} ·{" "}
          <span className={`pr-tone-${summary.tone}`}>{summary.text}</span>
        </h4>
        {head.length === 0 ? (
          <p className="pr-dim">No checks have run on this commit yet.</p>
        ) : (
          <ul className="pr-plain">
            {head.map((check) => (
              <CheckRow key={check.name} check={check} />
            ))}
          </ul>
        )}
      </section>
      {props.compact ? null : <Earlier checks={earlier} />}
    </>
  );
}
