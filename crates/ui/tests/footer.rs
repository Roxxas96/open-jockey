//! The footer: the open list's facts, the latest message and one pill per background job.

use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use library::{BrowserView, Focus};
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
