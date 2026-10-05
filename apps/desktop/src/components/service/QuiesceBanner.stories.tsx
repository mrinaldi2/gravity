import type { Story } from "@ladle/react";
import type { Quiesce } from "../../protocol/quiesce";
import { QuiesceBanner } from "./QuiesceBanner";

const noop = (): void => {};

const PAUSED: Quiesce = {
  id: "q1",
  reason: "install of 0.17.0",
  release_id: "0.17.0",
  version: "0.17.0",
  started_by: "bot:tester",
  started_at: "2026-10-06T09:00:00Z",
  deadline_at: "2026-10-06T09:30:00Z",
  phase: "paused",
};

/** Every project paused for an install (H-117), as the owner sees it. */
export const Paused: Story = () => (
  <div className="main" style={{ width: 900 }}>
    <QuiesceBanner quiesce={PAUSED} canResume onResume={noop} />
  </div>
);

/** The install waiting on programs that still hold the Hermes folder. */
export const Blocked: Story = () => (
  <div className="main" style={{ width: 900 }}>
    <QuiesceBanner
      quiesce={{
        ...PAUSED,
        phase: "blocked",
        report: {
          unresolved: [
            { pid: 4120, command: "node", bot_name: "Unity", project_name: "PhD" },
            { pid: 77, command: "sleep" },
          ],
        },
      }}
      canResume
      onResume={noop}
    />
  </div>
);
