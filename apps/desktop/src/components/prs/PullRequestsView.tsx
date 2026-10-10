// The project's Pull requests tab (H-276, UX-051): the list, and one PR in
// full. Read-only here; Approve and Ask for changes are H-277's. Both update
// live from `pr_updated` and `check_updated` pushes, with no reload. Back
// returns focus to the row that opened the PR (UX-012).

import { useEffect, useRef, useState } from "react";
import type { ReactElement } from "react";
import type { Project } from "../../protocol/entities";
import type { PrApi } from "../../protocol/prs";
import PrDetail from "./PrDetail";
import type { PrFilter } from "./PrList";
import PrList, { inFilter } from "./PrList";
import type { PullRequest } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import type { Loaded } from "./usePullRequests";
import { usePr, usePrList } from "./usePullRequests";

interface PullRequestsViewProps {
  readonly client: PrApi;
  readonly project: Project;
  readonly connected: boolean;
  readonly now: number;
  /** Opens on this PR, for stories and links. */
  readonly initialNumber?: number;
}

function Note({ text }: { readonly text: string }): ReactElement {
  return (
    <div className="empty-pane">
      <div className="empty-state">
        <p>{text}</p>
      </div>
    </div>
  );
}

/** What shows instead of the list: loading, offline, an error, or no PRs yet. */
function listNote(list: Loaded<PullRequest[]>, connected: boolean): string | null {
  if (list.data === null) {
    return list.error ?? (connected ? "Loading pull requests…" : "Offline");
  }
  return list.data.length === 0
    ? "No pull requests yet. A bot opens one when its card's change is ready."
    : null;
}

function OpenPr(
  props: PullRequestsViewProps & {
    readonly number: number;
    readonly onBack: () => void;
  },
): ReactElement {
  const { client, project, connected } = props;
  const one = usePr(client, project.id, props.number, connected);
  if (one.data === null) {
    return <Note text={one.error ?? `Loading #${props.number}…`} />;
  }
  return (
    <PrDetail
      client={client}
      pr={one.data}
      connected={connected}
      now={props.now}
      projectName={project.name}
      onBack={props.onBack}
    />
  );
}

export default function PullRequestsView(props: PullRequestsViewProps): ReactElement {
  const { client, project, connected, now } = props;
  const [open, setOpen] = useState<number | null>(props.initialNumber ?? null);
  const [chosen, setChosen] = useState<PrFilter | null>(null);
  const list = usePrList(client, project.id, connected);
  const backTo = useRef<number | null>(null);
  useEffect(() => {
    if (open === null && backTo.current !== null) {
      document.querySelector<HTMLElement>(`.pr-row-open[data-pr="${backTo.current}"]`)?.focus();
      backTo.current = null;
    }
  }, [open]);

  if (open !== null) {
    const back = (): void => {
      backTo.current = open;
      setOpen(null);
    };
    return <OpenPr {...props} number={open} onBack={back} />;
  }
  const note = listNote(list, connected);
  if (note !== null || list.data === null) {
    return <Note text={note ?? ""} />;
  }
  // "Waiting for you" is the default whenever anything waits for you (UX-051 list).
  const filter = chosen ?? (list.data.some((pr) => inFilter(pr, "you")) ? "you" : "open");
  return <PrList prs={list.data} filter={filter} now={now} onFilter={setChosen} onOpen={setOpen} />;
}
