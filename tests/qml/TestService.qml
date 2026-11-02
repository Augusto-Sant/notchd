import QtQuick
import Quickshell
import "core"

// A NotchService with a daemon of its own (the scripts copy this file next to
// the shell.qml they run, so `core` resolves from there), for the tests and renders that
// create services (the daemon is the authority for the config, the schema and
// IPC, so a service without one shows "notchd not running"). It runs the real
// binary (`notchd daemon --ensure`, the launcher's own path) on a scratch
// socket, with the service's `configPath` as its config, so a test keeps
// naming the file it writes and reads. The scripts that run the tests set:
//   NOTCHD_TEST_DAEMON   the notchd binary (`make daemon`)
//   NOTCHD_TEST_SOCKETS  a scratch directory for the sockets; the script kills
//                        every daemon whose socket is in it when the run ends
// `configPath` must be set (a file in the run's scratch dir): it names the
// daemon's socket and instance too.
NotchService {
  id: test

  readonly property string daemonBin: Quickshell.env("NOTCHD_TEST_DAEMON")
  readonly property string socketDir: Quickshell.env("NOTCHD_TEST_SOCKETS")
  readonly property string daemonName: String(test.configPath).replace(/^.*\//, "").replace(/\.json$/, "")
  readonly property string daemonSocket: test.socketDir + "/" + test.daemonName + ".sock"

  // The daemon up and its config in: what a case waits for before it starts.
  readonly property bool daemonReady: test.client.ready && test.configResolved

  daemonDownDelay: 400
  daemonCommand: [test.daemonBin, "--socket", test.daemonSocket, "--config", test.configPath, "daemon", "--ensure"]
}
