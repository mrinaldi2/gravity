// The board surface: protobuf `Envelope`s in WS binary frames (ADR-001 §1,
// B4). Requests go out under the client's `req_id`, the response comes back
// under the same id, and pushes carry `req_id` 0.

import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import type { MessageInitShape } from "@bufbuild/protobuf";
import type { BoardEvent, BoardResponse } from "./gen/hermes/board/v1/requests_pb";
import { BoardRequestSchema } from "./gen/hermes/board/v1/requests_pb";
import { EnvelopeSchema } from "./gen/hermes/wire/v1/envelope_pb";
import { DaemonError } from "./connection";

/** The encoding a daemon must list in `hello_ok.encodings` to serve the board. */
export const PROTO_ENCODING = "proto";

/** One board request: an arm of `BoardRequest.request`. */
export type BoardCall = Exclude<
  NonNullable<MessageInitShape<typeof BoardRequestSchema>["request"]>,
  { readonly case: undefined }
>;

/** One board response: an arm of `BoardResponse.response`. */
export type BoardReply = BoardResponse["response"];
type BoardReplyArm = Exclude<BoardReply, { case: undefined }>;
/** Each response arm's value, by its `case`. */
type BoardReplies = { readonly [A in BoardReplyArm as A["case"]]: A["value"] };

/** The board half of the daemon API; `DaemonClient` and the test fake both implement it. */
export interface BoardApi {
  /** Sends one board request; an `Envelope.error` rejects with a `DaemonError`. */
  board(call: BoardCall): Promise<BoardReply>;
  /** Subscribes to board pushes for every watched project. */
  onBoardEvent(handler: (event: BoardEvent) => void): () => void;
}

// A oneof is tagged by `case`, so the tag alone decides the value's type;
// TypeScript cannot follow that through a generic `K` on its own.
function isArm<K extends keyof BoardReplies>(
  reply: { readonly case: string | undefined; readonly value?: unknown },
  expect: K,
): reply is { readonly case: K; readonly value: BoardReplies[K] } {
  return reply.case === expect;
}

/** Sends `call` and narrows the response to the `expect` arm. */
export async function boardCall<K extends keyof BoardReplies>(
  api: BoardApi,
  call: BoardCall,
  expect: K,
): Promise<BoardReplies[K]> {
  const reply = await api.board(call);
  if (isArm(reply, expect)) {
    return reply.value;
  }
  throw new DaemonError("protocol_error", `expected ${expect} reply, got ${reply.case ?? "none"}`);
}

function encodeBoardRequest(reqId: bigint, call: BoardCall): Uint8Array {
  return toBinary(
    EnvelopeSchema,
    create(EnvelopeSchema, {
      reqId,
      body: { case: "boardRequest", value: create(BoardRequestSchema, { request: call }) },
    }),
  );
}

/** A decoded server frame on the board surface. */
type BoardFrame =
  | { readonly kind: "response"; readonly reqId: bigint; readonly response: BoardReply }
  | {
      readonly kind: "error";
      readonly reqId: bigint;
      readonly code: string;
      readonly message: string;
    }
  | { readonly kind: "event"; readonly event: BoardEvent };

/** Decodes one binary frame; null for garbage or arms a client never receives. */
function decodeBoardFrame(bytes: Uint8Array): BoardFrame | null {
  let envelope;
  try {
    envelope = fromBinary(EnvelopeSchema, bytes);
  } catch {
    return null;
  }
  const { reqId, body } = envelope;
  switch (body.case) {
    case "boardResponse":
      return { kind: "response", reqId, response: body.value.response };
    case "error":
      return { kind: "error", reqId, code: body.value.code, message: body.value.message };
    case "boardPush":
      return body.value.push.case === "boardEvent"
        ? { kind: "event", event: body.value.push.value }
        : null;
    default:
      return null;
  }
}

interface PendingBoard {
  readonly resolve: (reply: BoardReply) => void;
  readonly reject: (error: Error) => void;
}

/** The board's binary frames on a socket: requests in flight and push subscribers. */
export class BoardChannel {
  private readonly pending = new Map<bigint, PendingBoard>();
  private readonly handlers = new Set<(event: BoardEvent) => void>();

  send(ws: WebSocket, reqId: bigint, call: BoardCall): Promise<BoardReply> {
    return new Promise<BoardReply>((resolve, reject) => {
      this.pending.set(reqId, { resolve, reject });
      try {
        ws.send(encodeBoardRequest(reqId, call));
      } catch (error) {
        this.pending.delete(reqId);
        reject(
          error instanceof Error ? error : new DaemonError("disconnected", "failed to send frame"),
        );
      }
    });
  }

  on(handler: (event: BoardEvent) => void): () => void {
    this.handlers.add(handler);
    return () => {
      this.handlers.delete(handler);
    };
  }

  receive(bytes: Uint8Array): void {
    const frame = decodeBoardFrame(bytes);
    if (frame === null) {
      return;
    }
    if (frame.kind === "event") {
      for (const handler of this.handlers) {
        handler(frame.event);
      }
      return;
    }
    const pending = this.pending.get(frame.reqId);
    if (pending === undefined) {
      return;
    }
    this.pending.delete(frame.reqId);
    if (frame.kind === "error") {
      pending.reject(new DaemonError(frame.code, frame.message));
    } else {
      pending.resolve(frame.response);
    }
  }

  /** Rejects every request in flight, e.g. when the socket closes. */
  failAll(error: Error): void {
    const entries = [...this.pending.values()];
    this.pending.clear();
    for (const entry of entries) {
      entry.reject(error);
    }
  }
}
