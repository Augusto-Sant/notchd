import QtQuick
import Quickshell
import Quickshell.Io
import "Components.js" as Components

// The optional components (core/components/components.json: LocalAI and its
// voices, Voxtype, system packages) and their state, read from the
// installer (core/components/install.py status). The settings rows bind
// `catalog` and `states`; `install`, `cancel` and `remove` run the installer,
// which does the work detached, so closing the settings or the shell does not
// stop an installation, and the state is there when they are back. While the
// settings are `watching` and something installs, the state is read every
// second. Rules: Components.js.
Scope {
  id: store

  // The installer script (install.py, or a test's fake) and the registry.
  property string installer: ""
  property string catalogPath: ""
  // The settings window is open.
  property bool watching: false

  // [{ id, title, kind, size, ... }] and { id: state } (Components.view reads one).
  property var catalog: []
  property var states: ({})
  // The state was read at least once.
  property bool loaded: false
  // The installed ids, and the states as they were when that set last changed:
  // both stay put while an installation only progresses, so what is built
  // from them (the settings schema) is not rebuilt every second.
  property var installed: []
  property var settled: ({})
  property string installedKey: ""

  function refresh() {
    if (store.installer !== "" && !statusProcess.running) statusProcess.running = true
  }
  function install(id) { store.act("install", id) }
  function cancel(id) { store.act("cancel", id) }
  function remove(id) { store.act("remove", id) }
  function act(verb, id) {
    if (store.installer === "") return
    actionProcess.createObject(store, { command: Components.installerCommand(store.installer, [verb, id]), running: true })
  }

  function apply(text) {
    var s = Components.parseStatus(text)
    if (Object.keys(s).length === 0) return
    store.states = s
    var ids = Components.installedIds(s)
    var key = ids.join(",")
    if (!store.loaded || key !== store.installedKey) {
      store.installedKey = key
      store.installed = ids
      store.settled = s
    }
    store.loaded = true
  }

  onWatchingChanged: if (store.watching) store.refresh()

  FileView {
    path: store.catalogPath
    blockLoading: true
    printErrors: false
    onLoaded: store.catalog = Components.parseCatalog(text())
  }

  Process {
    id: statusProcess
    command: Components.installerCommand(store.installer, ["status"])
    stdout: StdioCollector { onStreamFinished: store.apply(text) }
  }

  Component {
    id: actionProcess
    Process {
      id: action
      onExited: {
        store.refresh()
        action.destroy()
      }
    }
  }

  Timer {
    interval: 1000
    repeat: true
    running: store.watching && Components.anyActive(store.states)
    onTriggered: store.refresh()
  }
}
