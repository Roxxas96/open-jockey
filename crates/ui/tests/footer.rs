//! The footer: the open list's facts, the latest message and one pill per background job.

use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use library::{BrowserView, Focus, TreeKind, TreeRow};
use ui::footer::eta_text;
use ui::{ControlFrame, EVENT_SHOWN, JobKind, JobOp, JobState, JobView, UiCommand, ViewState};

/// A browser on a list of `count` rows: the playlist "Midtempo Arsène" when `seconds` is
/// given, else a folder.
fn browser(count: usize, seconds: Option<f64>) -> BrowserView {
    BrowserView {
        path: if seconds.is_some() {
            "Playlists ▸ Midtempo Arsène".into()
        } else {
            "Macintosh HD ▸ Music".into()
        },
        index: 0,
        count,
        track: None,
        focus: Focus::List,
        tree: Vec::new(),
        tree_cursor: 0,
        list: Vec::new(),
        list_offset: 0,
        list_count: count,
        list_seconds: seconds,
        all_selected: false,
        list_cursor: 0,
        search: None,
        sort: None,
        seq: 0,
        playlist: seconds.map(|_| 1),
    }
}

#[test]
fn the_footer_reads_a_guest_list_like_a_playlist() {
    let mut view = browser(12, Some(3_600.0));
    view.path = "OJGUEST ▸ Serato ▸ OJ Serato A".into();
    // The browser holds no playlist for a guest list; its tree row says what it is.
    view.playlist = None;
    view.tree = vec![TreeRow {
        label: "OJ Serato A".into(),
        depth: 2,
        kind: TreeKind::GuestList,
        expanded: false,
        note: None,
        playlist: None,
        path: None,
    }];
    let (h, _) = full(ControlFrame {
        browser: Some(view),
        ..ControlFrame::default()
    });
    h.get_by_label("OJ Serato A · 12 tracks · 1 h");
}

fn job(kind: JobKind, label: &str) -> JobView {
    JobView {
        kind,
        label: label.into(),
        done: None,
        total: None,
        eta: None,
        state: JobState::Running,
        current: None,
        note: None,
        can_pause: true,
        can_stop: true,
        locked: None,
    }
}

fn analysis() -> JobView {
    JobView {
        done: Some(312),
        total: Some(4_536),
        eta: Some(Duration::from_secs(6 * 60)),
        current: Some("Nightdrive (Extended Mix).flac".into()),
        ..job(JobKind::Analysis, "Analysing")
    }
}

fn harness(frame: ControlFrame, size: [f32; 2]) -> (Harness<'static>, Receiver<UiCommand>) {
    let (commands, asked) = channel();
    let mut screen = ui::Screen {
        commands,
        ..ui::Screen::default()
    };
    let mut harness = Harness::builder()
        .with_size(size)
        .build_ui(move |ui| ui::root(ui, &frame, None, &mut screen));
    harness.run_steps(3);
    (harness, asked)
}

fn full(frame: ControlFrame) -> (Harness<'static>, Receiver<UiCommand>) {
    harness(frame, [1440.0, 900.0])
}

#[test]
fn the_footer_shows_the_list() {
    let mut frame = ControlFrame {
        browser: Some(browser(42, Some(9_060.0))),
        ..ControlFrame::default()
    };
    let (h, _) = full(frame.clone());
    h.get_by_label("Midtempo Arsène · 42 tracks · 2 h 31 min");
    frame.browser.as_mut().unwrap().all_selected = true;
    let (h, _) = full(frame);
    h.get_by_label("42 selected");
    let (h, _) = full(ControlFrame {
        browser: Some(browser(17, None)),
        ..ControlFrame::default()
    });
    h.get_by_label("17 files");
}

#[test]
fn the_footer_shows_the_latest_message_for_eight_seconds() {
    let now = Instant::now();
    let mut frame = ControlFrame {
        status: vec!["Deck 1 < A - B".into()],
        ..ControlFrame::default()
    };
    frame.events.push_back((now, "Added 3 tracks".into()));
    let (h, _) = full(frame.clone());
    let message = h.get_by_label("Added 3 tracks");
    assert!(message.rect().min.y > 800.0, "the message is in the footer");
    assert!(h.query_by_label("Deck 1 < A - B").is_none());
    frame.events[0].0 = now - EVENT_SHOWN - Duration::from_secs(1);
    let (h, _) = full(frame);
    h.get_by_label("Deck 1 < A - B");
    assert!(h.query_by_label("Added 3 tracks").is_none());
}

#[test]
fn an_error_stays_in_the_footer() {
    let frame = ControlFrame {
        error: Some("Bad mapping: no such file".into()),
        events: [(Instant::now(), "Added 3 tracks".into())].into(),
        ..ControlFrame::default()
    };
    let (h, _) = full(frame);
    let in_footer = h
        .query_all_by_label("Bad mapping: no such file")
        .any(|n| n.rect().min.y > 800.0);
    assert!(in_footer);
    assert!(h.query_by_label("Added 3 tracks").is_none());
}

#[test]
fn the_top_bar_has_no_status_line() {
    let frame = ControlFrame {
        status: vec!["Deck 1 < A - B".into()],
        ..ControlFrame::default()
    };
    let (h, _) = full(frame);
    let message = h.get_by_label("Deck 1 < A - B");
    assert!(message.rect().min.y > 800.0, "only the footer shows it");
}

#[test]
fn a_job_pill_shows_count_bar_and_time_left() {
    let frame = ControlFrame {
        jobs: vec![analysis()],
        ..ControlFrame::default()
    };
    let (h, _) = full(frame);
    h.get_by_label("Analysing 312 / 4,536");
    h.get_by_label("~6 min");
}

#[test]
fn eta_text_rounds() {
    let min = |m: u64| Duration::from_secs(m * 60);
    assert_eq!(eta_text(Duration::from_secs(30)), "~1 min");
    assert_eq!(eta_text(Duration::from_secs(89)), "~1 min");
    assert_eq!(eta_text(Duration::from_secs(6 * 60 + 20)), "~6 min");
    assert_eq!(eta_text(min(59)), "~59 min");
    assert_eq!(eta_text(min(61)), "~1 h");
    assert_eq!(eta_text(min(150)), "~3 h");
}

#[test]
fn an_indeterminate_job_has_no_bar() {
    let frame = ControlFrame {
        jobs: vec![job(JobKind::Scan, "Scanning Music…")],
        ..ControlFrame::default()
    };
    let (h, _) = full(frame);
    h.get_by_label("Scanning Music…");
    assert!(h.query_by_label("Scanning Music… bar").is_none());
    let with_bar = ControlFrame {
        jobs: vec![analysis()],
        ..ControlFrame::default()
    };
    let (h, _) = full(with_bar);
    h.get_by_label("Analysing bar");
}

#[test]
fn pause_and_stop_send_job_commands() {
    let frame = ControlFrame {
        jobs: vec![analysis()],
        ..ControlFrame::default()
    };
    let (mut h, asked) = full(frame);
    h.get_by_label("Pause Analysing").click();
    h.run_steps(2);
    h.get_by_label("Stop Analysing").click();
    h.run_steps(2);
    assert_eq!(
        asked.try_iter().collect::<Vec<_>>(),
        [
            UiCommand::Job {
                kind: JobKind::Analysis,
                op: JobOp::Pause
            },
            UiCommand::Job {
                kind: JobKind::Analysis,
                op: JobOp::Stop
            },
        ]
    );
    let paused = ControlFrame {
        jobs: vec![JobView {
            state: JobState::Paused,
            ..analysis()
        }],
        ..ControlFrame::default()
    };
    let (mut h, asked) = full(paused);
    h.get_by_label("Resume Analysing").click();
    h.run_steps(2);
    assert_eq!(
        asked.try_iter().collect::<Vec<_>>(),
        [UiCommand::Job {
            kind: JobKind::Analysis,
            op: JobOp::Resume
        }]
    );
}

#[test]
fn locked_buttons_are_disabled_with_a_tooltip() {
    let frame = ControlFrame {
        jobs: vec![JobView {
            locked: Some("Stems are being written"),
            ..analysis()
        }],
        ..ControlFrame::default()
    };
    let (mut h, asked) = full(frame);
    h.get_by_label("Pause Analysing").click();
    h.run_steps(2);
    assert!(asked.try_iter().next().is_none());
    h.get_by_label("Pause Analysing").hover();
    h.run_steps(4);
    h.get_by_label("Stems are being written");
}

#[test]
fn hovering_a_pill_shows_its_current_item() {
    let frame = ControlFrame {
        jobs: vec![analysis()],
        ..ControlFrame::default()
    };
    let (mut h, _) = full(frame);
    assert!(h.query_by_label("Nightdrive (Extended Mix).flac").is_none());
    h.get_by_label("Analysing 312 / 4,536").hover();
    h.run_steps(4);
    h.get_by_label("Nightdrive (Extended Mix).flac");
}

#[test]
fn the_footer_stays_one_line_when_narrow() {
    let long = "A long message about something that happened a moment ago, in words".repeat(3);
    let mut frame = ControlFrame {
        view: ViewState {
            four_decks: true,
            ..ViewState::default()
        },
        browser: Some(browser(4_536, Some(900_000.0))),
        jobs: vec![
            analysis(),
            JobView {
                done: Some(40),
                total: Some(120),
                eta: Some(Duration::from_secs(3 * 3600)),
                ..job(JobKind::Covers, "Covers")
            },
            JobView {
                done: Some(7),
                total: Some(9),
                ..job(JobKind::Export, "Exporting to SANDISK")
            },
        ],
        ..ControlFrame::default()
    };
    frame.events.push_back((Instant::now(), long));
    let (wide, _) = harness(frame.clone(), [1440.0, 900.0]);
    let (narrow, _) = harness(frame, [1024.0, 700.0]);
    let one_row = wide.get_by_label("footer").rect().height();
    let footer = narrow.get_by_label("footer").rect();
    assert_eq!(footer.height(), one_row);
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 700.0));
    let mut first_pill = f32::MAX;
    for name in ["Analysing", "Covers", "Exporting"] {
        for node in narrow.query_all_by_label_contains(name) {
            let rect = node.rect();
            assert!(window.contains_rect(rect), "{rect:?} leaves the window");
            assert!(rect.height() <= one_row);
            first_pill = first_pill.min(rect.min.x);
        }
    }
    // The message is cut before it reaches the first pill.
    let message = narrow.get_by_label_contains("A long message").rect();
    assert!(
        message.max.x <= first_pill,
        "{message:?} runs into a pill at {first_pill}"
    );
}

#[test]
fn the_import_while_writing_shows_disabled_buttons_with_why() {
    // The import's pill while it writes, as the app publishes it.
    let why = "The import is writing; it can't be stopped now.";
    let frame = ControlFrame {
        jobs: vec![JobView {
            can_pause: false,
            can_stop: false,
            locked: Some(why),
            ..job(JobKind::Import, "Writing playlists…")
        }],
        ..ControlFrame::default()
    };
    let (mut h, asked) = full(frame);
    h.get_by_label("Pause Writing playlists…").click();
    h.run_steps(2);
    h.get_by_label("Stop Writing playlists…").click();
    h.run_steps(2);
    assert!(asked.try_iter().next().is_none());
    h.get_by_label("Stop Writing playlists…").hover();
    h.run_steps(4);
    h.get_by_label(why);
}

#[test]
fn resume_works_while_pausing() {
    let frame = ControlFrame {
        jobs: vec![JobView {
            state: JobState::Pausing,
            ..analysis()
        }],
        ..ControlFrame::default()
    };
    let (mut h, asked) = full(frame);
    h.get_by_label("Resume Analysing").click();
    h.run_steps(2);
    assert_eq!(
        asked.try_iter().collect::<Vec<_>>(),
        [UiCommand::Job {
            kind: JobKind::Analysis,
            op: JobOp::Resume
        }]
    );
}

#[test]
fn the_message_falls_back_to_a_deck_line_never_the_browser_line() {
    let browser_line = "▸ Playlists ▸ Peak time  12/48   analyzing 0/0";
    let frame = ControlFrame {
        browser: Some(browser(48, Some(600.0))),
        status: vec![
            browser_line.into(),
            ">Deck 1 < A - B".into(),
            " Deck 2 < C - D".into(),
        ],
        ..ControlFrame::default()
    };
    let (h, _) = full(frame.clone());
    let message = h.get_by_label(">Deck 1 < A - B");
    assert!(
        message.rect().min.y > 800.0,
        "the deck line is in the footer"
    );
    assert!(h.query_by_label_contains("12/48").is_none());
    // With no deck loaded there is only the browser line: the middle stays blank.
    let alone = ControlFrame {
        status: vec![browser_line.into()],
        ..frame
    };
    let (h, _) = full(alone);
    assert!(h.query_by_label_contains("12/48").is_none());
}

#[test]
fn the_list_facts_follow_the_kind_of_list() {
    let shows = |view: BrowserView, text: &str| {
        let (h, _) = full(ControlFrame {
            browser: Some(view),
            ..ControlFrame::default()
        });
        h.get_by_label(text);
    };
    // A playlist is a playlist even before its length is known.
    let mut playlist = browser(42, None);
    playlist.path = "Playlists ▸ Midtempo Arsène".into();
    playlist.playlist = Some(1);
    shows(playlist, "Midtempo Arsène · 42 tracks");
    // An exact hour has no minutes; the minutes are not zero-padded.
    shows(
        browser(3, Some(7_200.0)),
        "Midtempo Arsène · 3 tracks · 2 h",
    );
    shows(
        browser(3, Some(3_900.0)),
        "Midtempo Arsène · 3 tracks · 1 h 5 min",
    );
    // The Playlists node lists nothing: no facts at all.
    let node = BrowserView {
        path: "Playlists".into(),
        tree: vec![library::TreeRow {
            label: "Playlists".into(),
            depth: 0,
            kind: library::TreeKind::Playlists,
            expanded: true,
            note: None,
            playlist: None,
            path: None,
        }],
        tree_cursor: 0,
        ..browser(0, None)
    };
    let (h, _) = full(ControlFrame {
        browser: Some(node),
        ..ControlFrame::default()
    });
    assert!(h.query_by_label("0 files").is_none());
}
