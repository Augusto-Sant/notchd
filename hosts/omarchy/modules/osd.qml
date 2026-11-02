import QtQuick
import Quickshell.Io
import "../../../core"

// Volume, brightness, mic, keyboard light... Every `omarchy osd` call arrives
// as `omarchy-shell osd show <json>` on the "osd" IPC target, which this file
// registers. The stock omarchy.osd plugin owns that target too, so it has to
// be disabled (`omarchy plugin disable omarchy.osd`) for the notch to get it.
//
// The module itself, what it shows and how it reads the payload, is the
// daemon's (native/notchd-modules/src/osd.rs): this is the one thing a host
// must keep, the IPC target. It forwards each call to the daemon as the `osd`
// method and answers as the target always has. An IPC handler cannot wait for
// the daemon, so `show` and `close` answer `ok` at once; `state` is read from
// the notch state, which this side has.
CoreModule {
  id: osd

  IpcHandler {
    target: "osd"
    function show(payloadJson: string): string { return osd.forward(["show", String(payloadJson || "{}")]) }
    function close(): string { return osd.forward(["close"]) }
    function state(): string { return osd.notch && osd.notch.peeking(osd) ? "open" : "closed" }
    function ping(): string { return "ok" }
  }

  function forward(args) {
    return osd.notch && osd.notch.notchIpc("osd", args) ? "ok" : "notchd is not running"
  }
}
