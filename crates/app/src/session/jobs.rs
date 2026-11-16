//! The background jobs the frame lists: the time left of each ([`Pace`]) and the analysis
//! pill. The footer reads these from the frame; nothing here waits on a job.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ui::{JobKind, JobState, JobView};

/// How far back [`Pace`] looks to measure a job's rate.
const WINDOW: Duration = Duration::from_secs(60);
/// A time left is given only after this many items are done ...
const MIN_DONE: usize = 3;
/// ... and the job has been sampled for this long.
const MIN_RUN: Duration = Duration::from_secs(5);

/// Estimates the time left of a job from how fast it went in the last minute.
#[derive(Debug, Default)]
pub(crate) struct Pace {
    /// `(when, done)`, oldest first, within [`WINDOW`] of the newest.
    samples: VecDeque<(Instant, usize)>,
    /// When the first sample of this run was taken.
    started: Option<Instant>,
    total: usize,
    paused: bool,
}

impl Pace {
    /// A measure with no samples: no estimate until some come in.
    pub fn new() -> Self {
        Self::default()
    }

    /// Notes that `done` of `total` items are finished at `now`. A `total` that shrinks
    /// starts the measure again (a new run); one that grows (more queued) keeps it. `done`
    /// never counts past `total`. Sampling also ends a [`Pace::pause`].
    pub fn sample(&mut self, now: Instant, done: usize, total: usize) {
        if total < self.total {
            self.samples.clear();
            self.started = None;
        }
        self.total = total;
        self.paused = false;
        self.started.get_or_insert(now);
        self.samples.push_back((now, done.min(total)));
        while self
            .samples
            .front()
            .is_some_and(|&(at, _)| now.saturating_duration_since(at) > WINDOW)
        {
            self.samples.pop_front();
        }
    }

    /// The job is paused: its samples are dropped and there is no time left to give until
    /// it is sampled again, so the pause is not counted as slowness.
    pub fn pause(&mut self) {
        self.samples.clear();
        self.started = None;
        self.paused = true;
    }

    /// The time left, once at least three items are done and the job has been sampled
    /// for 5 s; `None` while paused, or when nothing moved in the window.
    pub fn eta(&self, now: Instant) -> Option<Duration> {
        if self.paused {
            return None;
        }
        let (&(first_at, first_done), &(last_at, last_done)) =
            (self.samples.front()?, self.samples.back()?);
        let started = self.started?;
        if last_done < MIN_DONE || now.saturating_duration_since(started) < MIN_RUN {
            return None;
        }
        let remaining = self.total.saturating_sub(last_done);
        if remaining == 0 {
            return Some(Duration::ZERO);
        }
        let span = last_at.saturating_duration_since(first_at).as_secs_f64();
        let moved = last_done.saturating_sub(first_done);
        if span <= 0.0 || moved == 0 {
            return None;
        }
        let secs = remaining as f64 * span / moved as f64;
        secs.is_finite().then(|| Duration::from_secs_f64(secs))
    }
}

/// The jobs the frame lists, ordered by kind: library mode's, then the session's own
/// (none yet).
pub(super) fn gather(
    library: Option<&crate::library_mode::LibraryMode>,
    now: Instant,
) -> Vec<JobView> {
    let mut jobs = library.map(|l| l.jobs(now)).unwrap_or_default();
    jobs.sort_by_key(|job| job.kind);
    jobs
}

/// What the analysis pill needs to know.
pub(crate) struct AnalysisFacts {
    /// Tracks waiting or being analyzed.
    pub queued: usize,
    /// Of those, the tracks being analyzed right now.
    pub active: usize,
    /// Tracks finished in this run, and in all.
    pub done: usize,
    /// Tracks queued in this run.
    pub total: usize,
    /// The user paused the analysis.
    pub paused: bool,
    /// Stop was pressed: tracks in flight finish without a pill.
    pub stopped: bool,
    /// The track being analyzed, if any.
    pub current: Option<String>,
    /// The time left, once [`Pace`] can tell.
    pub eta: Option<Duration>,
}

/// The analysis pill, or `None` when nothing is queued or it was stopped.
pub(crate) fn analysis_job(facts: AnalysisFacts) -> Option<JobView> {
    if facts.queued == 0 || facts.stopped {
        return None;
    }
    let state = match (facts.paused, facts.active) {
        (false, _) => JobState::Running,
        (true, 0) => JobState::Paused,
        (true, _) => JobState::Pausing,
    };
    Some(JobView {
        kind: JobKind::Analysis,
        label: "Analysing".to_owned(),
        done: Some(facts.done.min(facts.total)),
        total: Some(facts.total),
        eta: if facts.paused { None } else { facts.eta },
        state,
        current: facts.current,
        note: None,
        can_pause: true,
        can_stop: true,
        locked: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn pace_has_no_eta_before_three_items_and_five_seconds() {
        let t0 = Instant::now();
        let mut pace = Pace::new();
        pace.sample(t0, 0, 100);
        pace.sample(t0 + secs(2), 2, 100);
        assert_eq!(pace.eta(t0 + secs(2)), None, "two items, two seconds");
        pace.sample(t0 + secs(3), 3, 100);
        assert_eq!(
            pace.eta(t0 + secs(3)),
            None,
            "three items but under five seconds"
        );
        pace.sample(t0 + secs(6), 6, 100);
        assert!(pace.eta(t0 + secs(6)).is_some());
        // Three items in over five seconds are enough; two are not.
        let mut few = Pace::new();
        few.sample(t0, 0, 100);
        few.sample(t0 + secs(30), 2, 100);
        assert_eq!(few.eta(t0 + secs(30)), None);
    }

    #[test]
    fn pace_estimates_from_the_last_minute() {
        let t0 = Instant::now();
        let mut pace = Pace::new();
        pace.sample(t0, 0, 110);
        pace.sample(t0 + secs(20), 10, 110);
        let eta = pace.eta(t0 + secs(20)).unwrap();
        assert!((eta.as_secs_f64() - 200.0).abs() < 1.0, "{eta:?}");

        // A slow start does not weigh on a fast minute: 10 items in the first 60 s, then
        // 60 in the next 60 s, 30 left.
        let mut pace = Pace::new();
        pace.sample(t0, 0, 100);
        pace.sample(t0 + secs(60), 10, 100);
        pace.sample(t0 + secs(120), 70, 100);
        let eta = pace.eta(t0 + secs(120)).unwrap();
        assert!((eta.as_secs_f64() - 30.0).abs() < 1.0, "{eta:?}");
    }

    #[test]
    fn pace_survives_a_growing_total() {
        let t0 = Instant::now();
        let mut pace = Pace::new();
        pace.sample(t0, 0, 10);
        pace.sample(t0 + secs(10), 5, 10);
        // Everything finished, then more tracks queued, then a count past the total.
        pace.sample(t0 + secs(20), 10, 10);
        pace.sample(t0 + secs(30), 12, 40);
        let eta = pace.eta(t0 + secs(30)).unwrap();
        assert!(eta.as_secs_f64().is_finite());
        // Remaining 28 at 12 items in 30 s.
        assert!((eta.as_secs_f64() - 70.0).abs() < 1.0, "{eta:?}");
        // A done above the total is held to it: nothing negative.
        pace.sample(t0 + secs(40), 50, 40);
        assert_eq!(pace.eta(t0 + secs(40)), Some(Duration::ZERO));
    }

    #[test]
    fn pace_starts_again_when_the_total_shrinks() {
        let t0 = Instant::now();
        let mut pace = Pace::new();
        pace.sample(t0, 0, 100);
        pace.sample(t0 + secs(20), 10, 100);
        pace.sample(t0 + secs(21), 0, 5);
        assert_eq!(pace.eta(t0 + secs(21)), None);
    }

    #[test]
    fn pace_gives_nothing_while_paused() {
        let t0 = Instant::now();
        let mut pace = Pace::new();
        pace.sample(t0, 0, 100);
        pace.sample(t0 + secs(20), 10, 100);
        assert!(pace.eta(t0 + secs(20)).is_some());
        pace.pause();
        assert_eq!(pace.eta(t0 + secs(40)), None);
        // After resuming, the pause does not count as slowness: the estimate waits for
        // fresh samples.
        pace.sample(t0 + secs(100), 10, 100);
        assert_eq!(pace.eta(t0 + secs(100)), None);
        pace.sample(t0 + secs(110), 20, 100);
        let eta = pace.eta(t0 + secs(110)).unwrap();
        assert!((eta.as_secs_f64() - 80.0).abs() < 1.0, "{eta:?}");
    }

    fn facts() -> AnalysisFacts {
        AnalysisFacts {
            queued: 5,
            active: 2,
            done: 7,
            total: 12,
            paused: false,
            stopped: false,
            current: Some("Artist – Title".to_owned()),
            eta: Some(secs(90)),
        }
    }

    #[test]
    fn the_analysis_pill_names_the_track_it_is_on() {
        let job = analysis_job(facts()).unwrap();
        assert_eq!(job.kind, JobKind::Analysis);
        assert_eq!(job.label, "Analysing");
        assert_eq!((job.done, job.total), (Some(7), Some(12)));
        assert_eq!(job.current.as_deref(), Some("Artist – Title"));
        assert_eq!(job.eta, Some(secs(90)));
        assert_eq!(job.state, JobState::Running);
        assert!(job.can_pause && job.can_stop && job.locked.is_none());
    }

    #[test]
    fn a_paused_analysis_says_pausing_until_in_flight_ends() {
        let pausing = analysis_job(AnalysisFacts {
            paused: true,
            ..facts()
        })
        .unwrap();
        assert_eq!(pausing.state, JobState::Pausing);
        assert_eq!(pausing.eta, None);
        let paused = analysis_job(AnalysisFacts {
            paused: true,
            active: 0,
            ..facts()
        })
        .unwrap();
        assert_eq!(paused.state, JobState::Paused);
        assert_eq!(paused.eta, None);
    }

    #[test]
    fn no_pill_when_idle_or_stopped() {
        assert!(
            analysis_job(AnalysisFacts {
                queued: 0,
                active: 0,
                ..facts()
            })
            .is_none()
        );
        assert!(
            analysis_job(AnalysisFacts {
                stopped: true,
                ..facts()
            })
            .is_none()
        );
    }
}
