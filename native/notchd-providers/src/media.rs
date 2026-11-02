//! The `media` provider: every player and its state, and the commands that
//! drive one (docs/architecture.md, Providers). The payload is
//! `notchd-proto::provider::MediaData`; a backend (MPRIS on Linux) reads the
//! players on its own thread and hands each change to a sink, and takes
//! [`MediaCommand`]s on a channel, so a call to a player never runs under the
//! hub's lock.

pub use notchd_proto::provider::{MediaData, Player};
use std::sync::mpsc::Sender;

/// What a module asks of a player. `player` is a [`Player::id`].
#[derive(Clone, Debug, PartialEq)]
pub enum MediaCommand {
    /// Read the players again and send them (when they differ from what was
    /// sent last, or when nothing was sent yet: a module that has just loaded
    /// wants the current state).
    Refresh,
    PlayPause {
        player: String,
    },
    Next {
        player: String,
    },
    Previous {
        player: String,
    },
    /// To this position, in seconds.
    Seek {
        player: String,
        seconds: f64,
    },
}

/// Where the backend sends the players each time something changed.
pub type MediaSink = Box<dyn Fn(MediaData) + Send>;

/// Whether `next`, read `elapsed_ms` after `prev`, tells the module anything
/// it does not know. A playing position moves by itself, so it counts only
/// when it is off from where it should be by more than a second and a half
/// (the player seeked without saying so).
pub fn changed(prev: &MediaData, elapsed_ms: u64, next: &MediaData) -> bool {
    if prev.players.len() != next.players.len() {
        return true;
    }
    prev.players.iter().zip(&next.players).any(|(a, b)| {
        let same = Player {
            position: b.position,
            ..a.clone()
        } == *b;
        if !same {
            return true;
        }
        let mut expected = a.position.0;
        if a.playing {
            expected += elapsed_ms as f64 / 1000.0;
            if b.length.0 > 0.0 {
                expected = expected.min(b.length.0);
            }
        }
        (b.position.0 - expected).abs() > if a.playing { 1.5 } else { 0.5 }
    })
}

/// Starts the backend of this OS: `wanted` says whether some module uses the
/// players (the backend stays off the bus while none does), `sink` takes each
/// change. The sender takes commands. `None` where there is no backend (the
/// module then shows its empty state).
pub fn start(
    wanted: Box<dyn Fn() -> bool + Send>,
    sink: MediaSink,
) -> Option<Sender<MediaCommand>> {
    #[cfg(target_os = "linux")]
    return Some(crate::linux::mpris::start(wanted, sink));
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (wanted, sink);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notchd_proto::Num;

    fn player(playing: bool, position: f64) -> Player {
        Player {
            id: "org.mpris.MediaPlayer2.x".into(),
            name: "X".into(),
            playing,
            title: "T".into(),
            artist: "A".into(),
            album: String::new(),
            art_url: String::new(),
            length: Num(200.0),
            position: Num(position),
            can_play: true,
            can_pause: true,
            can_next: true,
            can_prev: true,
            can_seek: true,
        }
    }

    fn data(p: Player) -> MediaData {
        MediaData { players: vec![p] }
    }

    #[test]
    fn a_playing_position_that_runs_on_by_itself_is_not_news() {
        let prev = data(player(true, 10.0));
        assert!(!changed(&prev, 5000, &data(player(true, 15.2))));
        assert!(changed(&prev, 5000, &data(player(true, 40.0))), "a seek");
        assert!(changed(&prev, 5000, &data(player(true, 2.0))), "a restart");
    }

    #[test]
    fn a_paused_position_that_moved_is_news() {
        let prev = data(player(false, 10.0));
        assert!(!changed(&prev, 9000, &data(player(false, 10.2))));
        assert!(changed(&prev, 9000, &data(player(false, 30.0))));
    }

    #[test]
    fn any_other_change_is_news() {
        let prev = data(player(true, 10.0));
        assert!(changed(&prev, 0, &data(player(false, 10.0))), "paused");
        let mut other = player(true, 10.0);
        other.title = "U".into();
        assert!(changed(&prev, 0, &data(other)), "a new track");
        assert!(changed(&prev, 0, &MediaData { players: vec![] }), "gone");
    }
}
