import QtQuick
import "client"

// A NotchModule whose logic lives in the daemon (docs/modules.md, "A module in
// the daemon"): the timer, the activities, the resources and the OSD, and any
// user module, which the daemon runs as a process. Nothing here decides
// anything. Its facts (title, priority, icon, attention...) are the daemon's
// `module` message, each slot's view is the daemon's `view` tree drawn by
// ViewTree, and a press in a view goes back to the daemon as an `event`. To
// the notch it is a module like any other: tabs, compact views, peeks, the
// settings page.
//
// The service makes one for every module the daemon announces that has no QML
// file, as the notch needs it (NotchService.loadUrl); a module the host ships
// that only owns an IPC target (the Omarchy `osd`) is a CoreModule with that
// target inside it. `moduleId` and `notch` are set as for any module.
//
// A slot has a view while the daemon sends a tree for it; with none the
// property is null, as a module without that view has.
NotchModule {
  id: module
  daemonOwned: true

  readonly property var daemon: module.notch ? module.notch.client : null
  // The daemon's last `module` message of this module, or null.
  readonly property var info: module.daemon && module.moduleId !== "" ? module.daemon.module(module.moduleId) : null

  title: module.info ? module.info.title : module.moduleId
  priority: module.info ? module.info.priority : 0
  active: module.info ? module.info.active : false
  icon: module.info ? module.info.icon : ""
  attention: module.info ? module.info.attention : false
  urgentPeek: module.info ? module.info.urgentPeek : false
  peekOnFocusedScreen: module.info ? module.info.peekOnFocusedScreen : false
  expandedWidth: module.info && module.info.expandedWidth > 0 ? module.info.expandedWidth : 400
  // The daemon's own words about it: an error while it is failing.
  readonly property string error: module.info && module.info.error ? module.info.error : ""

  // The tree the daemon sent for a slot, or null.
  function tree(slot) { return module.daemon && module.moduleId !== "" ? module.daemon.view(module.moduleId, slot) : null }
  function has(slot) { return module.tree(slot) !== null && module.tree(slot) !== undefined }

  compact: module.has("compact") ? compactView : null
  verticalCompact: module.has("verticalCompact") ? verticalCompactView : null
  peek: module.has("peek") ? peekView : null
  verticalPeek: module.has("verticalPeek") ? verticalPeekView : null
  expanded: module.has("expanded") ? expandedView : null

  // One component per slot, made once: the notch swaps its content when a
  // slot's Component changes, so a new tree must not make a new one.
  component SlotView: ViewTree {
    property string slot: ""
    // What a surface (a terminal) in the view asks its daemon of: the service.
    readonly property var surfaceHost: module.notch
    tree: module.tree(slot)
    onAction: (actionId, value) => { if (module.daemon) module.daemon.event(module.moduleId, slot, actionId, value) }
  }
  Component { id: compactView; SlotView { slot: "compact" } }
  Component { id: verticalCompactView; SlotView { slot: "verticalCompact" } }
  Component { id: peekView; SlotView { slot: "peek" } }
  Component { id: verticalPeekView; SlotView { slot: "verticalPeek" } }
  Component { id: expandedView; SlotView { slot: "expanded" } }
}
