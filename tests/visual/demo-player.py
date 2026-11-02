#!/usr/bin/env python3
"""A fake MPRIS player with a fixed, fictional track, for offscreen renders.

Run it on a private session bus (dbus-run-session) next to Quickshell, so the
media module shows this track instead of whatever the user is playing:

    demo-player.py <cover.png> [call-log]

Needs python-dbus and python-gobject.
"""
import sys

import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib

ROOT = "org.mpris.MediaPlayer2"
PLAYER = "org.mpris.MediaPlayer2.Player"
PROPS = "org.freedesktop.DBus.Properties"


class DemoPlayer(dbus.service.Object):
    def __init__(self, bus, cover, log=None):
        super().__init__(bus, "/org/mpris/MediaPlayer2")
        self.log = log
        self.props = {
            ROOT: {
                "Identity": "notchd demo",
                "CanQuit": False,
                "CanRaise": False,
                "HasTrackList": False,
                "SupportedUriSchemes": dbus.Array([], signature="s"),
                "SupportedMimeTypes": dbus.Array([], signature="s"),
            },
            PLAYER: {
                "PlaybackStatus": "Playing",
                "Metadata": dbus.Dictionary({
                    "mpris:trackid": dbus.ObjectPath("/org/notchd/demo/1"),
                    "xesam:title": "Night Drive",
                    "xesam:artist": dbus.Array(["Neon Harbor"], signature="s"),
                    "xesam:album": "Coastline",
                    "mpris:length": dbus.Int64(214_000_000),
                    "mpris:artUrl": "file://" + cover,
                }, signature="sv"),
                "Position": dbus.Int64(92_000_000),
                "Rate": 1.0,
                "Volume": 1.0,
                "CanGoNext": True,
                "CanGoPrevious": True,
                "CanPlay": True,
                "CanPause": True,
                "CanSeek": True,
                "CanControl": True,
            },
        }

    @dbus.service.method(PROPS, in_signature="ss", out_signature="v")
    def Get(self, interface, name):
        return self.props[interface][name]

    @dbus.service.method(PROPS, in_signature="s", out_signature="a{sv}")
    def GetAll(self, interface):
        return self.props.get(interface, {})

    @dbus.service.method(PROPS, in_signature="ssv")
    def Set(self, interface, name, value):
        pass

    @dbus.service.signal(PROPS, signature="sa{sv}as")
    def PropertiesChanged(self, interface, changed, invalidated):
        pass

    # The transport controls do nothing to the picture (the render is a
    # still) unless a log file is given: then each call is appended to it, one
    # line ("PlayPause", "Next", "Previous", "SetPosition <microseconds>"), and
    # PlayPause flips the status, as a test of the controls needs.
    def called(self, line):
        if self.log:
            with open(self.log, "a") as f:
                f.write(line + "\n")

    @dbus.service.method(PLAYER)
    def PlayPause(self):
        self.called("PlayPause")
        if self.log:
            status = self.props[PLAYER]["PlaybackStatus"]
            status = "Paused" if status == "Playing" else "Playing"
            self.props[PLAYER]["PlaybackStatus"] = status
            self.PropertiesChanged(PLAYER, {"PlaybackStatus": status}, [])

    @dbus.service.method(PLAYER)
    def Next(self):
        self.called("Next")

    @dbus.service.method(PLAYER)
    def Previous(self):
        self.called("Previous")

    @dbus.service.method(PLAYER, in_signature="ox")
    def SetPosition(self, track, position):
        self.called("SetPosition %d" % position)
        self.props[PLAYER]["Position"] = dbus.Int64(position)


def main():
    DBusGMainLoop(set_as_default=True)
    bus = dbus.SessionBus()
    name = dbus.service.BusName(ROOT + ".notchddemo", bus)  # noqa: F841 (keeps the name)
    DemoPlayer(bus, sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else None)
    GLib.MainLoop().run()


if __name__ == "__main__":
    main()
