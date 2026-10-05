// The parts of a package the owner reads before ruling (H-018 §4A.2) and
// its rollout after (§4A.4).

import { useState } from "react";
import type { ReactElement } from "react";
import type { Release } from "../../protocol/releases";
import type { BotName, StatusLabel } from "./labels";
import { rolloutLabel, targetMachines, testLabel } from "./labels";

export function Glyph({ label }: { readonly label: StatusLabel }): ReactElement {
  return (
    <span className={`release-tone release-tone-${label.tone}`}>
      <span aria-hidden="true">{label.glyph}</span> {label.word}
    </span>
  );
}

/** Each machine's result against the package's builds, above the tabs. */
export function TestSummary({
  release,
  botName,
}: {
  readonly release: Release;
  readonly botName: BotName;
}): ReactElement {
  const builds = new Map(release.builds.map((b) => [b.sha256, b]));
  return (
    <section className="release-tests" aria-label="Tests">
      <h3>Tests</h3>
      {release.tests.length === 0 ? (
        <p className="release-hint">No computer has reported a result yet.</p>
      ) : (
        release.tests.map((t) => {
          const build = builds.get(t.build_sha256);
          return (
            <div className="release-row" key={t.machine}>
              <b className="release-machine">{t.machine}</b>
              <Glyph label={testLabel(t.result)} />
              <span className="release-meta">
                {botName(t.tester) ?? "Unknown tester"}
                {build ? ` · ${build.platform} ${build.version}` : " · an older build"}
              </span>
            </div>
          );
        })
      )}
    </section>
  );
}

export interface LeftOut {
  readonly verdict: "hold" | "rework";
  readonly note: string;
}

/** One row per item, with Include / Leave out while the owner can rule. */
export function ItemsList({
  release,
  titles,
  leftOut,
  editable,
  onLeaveOut,
  onInclude,
}: {
  readonly release: Release;
  readonly titles: ReadonlyMap<string, string>;
  readonly leftOut: ReadonlyMap<string, LeftOut>;
  readonly editable: boolean;
  readonly onLeaveOut: (itemId: string) => void;
  readonly onInclude: (itemId: string) => void;
}): ReactElement {
  return (
    <ul className="release-items">
      {release.items.map((item) => {
        const out = leftOut.get(item.item_id);
        const decided = item.verdict !== "pending" ? item.verdict : null;
        return (
          <li className="release-row" key={item.item_id}>
            <span className="mono">{item.item_id}</span>
            <span className="release-item-title">{titles.get(item.item_id) ?? ""}</span>
            {decided ? <Outcome verdict={decided} note={item.owner_note} /> : null}
            {editable ? (
              out ? (
                <span className="release-include">
                  <span className="release-tone release-tone-off">
                    <span aria-hidden="true">⤼</span> Left out · {verdictWord(out.verdict)}
                  </span>
                  <button
                    type="button"
                    className="btn btn-small"
                    onClick={() => onInclude(item.item_id)}
                  >
                    Include
                  </button>
                </span>
              ) : (
                <span className="release-include">
                  <span className="release-tone release-tone-ok">
                    <span aria-hidden="true">✓</span> Included
                  </span>
                  <button
                    type="button"
                    className="btn btn-small"
                    onClick={() => onLeaveOut(item.item_id)}
                  >
                    Leave out
                  </button>
                </span>
              )
            ) : null}
          </li>
        );
      })}
    </ul>
  );
}

function verdictWord(verdict: "hold" | "rework"): string {
  return verdict === "hold" ? "Held for the next release" : "Back to Doing";
}

/** An item's ruling, in the words of the choice: Include or Leave out. */
function Outcome({
  verdict,
  note,
}: {
  readonly verdict: "ship" | "hold" | "rework";
  readonly note: string | null;
}): ReactElement {
  if (verdict === "ship") {
    return (
      <span className="release-tone release-tone-ok">
        <span aria-hidden="true">✓</span> Included
      </span>
    );
  }
  const where =
    verdict === "hold" ? "waits for the next package" : `back to Doing${note ? `: “${note}”` : ""}`;
  return (
    <span className="release-tone release-tone-off">
      <span aria-hidden="true">⤼</span> Left out · {where}
    </span>
  );
}

export function Changelog({ release }: { readonly release: Release }): ReactElement {
  return release.changelog.trim() ? (
    <div className="release-changelog">{release.changelog}</div>
  ) : (
    <p className="release-hint">DevOps hasn't written a changelog for this package.</p>
  );
}

/** The steps to try it, with boxes the owner ticks; they never block a ruling. */
export function HowToTest({
  release,
  titles,
}: {
  readonly release: Release;
  readonly titles: ReadonlyMap<string, string>;
}): ReactElement {
  const [tried, setTried] = useState<ReadonlySet<string>>(new Set());
  if (release.how_to_test.length === 0) {
    return <p className="release-hint">DevOps hasn't added test steps for this package.</p>;
  }
  const total = release.how_to_test.reduce((n, entry) => n + entry.steps.length, 0);
  return (
    <div className="release-howto">
      <p className="release-meta">
        {tried.size} of {total} steps tried
      </p>
      {release.how_to_test.map((entry, i) => (
        <section key={`${entry.platform}-${entry.item_id ?? i}`}>
          <h4>
            {entry.platform}
            {entry.item_id ? ` · ${entry.item_id} ${titles.get(entry.item_id) ?? ""}` : ""}
          </h4>
          <ol>
            {entry.steps.map((step, j) => {
              const key = `${i}.${j}`;
              return (
                <li key={key}>
                  <label>
                    <input
                      type="checkbox"
                      checked={tried.has(key)}
                      onChange={() => {
                        const next = new Set(tried);
                        if (!next.delete(key)) {
                          next.add(key);
                        }
                        setTried(next);
                      }}
                    />{" "}
                    {step}
                  </label>
                </li>
              );
            })}
          </ol>
        </section>
      ))}
    </div>
  );
}

/**
 * Every target computer's rollout, live from the deploy records (§4A.4), so
 * the owner sees which are still on the old version.
 */
export function Rollout({ release }: { readonly release: Release }): ReactElement {
  const machines = targetMachines(release);
  if (machines.length === 0) {
    return <p className="release-hint">DevOps hasn't started the rollout yet.</p>;
  }
  return (
    <div role="status" aria-live="polite">
      {machines.map((machine) => (
        <div className="release-row" key={machine}>
          <b className="release-machine">{machine}</b>
          <Glyph label={rolloutLabel(release, machine)} />
        </div>
      ))}
    </div>
  );
}
