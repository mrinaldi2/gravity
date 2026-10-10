// Pull requests (H-261 §9, H-273): `hermes.pr.v1` requests and pushes in
// binary frames, on the socket the board uses. A connection's first PR request
// for a project subscribes it to that project's `pr_updated`,
// `check_updated` and `merge_queue_changed` pushes.

import type { MessageInitShape } from "@bufbuild/protobuf";
import { DaemonError } from "./connection";
import type { PrPush, PrRequestSchema, PrResponse } from "./gen/hermes/pr/v1/pr_pb";

/** The capability a daemon lists in `hello_ok.capabilities` when it serves pull requests. */
export const PULL_REQUESTS = "pull_requests";

/** One pull request request: an arm of `PrRequest.request`. */
export type PrCall = Exclude<
  NonNullable<MessageInitShape<typeof PrRequestSchema>["request"]>,
  { readonly case: undefined }
>;

/** One pull request response: an arm of `PrResponse.response`. */
export type PrReply = PrResponse["response"];
type PrReplyArm = Exclude<PrReply, { case: undefined }>;
type PrReplies = { readonly [A in PrReplyArm as A["case"]]: A["value"] };

/** The pull request half of the daemon API; `DaemonClient` and the test fake implement it. */
export interface PrApi {
  /** Sends one pull request request; an `Envelope.error` rejects with a `DaemonError`. */
  pr(call: PrCall): Promise<PrReply>;
  /** Subscribes to pull request pushes for every project this connection asked about. */
  onPrPush(handler: (push: PrPush) => void): () => void;
}

function isArm<K extends keyof PrReplies>(
  reply: { readonly case: string | undefined; readonly value?: unknown },
  expect: K,
): reply is { readonly case: K; readonly value: PrReplies[K] } {
  return reply.case === expect;
}

/** Sends `call` and narrows the response to the `expect` arm. */
export async function prCall<K extends keyof PrReplies>(
  api: PrApi,
  call: PrCall,
  expect: K,
): Promise<PrReplies[K]> {
  const reply = await api.pr(call);
  if (isArm(reply, expect)) {
    return reply.value;
  }
  throw new DaemonError("protocol_error", `expected ${expect} reply, got ${reply.case ?? "none"}`);
}

/** The project a push is about. */
export function pushProject(push: PrPush): string {
  return push.push.case === undefined ? "" : push.push.value.projectId;
}
