import type { Story } from "@ladle/react";
import PermissionCards from "./PermissionCards";

const pending = [
  {
    id: "r1",
    bot_id: "b1",
    tool: "Bash",
    summary: "Bash: rm -rf build && cargo build --release",
    input: '{\n  "command": "rm -rf build && cargo build --release"\n}',
    created_at: "2025-01-15T10:00:00Z",
    expires_at: "2025-01-15T10:10:00Z",
  },
];
const answer = async (): Promise<void> => {};

export const Waiting: Story = () => (
  <div style={{ width: 760 }}>
    <PermissionCards permissions={{ pending, answer }} canAnswer />
  </div>
);

const origin = {
  command: "hermesd board import --dry-run",
  pid: 4242,
  process: "hermesd",
  launched_from: "Terminal",
  cwd: "/Users/me/Developer/gravity",
};

function terminal(over: Partial<typeof origin> & { readonly bot?: string } = {}) {
  const facts = { ...origin, ...over };
  return {
    id: "r2",
    bot_id: "terminal",
    tool: "Terminal command",
    summary: `Terminal command: ${facts.command}`,
    input: JSON.stringify(facts, null, 2),
    created_at: "2025-01-15T10:00:00Z",
    expires_at: "2025-01-15T10:10:00Z",
    origin: facts,
  };
}

/** A CLI owner command asking to act as the owner (H-044 T4, UX-014). */
export const FromTerminal: Story = () => (
  <div style={{ width: 760 }}>
    <PermissionCards
      permissions={{ pending: [terminal()], answer }}
      canAnswer
      botName={() => undefined}
      onOpenBot={() => {}}
    />
  </div>
);

/** Run inside a bot's workspace: the warning, and the card above the bot's own. */
export const FromTerminalInABotWorkspace: Story = () => (
  <div style={{ width: 760 }}>
    <PermissionCards
      permissions={{
        pending: [
          ...pending,
          terminal({
            cwd: "/Users/me/.thehermes/projects/gravity/bots/desktop-dev/workspace",
            launched_from: "claude",
            bot: "Desktop Dev",
          }),
        ],
        answer,
      }}
      canAnswer
      botName={() => "Desktop Dev"}
      onOpenBot={() => {}}
    />
  </div>
);

export const ReadOnly: Story = () => (
  <div style={{ width: 760 }}>
    <PermissionCards permissions={{ pending, answer }} canAnswer={false} />
  </div>
);
