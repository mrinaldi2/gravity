// The dashboard's two drawers, a release's review and an item (U4): one at a
// time, opening one closes the other, and the Review or "Open item" button
// that opened it gets focus back when it closes (UX-010, UX-012).

import { useCallback, useRef, useState } from "react";
import type { Release } from "../../protocol/releases";

export interface DashboardDrawers {
  readonly reviewing: Release | null;
  readonly openItem: string | null;
  readonly openReview: (release: Release) => void;
  readonly showItem: (itemId: string, opener: HTMLElement) => void;
  readonly close: () => void;
}

export function useDashboardDrawers(): DashboardDrawers {
  const [reviewing, setReviewing] = useState<Release | null>(null);
  const [openItem, setOpenItem] = useState<string | null>(null);
  const opener = useRef<HTMLElement | null>(null);
  const openReview = useCallback((release: Release): void => {
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setOpenItem(null);
    setReviewing(release);
  }, []);
  const showItem = useCallback((itemId: string, button: HTMLElement): void => {
    opener.current = button;
    setReviewing(null);
    setOpenItem(itemId);
  }, []);
  const close = useCallback((): void => {
    setReviewing(null);
    setOpenItem(null);
    if (opener.current?.isConnected) {
      opener.current.focus();
    }
    opener.current = null;
  }, []);
  return { reviewing, openItem, openReview, showItem, close };
}
