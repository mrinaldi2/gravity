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
