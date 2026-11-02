use super::*;
use notchd_proto::common::KeyboardFocus;
use notchd_proto::ui::{ModuleOp, ScreenInfo, Workspace};

fn screens(names: &[&str], focused: &str) -> Screens {
    Screens {
        screens: names
            .iter()
            .enumerate()
            .map(|(i, n)| ScreenInfo {
                name: (*n).into(),
                x: i as i32 * 1000,
                y: 0,
                w: 1000,
                h: 800,
                scale: Num(1.0),
            })
            .collect(),
        focused: focused.into(),
        fullscreen: Vec::new(),
        workspaces: BTreeMap::new(),
        known: Vec::new(),
        supports_workspaces: false,
        bar_edge: None,
    }
}

fn module(id: &str, priority: f64) -> ModuleFacts {
    let mut m = ModuleFacts::new(id);
    m.info.priority = priority;
    m.info.compact = true;
    m.info.peek = true;
    m.expanded = true;
    m
}

/// One notch ("main") with media and timer, one screen.
fn store(cfg: Value) -> Store {
    let mut s = Store::new();
    s.set_config(cfg);
    s.set_screens(screens(&["DP-1"], "DP-1"));
    s.set_module(module("media", 50.0));
    s.set_module(module("timer", 80.0));
    s.take_changes(0);
    s
}

fn mode(s: &Store) -> Mode {
    s.state().notches[0].copies[0].mode
}

fn peek_of(s: &Store) -> Option<String> {
    s.state().notches[0].copies[0]
        .peek
        .as_ref()
        .map(|p| p.module.clone())
}

fn cfg() -> Value {
    json!({"modules": ["media", "timer"], "hoverOpenDelay": 100, "hoverCloseDelay": 200})
}

#[test]
fn hover_opens_after_the_delay_and_closes_after_the_close_delay() {
    let mut s = store(cfg());
    assert_eq!(mode(&s), Mode::Compact);
    s.pointer("main", "DP-1", PointerEv::Enter, 1000);
    s.take_changes(1000);
    assert_eq!(mode(&s), Mode::Compact, "not yet");
    assert_eq!(s.next_due(), Some(1100));
    s.advance(1099);
    s.take_changes(1099);
    assert_eq!(mode(&s), Mode::Compact);
    s.advance(1100);
    s.take_changes(1100);
    assert_eq!(mode(&s), Mode::Expanded);
    s.pointer("main", "DP-1", PointerEv::Leave, 2000);
    s.advance(2199);
    s.take_changes(2199);
    assert_eq!(mode(&s), Mode::Expanded, "the close delay holds it");
    s.advance(2200);
    s.take_changes(2200);
    assert_eq!(mode(&s), Mode::Compact);
}

#[test]
fn leaving_before_the_open_delay_never_opens() {
    let mut s = store(cfg());
    s.pointer("main", "DP-1", PointerEv::Enter, 0);
    s.pointer("main", "DP-1", PointerEv::Leave, 50);
    s.advance(10_000);
    assert_eq!(mode(&s), Mode::Compact);
}

#[test]
fn coming_back_within_the_close_delay_keeps_it_open() {
    let mut s = store(cfg());
    s.pointer("main", "DP-1", PointerEv::Enter, 0);
    s.advance(100);
    s.pointer("main", "DP-1", PointerEv::Leave, 500);
    s.pointer("main", "DP-1", PointerEv::Enter, 600);
    s.advance(5000);
    assert_eq!(mode(&s), Mode::Expanded);
}

#[test]
fn a_zero_delay_opens_at_once() {
    let mut s = store(json!({"modules": ["media"], "hoverOpenDelay": 0}));
    s.pointer("main", "DP-1", PointerEv::Enter, 5);
    assert_eq!(mode(&s_after(s)), Mode::Expanded);
}

fn s_after(mut s: Store) -> Store {
    s.take_changes(5);
    s
}

#[test]
fn press_pins_and_again_unpins() {
    let mut s = store(cfg());
    s.pointer("main", "DP-1", PointerEv::Press, 0);
    s.take_changes(0);
    assert_eq!(mode(&s), Mode::Expanded);
    assert!(s.state().notches[0].copies[0].pinned);
    assert_eq!(
        s.state().notches[0].copies[0].keyboard,
        KeyboardFocus::Exclusive,
        "pinned open asks for the keyboard"
    );
    s.pointer("main", "DP-1", PointerEv::Press, 1);
    s.take_changes(1);
    assert_eq!(mode(&s), Mode::Compact);
}

#[test]
fn a_peek_shows_for_its_length_then_goes() {
    let mut s = store(cfg());
    s.request_peek("media", Some(1000.0), 10_000);
    let st = s.take_changes(10_000).state.unwrap();
    let p = st.notches[0].copies[0].peek.clone().unwrap();
    assert_eq!((p.module.as_str(), p.until), ("media", 11_000));
    assert_eq!(mode(&s), Mode::Peek);
    s.advance(10_999);
    assert_eq!(mode(&s), Mode::Peek);
    s.advance(11_000);
    s.take_changes(11_000);
    assert_eq!(mode(&s), Mode::Compact);
    assert_eq!(peek_of(&s), None);
}

#[test]
fn a_peek_is_at_least_400_ms_and_2500_by_default() {
    let mut s = store(cfg());
    s.request_peek("media", Some(10.0), 0);
    assert_eq!(s.next_due(), Some(400));
    s.request_peek("media", None, 0);
    assert_eq!(s.next_due(), Some(2500));
}

#[test]
fn a_lower_priority_peek_waits_for_the_one_showing_and_a_higher_one_takes_over() {
    let mut s = store(cfg());
    s.request_peek("timer", Some(1000.0), 0);
    s.request_peek("media", Some(1000.0), 100);
    assert_eq!(
        peek_of(&s).as_deref(),
        Some("timer"),
        "media (50) cannot cut timer (80)"
    );
    s.advance(1000);
    s.request_peek("media", Some(1000.0), 1000);
    assert_eq!(peek_of(&s).as_deref(), Some("media"));
    s.request_peek("timer", Some(1000.0), 1100);
    assert_eq!(
        peek_of(&s).as_deref(),
        Some("timer"),
        "the higher one takes over"
    );
}

#[test]
fn hover_drops_a_peek_unless_it_is_urgent_and_only_urgent_ones_get_through_a_hover() {
    let mut s = store(cfg());
    s.request_peek("media", Some(5000.0), 0);
    s.pointer("main", "DP-1", PointerEv::Enter, 10);
    s.advance(110);
    assert_eq!(peek_of(&s), None, "hover dismissed the plain peek");
    assert_eq!(mode(&s), Mode::Expanded);
    s.request_peek("media", Some(5000.0), 200);
    assert_eq!(peek_of(&s), None, "not through a hovered notch");
    let mut urgent = module("timer", 80.0);
    urgent.info.urgent_peek = true;
    s.set_module(urgent);
    s.request_peek("timer", Some(5000.0), 300);
    assert_eq!(peek_of(&s).as_deref(), Some("timer"));
    assert_eq!(mode(&s), Mode::Peek, "an urgent peek shows over the hover");
}

#[test]
fn a_pinned_notch_never_peeks_and_pinning_drops_the_peek() {
    let mut s = store(cfg());
    s.request_peek("media", Some(5000.0), 0);
    s.pointer("main", "DP-1", PointerEv::Press, 10);
    assert_eq!(peek_of(&s), None);
    s.request_peek("timer", Some(5000.0), 20);
    assert_eq!(peek_of(&s), None);
    assert_eq!(s.next_due(), None, "no peek timer left");
}

#[test]
fn a_refused_peek_of_a_module_with_news_shows_once_the_notch_closes() {
    let mut s = store(cfg());
    let mut media = module("media", 50.0);
    media.info.attention = true;
    s.set_module(media);
    s.pointer("main", "DP-1", PointerEv::Press, 0);
    s.request_peek("media", Some(1000.0), 10);
    assert_eq!(peek_of(&s), None);
    s.pointer("main", "DP-1", PointerEv::Press, 100);
    s.take_changes(100);
    assert_eq!(peek_of(&s).as_deref(), Some("media"), "as it closes");
    assert_eq!(mode(&s), Mode::Peek);
}

#[test]
fn a_refused_peek_of_a_module_that_has_seen_its_news_is_forgotten() {
    let mut s = store(cfg());
    let mut media = module("media", 50.0);
    media.info.attention = true;
    s.set_module(media);
    s.pointer("main", "DP-1", PointerEv::Press, 0);
    s.request_peek("media", Some(1000.0), 10);
    s.set_module(module("media", 50.0));
    s.pointer("main", "DP-1", PointerEv::Press, 100);
    assert_eq!(peek_of(&s), None);
}

#[test]
fn a_module_the_notch_does_not_show_does_not_peek_there() {
    let mut s = store(json!({"modules": ["media"]}));
    s.set_module(module("other", 99.0));
    s.request_peek("other", None, 0);
    assert_eq!(peek_of(&s), None);
}

#[test]
fn a_peek_on_focused_screen_peeks_once_in_the_first_notch_that_shows_it() {
    let mut s = store(json!({"notches": [
        {"id": "a", "modules": ["media"]},
        {"id": "b", "modules": ["media"]}
    ]}));
    let mut media = module("media", 50.0);
    media.peek_on_focused_screen = true;
    s.set_module(media);
    s.request_peek("media", Some(1000.0), 0);
    let st = s.state();
    let peeking: Vec<_> = st
        .notches
        .iter()
        .map(|n| n.copies[0].peek.is_some())
        .collect();
    assert_eq!(peeking, [true, false]);
    // a plain module peeks in every notch that shows it
    s.set_module(module("timer", 10.0));
    let mut s2 = store(json!({"notches": [
        {"id": "a", "modules": ["media"]},
        {"id": "b", "modules": ["media"]}
    ]}));
    s2.request_peek("media", Some(1000.0), 0);
    let st = s2.state();
    assert!(st.notches.iter().all(|n| n.copies[0].peek.is_some()));
}

#[test]
fn a_standalone_module_peeks_in_its_own_notch_on_the_focused_screen() {
    let mut s = store(cfg());
    s.set_screens(screens(&["DP-1", "DP-2"], "DP-2"));
    let mut osd = ModuleFacts::new("osd");
    osd.standalone = true;
    osd.info.peek = true;
    s.set_module(osd);
    s.request_peek("osd", Some(1000.0), 0);
    let st = s.take_changes(0).state.unwrap();
    assert_eq!(
        st.notches[0].copies[0].peek, None,
        "never in a notch of the config"
    );
    let own = &st.standalone[0];
    assert_eq!(own.id, "osd");
    let modes: Vec<_> = own
        .copies
        .iter()
        .map(|c| (c.screen.as_str(), c.mode))
        .collect();
    assert_eq!(modes, [("DP-1", Mode::Rest), ("DP-2", Mode::Peek)]);
    s.advance(1000);
    let st = s.take_changes(1000).state.unwrap();
    assert!(st.standalone[0].copies.iter().all(|c| c.mode == Mode::Rest));
    s.set_value("osd.enabled", Some(&json!(false)));
    assert!(
        s.state().standalone.is_empty(),
        "switched off in the config"
    );
}

#[test]
fn the_state_rev_moves_only_when_the_state_changes() {
    let mut s = store(cfg());
    let rev = s.state().rev;
    assert!(s.take_changes(1).state.is_none());
    assert!(s.take_changes(2).state.is_none());
    s.request_peek("media", Some(1000.0), 3);
    let a = s.take_changes(3).state.unwrap();
    assert_eq!(a.rev, rev + 1);
    s.advance(2000);
    let b = s.take_changes(2000).state.unwrap();
    assert_eq!(b.rev, rev + 2);
}

#[test]
fn a_notch_that_moves_follows_focus_and_one_per_screen_has_a_copy_each() {
    let mut s = store(cfg());
    s.set_screens(screens(&["DP-1", "DP-2"], "DP-2"));
    assert_eq!(s.state().notches[0].copies.len(), 1);
    assert_eq!(
        s.take_changes(0).state.unwrap().notches[0].copies[0].screen,
        "DP-2"
    );
    let mut s = store(json!({"modules": ["media"], "position": {"screen": "all"}}));
    s.set_screens(screens(&["DP-1", "DP-2"], "DP-2"));
    let st = s.take_changes(0).state.unwrap();
    let on: Vec<_> = st.notches[0]
        .copies
        .iter()
        .map(|c| c.screen.as_str())
        .collect();
    assert_eq!(on, ["DP-1", "DP-2"]);
    s.request_peek("media", Some(1000.0), 0);
    let st = s.take_changes(0).state.unwrap();
    let peeks: Vec<_> = st.notches[0]
        .copies
        .iter()
        .map(|c| c.peek.is_some())
        .collect();
    assert_eq!(peeks, [false, true], "the copy on the focused screen peeks");
}

#[test]
fn a_notch_homed_on_a_screen_stays_there_unless_its_workspace_leaves_it_out() {
    let mut s =
        store(json!({"modules": ["media"], "position": {"screen": "DP-2"}, "workspaces": [1]}));
    let mut sc = screens(&["DP-1", "DP-2"], "DP-1");
    sc.workspaces.insert(
        "DP-2".into(),
        Workspace {
            id: "1".into(),
            name: "1".into(),
        },
    );
    s.set_screens(sc.clone());
    assert_eq!(
        s.take_changes(0).state.unwrap().notches[0].copies[0].screen,
        "DP-2"
    );
    sc.workspaces.insert(
        "DP-2".into(),
        Workspace {
            id: "2".into(),
            name: "2".into(),
        },
    );
    s.set_screens(sc);
    let st = s.take_changes(0).state.unwrap();
    assert_eq!(
        st.notches[0].copies[0].screen, "DP-1",
        "off that workspace it goes to the focused screen"
    );
}

#[test]
fn tabs_pick_the_module_the_open_notch_shows() {
    let mut s = store(cfg());
    s.pointer("main", "DP-1", PointerEv::Press, 0);
    let c = &s.take_changes(0).state.unwrap().notches[0].copies[0];
    // by priority: timer (80) then media
    assert_eq!(c.shown, ["timer"]);
    s.tab("main", "DP-1", "media", 1);
    let c = &s.take_changes(1).state.unwrap().notches[0].copies[0];
    assert_eq!(
        (c.shown.clone(), c.tab.as_str()),
        (vec!["media".to_string()], "media")
    );
}

// ----------------------------------------------------------- config edits

#[test]
fn an_edit_applies_at_once_and_is_queued_for_the_file_without_defaults() {
    let mut s = store(json!({"theme": {"accent": "red"}, "hoverOpenDelay": 100}));
    s.set_value("theme.accent", None);
    assert_eq!(
        s.config(),
        &json!({"hoverOpenDelay": 100}),
        "the emptied parent goes"
    );
    assert_eq!(s.take_write(), Some(json!({"hoverOpenDelay": 100})));
    assert_eq!(s.take_write(), None);
    s.set_value("hoverOpenDelay", None);
    assert_eq!(s.config(), &json!({}), "a missing key means its default");
    let c = s.take_changes(0).config.unwrap();
    assert_eq!(c.resolved["hoverOpenDelay"], 120);
    assert!(c.cfg.as_object().unwrap().is_empty(), "no default written");
}

#[test]
fn the_config_rev_grows_with_every_change_and_a_bad_file_keeps_the_good_config() {
    let mut s = Store::new();
    assert!(s.config_update().is_none());
    s.set_config(json!({"a": 1}));
    s.set_config(json!({"a": 2}));
    let first = s.take_changes(0).config.unwrap();
    assert_eq!((first.rev, first.cfg["a"].clone()), (2, json!(2)));
    s.config_failed("oops".into());
    s.config_failed("oops".into());
    let c = s.take_changes(1);
    assert_eq!(c.config_error.unwrap().message, "oops");
    assert!(c.config.is_none());
    assert_eq!(s.config()["a"], 2);
    assert_eq!(s.take_changes(2).config_error, None, "reported once");
}

#[test]
fn edit_ops_change_the_module_list_of_a_notch() {
    let mut s =
        store(json!({"notches": [{"id": "a", "modules": ["media", "timer"]}, {"id": "b"}]}));
    s.edit("a", &EditOp::MoveModule("timer".into(), -1))
        .unwrap();
    assert_eq!(
        s.config()["notches"][0]["modules"],
        json!(["timer", "media"])
    );
    s.edit("a", &EditOp::ToggleModule("media".into())).unwrap();
    assert_eq!(s.config()["notches"][0]["modules"], json!(["timer"]));
    s.edit("b", &EditOp::ToggleWorkspace("2".into())).unwrap();
    assert_eq!(s.config()["notches"][1]["workspaces"], json!([2]));
    s.edit("b", &EditOp::ToggleWorkspace("2".into())).unwrap();
    assert!(s.config()["notches"][1].get("workspaces").is_none());
    assert!(s.edit("zzz", &EditOp::ToggleModule("x".into())).is_err());
}

#[test]
fn a_drop_writes_the_position_and_a_drop_on_the_default_removes_it() {
    let mut s = store(cfg());
    let to = navigate::Position {
        edge: navigate::Edge::Left,
        offset: 0.3,
        screen: "DP-1".into(),
    };
    s.drop_notch("main", "DP-1", to, 0);
    assert_eq!(
        s.config()["position"],
        json!({"edge": "left", "offset": 0.3, "screen": "DP-1"})
    );
    assert!(s.take_write().is_some());
    let back = navigate::Position {
        edge: navigate::Edge::Top,
        offset: 0.5,
        screen: String::new(),
    };
    s.drop_notch("main", "", back, 1);
    assert!(
        s.config().get("position").is_none(),
        "defaults are never written"
    );
    assert_eq!(s.state().notches[0].edge, Edge::Top);
}

#[test]
fn a_drop_closes_the_dragged_copy() {
    let mut s = store(cfg());
    s.pointer("main", "DP-1", PointerEv::Press, 0);
    let back = navigate::Position {
        edge: navigate::Edge::Top,
        offset: 0.5,
        screen: String::new(),
    };
    s.drop_notch("main", "DP-1", back, 1);
    s.take_changes(1);
    assert_eq!(mode(&s), Mode::Compact);
}

#[test]
fn recenter_removes_every_position() {
    let mut s = store(json!({"position": {"edge": "left"}, "notches": [
        {"id": "a", "position": {"edge": "right"}}, {"id": "b", "modules": ["media"]}]}));
    s.recenter();
    assert_eq!(s.config()["notches"][0].get("position"), None);
    assert!(s.config().get("position").is_none());
}

// ------------------------------------------------------------- navigation

fn two_notches() -> Store {
    store(json!({"notches": [
        {"id": "a", "modules": ["media", "timer"]},
        {"id": "b", "modules": ["media"], "position": {"edge": "bottom"}},
        {"id": "c", "modules": ["media"], "position": {"edge": "left"}}]}))
}

fn open_modes(s: &Store) -> Vec<Mode> {
    s.state().notches.iter().map(|n| n.copies[0].mode).collect()
}

#[test]
fn navigation_previews_the_selected_notch_and_escape_leaves() {
    let mut s = two_notches();
    assert_eq!(s.navigate(0), "ok");
    let st = s.take_changes(0).state.unwrap();
    let nav = st.navigation.clone().unwrap();
    assert_eq!((nav.mode, nav.selected), (ps::NavMode::Select, 0));
    assert_eq!(
        open_modes(&s),
        [Mode::Expanded, Mode::Compact, Mode::Compact]
    );
    s.key("2", &[], 1);
    s.take_changes(1);
    assert_eq!(s.state().navigation.unwrap().selected, 1);
    assert_eq!(
        open_modes(&s),
        [Mode::Compact, Mode::Expanded, Mode::Compact]
    );
    s.key("Right", &[], 2);
    assert_eq!(
        s.take_changes(2)
            .state
            .unwrap()
            .navigation
            .unwrap()
            .selected,
        2
    );
    s.key("Escape", &[], 3);
    let st = s.take_changes(3).state.unwrap();
    assert!(st.navigation.is_none());
    assert_eq!(open_modes(&s), [Mode::Compact; 3]);
}

#[test]
fn navigate_again_leaves_and_without_notches_there_is_nothing_to_navigate() {
    let mut s = two_notches();
    s.navigate(0);
    assert!(s.navigating());
    assert_eq!(s.navigate(1), "ok");
    assert!(!s.navigating());
    // a config always has at least its one notch
    let mut s = store(cfg());
    assert_eq!(s.navigate(0), "ok");
}

#[test]
fn tab_in_navigation_walks_the_tabs_and_escape_gives_them_back() {
    let mut s = two_notches();
    s.navigate(0);
    s.key("Tab", &[], 1);
    let c = s.take_changes(1).state.unwrap().notches[0].copies[0]
        .tab
        .clone();
    assert_eq!(c, "media", "timer is first, Tab goes on to media");
    s.key("Tab", &[Modifier::Shift], 2);
    assert_eq!(s.state().notches[0].copies[0].tab, "timer");
    s.key("Tab", &[], 3);
    s.key("Escape", &[], 4);
    assert_eq!(
        s.take_changes(4).state.unwrap().notches[0].copies[0].tab,
        ""
    );
}

#[test]
fn enter_pins_the_selected_notch_and_closes_the_others() {
    let mut s = two_notches();
    s.pointer("main", "", PointerEv::Press, 0);
    s.navigate(1);
    s.key("2", &[], 2);
    s.key("Return", &[], 3);
    let st = s.take_changes(3).state.unwrap();
    assert!(st.navigation.is_none());
    let pinned: Vec<_> = st.notches.iter().map(|n| n.copies[0].pinned).collect();
    assert_eq!(pinned, [false, true, false]);
    let kb: Vec<_> = st.notches.iter().map(|n| n.copies[0].keyboard).collect();
    assert_eq!(
        kb,
        [
            KeyboardFocus::None,
            KeyboardFocus::Exclusive,
            KeyboardFocus::None
        ]
    );
}

#[test]
fn x_twice_removes_the_selected_notch_and_any_other_key_keeps_it() {
    let mut s = two_notches();
    s.navigate(0);
    s.key("x", &[], 1);
    assert_eq!(s.state().navigation.unwrap().mode, ps::NavMode::Remove);
    s.key("Left", &[], 2);
    assert_eq!(s.config()["notches"].as_array().unwrap().len(), 3, "kept");
    s.key("x", &[], 3);
    s.key("x", &[], 4);
    let ids: Vec<_> = s.config()["notches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].clone())
        .collect();
    assert_eq!(ids, [json!("b"), json!("c")]);
    assert!(s.take_write().is_some());
}

#[test]
fn m_moves_the_selected_notch_live_and_escape_puts_it_back() {
    let mut s = store(json!({"notches": [{"id": "a", "modules": ["media"]}, {"id": "b"}]}));
    s.navigate(0);
    s.key("m", &[], 1);
    s.key("Left", &[], 2);
    assert_eq!(
        s.config()["notches"][0]["position"],
        json!({"edge": "left"})
    );
    s.key("Right", &[Modifier::Shift], 3);
    assert_eq!(s.config()["notches"][0]["position"]["offset"], 0.55);
    s.key("Escape", &[], 4);
    assert_eq!(
        s.config()["notches"][0].get("position"),
        None,
        "back where it was"
    );
    assert!(
        s.navigating(),
        "Escape in move undoes it and stays in navigation"
    );
    s.key("Escape", &[], 5);
    assert!(!s.navigating());
}

#[test]
fn m_then_enter_keeps_the_move_and_s_opens_the_notch_page_in_settings() {
    let mut s = store(json!({"notches": [{"id": "a", "modules": ["media"]}, {"id": "b"}]}));
    s.navigate(0);
    s.key("m", &[], 1);
    s.key("Down", &[], 2);
    s.key("Return", &[], 3);
    assert_eq!(s.config()["notches"][0]["position"]["edge"], "bottom");
    s.key("s", &[], 4);
    assert!(s.settings_open);
    assert_eq!(s.settings_page, "notch/0");
    assert!(!s.navigating());
}

#[test]
fn a_pin_while_navigating_ends_navigation_and_the_keyboard_is_settled_for_600_ms() {
    let mut s = two_notches();
    s.navigate(0);
    s.pointer("a", "", PointerEv::Press, 5);
    assert!(!s.navigating());
    assert_eq!(s.next_due(), Some(605));
    s.advance(605);
    assert_eq!(s.next_due(), None);
}

#[test]
fn ipc_commands_open_close_and_toggle_notches() {
    let mut s = two_notches();
    s.open(0);
    assert!(s.state().notches[0].copies[0].pinned);
    s.toggle(1);
    assert!(!s.state().notches[0].copies[0].pinned);
    assert!(s.notch_command("c", "toggle", 2));
    assert!(s.take_changes(2).state.unwrap().notches[2].copies[0].pinned);
    assert!(s.notch_command("b", "open", 3));
    s.close(4);
    assert!(
        s.take_changes(4)
            .state
            .unwrap()
            .notches
            .iter()
            .all(|n| !n.copies[0].pinned)
    );
    assert!(!s.notch_command("nope", "open", 5));
    assert!(s.open_module("timer", 6));
    let st = s.take_changes(6).state.unwrap();
    assert_eq!(
        (
            st.notches[0].copies[0].pinned,
            st.notches[0].copies[0].tab.as_str()
        ),
        (true, "timer")
    );
    s.close_module("timer", 7);
    assert!(!s.state().notches[0].copies[0].pinned);
    assert!(!s.open_module("unknown", 8));
}

#[test]
fn the_ipc_state_follows_docs_ipc_md() {
    let mut s = store(cfg());
    s.request_peek("media", Some(1000.0), 0);
    let v: Value = serde_json::from_str(&s.ipc_state()).unwrap();
    assert_eq!(v["navigation"], Value::Null);
    assert_eq!(v["notches"][0]["id"], "main");
    assert_eq!(v["notches"][0]["copies"][0]["mode"], "peek");
    assert_eq!(v["notches"][0]["copies"][0]["peek"], "media");
    assert_eq!(v["notches"][0]["position"]["edge"], "top");
    assert_eq!(v["notches"][0]["modules"], json!(["media", "timer"]));
    assert_eq!(v["modules"][0]["id"], "media");
    assert_eq!(v["visitor"], json!([]));
}

#[test]
fn wire_keys_read_as_navigate_key_names() {
    assert_eq!(wire_key("Escape", &[]), KeyName::Leave);
    assert_eq!(wire_key("Escape", &[Modifier::Ctrl]), KeyName::Leave);
    assert_eq!(wire_key("Tab", &[Modifier::Shift]), KeyName::PreviousTab);
    assert_eq!(wire_key("h", &[]), KeyName::Previous);
    assert_eq!(wire_key("h", &[Modifier::Ctrl]), KeyName::Other);
    assert_eq!(wire_key("5", &[]), KeyName::Notch(5));
    assert_eq!(wire_key("Left", &[Modifier::Shift]), KeyName::SlideBack);
    assert_eq!(wire_key("F13", &[]), KeyName::Other);
    assert_eq!(wire_key("Return", &[]), KeyName::Open);
}

// ------------------------------------------------------------------ visitor

fn visitor_store() -> Store {
    let mut s = Store::new();
    s.set_config(json!({
        "modules": ["media"], "position": {"screen": "DP-2"},
        "hoverOpenDelay": 100, "hoverCloseDelay": 200
    }));
    s.set_screens(screens(&["DP-1", "DP-2"], "DP-1"));
    s.set_module(module("media", 50.0));
    s.take_changes(0);
    s
}

#[test]
fn a_peek_that_no_notch_shows_on_the_focused_screen_goes_to_the_visitor() {
    // `NotchService.requestPeek`: the notch is kept on DP-2, you work on DP-1.
    let mut s = visitor_store();
    assert!(
        s.state().visitor.iter().all(|c| c.mode == Mode::Rest),
        "a copy per screen, closed"
    );
    s.request_peek("media", Some(1000.0), 0);
    let st = s.take_changes(0).state.unwrap();
    let modes: Vec<_> = st
        .visitor
        .iter()
        .map(|c| (c.screen.as_str(), c.mode))
        .collect();
    assert_eq!(modes, [("DP-1", Mode::Peek), ("DP-2", Mode::Rest)]);
    let peek = st.visitor[0].peek.as_ref().unwrap();
    assert_eq!((peek.module.as_str(), peek.until), ("media", 1000));
    assert!(
        st.notches[0].copies[0].peek.is_some(),
        "and the notch itself"
    );
    let ipc: Value = serde_json::from_str(&s.ipc_state()).unwrap();
    assert_eq!(
        ipc["visitor"],
        json!([{"screen": "DP-1", "mode": "peek", "peek": "media"}]),
        "ipc state lists the copies that show something"
    );
    s.advance(1000);
    let ipc: Value = serde_json::from_str(&s.ipc_state()).unwrap();
    assert_eq!(ipc["visitor"], json!([]));
}

#[test]
fn no_visitor_peek_while_a_notch_peeks_on_the_focused_screen() {
    let mut s = visitor_store();
    s.set_screens(screens(&["DP-1", "DP-2"], "DP-2"));
    s.request_peek("media", Some(1000.0), 0);
    let st = s.take_changes(0).state.unwrap();
    assert!(st.visitor.iter().all(|c| c.mode == Mode::Rest));
    // With the focused screen unknown the visitor is never needed either.
    let mut s = visitor_store();
    s.set_screens(screens(&["DP-1", "DP-2"], ""));
    s.request_peek("media", Some(1000.0), 0);
    assert!(s.state().visitor.iter().all(|c| c.mode == Mode::Rest));
}

#[test]
fn a_focused_screen_peek_with_no_notch_there_only_peeks_in_the_visitor() {
    let mut s = visitor_store();
    let mut m = module("media", 50.0);
    m.peek_on_focused_screen = true;
    s.set_module(m);
    s.request_peek("media", Some(1000.0), 0);
    let st = s.take_changes(0).state.unwrap();
    assert_eq!(st.visitor[0].mode, Mode::Peek);
    assert!(st.notches[0].copies[0].peek.is_none(), "once, not twice");
}

#[test]
fn a_standalone_module_never_goes_to_the_visitor() {
    let mut s = visitor_store();
    let mut osd = ModuleFacts::new("osd");
    osd.standalone = true;
    osd.info.peek = true;
    s.set_module(osd);
    s.request_peek("osd", Some(1000.0), 0);
    let st = s.take_changes(0).state.unwrap();
    assert!(st.visitor.iter().all(|c| c.mode == Mode::Rest));
    assert_eq!(st.standalone[0].id, "osd");
}

// ------------------------------------------------------------------- linger

#[test]
fn a_guests_window_lingers_after_it_closes_then_drops() {
    // `NotchCopy.windowLinger`: a guest has a window while open and 5 s after.
    let mut s = visitor_store();
    let needs = |s: &Store| {
        s.state()
            .visitor
            .iter()
            .map(|c| c.needs_window)
            .collect::<Vec<_>>()
    };
    assert_eq!(needs(&s), [false, false], "closed, never open: no window");
    s.request_peek("media", Some(1000.0), 0);
    s.take_changes(0);
    assert_eq!(needs(&s), [true, false], "open");
    s.advance(1000);
    s.take_changes(1000);
    assert_eq!(s.state().visitor[0].mode, Mode::Rest);
    assert_eq!(needs(&s), [true, false], "closed, lingering");
    s.advance(5999);
    assert_eq!(needs(&s), [true, false]);
    s.advance(6000);
    assert_eq!(needs(&s), [false, false], "5000 ms after it closed");
    assert!(
        s.state().notches[0].copies.iter().all(|c| c.needs_window),
        "a notch of the config always has its window"
    );
}

#[test]
fn a_standalone_copy_lingers_too_and_a_reopen_restarts_it() {
    let mut s = store(cfg());
    let mut osd = ModuleFacts::new("osd");
    osd.standalone = true;
    osd.info.peek = true;
    s.set_module(osd);
    s.set_window_linger(300);
    s.request_peek("osd", Some(400.0), 0);
    s.take_changes(0);
    assert!(s.state().standalone[0].copies[0].needs_window);
    s.advance(400);
    s.request_peek("osd", Some(400.0), 500);
    s.advance(900);
    s.take_changes(900);
    assert!(
        s.state().standalone[0].copies[0].needs_window,
        "second close at 900"
    );
    s.advance(1199);
    assert!(s.state().standalone[0].copies[0].needs_window);
    s.advance(1200);
    assert!(!s.state().standalone[0].copies[0].needs_window);
}

// ------------------------------------------------------------------ reserve

fn reserve_store(cfg: Value) -> Store {
    let mut s = Store::new();
    s.set_config(cfg);
    s.set_screens(screens(&["DP-1"], "DP-1"));
    s.set_module(module("media", 50.0));
    s.take_changes(0);
    s
}

fn depth(s: &Store) -> Option<f64> {
    s.state().reserve.first().map(|r| r.depth.0)
}

#[test]
fn nothing_is_reserved_until_the_ui_measures_and_not_without_reserve_space() {
    let mut s = reserve_store(json!({"modules": ["media"], "reserveSpace": true}));
    assert!(s.state().reserve.is_empty());
    s.measure("main", "", 6.0, 0.0, 30.0);
    s.take_changes(0);
    assert_eq!(depth(&s), Some(30.0), "a compact module: baseline");
    let r = &s.state().reserve[0];
    assert_eq!((r.screen.as_str(), r.edge), ("DP-1", Edge::Top));

    let mut s = reserve_store(json!({"modules": ["media"]}));
    s.measure("main", "", 6.0, 0.0, 30.0);
    s.take_changes(0);
    assert!(s.state().reserve.is_empty(), "reserveSpace is off");
}

#[test]
fn the_rest_lip_is_reserved_without_a_compact_view_rounded_up() {
    let mut s = reserve_store(json!({"modules": ["media"], "reserveSpace": true}));
    let mut m = module("media", 50.0);
    m.info.active = false;
    s.set_module(m);
    s.measure("main", "", 5.2, 0.0, 30.0);
    s.take_changes(0);
    assert_eq!(depth(&s), Some(6.0), "ceil(rest)");
}

#[test]
fn the_tallest_compact_view_counts_and_the_space_grows_at_once_and_shrinks_later() {
    let mut s = reserve_store(json!({"modules": ["media"], "reserveSpace": true}));
    s.measure("main", "", 6.0, 0.0, 30.0);
    s.take_changes(0);
    s.measure("main", "", 6.0, 44.5, 30.0);
    s.take_changes(1000);
    assert_eq!(depth(&s), Some(45.0), "grows at once");
    // The compact view is shallower again: the tallest seen is kept.
    s.measure("main", "", 6.0, 20.0, 30.0);
    s.take_changes(2000);
    assert_eq!(depth(&s), Some(45.0));
    // The baseline moving (the theme rescaled) forgets the tallest.
    s.measure("main", "", 6.0, 20.0, 28.0);
    s.take_changes(3000);
    assert_eq!(s.next_due(), Some(3400), "shrinks after the animation");
    assert_eq!(depth(&s), Some(45.0), "held until then");
    s.advance(3399);
    assert_eq!(depth(&s), Some(45.0));
    s.advance(3400);
    s.take_changes(3400);
    assert_eq!(depth(&s), Some(28.0));
}

#[test]
fn a_module_ending_as_another_starts_moves_windows_once() {
    // `reserveStep`: a shrink waits, and a target back at the held depth
    // cancels it.
    let mut s = reserve_store(json!({"modules": ["media"], "reserveSpace": true}));
    s.measure("main", "", 6.0, 0.0, 30.0);
    s.take_changes(0);
    let mut off = module("media", 50.0);
    off.info.active = false;
    s.set_module(off);
    s.take_changes(1000);
    assert_eq!(depth(&s), Some(30.0), "the rest lip is shallower: waits");
    assert_eq!(s.next_due(), Some(1400));
    s.set_module(module("media", 50.0));
    s.take_changes(1100);
    assert_eq!(
        s.next_due(),
        None,
        "back at the held depth: the shrink is cancelled"
    );
    assert_eq!(depth(&s), Some(30.0));
    // Gone altogether: not a shrink but nothing to keep, now.
    let mut s2 = reserve_store(json!({"modules": ["media"], "reserveSpace": true}));
    s2.measure("main", "", 6.0, 0.0, 30.0);
    s2.take_changes(0);
    s2.set_value("reserveSpace", Some(&json!(false)));
    s2.take_changes(10);
    assert!(s2.state().reserve.is_empty(), "dropped at once");
}

#[test]
fn notches_on_one_edge_do_not_add_up_and_a_notch_on_its_home_screen_reserves_there_only() {
    let mut s = Store::new();
    s.set_config(json!({"reserveSpace": true, "notches": [
        {"id": "a", "modules": ["media"], "position": {"screen": "DP-1"}},
        {"id": "b", "modules": ["media"], "position": {"screen": "DP-2"}},
        {"id": "c", "modules": ["media"], "position": {"screen": "DP-2", "edge": "left"}}
    ]}));
    s.set_screens(screens(&["DP-1", "DP-2"], "DP-1"));
    s.set_module(module("media", 50.0));
    s.measure("a", "", 6.0, 0.0, 30.0);
    s.measure("b", "", 6.0, 0.0, 40.0);
    s.measure("c", "", 6.0, 0.0, 22.0);
    s.take_changes(0);
    let got: Vec<_> = s
        .state()
        .reserve
        .iter()
        .map(|r| (r.screen.clone(), r.edge, r.depth.0))
        .collect();
    assert_eq!(
        got,
        [
            ("DP-1".to_string(), Edge::Top, 30.0),
            ("DP-2".to_string(), Edge::Left, 22.0),
            ("DP-2".to_string(), Edge::Top, 40.0),
        ]
    );
}

#[test]
fn a_copy_per_screen_reserves_on_its_own_screen() {
    let mut s = Store::new();
    s.set_config(
        json!({"reserveSpace": true, "modules": ["media"], "position": {"screen": "all"}}),
    );
    s.set_screens(screens(&["DP-1", "DP-2"], "DP-1"));
    s.set_module(module("media", 50.0));
    s.measure("main", "DP-1", 6.0, 0.0, 30.0);
    s.measure("main", "DP-2", 6.0, 0.0, 34.0);
    s.take_changes(0);
    let got: Vec<_> = s
        .state()
        .reserve
        .iter()
        .map(|r| (r.screen.clone(), r.depth.0))
        .collect();
    assert_eq!(
        got,
        [("DP-1".to_string(), 30.0), ("DP-2".to_string(), 34.0)]
    );
}

#[test]
fn a_measure_for_no_notch_is_ignored() {
    let mut s = reserve_store(json!({"modules": ["media"], "reserveSpace": true}));
    s.measure("zzz", "", 6.0, 0.0, 30.0);
    s.take_changes(0);
    assert!(s.state().reserve.is_empty());
}

// ------------------------------------------------------------------- schema

#[test]
fn the_schema_rev_moves_only_when_the_schema_changes() {
    let mut s = store(cfg());
    // `store()` took the first changes (and with them the first schema).
    assert!(s.take_changes(1).schema.is_none());
    s.set_value("hoverOpenDelay", Some(&json!(300)));
    let next = s.take_changes(2).schema.expect("a value changed");
    assert!(next.rev > 0);
    let f = next
        .fields
        .iter()
        .find(|f| f.key.as_deref() == Some("hoverOpenDelay"))
        .unwrap();
    assert_eq!(f.value, json!(300));
    assert!(s.take_changes(3).schema.is_none());
    // A module's page appears with its definition and goes with it.
    s.set_module_def(json!({"moduleId": "weather", "title": "Weather",
        "settings": [{"key": "city", "type": "string", "label": "City", "default": "Berlin"}]}));
    let with = s.take_changes(4).schema.expect("a page");
    assert!(with.pages.iter().any(|p| p.id == "modules/weather"));
    assert!(with.rev > next.rev);
    s.remove_module_def("weather");
    let without = s.take_changes(5).schema.expect("gone");
    assert!(!without.pages.iter().any(|p| p.id == "modules/weather"));
}

#[test]
fn the_built_in_catalog_shapes_the_schema_and_the_host_and_user_modules_win_their_ids() {
    let mut s = store(json!({}));
    let _ = s.take_changes(0);
    let entry = |title: &str| {
        json!({"moduleId": "media", "title": title, "settings": [
            {"key": "equalizer", "type": "string", "label": "Eq"}]})
    };
    s.set_builtin_catalog(vec![entry("Built-in")]);
    let schema = s.take_changes(1).schema.expect("the catalog changed it");
    assert!(schema.pages.iter().any(|p| p.label == "Built-in"));
    let mods = schema.modules.expect("listed");
    assert_eq!(mods.len(), 1);
    // The host's entry replaces it, not adds to it.
    s.set_ui_catalog(vec![entry("Host's")], None);
    let schema = s.take_changes(2).schema.expect("changed");
    assert_eq!(schema.modules.expect("listed").len(), 1);
    assert!(schema.pages.iter().any(|p| p.label == "Host's"));
    // A user module's own def replaces both.
    s.set_module_def(entry("Mine"));
    let schema = s.take_changes(3).schema.expect("changed");
    assert_eq!(schema.modules.expect("listed").len(), 1);
    assert!(schema.pages.iter().any(|p| p.label == "Mine"));
}

#[test]
fn the_uis_catalog_shapes_the_schema_and_a_user_module_wins_its_id() {
    let mut s = store(json!({}));
    let _ = s.take_changes(0);
    let agent = json!({"moduleId": "claude", "title": "Claude Code", "agent": true,
        "settings": [{"key": "directory", "type": "string", "label": "Folder"}]});
    s.set_ui_catalog(
        vec![
            agent.clone(),
            json!({"moduleId": "osd", "title": "OSD", "standalone": true, "settings": []}),
        ],
        None,
    );
    let schema = s.take_changes(1).schema.expect("the catalog changed it");
    // The lists of the Modules page that only exist with such modules.
    let parts: Vec<_> = schema
        .fields
        .iter()
        .filter_map(|f| f.part.as_deref())
        .collect();
    assert!(
        parts.contains(&"agent") && parts.contains(&"standalone"),
        "{parts:?}"
    );
    assert!(schema.pages.iter().any(|p| p.id == "modules/claude"));
    // A user module with the same id replaces the UI's entry.
    s.set_module_def(json!({"moduleId": "claude", "title": "Mine", "settings": [
        {"key": "x", "type": "string", "label": "X"}]}));
    let schema = s.take_changes(2).schema.expect("changed");
    assert!(
        schema
            .fields
            .iter()
            .all(|f| f.key.as_deref() != Some("claude.directory"))
    );
    // A new report replaces the last one.
    s.remove_module_def("claude");
    s.set_ui_catalog(Vec::new(), None);
    let schema = s.take_changes(3).schema.expect("changed");
    assert!(!schema.pages.iter().any(|p| p.id == "modules/claude"));
}

#[test]
fn showing_a_notchs_page_picks_the_notch_the_schema_edits() {
    let mut s = store(json!({"notches": [{"id": "a", "modules": ["media"]}, {"id": "b"}]}));
    s.settings(true, Some("notch/1".into()));
    let schema = s.compute_schema();
    assert!(
        schema
            .fields
            .iter()
            .any(|f| f.key.as_deref() == Some("notches.1.id"))
    );
    assert!(schema.pages.iter().any(|p| p.id == "notch/1"));
}

// ---------------------------------------------------------------- 3.2: the
// inputs the QML used to decide on its own.

fn mode_at(s: &Store, notch: usize) -> Mode {
    s.state().notches[notch].copies[0].mode
}

#[test]
fn a_dragged_copy_stays_closed_nothing_opens_it_and_a_drop_or_cancel_ends_it() {
    let mut s = store(cfg());
    s.pointer("main", "DP-1", PointerEv::Enter, 0);
    s.drag_start("main", "DP-1", 10);
    s.take_changes(10);
    assert_eq!(
        mode(&s),
        Mode::Compact,
        "closed while dragged, hovered or not"
    );
    s.advance(500);
    s.take_changes(500);
    assert_eq!(mode(&s), Mode::Compact, "the open timer does not open it");
    s.pointer("main", "DP-1", PointerEv::Leave, 600);
    s.pointer("main", "DP-1", PointerEv::Enter, 610);
    s.advance(1000);
    s.take_changes(1000);
    assert_eq!(
        mode(&s),
        Mode::Compact,
        "coming back mid-drag opens nothing"
    );
    s.drag_cancel("main", "DP-1", 1100);
    s.take_changes(1100);
    assert_eq!(
        mode(&s),
        Mode::Compact,
        "let go: closed until the pointer returns"
    );
    s.pointer("main", "DP-1", PointerEv::Leave, 1200);
    s.pointer("main", "DP-1", PointerEv::Enter, 1300);
    s.advance(1500);
    s.take_changes(1500);
    assert_eq!(mode(&s), Mode::Expanded, "and then it opens as usual");
}

#[test]
fn pin_opens_and_closes_a_copy_and_closing_ends_its_hover() {
    let mut s = store(cfg());
    s.pin("main", "DP-1", true, 0);
    s.take_changes(0);
    assert!(s.state().notches[0].copies[0].pinned);
    s.pointer("main", "DP-1", PointerEv::Enter, 1);
    s.advance(200);
    s.pin("main", "DP-1", false, 201);
    s.take_changes(201);
    assert_eq!(mode(&s), Mode::Compact, "unpinned and no longer hovered");
}

#[test]
fn the_keyboard_is_exclusive_until_the_window_has_it_then_on_demand_and_lost_it_closes() {
    let mut s = store(cfg());
    let kb = |s: &Store| s.state().notches[0].copies[0].keyboard;
    s.pin("main", "DP-1", true, 0);
    s.take_changes(0);
    assert_eq!(kb(&s), KeyboardFocus::Exclusive);
    s.focus("main", "DP-1", true, 1);
    s.take_changes(1);
    assert_eq!(kb(&s), KeyboardFocus::OnDemand, "the window has it");
    s.focus("main", "DP-1", false, 2);
    s.take_changes(2);
    assert_eq!(
        mode(&s),
        Mode::Compact,
        "a window took it: the notch closes"
    );
    assert_eq!(kb(&s), KeyboardFocus::None);
    // Never held, never closed by losing it.
    s.pin("main", "DP-1", true, 3);
    s.focus("main", "DP-1", false, 4);
    s.take_changes(4);
    assert_eq!(mode(&s), Mode::Expanded);
}

#[test]
fn a_keyboard_lost_as_navigation_ends_is_asked_for_again_not_closed() {
    let mut s = store(cfg());
    let kb = |s: &Store| s.state().notches[0].copies[0].keyboard;
    s.pin("main", "DP-1", true, 0);
    s.focus("main", "DP-1", true, 1);
    s.navigate(2);
    s.take_changes(2);
    assert_eq!(kb(&s), KeyboardFocus::None, "navigation has the keyboard");
    s.key("Return", &[], 3);
    s.take_changes(3);
    assert_eq!(
        kb(&s),
        KeyboardFocus::Exclusive,
        "not held any more: asks again"
    );
    s.focus("main", "DP-1", true, 4);
    s.navigate(5);
    s.key("Escape", &[], 6);
    s.focus("main", "DP-1", false, 7);
    s.take_changes(7);
    assert_eq!(
        mode(&s),
        Mode::Expanded,
        "lost within the settle time: no close"
    );
    assert_eq!(kb(&s), KeyboardFocus::Exclusive);
}

#[test]
fn facts_replace_the_ui_modules_and_leave_a_user_modules_alone() {
    let mut s = store(cfg());
    let mut user = module("weather", 1.0);
    user.info.compact = false;
    s.set_module(user);
    s.set_ui_facts(vec![module("media", 10.0), module("timer", 20.0)]);
    s.set_ui_facts(vec![module("media", 10.0)]);
    assert!(s.module("media").is_some());
    assert!(s.module("timer").is_none(), "left the report: gone");
    assert!(
        s.module("weather").is_some(),
        "a user module is not the UI's"
    );
    s.set_ui_facts(vec![]);
    assert!(s.module("media").is_none());
    assert!(s.module("weather").is_some());
}

#[test]
fn the_copy_says_what_the_closed_notch_shows_and_what_the_tabs_are() {
    let mut s = store(json!({"modules": ["media", "timer"]}));
    s.take_changes(0);
    let c = &s.state().notches[0].copies[0];
    assert_eq!(
        c.compact,
        ["timer", "media"],
        "by priority, side by side with tabs"
    );
    assert_eq!(c.tabs, ["timer", "media"]);
    assert_eq!(
        c.shown,
        ["timer"],
        "the open notch shows the first tab's module"
    );
    let mut s = store(json!({"modules": ["media", "timer"], "tabs": false}));
    s.take_changes(0);
    let c = &s.state().notches[0].copies[0];
    assert_eq!(
        c.compact,
        ["timer"],
        "without tabs the compact module alone"
    );
    assert!(c.tabs.is_empty());
}

#[test]
fn a_module_opens_closes_or_asks_the_keyboard_for_the_notches_that_show_it() {
    let mut s = store(cfg());
    s.module_op("timer", ModuleOp::Open, 0);
    s.take_changes(0);
    assert_eq!(mode_at(&s, 0), Mode::Expanded);
    assert!(s.state().notches[0].copies[0].pinned);
    s.module_op("timer", ModuleOp::Close, 1);
    s.take_changes(1);
    assert_eq!(mode_at(&s, 0), Mode::Compact);
    // Hovered open, not pinned: asking for the keyboard pins it.
    s.pointer("main", "DP-1", PointerEv::Enter, 2);
    s.advance(500);
    s.take_changes(500);
    assert!(!s.state().notches[0].copies[0].pinned);
    s.module_op("timer", ModuleOp::Keyboard, 501);
    s.take_changes(501);
    assert!(s.state().notches[0].copies[0].pinned);
    // A module the open notch does not show pins nothing.
    s.module_op("timer", ModuleOp::Close, 502);
    s.module_op("nobody", ModuleOp::Keyboard, 503);
    s.take_changes(503);
    assert!(!s.state().notches[0].copies[0].pinned);
}

#[test]
fn what_a_dragged_copy_reserves_holds_until_it_lands() {
    let mut s = store(json!({"modules": ["media", "timer"], "reserveSpace": true}));
    s.measure("main", "DP-1", 6.0, 0.0, 30.0);
    s.take_changes(0);
    s.drag_start("main", "DP-1", 1);
    s.measure("main", "DP-1", 6.0, 40.0, 30.0);
    s.take_changes(2);
    assert_eq!(depth(&s), Some(30.0), "held while dragged");
    s.drag_cancel("main", "DP-1", 3);
    s.take_changes(3);
    assert_eq!(depth(&s), Some(40.0), "grows once it is let go");
}

#[test]
fn the_state_carries_the_settings_page() {
    let mut s = store(cfg());
    s.settings(true, Some("notch/1".into()));
    assert_eq!(s.state().settings_page, "notch/1");
    assert!(s.state().settings_open);
}

#[test]
fn a_notch_lists_the_hosts_default_modules_when_the_config_lists_none() {
    let mut s = Store::new();
    s.set_config(json!({}));
    s.set_screens(screens(&["DP-1"], "DP-1"));
    assert_eq!(s.state().notches[0].modules, ["media", "activities"]);
    s.set_ui_catalog(Vec::new(), Some(vec!["timer".into()]));
    assert_eq!(s.state().notches[0].modules, ["timer"]);
    s.set_config(json!({"modules": ["media"]}));
    assert_eq!(s.state().notches[0].modules, ["media"], "the config wins");
    s.set_ui_catalog(Vec::new(), None);
    s.set_config(json!({}));
    assert_eq!(s.state().notches[0].modules, ["media", "activities"]);
}

#[test]
fn a_guest_copy_shows_the_expanded_views_of_its_modules_while_open() {
    let mut s = store(cfg());
    let mut osd = module("osd", 10.0);
    osd.standalone = true;
    s.set_module(osd);
    s.take_changes(0);
    s.pointer("osd", "DP-1", PointerEv::Press, 0);
    s.take_changes(0);
    let c = &s.state().standalone[0].copies[0];
    assert_eq!(c.mode, Mode::Expanded);
    assert!(c.pinned);
    assert_eq!(c.shown, ["osd"]);
    assert_eq!(c.keyboard, KeyboardFocus::Exclusive);
    // The visitor opens on every module's expanded view.
    s.pointer("", "DP-1", PointerEv::Press, 1);
    s.take_changes(1);
    assert_eq!(s.state().visitor[0].shown, ["timer", "media"]);
}
