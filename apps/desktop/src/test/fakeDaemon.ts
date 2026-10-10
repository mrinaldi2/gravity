import type { AttachResult, DaemonApi } from "../protocol/api";
import type { BoardCall, BoardReply } from "../protocol/board";
import type { ConnectionStatus, Endpoint } from "../protocol/connection";
import type { Grant } from "../protocol/entities";
import type { BoardEvent } from "../protocol/gen/hermes/board/v1/requests_pb";
import type { PrPush } from "../protocol/gen/hermes/pr/v1/pr_pb";
import type { PrCall, PrReply } from "../protocol/prs";
import type {
  PushOf,
  ReplyOf,
  ServerPushType,
  ServerReply,
  ServerReplyType,
} from "../protocol/messages";
import type { PushHandlerSets } from "../protocol/push";
import { emptyHandlers } from "../protocol/push";
import type { FireBody, RequestBody } from "../protocol/requests";
import { replyIs } from "../protocol/wire";

export interface RecordedRequest {
  readonly body: RequestBody;
  readonly expect: ServerReplyType;
}

/** Answers one request type. Throw to simulate a daemon error. */
type Responder = (body: RequestBody) => ServerReply;

/** Answers one board request arm. Throw (a `DaemonError`) to simulate an `Envelope.error`. */
type BoardResponder = (call: BoardCall) => BoardReply | Promise<BoardReply>;

/** Answers one pull request arm. Throw (a `DaemonError`) to simulate an `Envelope.error`. */
type PrResponder = (call: PrCall) => PrReply | Promise<PrReply>;

/**
 * In-memory `DaemonApi` for component tests: requests are answered from a
 * per-type responder table and pushes can be emitted on demand. Replies are
 * narrowed with the production type guard, so no test needs a cast.
 */
export class FakeDaemon implements DaemonApi {
  connectionGeneration = 0;
  status: ConnectionStatus = "connected";
  capabilities: readonly string[] = ["terminal", "routines", "decisions"];
  serverVersion = "0.1.0-test";
  grants: readonly Grant[] = ["read", "control"];
  deviceId: string | null = null;
  encodings: readonly string[] = ["proto"];

  readonly requests: RecordedRequest[] = [];
  readonly fired: FireBody[] = [];
  readonly boardCalls: BoardCall[] = [];
  readonly prCalls: PrCall[] = [];
  attachResult: AttachResult = { seq: 0, resumed: false };
  /** The `resume` flag of every attach, in order. */
  readonly attachResumes: boolean[] = [];
  /** Holds attach replies until `releaseAttach`, to test teardown races. */
  deferAttach = false;
  started = false;

  private endpoint: Endpoint = { host: "127.0.0.1", port: 7777 };
  private heldAttaches: (() => void)[] = [];
  private readonly responders = new Map<string, Responder>();
  private readonly handlers: PushHandlerSets = emptyHandlers();
  private readonly boardResponders = new Map<string, BoardResponder>();
  private readonly boardHandlers = new Set<(event: BoardEvent) => void>();
  private readonly prResponders = new Map<string, PrResponder>();
  private readonly prHandlers = new Set<(push: PrPush) => void>();
  private readonly statusListeners = new Set<(status: ConnectionStatus) => void>();

  /** Registers the reply for one request type; later calls replace earlier ones. */
  onRequest(type: RequestBody["type"], responder: Responder): this {
    this.responders.set(type, responder);
    return this;
  }

  /** Registers the reply for one board request arm; later calls replace earlier ones. */
  onBoard(arm: BoardCall["case"], responder: BoardResponder): this {
    this.boardResponders.set(arm, responder);
    return this;
  }

  emitBoardEvent(event: BoardEvent): void {
    for (const handler of this.boardHandlers) {
      handler(event);
    }
  }

  /** Registers the reply for one pull request arm; later calls replace earlier ones. */
  onPr(arm: PrCall["case"], responder: PrResponder): this {
    this.prResponders.set(arm, responder);
    return this;
  }

  emitPrPush(push: PrPush): void {
    for (const handler of this.prHandlers) {
      handler(push);
    }
  }

  emit<K extends ServerPushType>(type: K, push: PushOf<K>): void {
    for (const handler of this.handlers[type]) {
      handler(push);
    }
  }

  setStatus(status: ConnectionStatus): void {
    this.status = status;
    for (const listener of this.statusListeners) {
      listener(status);
    }
  }

  start(): void {
    this.started = true;
  }

  setEndpoint(endpoint: Endpoint): void {
    this.endpoint = endpoint;
    this.connectionGeneration += 1;
  }

  getEndpoint(): Endpoint {
    return this.endpoint;
  }

  hasGrant(grant: Grant): boolean {
    return this.grants.includes(grant);
  }

  onStatus(listener: (status: ConnectionStatus) => void): () => void {
    this.statusListeners.add(listener);
    return () => {
      this.statusListeners.delete(listener);
    };
  }

  on<K extends ServerPushType>(type: K, handler: (push: PushOf<K>) => void): () => void {
    const set = this.handlers[type];
    set.add(handler);
    return () => {
      set.delete(handler);
    };
  }

  request<K extends ServerReplyType>(body: RequestBody, expect: K): Promise<ReplyOf<K>> {
    this.requests.push({ body, expect });
    const responder = this.responders.get(body.type);
    if (responder === undefined) {
      return Promise.reject(new Error(`no fake responder for '${body.type}'`));
    }
    let reply: ServerReply;
    try {
      reply = responder(body);
    } catch (error) {
      return Promise.reject(error instanceof Error ? error : new Error(String(error)));
    }
    if (replyIs(reply, expect)) {
      return Promise.resolve(reply);
    }
    return Promise.reject(new Error(`fake replied '${reply.type}', expected '${expect}'`));
  }

  // fallow-ignore-next-line unused-class-member -- reached through BoardApi and the board tests
  async board(call: BoardCall): Promise<BoardReply> {
    this.boardCalls.push(call);
    const responder = this.boardResponders.get(call.case);
    if (responder === undefined) {
      throw new Error(`no fake board responder for '${call.case}'`);
    }
    return responder(call);
  }

  onBoardEvent(handler: (event: BoardEvent) => void): () => void {
    this.boardHandlers.add(handler);
    return () => {
      this.boardHandlers.delete(handler);
    };
  }

  // fallow-ignore-next-line unused-class-member -- reached through PrApi and the PR tab tests
  async pr(call: PrCall): Promise<PrReply> {
    this.prCalls.push(call);
    const responder = this.prResponders.get(call.case);
    if (responder === undefined) {
      throw new Error(`no fake pull request responder for '${call.case}'`);
    }
    return responder(call);
  }

  onPrPush(handler: (push: PrPush) => void): () => void {
    this.prHandlers.add(handler);
    return () => {
      this.prHandlers.delete(handler);
    };
  }

  fire(body: FireBody): void {
    this.fired.push(body);
  }

  attach(_botId: string, resume: boolean): Promise<AttachResult> {
    this.attachResumes.push(resume);
    if (!this.deferAttach) {
      return Promise.resolve(this.attachResult);
    }
    return new Promise<AttachResult>((resolve) => {
      this.heldAttaches.push(() => {
        resolve(this.attachResult);
      });
    });
  }

  /** Answers every attach held back by `deferAttach`. */
  releaseAttaches(): void {
    const held = this.heldAttaches;
    this.heldAttaches = [];
    for (const resolve of held) {
      resolve();
    }
  }
}
