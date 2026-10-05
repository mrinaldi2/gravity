import type { Story } from "@ladle/react";
import type { ReactElement } from "react";
import type { OwnerAction } from "../../protocol/ownerActions";
import { ownerAction } from "../../test/ownerActionFixtures";
import OwnerActionCard from "./OwnerActionCard";
import OwnerActionConfirm from "./OwnerActionConfirm";

const noop = (): void => undefined;

function Frame(props: { readonly children: ReactElement }): ReactElement {
  return (
    <div className="main" style={{ width: 440, padding: 16 }}>
      {props.children}
    </div>
  );
}

function Card(props: { readonly action: OwnerAction; readonly canRun?: boolean }): ReactElement {
  return (
    <Frame>
      <OwnerActionCard
        action={props.action}
        proposer="DevOps"
        canRun={props.canRun ?? true}
        onRun={noop}
        onReject={noop}
      />
    </Frame>
  );
}

export const Waiting: Story = () => (
  <Card
    action={ownerAction({
      flags: [
        "/Users/~/Developer/gravity/scripts/stop.sh can be changed by a bot and isn't pinned",
      ],
    })}
  />
);

export const ReadOnly: Story = () => <Card action={ownerAction()} canRun={false} />;

export const Failed: Story = () => (
  <Card
    action={ownerAction({
      state: "failed",
      exit_code: 1,
      output_tail: "Error: colima is not running\n",
    })}
  />
);

export const Confirm: Story = () => (
  <Frame>
    <OwnerActionConfirm
      action={ownerAction({
        target_name: "win-pc",
        shell: "powershell",
        content: "Stop-Service docker\nGet-Process com.docker.* | Stop-Process",
      })}
      onRun={noop}
      onCancel={noop}
      hold={false}
    />
  </Frame>
);
