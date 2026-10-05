import type { Story } from "@ladle/react";
import type { ReactElement } from "react";
import { defaultColumns } from "../../../test/boardFixtures";
import { DASH_BOTS } from "../../../test/dashboardFixtures";
import { DRAWER_NOW, drawerCheck, itemDetail } from "../../../test/drawerFixtures";
import { FakeDaemon } from "../../../test/fakeDaemon";
import { bot } from "../../../test/fixtures";
import ItemDrawer from "./ItemDrawer";

const BOTS = [...DASH_BOTS, bot({ id: "lead", name: "Team Lead" })];
const noop = (): void => {};

function Drawer(props: { readonly tab: "overview" | "links" | "activity" }): ReactElement {
  const client = new FakeDaemon()
    .onBoard("itemGet", () => ({ case: "item", value: itemDetail() }))
    .onBoard("itemMoveCheck", () => ({ case: "moveCheck", value: drawerCheck() }));
  return (
    <div className="board-view" style={{ height: 720 }}>
      <ItemDrawer
        api={client}
        itemId="H-017"
        columns={defaultColumns()}
        bots={BOTS}
        canComment
        onClose={noop}
        onMove={noop}
        now={() => DRAWER_NOW}
        initialTab={props.tab}
      />
    </div>
  );
}

/** Where H-017 stands, its next step, and its criteria. */
export const Overview: Story = () => <Drawer tab="overview" />;
/** Its tasks, branch, change note and the item it blocks. */
export const Links: Story = () => <Drawer tab="links" />;
/** Comments and moves as one timeline, with the owner's composer. */
export const Activity: Story = () => <Drawer tab="activity" />;
