import type { Story } from "@ladle/react";
import type { ReactElement } from "react";
import type { AddToast } from "../../app/useToasts";
import { FakeDaemon } from "../../test/fakeDaemon";
import { bot, project } from "../../test/fixtures";
import { HOME_NOW, overviewJson } from "../../test/homeFixtures";
import ProjectsHome from "./ProjectsHome";
import Rail from "./Rail";

const noop = (): void => {};
const noToast: AddToast = () => {};

function Home({ legacy = false }: { readonly legacy?: boolean }): ReactElement {
  const client = new FakeDaemon().onRequest("projects_overview", () => ({
    type: "projects_overview",
    req_id: "1",
    overview: overviewJson(),
  }));
  if (!legacy) {
    client.capabilities = [...client.capabilities, "projects_overview"];
  }
  return (
    <div className="app" style={{ height: 700 }}>
      <Rail selection={{ kind: "home" }} needsYou={5} onSelect={noop} onOpenSettings={noop} />
      <main className="main">
        <ProjectsHome
          client={client}
          projects={[
            project({ id: "p1", name: "The Hermes", lead_bot_id: "b1" }),
            project({ id: "p2", name: "PhD" }),
          ]}
          bots={[bot({ id: "b1", name: "Team Lead", project_id: "p1" })]}
          connected
          canControl
          addToast={noToast}
          onOpenProject={noop}
          onCreateProject={async () => {}}
          now={HOME_NOW}
        />
      </main>
    </div>
  );
}

/** Ranked by needs-you: the first card is outlined; PhD's imac is away. */
export const Ranked: Story = () => <Home />;

/** An older service: names only, "Update needed for full info". */
export const OlderService: Story = () => <Home legacy />;
