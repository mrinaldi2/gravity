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

/** A CLI owner command asking to act as the owner (H-044 T4). */
export const FromTerminal: Story = () => (
  <div style={{ width: 760 }}>
    <PermissionCards
      permissions={{
        pending: [
          {
            id: "r2",
            bot_id: "terminal",
            tool: "Allow from Terminal",
            summary: "Allow from Terminal: hermesd board import --dry-run (pid 4242)",
            input: '{\n  "command": "hermesd board import --dry-run (pid 4242)"\n}',
            created_at: "2025-01-15T10:00:00Z",
            expires_at: "2025-01-15T10:10:00Z",
          },
        ],
        answer,
      }}
      canAnswer
      botName={() => undefined}
      onOpenBot={() => {}}
    />
  </div>
);

export const ReadOnly: Story = () => (
  <div style={{ width: 760 }}>
    <PermissionCards permissions={{ pending, answer }} canAnswer={false} />
  </div>
);
