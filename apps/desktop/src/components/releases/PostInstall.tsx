import type { ReactElement } from "react";
import type { Release } from "../../protocol/releases";
import type { BotName } from "./labels";

/**
 * The packaged items' criteria proven only after install (ARCH-R53 S1): the
 * owner approves them unproven, so the review says which they are and,
 * after the rollout, who ticked them.
 */
export function PostInstall(props: {
  readonly release: Release;
  readonly botName: BotName;
}): ReactElement | null {
  const criteria = props.release.post_install ?? [];
  if (criteria.length === 0) {
    return null;
  }
  return (
    <section className="release-post-install" aria-label="Checked after install">
      <p>
        Checked only once it is installed, so you approve{" "}
        {criteria.length === 1 ? "this one" : "these"} unproven:
      </p>
      <ul>
        {criteria.map((c) => (
          <li key={`${c.item_id}-${c.index}`}>
            <span aria-hidden="true">{c.checked ? "☑ " : "☐ "}</span>
            <span className="visually-hidden">{c.checked ? "Checked: " : "Not checked yet: "}</span>
            <span className="mono">{c.item_id}</span> {c.text}
            {c.checked && c.checked_by
              ? ` · ${props.botName(c.checked_by.replace(/^bot:/, "")) ?? "checked"}`
              : null}
          </li>
        ))}
      </ul>
    </section>
  );
}
