import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { ownerDaemon } from "../../test/ownerReviewFixtures";
import OwnerReviewSettings from "./OwnerReviewSettings";

describe("Settings › Owner review (AC4)", () => {
  it("starts on Every pull request and saves Some areas from reviewers.toml", async () => {
    const client = ownerDaemon();
    render(<OwnerReviewSettings client={client} projectId="p1" connected focus />);
    const every = await screen.findByRole("radio", { name: /^Every pull request/ });
    await waitFor(() => expect(every).toBeChecked());
    expect(screen.getByRole("heading", { name: "Owner review" })).toHaveFocus();
    expect(screen.getByText(/You approve every release either way\./)).toBeInTheDocument();
    const save = screen.getByRole("button", { name: "Save" });
    expect(save).toBeDisabled();

    await userEvent.click(screen.getByRole("radio", { name: /^Some areas/ }));
    const areas = screen.getByRole("group", { name: "Areas" });
    expect(areas).toHaveTextContent("securityreleasesdocsdesktop");
    await userEvent.click(screen.getByRole("checkbox", { name: "security" }));
    await userEvent.click(screen.getByRole("checkbox", { name: "docs" }));
    await userEvent.click(save);

    expect(
      client.requests.map((r) => r.body).filter((b) => b.type === "review_settings_set"),
    ).toEqual([
      {
        type: "review_settings_set",
        project_id: "p1",
        owner_review: "areas",
        owner_review_areas: ["security", "docs"],
      },
    ]);
    expect(await screen.findByRole("status")).toHaveTextContent("Saved.");
    expect(screen.getByRole("radio", { name: /^Some areas/ })).toBeChecked();
  });

  it("saves None with no areas, and is the owner's only", async () => {
    const client = ownerDaemon();
    const { unmount } = render(<OwnerReviewSettings client={client} projectId="p1" connected />);
    await userEvent.click(await screen.findByRole("radio", { name: /^None/ }));
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(client.requests.at(-1)?.body).toEqual({
      type: "review_settings_set",
      project_id: "p1",
      owner_review: "none",
      owner_review_areas: [],
    });
    unmount();

    client.grants = ["read", "control"];
    render(<OwnerReviewSettings client={client} projectId="p1" connected />);
    expect(await screen.findByText(/Only the owner can change this/)).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /^None/ })).toBeDisabled();
  });
});
