// The binary surfaces on the control socket: protobuf `Envelope`s in WS
// binary frames (ADR-001 §1, B4). The board (`hermes.board.v1`) and pull
// requests (`hermes.pr.v1`, H-273) share one request-id space: a request goes
// out under the client's `req_id`, its response or `Envelope.error` comes back
// under the same id, and pushes carry `req_id` 0.

import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import type { BoardCall, BoardReply } from "./board";
import { DaemonError } from "./connection";
import type { BoardEvent } from "./gen/hermes/board/v1/requests_pb";
import { BoardRequestSchema } from "./gen/hermes/board/v1/requests_pb";
import type { PrPush } from "./gen/hermes/pr/v1/pr_pb";
import { PrRequestSchema } from "./gen/hermes/pr/v1/pr_pb";
import type { Envelope } from "./gen/hermes/wire/v1/envelope_pb";
import { EnvelopeSchema } from "./gen/hermes/wire/v1/envelope_pb";
import type { PrCall, PrReply } from "./prs";

type Body = Envelope["body"];

interface Pending {
  readonly resolve: (body: Body) => void;
  readonly reject: (error: Error) => void;
}

function decode(bytes: Uint8Array): Envelope | null {
  try {
    return fromBinary(EnvelopeSchema, bytes);
  } catch {
    return null;
  }
}

function unexpected(expect: string, body: Body): DaemonError {
  return new DaemonError("protocol_error", `expected ${expect}, got ${body.case ?? "none"}`);
}

/** The binary frames on a socket: requests in flight and push subscribers. */
export class BinaryChannel {
  private readonly pending = new Map<bigint, Pending>();
  private readonly boardHandlers = new Set<(event: BoardEvent) => void>();
  private readonly prHandlers = new Set<(push: PrPush) => void>();

  async board(ws: WebSocket, reqId: bigint, call: BoardCall): Promise<BoardReply> {
    const body = await this.send(ws, reqId, {
      case: "boardRequest",
      value: create(BoardRequestSchema, { request: call }),
    });
    if (body.case !== "boardResponse") {
      throw unexpected("a board response", body);
    }
    return body.value.response;
  }

  async pr(ws: WebSocket, reqId: bigint, call: PrCall): Promise<PrReply> {
    const body = await this.send(ws, reqId, {
      case: "prRequest",
      value: create(PrRequestSchema, { request: call }),
    });
    if (body.case !== "prResponse") {
      throw unexpected("a pull request response", body);
    }
    return body.value.response;
  }

  onBoard(handler: (event: BoardEvent) => void): () => void {
    this.boardHandlers.add(handler);
    return () => {
      this.boardHandlers.delete(handler);
    };
  }

  onPr(handler: (push: PrPush) => void): () => void {
    this.prHandlers.add(handler);
    return () => {
      this.prHandlers.delete(handler);
    };
  }

  receive(bytes: Uint8Array): void {
    const envelope = decode(bytes);
    if (envelope === null) {
      return;
    }
    const { reqId, body } = envelope;
    if (body.case === "boardPush") {
      if (body.value.push.case === "boardEvent") {
        const event = body.value.push.value;
        for (const handler of this.boardHandlers) {
          handler(event);
        }
      }
      return;
    }
    if (body.case === "prPush") {
      for (const handler of this.prHandlers) {
        handler(body.value);
      }
      return;
    }
    const pending = this.pending.get(reqId);
    if (pending === undefined) {
      return;
    }
    this.pending.delete(reqId);
    if (body.case === "error") {
      pending.reject(new DaemonError(body.value.code, body.value.message));
    } else {
      pending.resolve(body);
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

  private send(ws: WebSocket, reqId: bigint, body: Body): Promise<Body> {
    return new Promise<Body>((resolve, reject) => {
      this.pending.set(reqId, { resolve, reject });
      try {
        ws.send(toBinary(EnvelopeSchema, create(EnvelopeSchema, { reqId, body })));
      } catch (error) {
        this.pending.delete(reqId);
        reject(
          error instanceof Error ? error : new DaemonError("disconnected", "failed to send frame"),
        );
      }
    });
  }
}
