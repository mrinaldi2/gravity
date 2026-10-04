/** The tabs of a project window, in tab-bar and ⌘1…⌘6 order. */
export const PROJECT_TABS = [
  "dashboard",
  "board",
  "releases",
  "meetings",
  "conversations",
  "settings",
] as const;

export type ProjectTab = (typeof PROJECT_TABS)[number];

/** What the main pane is currently showing. */
export type Selection =
  | { readonly kind: "none" }
  | { readonly kind: "bot"; readonly botId: string }
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
