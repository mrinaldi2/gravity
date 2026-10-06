// The computers a package must pass on before DevOps can submit it (H-115):
// one per computer, each tested by that computer's own tester. By default
// every tester's computer; the owner can narrow it, or go back to all.

import { useCallback, useState } from "react";
import type { ReactElement } from "react";
import type { AddToast } from "../../app/useToasts";
import { useLoadOnConnect } from "../../hooks/useLoadOnConnect";
import type { DaemonApi } from "../../protocol/api";
import type { ReleaseMachines } from "../../protocol/releases";
import { errText } from "../../util";
import type { BotName } from "./labels";

interface TestedOnProps {
  readonly client: DaemonApi;
  readonly projectId: string;
  readonly connected: boolean;
  /** Choosing the computers is the owner's (the approve grant). */
  readonly canApprove: boolean;
  readonly botName: BotName;
  readonly addToast: AddToast;
}

/** Each computer some tester tests on, with its testers' names. */
function computers(m: ReleaseMachines, botName: BotName): [string, string][] {
  const by = new Map<string, string[]>();
  for (const t of m.testers) {
    by.set(t.machine, [...(by.get(t.machine) ?? []), botName(t.bot_id) ?? "a tester"]);
  }
  const rows = [...by.entries()].map(([machine, names]): [string, string] => [
    machine,
    names.join(", "),
  ]);
  // A fresh array, so sorting it in place mutates nothing shared.
  // oxlint-disable-next-line unicorn/no-array-sort
  return rows.sort(([a], [b]) => a.localeCompare(b));
}

/** What a package needs, and where it goes (ARCH-R55: only your list narrows deploys). */
function hint(m: ReleaseMachines, chosen: boolean): string {
  if (!chosen) {
    return "Each package needs a pass on every tester's computer, and goes to all of them.";
  }
  if (m.set_by === "lead") {
    return "The lead chose where packages are tested; they still go to every tester's computer.";
  }
  return "Each package is tested on, and goes to, the computers you chose.";
}

export default function TestedOn(props: TestedOnProps): ReactElement | null {
  const { client, projectId, addToast } = props;
  const [machines, setMachines] = useState<ReleaseMachines | null>(null);
  const load = useCallback(async (): Promise<void> => {
    try {
      const reply = await client.request(
        { type: "release_machines", project_id: projectId },
        "release_machines",
      );
      setMachines(reply.machines);
    } catch {
      // Older daemons don't know it; the list just isn't shown.
    }
  }, [client, projectId]);
  useLoadOnConnect(props.connected, load);

  const save = async (next: readonly string[]): Promise<void> => {
    try {
      const reply = await client.request(
        { type: "release_machines_set", project_id: projectId, machines: next },
        "release_machines",
      );
      setMachines(reply.machines);
    } catch (error) {
      addToast("error", "Couldn't change the computers", errText(error));
    }
  };

  if (machines === null) {
    return null;
  }
  const all = computers(machines, props.botName);
  const chosen = machines.set.length > 0;
  const toggle = (machine: string): void => {
    const on = machines.required.includes(machine);
    const next = on
      ? machines.required.filter((m) => m !== machine)
      : [...machines.required, machine];
    // None left would hold every package: go back to every computer.
    void save(next.length === all.length ? [] : next);
  };
  return (
    <section className="release-tested-on" aria-labelledby="release-tested-on">
      <h3 id="release-tested-on">Tested on</h3>
      {all.length === 0 ? (
        <p className="release-hint">No tester yet: give a bot the tester role.</p>
      ) : (
        <>
          <p className="release-hint">{hint(machines, chosen)}</p>
          <ul>
            {all.map(([machine, names]) => (
              <li key={machine}>
                <label>
                  <input
                    type="checkbox"
                    checked={machines.required.includes(machine)}
                    disabled={
                      !props.canApprove ||
                      (machines.required.length === 1 && machines.required.includes(machine))
                    }
                    onChange={() => toggle(machine)}
                  />{" "}
                  {machine} <span className="release-meta">· {names}</span>
                </label>
              </li>
            ))}
          </ul>
          {machines.machine_name ? (
            <p className="release-hint">This computer is called {machines.machine_name}.</p>
          ) : null}
          {chosen && props.canApprove ? (
            <button type="button" className="btn btn-small" onClick={() => void save([])}>
              Use every tester's computer
            </button>
          ) : null}
        </>
      )}
    </section>
  );
}
