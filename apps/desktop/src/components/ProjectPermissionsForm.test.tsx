import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { FakeDaemon } from "../test/fakeDaemon";
import * as fx from "../test/fixtures";
import { toastSpy } from "../test/spies";
import BotPermissionExtras from "./bot/BotPermissionExtras";
import ProjectPermissionsForm from "./ProjectPermissionsForm";

function owner(): FakeDaemon {
  const daemon = new FakeDaemon();
  daemon.capabilities = [...daemon.capabilities, "permission_profiles"];
  daemon.grants = ["read", "control", "approve"];
  return daemon;
}

describe("ProjectPermissionsForm", () => {
  it("applies a profile only after the owner confirms the restart", async () => {
    const user = userEvent.setup();
    const daemon = owner().onRequest("set_project_permission_profile", () => ({
      type: "project",
      req_id: "1",
      project: fx.project({ permission_profile: "trusted" }),
    }));
    render(
      <ProjectPermissionsForm
        client={daemon}
        project={fx.project()}
        connected
        onToast={toastSpy()}
      />,
    );
    expect(screen.getByRole("radio", { name: /Standard/ })).toBeChecked();
    await user.click(screen.getByRole("radio", { name: /Trusted/ }));
    await user.click(screen.getByRole("button", { name: "Apply profile" }));
    const dialog = screen.getByRole("dialog", { name: "Switch Acme to Trusted?" });
    expect(within(dialog).getByText(/restarts to pick up the change/)).toBeInTheDocument();
    expect(daemon.requests).toHaveLength(0);
    await user.click(within(dialog).getByRole("button", { name: "Switch to Trusted" }));
    expect(daemon.requests.map((r) => r.body)).toContainEqual({
      type: "set_project_permission_profile",
      project_id: "p1",
      profile: "trusted",
    });
  });

  it("warns plainly before Full", async () => {
    const user = userEvent.setup();
    render(
      <ProjectPermissionsForm
        client={owner()}
        project={fx.project()}
        connected
        onToast={toastSpy()}
      />,
    );
    await user.click(screen.getByRole("radio", { name: /Full/ }));
    await user.click(screen.getByRole("button", { name: "Apply profile" }));
    expect(screen.getByRole("dialog")).toHaveTextContent(
      /container, a VM or a separate user account/,
    );
  });

  it("lets a connection without approve see the profile but not change it", () => {
    const daemon = owner();
    daemon.grants = ["read", "control"];
    render(
      <ProjectPermissionsForm
        client={daemon}
        project={fx.project({ permission_profile: "trusted" })}
        connected
        onToast={toastSpy()}
      />,
    );
    expect(screen.getByRole("radio", { name: /Trusted/ })).toBeChecked();
    expect(screen.getByRole("radio", { name: /Full/ })).toBeDisabled();
    expect(screen.getByText(/Only the owner can change this/)).toBeInTheDocument();
  });

  it("is hidden from daemons without profiles", () => {
    const { container } = render(
      <ProjectPermissionsForm
        client={new FakeDaemon()}
        project={fx.project()}
        connected
        onToast={toastSpy()}
      />,
    );
    expect(container).toBeEmptyDOMElement();
  });
});

describe("BotPermissionExtras", () => {
  it("adds an extra to the ones the bot already has", async () => {
    const user = userEvent.setup();
    const daemon = owner().onRequest("set_bot_permission_extras", () => ({
      type: "bot",
      req_id: "1",
      bot: fx.bot({ permission_extras: ["publish", "daemon_restart"] }),
    }));
    render(
      <BotPermissionExtras
        client={daemon}
        bot={fx.bot({ permission_extras: ["publish"] })}
        connected
        onBotUpdated={() => {}}
        onToast={toastSpy()}
      />,
    );
    expect(screen.getByRole("checkbox", { name: /Publish builds/ })).toBeChecked();
    await user.click(screen.getByRole("checkbox", { name: /Restart the Hermes service/ }));
    expect(daemon.requests.map((r) => r.body)).toContainEqual({
      type: "set_bot_permission_extras",
      bot_id: "b1",
      extras: ["publish", "daemon_restart"],
    });
  });
});
