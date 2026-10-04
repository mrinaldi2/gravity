import { useEffect, useState } from "react";
import type { ReactElement } from "react";
import { onHomeMigrationRequest } from "../../app/homeMigration";
import type { HomeMigrationRequest } from "../../app/homeMigration";
import ConfirmDialog from "./ConfirmDialog";

/**
 * Hosts the confirmation for a service install that migrates the home.
 * Cancel holds the focus, as in every disruptive prompt.
 */
export default function HomeMigrationConfirm(): ReactElement | null {
  const [request, setRequest] = useState<HomeMigrationRequest | null>(null);

  useEffect(() => onHomeMigrationRequest(setRequest), []);

  if (request === null) {
    return null;
  }
  return (
    <ConfirmDialog
      title="Move your Hermes data?"
      body={`Updating the Hermes service migrates this computer. ${request.summary}`}
      confirmLabel="Move and restart"
      onConfirm={() => {
        request.answer(true);
      }}
      onCancel={() => {
        request.answer(false);
      }}
    />
  );
}
