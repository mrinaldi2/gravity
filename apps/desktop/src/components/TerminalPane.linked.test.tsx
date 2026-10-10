import { render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { FakeDaemon } from "../test/fakeDaemon";
import * as fx from "../test/fixtures";
import { fileDropDouble, terminals } from "../test/terminalPaneDoubles";
import { doElsewhere } from "./bot/doElsewhere";
import TerminalPane from "./TerminalPane";
import { clearTerminalCache } from "./terminalCache";

// A linked bot's terminal (H-303): its computer takes no typing from here.
vi.mock("@xterm/xterm", async () => {
  const { terminalDouble } = await import("../test/terminalPaneDoubles");
  return {
    Terminal: function TerminalDouble(): typeof terminalDouble {
      return terminalDouble;
    },
  };
});
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    fit = vi.fn<() => void>();
  },
}));
vi.mock("@xterm/addon-webgl", () => ({
  WebglAddon: class {
    onContextLoss = vi.fn<(handler: () => void) => void>();
    dispose = vi.fn<() => void>();
  },
}));
vi.mock("@xterm/addon-web-links", () => ({
  WebLinksAddon: class {
    dispose = vi.fn<() => void>();
  },
}));
vi.mock("../analytics", async () => (await import("../test/terminalPaneDoubles")).analyticsDouble);
vi.mock("./terminalFileDrop", async () => ({
  listenForTerminalFileDrops: (await import("../test/terminalPaneDoubles")).fileDropDouble.listen,
}));

vi.stubGlobal(
  "ResizeObserver",
  class {
    observe = vi.fn<(target: Element) => void>();
    disconnect = vi.fn<() => void>();
  },
);

describe("TerminalPane for a linked bot", () => {
  beforeEach(() => {
    clearTerminalCache();
    vi.clearAllMocks();
    terminals.reset();
    fileDropDouble.reset();
  });

  it("sends nothing typed, dropped or chorded", async () => {
    const fake = new FakeDaemon().onRequest("detach", () => ({ type: "ok", req_id: "1" }));
    render(<TerminalPane client={fake} botId="b1" canWrite typingRefused />);
    await vi.waitFor(() => {
      expect(terminals.onDataHandler).toBeDefined();
    });

    terminals.onDataHandler?.("1");
    expect(fileDropDouble.canDrop?.()).toBe(false);
    terminals.keyHandler?.(new KeyboardEvent("keydown", { key: "Enter", shiftKey: true }));

    expect(fake.fired.filter((frame) => frame.type === "input")).toHaveLength(0);
  });

  it("says where to do it instead", () => {
    const linked = fx.bot({ peer: { id: "p", name: "win-pc", online: true } });
    expect(doElsewhere(linked)).toBe(
      "Do it on win-pc or your phone: a linked computer can't type into or drive a bot there yet.",
    );
    expect(doElsewhere(fx.bot())).toBeNull();
  });
});
