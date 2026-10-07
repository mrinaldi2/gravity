import type { Story } from "@ladle/react";
import frame from "../../test/browserFrame.jpg?inline";
import { browserActivity, browserTabs } from "../../test/agentFixtures";
import { bot } from "../../test/fixtures";
import BrowserActivityList from "./BrowserActivityList";
import { BrowserScreen, ControlBar, TabStrip } from "./BrowserPane";

const FRAME = {
  type: "browser_frame",
  bot_id: "b1",
  tab_id: "tab-1",
  data: frame.slice(frame.indexOf(",") + 1),
  width: 640,
  height: 400,
} as const;

export const Live: Story = () => (
  <div className="browser-pane" style={{ width: 1024, height: 640 }}>
    <div className="browser-main">
      {/* The pane's own bar, so the story keeps its layout (H-174). */}
      <ControlBar
        bot={bot({ name: "lead" })}
        offered={false}
        controlling={false}
        onToggle={() => undefined}
      />
      <TabStrip tabs={browserTabs} onPick={() => undefined} />
      <BrowserScreen tabs={browserTabs} frame={FRAME} name="lead" />
    </div>
    <BrowserActivityList activity={browserActivity} error={null} />
  </div>
);

export const Closed: Story = () => (
  <div className="browser-pane" style={{ width: 1024, height: 400 }}>
    <div className="browser-main">
      <BrowserScreen
        tabs={{ ...browserTabs, open: false, tabs: [], active: null }}
        frame={null}
        name="lead"
      />
    </div>
    <BrowserActivityList activity={[]} error={null} />
  </div>
);
