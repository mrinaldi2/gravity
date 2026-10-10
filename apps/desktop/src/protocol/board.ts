// The board surface: protobuf `Envelope`s in WS binary frames (ADR-001 §1,
// B4), sent through the socket's `BinaryChannel` (channel.ts).

import type { MessageInitShape } from "@bufbuild/protobuf";
import type {
  BoardEvent,
  BoardRequestSchema,
  BoardResponse,
} from "./gen/hermes/board/v1/requests_pb";
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
