/**
 * The one question a migrating service install asks. Installing the 0.15
 * service on a machine that still has `~/.gravity` moves it to `~/.thehermes`
 * and restarts every bot, so it waits for an explicit yes. Whoever starts the
 * install awaits {@link confirmHomeMigration}; the host component mounted at
 * the app root shows the dialog.
 */

export interface HomeMigrationRequest {
  readonly summary: string;
  readonly answer: (confirmed: boolean) => void;
}

type Listener = (request: HomeMigrationRequest | null) => void;

let listener: Listener | null = null;

/** Thrown by an install the user declined; nothing was changed. */
export class InstallCancelled extends Error {
  constructor() {
    super("Cancelled. The Hermes service was not updated and nothing was moved.");
    this.name = "InstallCancelled";
  }
}

/**
 * Resolves with the user's answer. With no dialog host mounted there is no
 * one to ask, and the answer is no.
 */
export function confirmHomeMigration(summary: string): Promise<boolean> {
  const show = listener;
  if (show === null) {
    return Promise.resolve(false);
  }
  return new Promise((resolve) => {
    show({
      summary,
      answer: (confirmed) => {
        show(null);
        resolve(confirmed);
      },
    });
  });
}

/** Registers the dialog host; returns the unsubscribe. */
export function onHomeMigrationRequest(next: Listener): () => void {
  listener = next;
  return (): void => {
    if (listener === next) {
      listener = null;
    }
  };
}
