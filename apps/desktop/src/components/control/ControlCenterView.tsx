import type { ReactElement } from "react";
import PermissionCards from "../permissions/PermissionCards";
import ControlBody from "./ControlBody";
import ControlTop from "./ControlTop";
import DeleteDecisionDialog from "./DeleteDecisionDialog";
import OwnerActionList from "../ownerActions/OwnerActionList";
import ReleaseReview from "../releases/ReleaseReview";
import { botNamer, useItemTitles } from "../releases/ReleasesView";
import { useDecisionRelease } from "../releases/useDecisionRelease";
import ReadingPane from "./ReadingPane";
import { useControlCenter } from "./useControlCenter";
import type { ControlCenterOptions } from "./useControlCenter";

/** One inbox for every question a bot has asked, across every project. */
export default function ControlCenterView(props: ControlCenterOptions): ReactElement {
  const cc = useControlCenter(props);
  const { api, state, actions, notify, canControl } = { ...cc, canControl: props.canControl };
  const linked = useDecisionRelease(props.client, props.connected, state.reading, props.onToast);
  const titles = useItemTitles(
    props.client,
    linked.release?.project_id ?? "",
    linked.release !== undefined,
  );
  const release =
    linked.release === undefined ? undefined : (
      <ReleaseReview
        key={linked.release.id}
        release={linked.release}
        titles={titles}
        botName={botNamer(props.bots)}
        actions={linked.actions}
        canControl={canControl}
      />
    );

  const pane = (showBack: boolean): ReactElement => (
    <ReadingPane
      decision={state.reading}
      showBack={showBack}
      now={cc.now}
      canControl={canControl}
      api={api}
      state={state}
      composer={cc.composer}
      actions={actions}
      notify={notify}
      candidatesFor={cc.candidatesFor}
      botAvatar={cc.botAvatar}
      textareaRef={cc.textareaRef}
      onDelete={cc.setDeleting}
      release={release}
      ownerActions={
        state.reading === undefined ? undefined : (
          <OwnerActionList
            client={props.client}
            connected={props.connected}
            scope={{ projectId: state.reading.project_id, decisionId: state.reading.id }}
            addToast={props.onToast}
            botName={(id) => props.bots.find((bot) => bot.id === id)?.name ?? "a bot"}
          />
        )
      }
    />
  );

  return (
    <div className="control-center">
      <ControlTop
        api={api}
        state={state}
        actions={actions}
        notify={notify}
        candidatesFor={cc.candidatesFor}
        canControl={canControl}
      />
      {props.permissions === undefined ? null : (
        <PermissionCards
          permissions={props.permissions}
          canAnswer={props.connected && canControl}
          botName={(botId) => props.bots.find((bot) => bot.id === botId)?.name}
          onOpenBot={props.onOpenBot}
        />
      )}
      <ControlBody
        api={api}
        state={state}
        now={cc.now}
        canControl={canControl}
        tagAdmin={cc.tagAdmin}
        filter={cc.filter}
        onQuery={cc.setQuery}
        onTagFilter={cc.setTagFilter}
        onProjectFilter={cc.setProjectFilter}
        onBotFilter={cc.setBotFilter}
        searchRef={cc.searchRef}
        pane={pane}
      />
      {cc.deleting === undefined ? null : (
        <DeleteDecisionDialog
          decision={cc.deleting}
          onCancel={() => cc.setDeleting(undefined)}
          onConfirm={(id) => {
            cc.setDeleting(undefined);
            void api.remove(id);
          }}
        />
      )}
    </div>
  );
}
