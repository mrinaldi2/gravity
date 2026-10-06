import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { FakeDaemon } from "../../test/fakeDaemon";
import * as fx from "../../test/fixtures";
import BotPermissionExtras from "./BotPermissionExtras";

function setup(grants: readonly unknown[]) {
  const client = new FakeDaemon();
  client.capabilities = [...client.capabilities, "permission_profiles"];
  client.grants = ["read", "control", "approve"];
  client.onRequest("bot_grants", () => ({ type: "bot_grants", req_id: "1", grants }) as never);
  render(
    <BotPermissionExtras
      client={client}
      bot={fx.bot({ id: "tw", name: "Tester Win" })}
      connected
      onBotUpdated={vi.fn<() => void>()}
      onToast={vi.fn<() => void>()}
    />,
  );
  return client;
}

describe("BotPermissionExtras", () => {
  it("offers the Windows installer build as its own extra", () => {
    setup([]);
    expect(
      screen.getByRole("checkbox", { name: /Build the Windows installer/ }),
    ).toBeInTheDocument();
  });

  it("lists what rulings on linked computers granted the bot", async () => {
    setup([
      {
        at: "2026-10-06T10:02:00Z",
        from: "mac",
        extras: ["install", "quiesce"],
        decision: "Let the bots install releases?",
      },
    ]);
    const list = await screen.findByRole("list", {
      name: "Granted by rulings on linked computers",
    });
    expect(list).toHaveTextContent(
      "Granted from mac by a ruling: Install builds, Pause all projects for an install",
    );
    expect(list).toHaveTextContent("Let the bots install releases?");
  });
});
