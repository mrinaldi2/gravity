import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { card, snapshot } from "../test/boardFixtures";
import { installFakeWebSocket } from "../test/fakeWebSocket";
import type { FakeSocket, SocketHarness } from "../test/fakeWebSocket";
import { DaemonClient } from "./client";
import { DaemonError } from "./connection";
import { BoardEventKind } from "./gen/hermes/board/v1/requests_pb";
import type { Envelope } from "./gen/hermes/wire/v1/envelope_pb";
import { EnvelopeSchema } from "./gen/hermes/wire/v1/envelope_pb";
import type { MessageInitShape } from "@bufbuild/protobuf";

/** Opens a socket and completes a handshake that advertises `encodings`. */
async function connectWith(
  client: DaemonClient,
  harness: SocketHarness,
  encodings: readonly string[] | undefined,
): Promise<FakeSocket> {
  client.start();
  const socket = harness.latest();
  socket.open();
  await vi.waitFor(() => {
    expect(socket.sent).toHaveLength(1);
  });
  socket.receive({
    type: "hello_ok",
    req_id: socket.reqId(0),
    protocol_version: 2,
    server_version: "0.1.0",
    capabilities: [],
    grants: ["read", "control"],
    device_id: null,
    ...(encodings === undefined ? {} : { encodings }),
  });
  await vi.waitFor(() => {
    expect(client.status).toBe("connected");
  });
  return socket;
}

function lastRequest(socket: FakeSocket): Envelope {
  const frame = socket.sentBinary.at(-1);
  if (frame === undefined) {
    throw new Error("no binary frame was sent");
  }
  return fromBinary(EnvelopeSchema, frame);
}

/** Delivers one server envelope as an ArrayBuffer, the way `binaryType = "arraybuffer"` does. */
function deliver(socket: FakeSocket, init: MessageInitShape<typeof EnvelopeSchema>): void {
  const bytes = toBinary(EnvelopeSchema, create(EnvelopeSchema, init));
  socket.receiveRaw(bytes.slice().buffer);
}

describe("DaemonClient board surface", () => {
  let harness: SocketHarness;
  let client: DaemonClient;

  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    harness = installFakeWebSocket();
    client = new DaemonClient({ host: "mini", port: 7777 }, () => Promise.resolve("token"));
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("sends a board request as a binary envelope and resolves with its response", async () => {
    const socket = await connectWith(client, harness, ["proto"]);
    expect(client.encodings).toEqual(["proto"]);

    const reply = client.board({ case: "boardWatch", value: { projectId: "p1" } });
    const request = lastRequest(socket);
    expect(request.body.case).toBe("boardRequest");
    expect(request.reqId).toBeGreaterThan(0n);
    expect(request.body.value).toMatchObject({ request: { case: "boardWatch" } });

    deliver(socket, {
      reqId: request.reqId,
      body: {
        case: "boardResponse",
        value: { response: { case: "board", value: snapshot([], 4n) } },
      },
    });
    await expect(reply).resolves.toMatchObject({ case: "board", value: { seq: 4n } });
  });

  it("rejects with the envelope error's code", async () => {
    const socket = await connectWith(client, harness, ["proto"]);
    const reply = client.board({ case: "boardGet", value: { projectId: "p1" } });
    deliver(socket, {
      reqId: lastRequest(socket).reqId,
      body: { case: "error", value: { code: "no_board", message: "not enabled" } },
    });
    await expect(reply).rejects.toMatchObject({ code: "no_board", message: "not enabled" });
  });

  it("hands pushes to board subscribers until they unsubscribe", async () => {
    const socket = await connectWith(client, harness, ["proto"]);
    const seen: bigint[] = [];
    const off = client.onBoardEvent((event) => {
      seen.push(event.seq);
    });
    const push = (seq: bigint): void => {
      deliver(socket, {
        body: {
          case: "boardPush",
          value: {
            push: {
              case: "boardEvent",
              value: {
                projectId: "p1",
                seq,
                kind: BoardEventKind.ITEM_MOVED,
                itemId: "H-1",
                card: card(),
              },
            },
          },
        },
      });
    };
    push(1n);
    off();
    push(2n);
    expect(seen).toEqual([1n]);
  });

  it("ignores garbage binary frames", async () => {
    const socket = await connectWith(client, harness, ["proto"]);
    const reply = client.board({ case: "boardGet", value: { projectId: "p1" } });
    socket.receiveRaw(new Uint8Array([0xff, 0xff, 0xff]).buffer);
    deliver(socket, {
      reqId: lastRequest(socket).reqId,
      body: { case: "boardResponse", value: { response: { case: "board", value: snapshot() } } },
    });
    await expect(reply).resolves.toMatchObject({ case: "board" });
  });

  it("refuses the board when the daemon has no proto encoding", async () => {
    const socket = await connectWith(client, harness, undefined);
    await expect(
      client.board({ case: "boardGet", value: { projectId: "p1" } }),
    ).rejects.toMatchObject({ code: "unsupported" });
    expect(socket.sentBinary).toHaveLength(0);
  });

  it("fails board requests in flight when the socket closes", async () => {
    const socket = await connectWith(client, harness, ["proto"]);
    const reply = client.board({ case: "boardGet", value: { projectId: "p1" } });
    socket.close();
    await expect(reply).rejects.toBeInstanceOf(DaemonError);
    await expect(reply).rejects.toMatchObject({ code: "disconnected" });
  });
});

describe("DaemonClient pull request surface (H-273)", () => {
  let harness: SocketHarness;
  let client: DaemonClient;

  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    harness = installFakeWebSocket();
    client = new DaemonClient({ host: "mini", port: 7777 }, () => Promise.resolve("token"));
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("sends a PR request on the board's request ids and resolves with its response", async () => {
    const socket = await connectWith(client, harness, ["proto"]);
    const board = client.board({ case: "boardGet", value: { projectId: "p1" } });
    const boardId = lastRequest(socket).reqId;
    const reply = client.pr({ case: "prGet", value: { projectId: "p1", number: 42 } });
    const request = lastRequest(socket);
    expect(request.body.case).toBe("prRequest");
    expect(request.reqId).not.toBe(boardId);
    expect(request.body.value).toMatchObject({ request: { case: "prGet", value: { number: 42 } } });

    deliver(socket, {
      reqId: request.reqId,
      body: { case: "prResponse", value: { response: { case: "pr", value: { number: 42 } } } },
    });
    await expect(reply).resolves.toMatchObject({ case: "pr", value: { number: 42 } });
    deliver(socket, {
      reqId: boardId,
      body: { case: "prResponse", value: { response: { case: "pr", value: { number: 1 } } } },
    });
    await expect(board).rejects.toMatchObject({ code: "protocol_error" });
  });

  it("rejects a PR request with the envelope error's code", async () => {
    const socket = await connectWith(client, harness, ["proto"]);
    const reply = client.pr({ case: "prGet", value: { projectId: "p1", number: 9 } });
    deliver(socket, {
      reqId: lastRequest(socket).reqId,
      body: { case: "error", value: { code: "not_found", message: "no PR #9" } },
    });
    await expect(reply).rejects.toMatchObject({ code: "not_found" });
  });

  it("hands PR pushes to their subscribers, not the board's", async () => {
    const socket = await connectWith(client, harness, ["proto"]);
    const prs: string[] = [];
    const boards: bigint[] = [];
    const off = client.onPrPush((push) => {
      prs.push(push.push.case ?? "");
    });
    client.onBoardEvent((event) => {
      boards.push(event.seq);
    });
    deliver(socket, {
      body: {
        case: "prPush",
        value: {
          push: { case: "checkUpdated", value: { projectId: "p1", sha: "abc", name: "rust" } },
        },
      },
    });
    off();
    deliver(socket, {
      body: {
        case: "prPush",
        value: { push: { case: "prUpdated", value: { projectId: "p1", number: 1 } } },
      },
    });
    expect(prs).toEqual(["checkUpdated"]);
    expect(boards).toEqual([]);
  });
});
