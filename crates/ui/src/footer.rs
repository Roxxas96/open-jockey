//! The footer under the browser: one line with the open list's facts on the left, the
//! latest message in the middle and one pill per background job on the right.
//!
//! The row is laid out by hand so it never takes a second line. When it is too narrow
//! the time left goes first, then the list's total length, then the message is cut with
//! "…"; pills never wrap, and when they still do not fit the rightmost collapse to their
//! label and bar.

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use egui::{
    Align, CornerRadius, FontId, Id, Layout, Rect, Sense, Stroke, Ui, UiBuilder, Vec2, pos2,
};

use library::TreeKind;

use crate::command::UiCommand;
use crate::model::{ControlFrame, JobOp, JobState, JobView};
use crate::theme;

/// The footer's inner height: one row of text.
pub const ROW_HEIGHT: f32 = 20.0;
/// The space between the left text, the message and the pills.
const GAP: f32 = 10.0;
/// The space between two pills.
const PILL_GAP: f32 = 6.0;
/// The space inside a pill, around its parts.
const PILL_PAD: f32 = 6.0;
const PART_GAP: f32 = 5.0;
const BAR: Vec2 = Vec2::new(54.0, 4.0);
const BUTTON: f32 = 16.0;
/// A message takes at most this share of the row before the rest gives way to it.
const MESSAGE_SHARE: f32 = 0.5;

/// The time left as the pill says it: "~1 min" under 90 s, "~N min" under an hour, else
/// "~N h", rounded to whole hours and at least 1.
#[must_use]
pub fn eta_text(left: Duration) -> String {
    let secs = left.as_secs_f64();
    if secs < 90.0 {
        "~1 min".to_owned()
    } else if secs < 3600.0 {
        format!("~{} min", (secs / 60.0).round())
    } else {
        format!("~{} h", (secs / 3600.0).round().max(1.0))
    }
}

/// `4536` as "4,536".
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// A length as "2 h 31 min", "2 h", "12 min" or "40 s".
fn length_text(seconds: f64) -> String {
    let minutes = (seconds / 60.0).round() as u64;
    match (minutes / 60, minutes % 60) {
        _ if minutes == 0 => format!("{} s", seconds.round() as u64),
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// The left text, without or with the list's total length; nothing for the Playlists
/// node, which lists nothing.
fn list_text(frame: &ControlFrame, with_length: bool) -> String {
    let Some(view) = &frame.browser else {
        return String::new();
    };
    let n = view.list_count;
    if view.all_selected {
        return format!("{} selected", grouped(n));
    }
    if view.playlist.is_none() {
        let node = view.tree.get(view.tree_cursor);
        if node.is_some_and(|row| row.kind == TreeKind::Playlists) {
            return String::new();
        }
        let files = if n == 1 { "file" } else { "files" };
        return format!("{} {files}", grouped(n));
    }
    let name = view.path.rsplit('▸').next().unwrap_or("").trim();
    let tracks = if n == 1 { "track" } else { "tracks" };
    let mut text = format!("{name} · {} {tracks}", grouped(n));
    if let Some(seconds) = view.list_seconds
        && with_length
        && seconds >= 1.0
    {
        text.push_str(" · ");
        text.push_str(&length_text(seconds));
    }
    text
}

/// What a pill shows, from widest to narrowest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
    /// The label and count, the bar, the time left (or note) and the buttons.
    Full,
    /// [`Form::Full`] without the time left.
    NoEta,
    /// The label and the bar.
    Short,
}

/// The parts of one pill's text.
struct Parts {
    head: String,
    /// The time left, a note or the state; the time left is the first to go.
    tail: Option<(String, bool)>,
    bar: Option<f32>,
}

fn parts(job: &JobView) -> Parts {
    let mut head = job.label.clone();
    if let Some(done) = job.done {
        head.push(' ');
        head.push_str(&grouped(done));
        if let Some(total) = job.total {
            head.push_str(" / ");
            head.push_str(&grouped(total));
        }
    }
    let tail = match job.state {
        JobState::Paused => Some(("Paused".to_owned(), false)),
        JobState::Pausing => Some(("Pausing…".to_owned(), false)),
        JobState::Running => match (&job.note, job.eta) {
            (Some(note), _) => Some((note.clone(), false)),
            (None, Some(eta)) => Some((eta_text(eta), true)),
            (None, None) => None,
        },
    };
    let bar = match (job.done, job.total) {
        (Some(done), Some(total)) if total > 0 => {
            Some((done.min(total) as f64 / total as f64) as f32)
        }
        _ => None,
    };
    Parts { head, tail, bar }
}

/// Whether a pill has a Pause (or Resume) button and a Stop button: a locked job shows
/// both, disabled, so the pointer can find out why.
fn has_buttons(job: &JobView) -> (bool, bool) {
    let locked = job.locked.is_some();
    (job.can_pause || locked, job.can_stop || locked)
}

/// How many buttons a pill has.
fn buttons(job: &JobView) -> usize {
    let (pause, stop) = has_buttons(job);
    usize::from(pause) + usize::from(stop)
}

/// The text widths a pill needs, measured once.
struct Measured {
    head: f32,
    tail: f32,
    /// Whether `tail` is a time left.
    tail_is_eta: bool,
    bar: bool,
    buttons: usize,
}

impl Measured {
    fn width(&self, form: Form) -> f32 {
        let mut width = PILL_PAD * 2.0 + self.head;
        let bar = self.bar.then_some(BAR.x);
        if let Some(bar) = bar {
            width += PART_GAP + bar;
        }
        if form == Form::Short {
            return width;
        }
        if self.tail > 0.0 && (form == Form::Full || !self.tail_is_eta) {
            width += PART_GAP + self.tail;
        }
        width + self.buttons as f32 * (PART_GAP + BUTTON)
    }
}

fn text_width(ui: &Ui, text: &str, font: &FontId) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    ui.painter()
        .layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::WHITE)
        .size()
        .x
}

/// Draws the footer row in `ui`: the list's facts, the message and the job pills.
/// Pause, resume and stop on a pill send [`UiCommand::Job`].
pub fn show(ui: &mut Ui, frame: &ControlFrame, commands: &Sender<UiCommand>) {
    let p = theme::palette(ui.ctx());
    let font = egui::TextStyle::Body.resolve(ui.style());
    let row = ui.max_rect();
    let row = Rect::from_min_size(row.min, Vec2::new(row.width(), ROW_HEIGHT));
    ui.allocate_rect(row, Sense::hover());

    // The message: the error, else the newest event still shown, else the status text.
    let (message, message_colour, strong) = match &frame.error {
        Some(error) => (error.as_str(), p.error, true),
        None => {
            let own = crate::browser::notice(ui.ctx());
            (
                frame.status_line_with(Instant::now(), own),
                ui.visuals().text_color(),
                false,
            )
        }
    };
    let message_width = text_width(ui, message, &font);
    let measured: Vec<Measured> = frame
        .jobs
        .iter()
        .map(|job| {
            let parts = parts(job);
            Measured {
                head: text_width(ui, &parts.head, &font),
                tail: parts
                    .tail
                    .as_ref()
                    .map_or(0.0, |(t, _)| text_width(ui, t, &font)),
                tail_is_eta: parts.tail.as_ref().is_some_and(|(_, eta)| *eta),
                bar: parts.bar.is_some(),
                buttons: buttons(job),
            }
        })
        .collect();
    let pills_width = |forms: &[Form]| -> f32 {
        let widths: f32 = measured.iter().zip(forms).map(|(m, f)| m.width(*f)).sum();
        widths + PILL_GAP * measured.len().saturating_sub(1) as f32
    };

    // Give up the time left first, then the list's length, before the message is cut.
    let mut forms = vec![Form::Full; measured.len()];
    let wanted = message_width.min(row.width() * MESSAGE_SHARE);
    let fits =
        |left: f32, pills: f32, wanted: f32| left + wanted + pills + GAP * 2.0 <= row.width();
    let mut left = list_text(frame, true);
    if !fits(text_width(ui, &left, &font), pills_width(&forms), wanted) {
        forms.fill(Form::NoEta);
    }
    if !fits(text_width(ui, &left, &font), pills_width(&forms), wanted) {
        left = list_text(frame, false);
    }
    let left_width = text_width(ui, &left, &font);
    // Still too wide with no message: the rightmost pills collapse to label and bar.
    let mut collapsed = forms.len();
    while pills_width(&forms) + left_width + GAP > row.width() && collapsed > 0 {
        collapsed -= 1;
        forms[collapsed] = Form::Short;
    }

    // Pills from the right edge; one that would leave the row is not drawn.
    let mut x = row.max.x;
    let floor = row.min.x + left_width.min(row.width() * 0.3);
    let mut pills_left = row.max.x;
    for ((job, m), form) in frame.jobs.iter().zip(&measured).zip(&forms).rev() {
        let width = m.width(*form);
        if x - width < floor {
            break;
        }
        let rect = Rect::from_min_size(pos2(x - width, row.min.y), Vec2::new(width, ROW_HEIGHT));
        pill(ui, rect, job, *form, commands);
        x -= width + PILL_GAP;
        pills_left = rect.min.x;
    }

    let left_room = (pills_left - row.min.x - GAP)
        .max(0.0)
        .min(left_width.ceil() + 1.0);
    let left_rect = Rect::from_min_size(row.min, Vec2::new(left_room, ROW_HEIGHT));
    label(ui, left_rect, &left, ui.visuals().text_color(), false);
    let message_x = left_rect.max.x + GAP;
    let message_room = pills_left - GAP - message_x;
    if message_room > 12.0 {
        let rect = Rect::from_min_size(
            pos2(message_x, row.min.y),
            Vec2::new(message_room, ROW_HEIGHT),
        );
        label(ui, rect, message, message_colour, strong);
    }
}

/// `text` truncated with "…" inside `rect`, on one line.
fn label(ui: &mut Ui, rect: Rect, text: &str, colour: egui::Color32, strong: bool) {
    if text.is_empty() || rect.width() <= 0.0 {
        return;
    }
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            let mut rich = egui::RichText::new(text).color(colour);
            if strong {
                rich = rich.strong();
            }
            ui.add(egui::Label::new(rich).truncate());
        },
    );
}

/// One job's pill in `rect`.
fn pill(ui: &mut Ui, rect: Rect, job: &JobView, form: Form, commands: &Sender<UiCommand>) {
    let p = theme::palette(ui.ctx());
    let id = Id::NULL.with(("footer pill", job.kind));
    let hover = ui.interact(rect, id, Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(3), p.row_alt);
    let parts = parts(job);
    let colour = if job.state == JobState::Paused {
        p.text_dim
    } else {
        ui.visuals().text_color()
    };
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(PILL_PAD, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = PART_GAP;
            ui.add(
                egui::Label::new(egui::RichText::new(&parts.head).color(colour))
                    .extend()
                    .selectable(false),
            );
            if let Some(share) = parts.bar {
                bar(ui, job, share);
            }
            if form == Form::Short {
                return;
            }
            if let Some((tail, is_eta)) = &parts.tail
                && (form == Form::Full || !*is_eta)
            {
                ui.add(
                    egui::Label::new(egui::RichText::new(tail).color(p.text_dim))
                        .extend()
                        .selectable(false),
                );
            }
            let (pause, stop) = has_buttons(job);
            if pause {
                let (op, name) = match job.state {
                    JobState::Running => (JobOp::Pause, "Pause"),
                    JobState::Pausing | JobState::Paused => (JobOp::Resume, "Resume"),
                };
                let enabled = job.locked.is_none();
                let sent = icon_button(ui, job, name, op, enabled);
                if sent {
                    let _ = commands.send(UiCommand::Job { kind: job.kind, op });
                }
            }
            if stop {
                let enabled = job.locked.is_none();
                if icon_button(ui, job, "Stop", JobOp::Stop, enabled) {
                    let _ = commands.send(UiCommand::Job {
                        kind: job.kind,
                        op: JobOp::Stop,
                    });
                }
            }
        },
    );
    hover.on_hover_ui(|ui| {
        match (&job.current, &job.note) {
            (Some(current), _) => ui.label(current),
            (None, Some(note)) => ui.label(note),
            (None, None) => ui.label(&job.label),
        };
    });
}

/// A thin bar filled to `share`, named for screen readers.
fn bar(ui: &mut Ui, job: &JobView, share: f32) {
    let p = theme::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(BAR, Sense::hover());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::ProgressIndicator,
            true,
            format!("{} bar", job.label),
        )
    });
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(2), p.lane);
    let filled = Rect::from_min_size(
        rect.min,
        Vec2::new(rect.width() * share.clamp(0.0, 1.0), rect.height()),
    );
    let colour = if job.state == JobState::Running {
        p.accent_left
    } else {
        p.text_dim
    };
    painter.rect_filled(filled, CornerRadius::same(2), colour);
}

/// A small button painted as a shape (pause bars, resume triangle, stop square), since
/// no font has the glyphs for sure. Returns whether it was clicked; a disabled one shows
/// the job's `locked` reason on hover.
fn icon_button(ui: &mut Ui, job: &JobView, name: &str, op: JobOp, enabled: bool) -> bool {
    let p = theme::palette(ui.ctx());
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(BUTTON), sense);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, enabled, format!("{name} {}", job.label))
    });
    let colour = match (enabled, response.hovered()) {
        (false, _) => p.text_dim.gamma_multiply(0.5),
        (true, true) => p.text,
        (true, false) => p.text_dim,
    };
    let painter = ui.painter();
    let c = rect.center();
    match op {
        JobOp::Pause => {
            for dx in [-2.5, 2.5] {
                let bar = Rect::from_center_size(pos2(c.x + dx, c.y), Vec2::new(2.5, 8.0));
                painter.rect_filled(bar, CornerRadius::ZERO, colour);
            }
        }
        JobOp::Resume => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pos2(c.x - 3.0, c.y - 4.5),
                    pos2(c.x - 3.0, c.y + 4.5),
                    pos2(c.x + 4.5, c.y),
                ],
                colour,
                Stroke::NONE,
            ));
        }
        JobOp::Stop => {
            let square = Rect::from_center_size(c, Vec2::splat(8.0));
            painter.rect_filled(square, CornerRadius::ZERO, colour);
        }
    }
    if let Some(why) = job.locked {
        response.on_hover_text(why);
        return false;
    }
    response.on_hover_text(name).clicked()
}
