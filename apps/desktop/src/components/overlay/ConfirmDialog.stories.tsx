import type { Story } from "@ladle/react";
import ConfirmDialog from "./ConfirmDialog";

const noop = (): void => {};

export const DeleteBot: Story = () => (
  <ConfirmDialog
    title="Delete alice?"
    body="Its session is stopped and the bot is archived along with its conversations."
    confirmLabel="Delete bot"
    onConfirm={noop}
    onCancel={noop}
  />
);
