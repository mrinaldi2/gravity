import { SquareKanban } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import type { ReactElement } from "react";
import type { AddToast } from "../../app/useToasts";
import type { DaemonApi } from "../../protocol/api";
import { boardCall, PROTO_ENCODING } from "../../protocol/board";
import type { Bot, Project } from "../../protocol/entities";
import type { BoardColumn, ItemCard } from "../../protocol/gen/hermes/board/v1/board_pb";
import { ColumnCategory } from "../../protocol/gen/hermes/board/v1/board_pb";
import BoardCard from "./BoardCard";
import { ItemOwnerActions } from "../ownerActions/OwnerActionList";
import BoardDrawer from "./BoardDrawer";
import BoardColumnView, { InboxRail } from "./BoardColumnView";
import type { DropState } from "./BoardColumnView";
import BoardFilterBar from "./BoardFilterBar";
import type { BotOption } from "./BoardFilterBar";
import type { BoardPrefs } from "./boardPrefs";
import { loadBoardPrefs, saveBoardPrefs } from "./boardPrefs";
import type { BoardModel } from "./boardSync";
import { matchesFilters } from "./filters";
import type { ColumnChecks } from "./moves";
import { planMove, reasonChip } from "./moves";
import { MoveDialog, MoveMenu, RefusalPopover } from "./MovePopovers";
import { useBoard } from "./useBoard";
import type { Anchor } from "./useBoardMoves";
import { useBoardMoves } from "./useBoardMoves";
import { wipSummary } from "./wip";

interface BoardViewProps {
  readonly client: DaemonApi;
  readonly project: Project;
  /** This project's bots, for assignee names and avatars. */
  readonly bots: readonly Bot[];
  readonly connected: boolean;
  readonly canControl: boolean;
  readonly addToast: AddToast;
}

function Notice({
  title,
  text,
  action,
}: {
  readonly title: string;
  readonly text?: string;
  readonly action?: ReactElement;
}): ReactElement {
  return (
    <div className="empty-pane">
      <div className="empty-state">
        <SquareKanban className="coming-soon-icon" aria-hidden="true" />
        <h1>{title}</h1>
        {text === undefined ? null : <p>{text}</p>}
        {action}
      </div>
    </div>
  );
}

/**
 * A project with no board here. The owner can make this computer its home
 * (H-037); a linked team's home is their choice, so the daemon never picks one.
 */
function NoBoard(
  props: BoardViewProps & { readonly message: string; readonly refresh: () => void },
): ReactElement {
  const { client, project, message, refresh } = props;
  const [starting, setStarting] = useState(false);
  // Why the daemon refused to start it here: the board already lives on a
  // linked computer, or one can't confirm it has none (ARCH-R18 M1).
  const [refusal, setRefusal] = useState<string | null>(null);
  const owner = client.hasGrant("approve");
  const start = (): void => {
    setStarting(true);
    setRefusal(null);
    void (async () => {
      try {
        await boardCall(client, { case: "boardEnable", value: { projectId: project.id } }, "board");
        refresh();
      } catch (error) {
        setRefusal(error instanceof Error ? error.message : String(error));
      } finally {
        setStarting(false);
      }
    })();
  };
  return (
    <Notice
      title="This board isn't set up yet"
      text={
        owner
          ? `${message} Start it on one computer only: that computer becomes the board's home.`
          : message
      }
      action={
        owner ? (
          <>
            <button type="button" className="btn btn-small" disabled={starting} onClick={start}>
              Start the board on this computer
            </button>
            {refusal === null ? null : (
              <p className="field-error" role="alert">
                {refusal}
              </p>
            )}
          </>
        ) : undefined
      }
    />
  );
}

/** The Board tab: the project's columns and cards, live (H-018 §3). */
export default function BoardView(props: BoardViewProps): ReactElement {
  const { client, project, connected } = props;
  const supported = client.encodings.includes(PROTO_ENCODING);
  const { status, refresh } = useBoard(client, project.id, connected && supported);

  if (!supported) {
    return <Notice title="The board needs a newer Hermes service" />;
  }
  if (status.kind === "loading") {
    return <Notice title={connected ? "Loading the board…" : "Waiting for the Hermes service…"} />;
  }
  if (status.kind === "failed") {
    if (status.code === "no_board") {
      return <NoBoard {...props} message={status.message} refresh={refresh} />;
    }
    return (
      <Notice
        title="Couldn't load the board"
        text={status.message}
        action={
          <button type="button" className="btn btn-small" onClick={refresh}>
            Try again
          </button>
        }
      />
    );
  }
  return <Board {...props} model={status.model} refresh={refresh} />;
}

interface BoardProps extends BoardViewProps {
  readonly model: BoardModel;
  readonly refresh: () => void;
}

interface Drag {
  readonly card: ItemCard;
  /** Null until the move check answers. */
  readonly checks: ColumnChecks | null;
}

function byRank(a: ItemCard, b: ItemCard): number {
  if (a.rank === b.rank) {
    return a.id.localeCompare(b.id);
  }
  return a.rank < b.rank ? -1 : 1;
}

function dropStateFor(column: BoardColumn, drag: Drag | null): DropState {
  if (drag === null) {
    return { kind: "idle" };
  }
  if (column.key === drag.card.columnKey || drag.checks === null) {
    return { kind: "allowed" };
  }
  const unmet = drag.checks.get(column.key) ?? [];
  return planMove(unmet).kind === "refused"
    ? { kind: "refused", chip: reasonChip(unmet) }
    : { kind: "allowed" };
}

function Board(props: BoardProps): ReactElement {
  const { client, project, model, bots, canControl, addToast, refresh } = props;
  const [prefs, setPrefs] = useState<BoardPrefs>(() => loadBoardPrefs(project.id));
  const [drag, setDrag] = useState<Drag | null>(null);
  const [announce, setAnnounce] = useState("");
  const moves = useBoardMoves({ api: client, columns: model.columns, refresh, addToast });

  const updatePrefs = useCallback(
    (next: BoardPrefs): void => {
      setPrefs(next);
      saveBoardPrefs(project.id, next);
    },
    [project.id],
  );

  const botsById = useMemo(() => new Map(bots.map((bot) => [bot.id, bot])), [bots]);
  const cards = useMemo(() => {
    const sorted = [...model.cards.values()];
    // oxlint-disable-next-line unicorn/no-array-sort
    sorted.sort(byRank);
    return sorted;
  }, [model.cards]);
  const botOptions = useMemo((): readonly BotOption[] => {
    const options: BotOption[] = bots.map((bot) => ({ id: bot.id, bot }));
    for (const card of cards) {
      const id = card.assignee;
      if (id !== undefined && !options.some((option) => option.id === id)) {
        options.push({ id, bot: undefined });
      }
    }
    return options;
  }, [bots, cards]);

  const columns = model.columns.filter((column) => column.visible || prefs.showHidden);
  const hiddenNames = model.columns.filter((column) => !column.visible).map((c) => c.name);
  const shownTotal = cards.filter((card) => matchesFilters(card, prefs.filters)).length;

  const onDragStart = (card: ItemCard): void => {
    setDrag({ card, checks: null });
    void (async () => {
      try {
        const checks = await moves.checksFor(card);
        setDrag((current) => (current?.card.id === card.id ? { card, checks } : current));
      } catch {
        // The drop runs the check again and reports the failure.
      }
    })();
  };

  const onDrop = (column: BoardColumn, anchor: Anchor): void => {
    const current = drag;
    setDrag(null);
    setAnnounce("");
    if (current !== null) {
      moves.requestMove(current.card, column.key, anchor);
    }
  };

  const canMove = canControl ? moves.openMenu : null;
  const [opened, setOpened] = useState<string | null>(null);
  // The owner's call, on the board's home (H-099).
  const setLimit =
    (column: BoardColumn) =>
    async (wipLimit: number | undefined): Promise<void> => {
      await boardCall(
        client,
        {
          case: "columnSetLimit",
          value: { projectId: project.id, columnKey: column.key, wipLimit },
        },
        "board",
      );
      refresh();
    };

  const { ui } = moves;
  return (
    <div className="board-view">
      <BoardFilterBar
        filters={prefs.filters}
        onChange={(filters) => {
          updatePrefs({ ...prefs, filters });
        }}
        botOptions={botOptions}
        shown={shownTotal}
        total={cards.length}
        hiddenNames={hiddenNames}
        showHidden={prefs.showHidden}
        onShowHidden={(showHidden) => {
          updatePrefs({ ...prefs, showHidden });
        }}
      />
      {cards.length === 0 ? (
        <div className="board-empty" role="status">
          <h2>No items yet</h2>
          <p>
            When the team plans work for {project.name}, its items show up here, column by column.
          </p>
        </div>
      ) : null}
      <div className={`board-columns${drag === null ? "" : " board-dragging"}`}>
        {columns.map((column) => {
          const inColumn = cards.filter((card) => card.columnKey === column.key);
          const visible = inColumn.filter((card) => matchesFilters(card, prefs.filters));
          const summary = wipSummary(column, inColumn, visible.length);
          const drop = dropStateFor(column, drag);
          const handlers = {
            onDragEnter: () => {
              setAnnounce(drop.kind === "refused" ? `${column.name}: ${drop.chip}` : "");
            },
            onDrop: (anchor: Anchor) => {
              onDrop(column, anchor);
            },
          };
          if (column.category === ColumnCategory.INBOX && !prefs.inboxOpen) {
            return (
              <InboxRail
                key={column.key}
                column={column}
                summary={summary}
                drop={drop}
                onExpand={() => {
                  updatePrefs({ ...prefs, inboxOpen: true });
                }}
                {...handlers}
              />
            );
          }
          return (
            <BoardColumnView
              key={column.key}
              column={column}
              summary={summary}
              drop={drop}
              botsById={botsById}
              onSetLimit={model.canRule ? setLimit(column) : undefined}
              onCollapse={
                column.category === ColumnCategory.INBOX
                  ? () => {
                      updatePrefs({ ...prefs, inboxOpen: false });
                    }
                  : undefined
              }
              {...handlers}
            >
              {visible.map((card) => (
                <BoardCard
                  key={card.id}
                  card={card}
                  columnName={column.name}
                  assignee={card.assignee === undefined ? undefined : botsById.get(card.assignee)}
                  onOpenMoveMenu={canMove}
                  onOpen={setOpened}
                  onDragStart={onDragStart}
                  onDragEnd={() => {
                    setDrag(null);
                    setAnnounce("");
                  }}
                  dragging={drag?.card.id === card.id}
                />
              ))}
            </BoardColumnView>
          );
        })}
      </div>
      <div className="visually-hidden" aria-live="polite">
        {announce}
      </div>
      <BoardDrawer
        api={client}
        openId={opened}
        shown={cards.filter((card) => matchesFilters(card, prefs.filters))}
        columns={model.columns}
        bots={bots}
        canControl={canControl}
        onOpen={setOpened}
        onMove={canMove}
      >
        <ItemOwnerActions {...props} projectId={project.id} itemId={opened} />
      </BoardDrawer>
      {ui.kind === "menu" ? (
        <MoveMenu
          card={ui.card}
          anchor={ui.anchor}
          columns={model.columns}
          checks={ui.checks}
          onChoose={(toKey) => {
            moves.requestMove(ui.card, toKey, ui.anchor);
          }}
          onClose={moves.close}
        />
      ) : null}
      {ui.kind === "refused" ? (
        <RefusalPopover
          card={ui.card}
          to={ui.to}
          anchor={ui.anchor}
          unmet={ui.unmet}
          onClose={moves.close}
        />
      ) : null}
      {ui.kind === "input" ? (
        <MoveDialog
          card={ui.card}
          to={ui.to}
          needsReason={ui.needsReason}
          needsOverride={ui.needsOverride}
          unmet={ui.unmet}
          onConfirm={moves.confirm}
          onCancel={moves.close}
        />
      ) : null}
    </div>
  );
}
