import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { PermissionRequest, TerminalOrigin } from "../../protocol/chat";
import PermissionCards from "./PermissionCards";
import { originLine, permissionDetail, permissionTitle } from "./permissionTitle";
import type { Permissions } from "./usePermissions";

const COMMAND = "hermesd board import --dry-run";
const BODY =
  "If you allow it, this one command runs with your owner rights: it can do anything you can " +
  "do in The Hermes, including approving releases and deleting bots. Bots can run terminal " +
  "commands too. Allow it only if you just ran it yourself.";

function terminal(over: Partial<TerminalOrigin> = {}): PermissionRequest {
  const origin: TerminalOrigin = {
    command: COMMAND,
    pid: 4242,
    process: "hermesd",
    launched_from: "Terminal",
    cwd: "/Users/me/Developer/gravity",
    ...over,
  };
  return {
    id: "t1",
    bot_id: "terminal",
    tool: "Terminal command",
    summary: `Terminal command: ${COMMAND}`,
    input: JSON.stringify(origin),
    created_at: "2025-01-15T10:00:00Z",
    expires_at: "2025-01-15T10:10:00Z",
    origin,
  };
}

const bash: PermissionRequest = {
  id: "b1",
  bot_id: "bot-1",
  tool: "Bash",
  summary: "Bash: rm -rf build",
  input: "{}",
  created_at: "2025-01-15T09:00:00Z",
  expires_at: "2025-01-15T09:10:00Z",
};

type Answer = Permissions["answer"];

function show(
  pending: readonly PermissionRequest[],
  answer = vi.fn<Answer>(async () => {}),
): ReturnType<typeof vi.fn<Answer>> {
  render(
    <PermissionCards
      permissions={{ pending, answer }}
      canAnswer
      botName={() => "Desktop Dev"}
      onOpenBot={() => {}}
    />,
  );
  return answer;
}

describe("TerminalCard (UX-014)", () => {
  it("says what the command is, where it came from and what allowing it means", () => {
    show([terminal()]);
    const card = screen.getByRole("region", { name: "A terminal command wants to act as you" });
    const inCard = within(card);
    expect(inCard.getByText(COMMAND)).toHaveClass("permission-summary");
    expect(
      inCard.getByText("From hermesd in Terminal · in ~/Developer/gravity · process 4242"),
    ).toBeInTheDocument();
    expect(inCard.getByText(BODY)).toBeInTheDocument();
    expect(inCard.getByText(/^Denied automatically at .+ if you don't answer\.$/)).toBeVisible();
    expect(inCard.queryByText(/⚠/)).toBeNull();
    expect(inCard.queryByText(`Terminal command: ${COMMAND}`)).toBeNull();
    expect(card.querySelector("svg")).not.toBeNull();
    expect(card.querySelector("kbd")).toBeNull();
    expect(
      inCard.getByRole("button", { name: `Allow this command: ${COMMAND}` }),
    ).toHaveTextContent(/^Allow this command$/);
    expect(inCard.getByRole("button", { name: `Deny this command: ${COMMAND}` })).toHaveTextContent(
      /^Deny$/,
    );
    expect(inCard.queryByRole("button", { name: /Allow for session|Allow once|^Open/ })).toBeNull();
  });

  it("warns when the command started in a bot's workspace, and sits above the bots' cards", () => {
    show([bash, terminal({ bot: "Desktop Dev" })]);
    const warning = screen.getByText(
      "⚠ It was started inside Desktop Dev's workspace, so a bot is probably asking, not you.",
    );
    const terminalCard = warning.closest("section");
    const botCard = screen.getByRole("group", { name: "Bash: rm -rf build" });
    expect(terminalCard).not.toBeNull();
    expect(
      (terminalCard?.compareDocumentPosition(botCard) ?? 0) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(screen.getAllByRole("button", { name: /^(Allow|Deny)/ })[0]).toHaveAccessibleName(
      `Allow this command: ${COMMAND}`,
    );
  });

  it("takes no initial focus and no single-key answers", async () => {
    const answer = show([terminal()]);
    expect(document.activeElement).toBe(document.body);
    const allow = screen.getByRole("button", { name: /^Allow this command/ });
    allow.focus();
    await userEvent.keyboard("ads");
    expect(answer).not.toHaveBeenCalled();
    expect(screen.queryByRole("textbox")).toBeNull();
  });

  it("denies at once, with no reason to give", async () => {
    const answer = show([terminal()]);
    await userEvent.click(screen.getByRole("button", { name: /^Deny this command/ }));
    expect(answer).toHaveBeenCalledWith("t1", "deny");
    expect(screen.queryByRole("textbox")).toBeNull();
  });

  it("allows once", async () => {
    const answer = show([terminal()]);
    await userEvent.click(screen.getByRole("button", { name: /^Allow this command/ }));
    expect(answer).toHaveBeenCalledWith("t1", "allow_once");
  });

  it("announces the title with the command", () => {
    show([terminal()]);
    expect(document.querySelector('[aria-live="polite"]')?.textContent).toBe(
      `A terminal command wants to act as you: ${COMMAND}`,
    );
  });

  it("falls back on the origin line when the OS told less", () => {
    const base = { command: COMMAND, pid: 4242 };
    expect(originLine({ ...base, process: "hermesd" })).toBe("From hermesd · process 4242");
    expect(originLine(base)).toBe(
      "From process 4242. The Hermes couldn't tell which app started it.",
    );
    expect(originLine({ ...base, process: "hermesd", cwd: "C:\\Users\\me\\src" })).toBe(
      "From hermesd · in ~\\src · process 4242",
    );
  });

  it("titles the toast and names the bot's workspace in its body", () => {
    const request = terminal({ bot: "Desktop Dev" });
    expect(permissionTitle(request, undefined)).toBe("A terminal command wants to act as you");
    expect(permissionDetail(terminal())).toBe(`${COMMAND} · open The Hermes to allow or deny it.`);
    expect(permissionDetail(request)).toBe(
      `Started in Desktop Dev's workspace. ${COMMAND} · open The Hermes to allow or deny it.`,
    );
    expect(permissionDetail(bash)).toBe("Bash: rm -rf build");
  });
});
