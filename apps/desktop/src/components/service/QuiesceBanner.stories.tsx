import type { Story } from "@ladle/react";
import { QuiesceBanner } from "./QuiesceBanner";

const noop = (): void => {};

/** Every project paused for an install (H-117), as the owner sees it. */
export const Paused: Story = () => (
  <div className="main" style={{ width: 900 }}>
    <QuiesceBanner
      quiesce={{
        id: "q1",
        reason: "install of 0.17.0",
        release_id: "0.17.0",
        started_by: "bot:tester",
        started_at: "2026-10-06T09:00:00Z",
        deadline_at: "2026-10-06T09:30:00Z",
        phase: "paused",
      }}
      canResume
      onResume={noop}
    />
  </div>
);
