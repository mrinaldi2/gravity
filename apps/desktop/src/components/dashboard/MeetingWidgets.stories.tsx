import type { Story } from "@ladle/react";
import { DASH_BOTS } from "../../test/dashboardFixtures";
import { ACTION_ITEMS, MEETING_ROWS } from "../../test/meetingFixtures";
import { ActionItemsWidget, MeetingsWidget } from "./MeetingWidgets";

const noop = (): void => {};
const botName = (id: string): string => DASH_BOTS.find((b) => b.id === id)?.name ?? "a bot";

/** Widgets 5 and 6 (H-102): a series held and one collecting; actions with
 *  one overdue, one the owner's and one promoted. */
export const Filled: Story = () => (
  <div className="dash" style={{ width: 900 }}>
    <div className="dash-grid">
      <MeetingsWidget rows={MEETING_ROWS} leadName="Team Lead" offHome={null} />
      <ActionItemsWidget
        actions={ACTION_ITEMS}
        botName={botName}
        canControl
        offHome={null}
        onDone={noop}
        onDrop={noop}
        onPromote={noop}
        onItem={noop}
      />
    </div>
  </div>
);
