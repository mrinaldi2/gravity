import type { PushOf, ServerPush, ServerPushType } from "./messages";

/** One handler set per push type. */
export type PushHandlerSets = {
  readonly [K in ServerPushType]: Set<(push: PushOf<K>) => void>;
};

export function emptyHandlers(): PushHandlerSets {
  return {
    term: new Set(),
    bot_state: new Set(),
    message_new: new Set(),
    bot_updated: new Set(),
    project_updated: new Set(),
    workers_updated: new Set(),
    meeting_event: new Set(),
    activity_update: new Set(),
    delivery_update: new Set(),
    routine_run_update: new Set(),
    approval_pending: new Set(),
    notify: new Set(),
    decision_update: new Set(),
    decision_deleted: new Set(),
    decision_comment_new: new Set(),
    chat_turns: new Set(),
    permission_request: new Set(),
    permission_resolved: new Set(),
    browser_tabs: new Set(),
    browser_frame: new Set(),
  };
}

/**
 * Route a parsed push to its handler set.
 *
 * Every push type has a set: `PushHandlerSets` is keyed by the whole
 * `ServerPushType` union, so a new push type without a set in
 * `emptyHandlers` is a type error rather than a frame parsed and then
 * dropped, which is how `bot_updated` went missing once.
 */
export function dispatchPush(handlers: PushHandlerSets, push: ServerPush): void {
  // The set for `push.type` takes exactly that push; TypeScript can't relate
  // the two through the union, hence the widening.
  const set = handlers[push.type] as ReadonlySet<(push: ServerPush) => void>;
  for (const handler of set) {
    handler(push);
  }
}
