// Widgets 2–6 of the dashboard (H-018 §2.1): the board strip, releases, the
// team, and meetings and action items, empty until meetings land (H-102).
// Every state carries a glyph and a word, never colour alone.

import type { ReactElement, ReactNode } from "react";
import type { BoardSummary, TeamRow } from "../../protocol/dashboard";
import type { Bot } from "../../protocol/entities";
import type { Release } from "../../protocol/releases";
import BotAvatar from "../BotAvatar";
import { BOT_STATE_LABEL } from "../bot/botStates";
import { releaseTitle, rolloutLabel, statusLabel, testLabel } from "../releases/labels";

function Widget(props: {
  readonly id: string;
  readonly title: string;
  readonly wide?: boolean;
  readonly more?: { readonly label: string; readonly onClick: () => void };
  readonly children: ReactNode;
}): ReactElement {
  return (
    <section
      className={props.wide ? "dash-widget dash-wide" : "dash-widget"}
      aria-labelledby={props.id}
    >
      <h2 id={props.id}>
        {props.title}
        {props.more ? (
          <button type="button" className="dash-more" onClick={props.more.onClick}>
            {props.more.label} ›
          </button>
        ) : null}
      </h2>
      {props.children}
    </section>
  );
}

function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

function cellCount(c: BoardSummary["columns"][number]): string {
  return c.wip_limit === null || c.wip_scope === "per_assignee"
    ? String(c.count)
    : `${c.count}/${c.wip_limit}`;
}

function cellState(c: BoardSummary["columns"][number]): string | null {
  if (c.wip_limit === null || c.wip_scope === "per_assignee") {
    return null;
  }
  if (c.count > c.wip_limit) {
    return "▲ Over";
  }
  return c.count === c.wip_limit ? "● Full" : null;
}

/** Widget 2: the column strip, then blocked, stale and rework. */
export function BoardWidget(props: {
  readonly board: BoardSummary | null;
  readonly home: string | null;
  readonly onOpen: () => void;
}): ReactElement {
  const { board } = props;
  const more = { label: "Open board", onClick: props.onOpen };
  if (board === null) {
    return (
      <Widget id="dash-board" title="Board" wide more={more}>
        <p className="dash-empty">No board yet. Start it from the Board tab.</p>
      </Widget>
    );
  }
  const shown = board.columns.filter((c) => c.category !== "done" && c.category !== "cancelled");
  return (
    <Widget id="dash-board" title="Board" wide more={more}>
      <ul className="dash-strip" aria-label="Columns">
        {shown.map((c) => {
          const state = cellState(c);
          return (
            <li key={c.key} className={state ? "dash-cell dash-cell-alert" : "dash-cell"}>
              <button type="button" onClick={props.onOpen}>
                <b>{cellCount(c)}</b>
                <small>
                  {c.name}
                  {c.wip_scope === "per_assignee" && c.wip_limit !== null
                    ? ` · ${c.wip_limit} per bot`
                    : ""}
                  {state ? ` · ${state}` : ""}
                </small>
              </button>
            </li>
          );
        })}
        {board.done_this_week === null ? null : (
          <li className="dash-cell">
            <button type="button" onClick={props.onOpen}>
              <b>{board.done_this_week}</b>
              <small>Done this week</small>
            </button>
          </li>
        )}
      </ul>
      <p className="dash-meta-line">
        <span className="dash-tone-bad">⛔ {board.blocked} blocked</span>
        <span className="dash-tone-you">⏱ {board.stale} stale</span>
        {board.rework_this_week === null ? null : (
          <span>↺ {board.rework_this_week} rework this week</span>
        )}
      </p>
      {props.home ? (
        <p className="dash-hint">
          The board lives on {props.home}; this is its last copy here, read-only.
        </p>
      ) : null}
    </Widget>
  );
}

function releaseMeta(r: Release): string {
  const items = plural(r.items.length, "item");
  if (r.deployments.length > 0) {
    const machines = [...new Set(r.deployments.map((d) => d.machine))];
    return [
      items,
      ...machines.map((m) => `${m} ${rolloutLabel(r, m).glyph} ${rolloutLabel(r, m).word}`),
    ].join(" · ");
  }
  const tests = r.tests.map((t) => `${t.machine} ${testLabel(t.result).glyph}`);
  return tests.length ? [items, `Tests: ${tests.join(" ")}`].join(" · ") : items;
}

/** Widget 3: the current release and the last two. */
export function ReleasesWidget(props: {
  readonly releases: readonly Release[];
  readonly onOpen: () => void;
}): ReactElement {
  return (
    <Widget id="dash-releases" title="Releases" more={{ label: "Releases", onClick: props.onOpen }}>
      {props.releases.length === 0 ? (
        <p className="dash-empty">No release yet. DevOps packages the items in Verify.</p>
      ) : (
        <ul className="dash-rows">
          {props.releases.map((r) => {
            const label = statusLabel(r.status);
            return (
              <li key={r.id} className="dash-row">
                <span className="dash-glyph" aria-hidden="true">
                  ▣
                </span>
                <button
                  type="button"
                  className="dash-row-text dash-row-link"
                  onClick={props.onOpen}
                >
                  <span className="dash-row-title">
                    {releaseTitle(r)}{" "}
                    <span className={`release-pill release-tone-${label.tone}`}>
                      {label.glyph} {label.word}
                    </span>
                  </span>
                  <span className="dash-row-meta">{releaseMeta(r)}</span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </Widget>
  );
}

/** Widget 4: each bot, its state, its current item and open tasks. */
export function TeamWidget(props: {
  readonly team: readonly TeamRow[];
  /** The live bots, whose state moves between reads. */
  readonly bots: readonly Bot[];
  readonly onOpenBot: (botId: string) => void;
}): ReactElement {
  // Workers last (§2.1).
  const rows = [
    ...props.team.filter((r) => !r.bot.temporary),
    ...props.team.filter((r) => r.bot.temporary),
  ];
  return (
    <Widget id="dash-team" title="Team">
      {rows.length === 0 ? (
        <p className="dash-empty">No bots in this project yet.</p>
      ) : (
        <ul className="dash-rows">
          {rows.map((row) => {
            const bot = props.bots.find((b) => b.id === row.bot.id) ?? row.bot;
            const item = row.items[0];
            const work = item ? `${item.id} ${item.title}` : "No current item";
            return (
              <li key={bot.id} className="dash-row">
                <BotAvatar avatar={bot.avatar} name={bot.name} id={bot.id} size="sm" />
                <button
                  type="button"
                  className="dash-row-text dash-row-link"
                  onClick={() => props.onOpenBot(bot.id)}
                >
                  <span className="dash-row-title">
                    {bot.name}{" "}
                    <span className="dash-state">
                      <span className={`dot dot-${bot.state}`} aria-hidden="true" />
                      {BOT_STATE_LABEL[bot.state] ?? bot.state}
                    </span>
                  </span>
                  <span className="dash-row-meta">
                    {work}
                    {row.items.length > 1 ? ` (+${row.items.length - 1})` : ""} ·{" "}
                    {plural(row.open_tasks, "task")}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </Widget>
  );
}

/** Widgets 5 and 6, until meetings land (H-102). */
export function MeetingsWidgets(): ReactElement {
  return (
    <>
      <Widget id="dash-meetings" title="Meetings">
        <p className="dash-empty">No meetings yet.</p>
      </Widget>
      <Widget id="dash-actions" title="Action items">
        <p className="dash-empty">No open action items.</p>
      </Widget>
    </>
  );
}
