/**
 * The tabs of a project window (UX-024 §2): the first five are in the tab bar
 * on ⌘1…⌘5; the rest sit under More.
 */
const PROJECT_TABS = [
  "overview",
  "board",
  "team",
  "releases",
  "meetings",
  "conversations",
  "settings",
] as const;

export type ProjectTab = (typeof PROJECT_TABS)[number];

/** The tabs in the tab bar itself, in ⌘1…⌘5 order. */
export const PRIMARY_TABS = PROJECT_TABS.slice(0, 5);

/** The tabs under More. */
export const MORE_TABS = PROJECT_TABS.slice(5);

/** What the main pane is currently showing. */
export type Selection =
  /** The projects home, ranked by what needs the owner: where the app opens (H-133). */
  | { readonly kind: "home" }
  /**
   * A bot's page. `tab: "chat"` opens it on its Chat, the way back from the
   * main chat (H-192); without it, the page opens on its first tab. `press`
   * counts those asks, so a second one lands on Chat even when the page is
   * already open and the owner moved to another tab (UX-034).
   */
  | {
      readonly kind: "bot";
      readonly botId: string;
      readonly tab?: "chat";
      readonly press?: number;
    }
  /**
   * A project window. Without `tab` it reopens on the tab last used there,
   * which is the Dashboard the first time.
   */
  | { readonly kind: "project"; readonly projectId: string; readonly tab?: ProjectTab }
  /**
   * The Control center. `decisionId` opens straight onto one record, which is
   * what a notification or a palette entry hands over.
   */
  | { readonly kind: "control"; readonly decisionId?: string };

/** Narrows a stored string to a project tab. */
export function isProjectTab(value: string): value is ProjectTab {
  return PROJECT_TABS.some((tab) => tab === value);
}
