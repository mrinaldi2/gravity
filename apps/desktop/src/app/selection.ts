import type { BotTab } from "../components/bot/BotTabs";

/**
 * The tabs of a project window (UX-024 §2, UX-051 §1): the first six are in
 * the tab bar on ⌘1…⌘6; the rest sit under More. Pull requests shows only when
 * the service serves them.
 */
const PROJECT_TABS = [
  "overview",
  "board",
  "prs",
  "team",
  "releases",
  "meetings",
  "conversations",
  "settings",
] as const;

export type ProjectTab = (typeof PROJECT_TABS)[number];

const WITH_PRS: readonly ProjectTab[] = PROJECT_TABS.slice(0, 6);
const WITHOUT_PRS: readonly ProjectTab[] = WITH_PRS.filter((tab) => tab !== "prs");

/** The tabs in the tab bar itself, in ⌘1… order; `prs` when the service serves pull requests. */
export function primaryTabs(prs: boolean): readonly ProjectTab[] {
  return prs ? WITH_PRS : WITHOUT_PRS;
}

/** The tabs under More. */
export const MORE_TABS = PROJECT_TABS.slice(6);

/** What the main pane is currently showing. */
export type Selection =
  /** The projects home, ranked by what needs the owner: where the app opens (H-133). */
  | { readonly kind: "home" }
  /**
   * A bot's page. `tab: "chat"` opens it on its Chat, the way back from the
   * main chat (H-192); without it, the page opens on its first tab. History
   * records the tab the owner moved to, so going back reopens it. `press`
   * counts those asks, so a second one lands on Chat even when the page is
   * already open and the owner moved to another tab (UX-034).
   */
  | {
      readonly kind: "bot";
      readonly botId: string;
      readonly tab?: BotTab;
      readonly press?: number;
    }
  /**
   * A project window. Without `tab` it reopens on the tab last used there,
   * which is the Dashboard the first time. `item` opens that card's drawer
   * on the Board ("Open on the board", UX-035 §4).
   */
  | {
      readonly kind: "project";
      readonly projectId: string;
      readonly tab?: ProjectTab;
      readonly item?: string;
      /** Pull requests: open this PR (Needs you's Review…, H-277). */
      readonly pr?: number;
      /** With `pr`: open its delta since your approval (a Re-check row). */
      readonly recheck?: boolean;
      /** Settings: focus this section ("Change" on a PR's Owner review line). */
      readonly section?: "owner_review";
    }
  /**
   * The Control center. `decisionId` opens straight onto one record, which is
   * what a notification or a palette entry hands over.
   */
  | { readonly kind: "control"; readonly decisionId?: string };

/** Narrows a stored string to a project tab. */
export function isProjectTab(value: string): value is ProjectTab {
  return PROJECT_TABS.some((tab) => tab === value);
}
