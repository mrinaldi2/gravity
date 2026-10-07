// What the item drawer reads (H-018 §3.6): the item with its links, comments
// and history (`item_get`), and what stands between it and each column for
// this connection (`item_move_check`). Off the board's home both come
// forwarded from it (H-099). Read again whenever the card's version moves.

import { useCallback, useEffect, useState } from "react";
import type { BoardApi } from "../../../protocol/board";
import { boardCall } from "../../../protocol/board";
import type { ItemDetail, MoveCheck } from "../../../protocol/gen/hermes/board/v1/requests_pb";
import { errText } from "../../../util";
import type { PostComment } from "./DrawerActivity";

export interface ItemDetailState {
  readonly detail: ItemDetail | null;
  readonly check: MoveCheck | null;
  readonly error: string | null;
  /** Posts the owner's comment, a reply when `replyTo` is set, and resolves
   * with the bots told; rejects with the daemon's refusal. */
  readonly comment: PostComment;
}

/** The owner's comment with this text is on the card and wasn't before. */
function landed(
  detail: ItemDetail,
  body: string,
  replyTo: string | undefined,
  seenBefore: ReadonlySet<string>,
): boolean {
  return detail.comments.some(
    (c) =>
      !seenBefore.has(c.id) &&
      (c.author === "user" || c.author.startsWith("device:")) &&
      c.body === body &&
      c.replyTo === replyTo,
  );
}

export function useItemDetail(
  api: BoardApi,
  itemId: string,
  /** The card's version: a push that changes it reads the item again. */
  version: bigint | undefined,
): ItemDetailState {
  const [detail, setDetail] = useState<ItemDetail | null>(null);
  const [check, setCheck] = useState<MoveCheck | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reads, setReads] = useState(0);

  useEffect(() => {
    let live = true;
    const load = async (): Promise<void> => {
      try {
        const item = await boardCall(api, { case: "itemGet", value: { id: itemId } }, "item");
        if (live) {
          setDetail(item);
          setError(null);
        }
      } catch (failure) {
        if (live) {
          setError(errText(failure));
        }
        return;
      }
      try {
        const moves = await boardCall(
          api,
          { case: "itemMoveCheck", value: { id: itemId } },
          "moveCheck",
        );
        if (live) {
          setCheck(moves);
        }
      } catch {
        // The next step is guidance: the drawer reads well without it.
      }
    };
    void load();
    return () => {
      live = false;
    };
    // `version` and `reads` are triggers, not inputs: a push that moves the
    // card, or a comment just posted, reads the item again.
    // oxlint-disable-next-line react/exhaustive-effect-dependencies
  }, [api, itemId, version, reads]);

  // Resolves once the item read back holds the comment (H-201), so the
  // drawer swaps "Sending…" for the posted comment without a gap. A retry
  // first looks for the comment: a try whose answer was lost may have
  // posted it (S4).
  const comment = useCallback(
    async (
      body: string,
      replyTo?: string,
      seenBefore?: ReadonlySet<string>,
    ): Promise<readonly string[]> => {
      const read = (): Promise<ItemDetail> =>
        boardCall(api, { case: "itemGet", value: { id: itemId } }, "item");
      if (seenBefore !== undefined) {
        const now = await read();
        if (landed(now, body, replyTo, seenBefore)) {
          setDetail(now);
          return [];
        }
      }
      const edited = await boardCall(
        api,
        { case: "itemComment", value: { id: itemId, body, replyTo } },
        "edited",
      );
      try {
        setDetail(await read());
      } catch {
        // Posted all the same: the next read shows it.
        setReads((n) => n + 1);
      }
      return edited.told;
    },
    [api, itemId],
  );

  return { detail, check, error, comment };
}
