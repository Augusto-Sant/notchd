//! MPRIS over D-Bus (media provider): every `org.mpris.MediaPlayer2.*` name on
//! the session bus is a player.
//!
//! Two threads, none of them the hub's. A reader blocks on the signals that
//! mean a player changed (`PropertiesChanged` of an MPRIS object, `Seeked`,
//! an MPRIS name coming or going) and only says "look again". The worker,
//! which also takes the commands, reads each player's properties on that
//! word, after 40 ms so a burst is one read, and hands the sink the players
//! when they differ from what it last sent ([`crate::media::changed`]). A
//! playing position moves by itself, so the worker reads again every 5 s only
//! to catch a seek that went unsignalled. Nothing is read while no module
//! wants the players, and the bus is not even opened until one does.

use crate::media::{MediaCommand, MediaData, MediaSink, Player, changed};
use notchd_proto::Num;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};
use zbus::blocking::{Connection, MessageIterator, connection::Builder};
use zbus::message::Type as MessageType;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const ROOT: &str = "org.mpris.MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const PROPS: &str = "org.freedesktop.DBus.Properties";
const BUS: &str = "org.freedesktop.DBus";

/// How long a call to a player may take before it is given up on: a hung
/// player must not stall the others.
const CALL_TIMEOUT: Duration = Duration::from_secs(2);
/// Quiet time after a signal, so a burst of them is read once.
const DEBOUNCE: Duration = Duration::from_millis(40);
/// Re-read while something plays, for a seek the player did not announce.
const PLAYING_EVERY: Duration = Duration::from_secs(5);
/// Re-read otherwise (a player that never signals); also how often a bus
/// that was not there is tried again.
const IDLE_EVERY: Duration = Duration::from_secs(60);

pub fn start(wanted: Box<dyn Fn() -> bool + Send>, sink: MediaSink) -> Sender<MediaCommand> {
    let (tx, rx) = channel();
    let own = tx.clone();
    let spawned = thread::Builder::new()
        .name("notchd-mpris".into())
        .spawn(move || Worker::new(own, rx, wanted, sink).run());
    if let Err(e) = spawned {
        eprintln!("notchd: cannot start the media reader: {e}");
    }
    tx
}

struct Worker {
    /// Handed to the reader, which sends `Refresh` on a signal.
    wake: Sender<MediaCommand>,
    rx: Receiver<MediaCommand>,
    wanted: Box<dyn Fn() -> bool + Send>,
    sink: MediaSink,
    conn: Option<Connection>,
    /// A player's `Identity`, per bus name, until its name goes.
    identities: HashMap<String, String>,
    /// What the sink last got, and when.
    last: Option<(MediaData, Instant)>,
    /// Said once, not on every retry.
    complained: bool,
}

impl Worker {
    fn new(
        wake: Sender<MediaCommand>,
        rx: Receiver<MediaCommand>,
        wanted: Box<dyn Fn() -> bool + Send>,
        sink: MediaSink,
    ) -> Worker {
        Worker {
            wake,
            rx,
            wanted,
            sink,
            conn: None,
            identities: HashMap::new(),
            last: None,
            complained: false,
        }
    }

    fn run(mut self) {
        loop {
            let playing = self
                .last
                .as_ref()
                .is_some_and(|(d, _)| d.players.iter().any(|p| p.playing));
            let wait = if playing { PLAYING_EVERY } else { IDLE_EVERY };
            let mut batch = Vec::new();
            match self.rx.recv_timeout(wait) {
                Ok(c) => batch.push(c),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if !batch.is_empty() {
                thread::sleep(DEBOUNCE);
                batch.extend(self.rx.try_iter());
            }
            if !(self.wanted)() {
                // Forget what was sent: the next time a module wants the
                // players it gets them whole.
                self.last = None;
                continue;
            }
            if self.connection().is_none() {
                continue;
            }
            for c in batch.iter().filter(|c| **c != MediaCommand::Refresh) {
                self.command(c);
            }
            self.refresh();
        }
    }

    /// The bus, opened on first use.
    fn connection(&mut self) -> Option<&Connection> {
        if self.conn.is_none() {
            match open() {
                Ok(c) => {
                    self.listen(&c);
                    self.conn = Some(c);
                }
                Err(e) => {
                    if !self.complained {
                        eprintln!("notchd: media: no session bus ({e}); retrying");
                        self.complained = true;
                    }
                }
            }
        }
        self.conn.as_ref()
    }

    /// The reader: tells the worker to look again on any signal the rules
    /// below let through.
    fn listen(&self, conn: &Connection) {
        for rule in [
            format!("type='signal',path='{PATH}',interface='{PROPS}'"),
            format!("type='signal',path='{PATH}',interface='{PLAYER}'"),
            format!(
                "type='signal',sender='{BUS}',interface='{BUS}',member='NameOwnerChanged',arg0namespace='{ROOT}'"
            ),
        ] {
            if let Err(e) = conn.call_method(
                Some(BUS),
                "/org/freedesktop/DBus",
                Some(BUS),
                "AddMatch",
                &(rule.as_str(),),
            ) {
                eprintln!("notchd: media: cannot subscribe to signals: {e}");
            }
        }
        let wake = self.wake.clone();
        let iter = MessageIterator::from(conn.clone());
        let spawned = thread::Builder::new()
            .name("notchd-mpris-signals".into())
            .spawn(move || {
                for msg in iter {
                    let Ok(msg) = msg else { break };
                    // Replies to our own calls pass here too: only signals.
                    if msg.header().message_type() == MessageType::Signal
                        && wake.send(MediaCommand::Refresh).is_err()
                    {
                        break;
                    }
                }
            });
        if let Err(e) = spawned {
            eprintln!("notchd: cannot start the media signal reader: {e}");
        }
    }

    fn command(&self, c: &MediaCommand) {
        let Some(conn) = &self.conn else { return };
        let result = match c {
            MediaCommand::Refresh => return,
            MediaCommand::PlayPause { player } => ask(conn, player, "PlayPause"),
            MediaCommand::Next { player } => ask(conn, player, "Next"),
            MediaCommand::Previous { player } => ask(conn, player, "Previous"),
            MediaCommand::Seek { player, seconds } => seek(conn, player, *seconds),
        };
        if let Err(e) = result {
            eprintln!("notchd: media: {c:?}: {e}");
        }
    }

    fn refresh(&mut self) {
        let Some(conn) = self.conn.clone() else {
            return;
        };
        let data = match self.read(&conn) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("notchd: media: cannot list the players: {e}");
                return;
            }
        };
        let news = match &self.last {
            None => true,
            Some((prev, at)) => changed(prev, at.elapsed().as_millis() as u64, &data),
        };
        if news {
            self.last = Some((data.clone(), Instant::now()));
            (self.sink)(data);
        }
    }

    fn read(&mut self, conn: &Connection) -> zbus::Result<MediaData> {
        let mut names: Vec<String> = conn
            .call_method(
                Some(BUS),
                "/org/freedesktop/DBus",
                Some(BUS),
                "ListNames",
                &(),
            )?
            .body()
            .deserialize()?;
        names.retain(|n| n.starts_with(PREFIX));
        names.sort();
        self.identities.retain(|n, _| names.contains(n));
        let mut players = Vec::new();
        for name in names {
            // A player that went away between the list and the read, or one
            // that does not answer, is left out.
            let Ok(props) = all(conn, &name) else {
                continue;
            };
            let identity = match self.identities.get(&name) {
                Some(i) => i.clone(),
                None => {
                    let i = identity(conn, &name);
                    self.identities.insert(name.clone(), i.clone());
                    i
                }
            };
            players.push(parse_player(&name, &identity, &props));
        }
        Ok(MediaData { players })
    }
}

fn open() -> zbus::Result<Connection> {
    Builder::session()?.method_timeout(CALL_TIMEOUT).build()
}

fn all(conn: &Connection, name: &str) -> zbus::Result<HashMap<String, OwnedValue>> {
    conn.call_method(Some(name), PATH, Some(PROPS), "GetAll", &(PLAYER,))?
        .body()
        .deserialize()
}

fn identity(conn: &Connection, name: &str) -> String {
    let said = conn
        .call_method(Some(name), PATH, Some(PROPS), "Get", &(ROOT, "Identity"))
        .and_then(|m| m.body().deserialize::<OwnedValue>())
        .ok()
        .map(|v| text(&v));
    match said {
        Some(s) if !s.is_empty() => s,
        _ => name.strip_prefix(PREFIX).unwrap_or(name).to_string(),
    }
}

fn ask(conn: &Connection, name: &str, method: &str) -> zbus::Result<()> {
    conn.call_method(Some(name), PATH, Some(PLAYER), method, &())
        .map(|_| ())
}

/// `SetPosition` needs the track's id, which the metadata has; a player
/// without one is told the distance (`Seek`) instead.
fn seek(conn: &Connection, name: &str, seconds: f64) -> zbus::Result<()> {
    let micros = (seconds.max(0.0) * 1e6) as i64;
    let props = all(conn, name)?;
    let track = props
        .get("Metadata")
        .and_then(|m| metadata(m).remove("mpris:trackid"))
        .map(|v| text(&v))
        .and_then(|s| ObjectPath::try_from(s).ok());
    if let Some(track) = track {
        return conn
            .call_method(
                Some(name),
                PATH,
                Some(PLAYER),
                "SetPosition",
                &(track, micros),
            )
            .map(|_| ());
    }
    let now = props.get("Position").and_then(|v| int(v)).unwrap_or(0);
    conn.call_method(Some(name), PATH, Some(PLAYER), "Seek", &(micros - now,))
        .map(|_| ())
}

/// The value inside the variants around it.
fn unwrap<'a, 'v>(v: &'a Value<'v>) -> &'a Value<'v> {
    match v {
        Value::Value(inner) => unwrap(inner),
        other => other,
    }
}

/// A string, from a `s`, an `o`, or one of those in a variant.
fn text(v: &Value) -> String {
    match v {
        Value::Str(s) => s.to_string(),
        Value::ObjectPath(p) => p.to_string(),
        Value::Value(inner) => text(inner),
        _ => String::new(),
    }
}

/// An integer, from any of the widths a player might use (Chrome sends its
/// lengths unsigned, some players as floating point).
fn int(v: &Value) -> Option<i64> {
    Some(match v {
        Value::I64(n) => *n,
        Value::U64(n) => i64::try_from(*n).unwrap_or(i64::MAX),
        Value::I32(n) => i64::from(*n),
        Value::U32(n) => i64::from(*n),
        Value::F64(f) if f.is_finite() => *f as i64,
        Value::Value(inner) => return int(inner),
        _ => return None,
    })
}

fn boolean(props: &HashMap<String, OwnedValue>, key: &str) -> bool {
    matches!(props.get(key).map(|v| &**v), Some(Value::Bool(true)))
}

/// The `a{sv}` of `Metadata`, by key.
fn metadata(v: &Value) -> HashMap<String, Value<'static>> {
    let mut out = HashMap::new();
    let inner = match v {
        Value::Value(i) => i,
        other => other,
    };
    if let Value::Dict(d) = inner {
        for (k, val) in d.iter() {
            let key = text(k);
            if let Ok(owned) = val.try_to_owned() {
                out.insert(key, Value::from(owned));
            }
        }
    }
    out
}

/// A string, or a list of them joined (`xesam:artist` is a list).
fn joined(v: Option<&Value>) -> String {
    match v.map(unwrap) {
        Some(Value::Array(a)) => a
            .iter()
            .map(text)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        Some(other) => text(other),
        None => String::new(),
    }
}

/// One player's properties as the payload says them. Lengths and positions
/// are microseconds on the bus and seconds here; a length that is not a
/// number (a stream) is 0 and the module decides what that means.
pub(crate) fn parse_player(id: &str, name: &str, props: &HashMap<String, OwnedValue>) -> Player {
    let meta = props
        .get("Metadata")
        .map(|m| metadata(m))
        .unwrap_or_default();
    let seconds = |micros: Option<i64>| Num(micros.map_or(0.0, |u| u as f64 / 1e6));
    Player {
        id: id.to_string(),
        name: name.to_string(),
        playing: props
            .get("PlaybackStatus")
            .is_some_and(|s| text(s) == "Playing"),
        title: joined(meta.get("xesam:title")),
        artist: joined(meta.get("xesam:artist")),
        album: joined(meta.get("xesam:album")),
        art_url: joined(meta.get("mpris:artUrl")),
        length: seconds(meta.get("mpris:length").and_then(int)),
        position: seconds(props.get("Position").and_then(|v| int(v))),
        can_play: boolean(props, "CanPlay"),
        can_pause: boolean(props, "CanPause"),
        can_next: boolean(props, "CanGoNext"),
        can_prev: boolean(props, "CanGoPrevious"),
        can_seek: boolean(props, "CanSeek"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap as Map;
    use zbus::zvariant::Dict;

    fn owned<'a>(v: impl Into<Value<'a>>) -> OwnedValue {
        v.into().try_to_owned().expect("a value without fds")
    }

    fn meta(entries: Vec<(&str, OwnedValue)>) -> OwnedValue {
        let mut d = Dict::new(
            &zbus::zvariant::Signature::Str,
            &zbus::zvariant::Signature::Variant,
        );
        for (k, v) in entries {
            d.append(Value::from(k), Value::Value(Box::new(Value::from(v))))
                .expect("a{sv}");
        }
        owned(Value::Dict(d))
    }

    fn props(entries: Vec<(&str, OwnedValue)>) -> Map<String, OwnedValue> {
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect()
    }

    #[test]
    fn a_players_properties_become_the_payload() {
        let p = props(vec![
            ("PlaybackStatus", owned("Playing")),
            ("Position", owned(92_000_000_i64)),
            ("CanPlay", owned(true)),
            ("CanGoNext", owned(true)),
            (
                "Metadata",
                meta(vec![
                    ("xesam:title", owned("Night Drive")),
                    (
                        "xesam:artist",
                        owned(zbus::zvariant::Array::from(vec!["Neon Harbor", "Guest"])),
                    ),
                    ("mpris:length", owned(214_000_000_i64)),
                    ("mpris:artUrl", owned("file:///tmp/cover.png")),
                ]),
            ),
        ]);
        let got = parse_player("org.mpris.MediaPlayer2.demo", "Demo", &p);
        assert_eq!(got.title, "Night Drive");
        assert_eq!(got.artist, "Neon Harbor, Guest");
        assert_eq!(got.art_url, "file:///tmp/cover.png");
        assert!(got.playing && got.can_play && got.can_next);
        assert!(!got.can_pause && !got.can_prev && !got.can_seek);
        assert_eq!(got.length.0, 214.0);
        assert_eq!(got.position.0, 92.0);
    }

    #[test]
    fn lengths_come_in_any_width_and_a_missing_one_is_zero() {
        let unsigned = props(vec![(
            "Metadata",
            meta(vec![("mpris:length", owned(214_000_000_u64))]),
        )]);
        assert_eq!(parse_player("i", "n", &unsigned).length.0, 214.0);
        let none = props(vec![("PlaybackStatus", owned("Paused"))]);
        let p = parse_player("i", "n", &none);
        assert_eq!((p.length.0, p.position.0, p.playing), (0.0, 0.0, false));
        assert_eq!((p.title.as_str(), p.artist.as_str()), ("", ""));
    }

    #[test]
    fn an_artist_that_is_a_plain_string_is_kept() {
        let p = props(vec![(
            "Metadata",
            meta(vec![("xesam:artist", owned("Solo"))]),
        )]);
        assert_eq!(parse_player("i", "n", &p).artist, "Solo");
    }
}
