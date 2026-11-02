use super::*;
use notchd_proto::ui::{ScreenInfo, Workspace};

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
    assert!(
        s.state().notches[0].copies[0].keyboard,
        "pinned open takes the keyboard"
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
    assert_eq!(kb, [false, true, false]);
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
