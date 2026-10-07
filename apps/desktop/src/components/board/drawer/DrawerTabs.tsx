// The item drawer's Overview (description, acceptance criteria,
// verification, people, parent) and Links grouped by kind (H-018 §3.6).
// Activity lives in DrawerActivity.

import type { ReactElement } from "react";
import type { ItemLink } from "../../../protocol/gen/hermes/board/v1/board_pb";
import {
  LinkKind,
  PersonRole,
  VerificationResult,
} from "../../../protocol/gen/hermes/board/v1/board_pb";
import type { ItemDetail } from "../../../protocol/gen/hermes/board/v1/requests_pb";
import Markdown from "../../control/Markdown";
import { when } from "./drawerText";

const VERIFIED: Readonly<Record<VerificationResult, string>> = {
  [VerificationResult.UNSPECIFIED]: "◌",
  [VerificationResult.PASS]: "✓",
  [VerificationResult.FAIL]: "✗",
  [VerificationResult.BLOCKED]: "!",
};

export function Overview(props: {
  readonly detail: ItemDetail;
  readonly who: (actor: string) => string;
}): ReactElement {
  const item = props.detail.item;
  if (item === undefined) {
    return <p className="drawer-empty">This item has no details.</p>;
  }
  const ac = item.acceptanceCriteria;
  const reviewers = item.people.filter((p) => p.role === PersonRole.REVIEWER);
  const verifiers = item.people.filter((p) => p.role === PersonRole.VERIFIER);
  return (
    <div className="drawer-overview">
      {item.description.trim() ? (
        <Markdown>{item.description}</Markdown>
      ) : (
        <p className="drawer-empty">No description.</p>
      )}
      <h4>
        Acceptance criteria {ac.length ? `${ac.filter((c) => c.checked).length}/${ac.length}` : ""}
      </h4>
      {ac.length === 0 ? (
        <p className="drawer-empty">None yet.</p>
      ) : (
        <ul className="drawer-ac">
          {ac.map((c) => (
            <li key={c.idx}>
              <span aria-hidden="true">{c.checked ? "☑" : "☐"}</span>
              <span className="visually-hidden">{c.checked ? "Checked: " : "Not checked: "}</span>
              <span>{c.text}</span>
              {c.checked ? (
                <span className="drawer-dim">
                  {[c.checkedBy ? props.who(c.checkedBy) : "", c.machine, when(c.checkedAt)]
                    .filter(Boolean)
                    .join(" · ")}
                </span>
              ) : c.postInstall ? (
                <span className="drawer-dim">checked after install</span>
              ) : null}
            </li>
          ))}
        </ul>
      )}
      {item.verifications.length ? (
        <p className="drawer-line">
          <b>Verification</b>{" "}
          {item.verifications.map((v) => `${v.machine} ${VERIFIED[v.result]}`).join("  ")}
        </p>
      ) : null}
      <p className="drawer-line">
        <b>People</b> Assignee {item.assignee ? props.who(`bot:${item.assignee}`) : "none"}
        {reviewers.length
          ? ` · Reviewers ${reviewers.map((p) => props.who(`bot:${p.botId}`)).join(", ")}`
          : ""}
        {verifiers.length
          ? ` · Verifiers ${verifiers.map((p) => props.who(`bot:${p.botId}`)).join(", ")}`
          : ""}
      </p>
      {item.parentId ? (
        <p className="drawer-line">
          <b>Parent</b> <span className="mono">{item.parentId}</span>
        </p>
      ) : null}
    </div>
  );
}

const LINK_GROUPS: readonly { readonly title: string; readonly kinds: readonly LinkKind[] }[] = [
  { title: "Tasks", kinds: [LinkKind.TASK] },
  { title: "Decisions", kinds: [LinkKind.DECISION] },
  { title: "Artifacts", kinds: [LinkKind.ARTIFACT] },
  { title: "Branches", kinds: [LinkKind.BRANCH, LinkKind.PR] },
  { title: "Meetings", kinds: [LinkKind.MEETING] },
  {
    title: "Items",
    kinds: [LinkKind.ITEM_BLOCKS, LinkKind.ITEM_RELATES, LinkKind.ITEM_DUPLICATES],
  },
];

function linkText(link: ItemLink): string {
  const prefix =
    link.kind === LinkKind.ITEM_BLOCKS
      ? "blocks "
      : link.kind === LinkKind.ITEM_RELATES
        ? "relates "
        : link.kind === LinkKind.ITEM_DUPLICATES
          ? "duplicates "
          : link.kind === LinkKind.PR
            ? "PR "
            : "";
  const shown =
    link.kind === LinkKind.TASK || link.kind === LinkKind.DECISION
      ? link.ref.slice(0, 8)
      : link.ref;
  return `${prefix}${shown}${link.label ? ` · ${link.label}` : ""}`;
}

export function Links(props: { readonly links: readonly ItemLink[] }): ReactElement {
  if (props.links.length === 0) {
    return <p className="drawer-empty">Nothing linked yet.</p>;
  }
  return (
    <dl className="drawer-links">
      {LINK_GROUPS.map((group) => {
        const links = props.links.filter((l) => group.kinds.includes(l.kind));
        if (links.length === 0) {
          return null;
        }
        return (
          <div key={group.title} className="drawer-link-group">
            <dt>{group.title}</dt>
            {links.map((l) => (
              <dd key={`${l.kind}-${l.ref}`} title={l.ref}>
                {linkText(l)}
              </dd>
            ))}
          </div>
        );
      })}
    </dl>
  );
}
