import { toJson } from "@bufbuild/protobuf";
import { act, fireEvent, render, screen } from "@testing-library/react";
import type { ReactElement } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { card } from "../../test/boardFixtures";
import { itemDetail } from "../../test/drawerFixtures";
import { FakeDaemon } from "../../test/fakeDaemon";
import { bot, project } from "../../test/fixtures";
import { ItemCardSchema, ItemType, Priority } from "../../protocol/gen/hermes/board/v1/board_pb";
import type { ItemCardEntry } from "../../protocol/itemCards";
import ChatMarkdown from "../chat/ChatMarkdown";
import CardDrawer, { useCardDrawer } from "./CardDrawer";
import { HOVER_MS, LEAVE_MS, SLOW_MS } from "./CardLink";
import { CardLinksProvider } from "./CardLinks";
import LinkedText from "./LinkedText";

const BOTS = [bot({ id: "dd", name: "Desktop Dev" })];

function entry(id: string, title: string): ItemCardEntry {
  const json = toJson(
    ItemCardSchema,
    card({
      id,
      title,
      type: ItemType.BUG,
      priority: Priority.P0,
      assignee: "dd",
      columnKey: "doing",
    }),
  );
  return {
    id,
    project_id: "p1",
    project_name: "The Hermes",
    computer: "mac",
    card: json as ItemCardEntry["card"],
    column_name: "Doing",
  };
}

const ENTRIES: Record<string, ItemCardEntry> = {
  "H-293": entry("H-293", "Clicks open the wrong bot"),
  "H-292": entry("H-292", "Card ids as links"),
  "H-999": { id: "H-999", project_id: "p1", project_name: "The Hermes", missing: true },
  "H-500": {
    ...entry("H-500", "Kept elsewhere"),
    unreachable: { computer: "win-pc", last_seen: "2026-10-08T13:30:00Z" },
  },
};

function fakeDaemon(): FakeDaemon {
  const fake = new FakeDaemon();
  fake.capabilities = [...fake.capabilities, "item_cards"];
  fake.onRequest("item_cards_get", (body) => ({
    type: "item_cards",
    req_id: "1",
    cards: (body.type === "item_cards_get" ? body.ids : []).map(
      (id) => ENTRIES[id] ?? { id, missing: true },
    ),
  }));
  fake.onBoard("boardGet", () => {
    throw new Error("no board here");
  });
  fake.onBoard("itemMoveCheck", () => {
    throw new Error("no check here");
  });
  fake.onBoard("itemGet", (call) => {
    const id: string = (call.case === "itemGet" ? call.value.id : undefined) ?? "";
    const detail = itemDetail();
    if (detail.item) {
      detail.item.id = id;
      detail.item.title = ENTRIES[id]?.card?.title ?? id;
      detail.item.description = id === "H-293" ? "Same as H-292." : "";
    }
    return { case: "item", value: detail };
  });
  return fake;
}

function Harness(props: {
  readonly fake: FakeDaemon;
  readonly text: string;
  readonly markdown?: boolean;
  readonly onOpenBoard?: () => void;
}): ReactElement {
  const cards = useCardDrawer();
  return (
    <CardLinksProvider
      client={props.fake}
      projects={[project({ id: "p1", name: "The Hermes", item_prefix: "H" })]}
      bots={BOTS}
      currentProjectId="p1"
      onOpen={cards.open}
    >
      {props.markdown ? (
        <ChatMarkdown>{props.text}</ChatMarkdown>
      ) : (
        <p>
          <LinkedText text={props.text} />
        </p>
      )}
      <CardDrawer
        drawer={cards}
        client={props.fake}
        bots={BOTS}
        canComment={false}
        onOpenBoard={props.onOpenBoard ?? (() => undefined)}
      />
    </CardLinksProvider>
  );
}

async function settle(ms = 0): Promise<void> {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

async function hover(name: RegExp): Promise<HTMLElement> {
  const link = screen.getByRole("link", { name });
  fireEvent.mouseEnter(link);
  await settle(HOVER_MS);
  return link;
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("card links", () => {
  it("links only ids in text, not ids in code or longer words (UX-035 test 1)", () => {
    render(
      <Harness
        fake={fakeDaemon()}
        markdown
        text={"see H-189 and UTF-8, branch H-189-project-clicks, `H-190`\n\n```\nH-191\n```"}
      />,
    );
    const links = screen.getAllByRole("link").map((a) => a.textContent);
    expect(links).toEqual(["H-189", "H-190"]);
  });

  it("keeps a written link that isn't a card id inert", () => {
    render(<Harness fake={fakeDaemon()} markdown text="[click me](hermes://item/evil)" />);
    expect(screen.queryByRole("link")).toBeNull();
    expect(screen.getByText("click me")).toBeTruthy();
  });

  it("previews a card after a 400 ms hover and closes after leaving (test 2, 9)", async () => {
    render(<Harness fake={fakeDaemon()} text="Fixed in H-293." />);
    const link = screen.getByRole("link", { name: /^H-293/ });
    fireEvent.mouseEnter(link);
    await settle(HOVER_MS - 50);
    expect(screen.queryByRole("tooltip")).toBeNull();
    await settle(50);
    const tip = screen.getByRole("tooltip");
    expect(tip.textContent).toContain("H-293 · Bug · P0");
    expect(tip.textContent).toContain("Clicks open the wrong bot");
    expect(tip.textContent).toContain("Doing · Desktop Dev");
    expect(link.getAttribute("aria-describedby")).toBe(tip.id);
    expect(link.getAttribute("aria-label")).toBe("H-293: Clicks open the wrong bot");
    fireEvent.mouseLeave(link);
    await settle(LEAVE_MS);
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("shows the preview on focus and Esc closes it", async () => {
    render(<Harness fake={fakeDaemon()} text="H-293" />);
    act(() => {
      screen.getByRole("link").focus();
    });
    await settle();
    expect(screen.getByRole("tooltip")).toBeTruthy();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("says a missing card isn't on the board, and a click opens nothing (test 6)", async () => {
    render(<Harness fake={fakeDaemon()} text="See H-999" />);
    const link = await hover(/^H-999/);
    expect(screen.getByRole("tooltip").textContent).toBe(
      "H-999 isn't on The Hermes's board. It may have been deleted or mistyped.",
    );
    fireEvent.click(link);
    await settle();
    expect(screen.queryByRole("complementary")).toBeNull();
    expect(link.className).toContain("card-link--dim");
  });

  it("says where a card on an offline computer is kept (test 7)", async () => {
    render(<Harness fake={fakeDaemon()} text="H-500" />);
    await hover(/^H-500/);
    const text = screen.getByRole("tooltip").textContent ?? "";
    expect(text).toMatch(
      /^H-500 is on The Hermes's board, kept on win-pc\. win-pc is offline · last seen .+\./,
    );
    expect(text).toContain("Last seen as: Kept elsewhere");
  });

  it("asks for a newer service when it can't look cards up", async () => {
    const fake = fakeDaemon();
    fake.capabilities = ["terminal"];
    render(<Harness fake={fake} text="H-293" />);
    await hover(/^H-293/);
    expect(screen.getByRole("tooltip").textContent).toBe(
      "Update the Hermes service on this computer to see cards from here.",
    );
    expect(fake.requests).toHaveLength(0);
  });

  it("says when the service doesn't answer", async () => {
    const fake = fakeDaemon();
    fake.onRequest("item_cards_get", () => {
      throw new Error("gone");
    });
    render(<Harness fake={fake} text="H-293" />);
    await hover(/^H-293/);
    expect(screen.getByRole("tooltip").textContent).toBe("Loading H-293…");
    await settle(SLOW_MS);
    expect(screen.getByRole("tooltip").textContent).toBe(
      "H-293 can't be looked up right now: the Hermes service didn't answer.",
    );
  });

  it("asks once for every id on screen", async () => {
    const fake = fakeDaemon();
    render(<Harness fake={fake} text="H-293 and H-292" />);
    fireEvent.mouseEnter(screen.getByRole("link", { name: /^H-293/ }));
    fireEvent.mouseEnter(screen.getByRole("link", { name: /^H-292/ }));
    await settle();
    const asks = fake.requests.filter((r) => r.body.type === "item_cards_get");
    expect(asks).toHaveLength(1);
  });
});

describe("the card drawer", () => {
  async function openFirst(fake: FakeDaemon, onOpenBoard?: () => void): Promise<HTMLElement> {
    render(<Harness fake={fake} text="Fixed in H-293." onOpenBoard={onOpenBoard} />);
    const link = await hover(/^H-293/);
    act(() => {
      link.focus();
    });
    fireEvent.click(link);
    await settle();
    return link;
  }

  it("opens over the view and Esc returns focus to the link (test 3)", async () => {
    const link = await openFirst(fakeDaemon());
    expect(screen.getByRole("complementary", { name: "Item H-293" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /^Back to/ })).toBeNull();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("complementary")).toBeNull();
    expect(document.activeElement).toBe(link);
  });

  it("stacks a card opened inside it and goes back (test 4)", async () => {
    await openFirst(fakeDaemon());
    const inner = await hover(/^H-292/);
    fireEvent.click(inner);
    await settle();
    expect(screen.getByRole("complementary", { name: "Item H-292" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Back to H-293" }));
    await settle();
    expect(screen.getByRole("complementary", { name: "Item H-293" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /^Back to/ })).toBeNull();
  });

  it("goes back with ⌘[, closing on the first card", async () => {
    await openFirst(fakeDaemon());
    const inner = await hover(/^H-292/);
    fireEvent.click(inner);
    await settle();
    fireEvent.keyDown(window, { key: "[", metaKey: true });
    await settle();
    expect(screen.getByRole("complementary", { name: "Item H-293" })).toBeTruthy();
    fireEvent.keyDown(window, { key: "[", metaKey: true });
    expect(screen.queryByRole("complementary")).toBeNull();
  });

  it("opens the card on its board", async () => {
    const onOpenBoard = vi.fn<() => void>();
    await openFirst(fakeDaemon(), onOpenBoard);
    fireEvent.click(screen.getByRole("button", { name: "Open on the board" }));
    expect(onOpenBoard).toHaveBeenCalledWith({ id: "H-293", projectId: "p1" });
    expect(screen.queryByRole("complementary")).toBeNull();
  });
});
