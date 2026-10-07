import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AddToast } from "../../app/useToasts";
import { Platform } from "../../protocol/gen/hermes/board/v1/board_pb";
import { card, snapshot } from "../../test/boardFixtures";
import { FakeDaemon } from "../../test/fakeDaemon";
import { bot, project } from "../../test/fixtures";
import BoardView from "./BoardView";

const desktop = card({ id: "H-024", columnKey: "ready", platforms: [Platform.DESKTOP] });
const ios = card({ id: "H-030", columnKey: "ready", rank: "a5", platforms: [Platform.IOS] });

function renderBoard() {
  const fake = new FakeDaemon();
  fake.onBoard("boardWatch", () => ({ case: "board", value: snapshot([desktop, ios], 1n) }));
  return render(
    <BoardView
      client={fake}
      project={project()}
      bots={[bot({ id: "b1", name: "Desktop Dev" })]}
      connected
      canControl
      addToast={vi.fn<AddToast>()}
    />,
  );
}

// The two tests run in order: the second proves the filter the first saved is
// gone, so one test's board filters cannot hide another test's cards (H-177).
describe("BoardView saved filters", () => {
  it("remembers a filter for the project across a remount", async () => {
    const user = userEvent.setup();
    const { unmount } = renderBoard();
    await screen.findByRole("article", { name: /^H-024/ });
    await user.click(screen.getByRole("button", { name: "ios" }));
    expect(localStorage.getItem("hermes.board.p1")).not.toBeNull();
    unmount();

    renderBoard();
    expect(await screen.findByRole("article", { name: /^H-030/ })).toBeInTheDocument();
    expect(screen.queryByRole("article", { name: /^H-024/ })).toBeNull();
  });

  it("starts the next test with no saved filter", async () => {
    expect(localStorage).toHaveLength(0);
    renderBoard();
    expect(await screen.findByRole("article", { name: /^H-024/ })).toBeInTheDocument();
    expect(screen.getAllByRole("article")).toHaveLength(2);
  });
});
