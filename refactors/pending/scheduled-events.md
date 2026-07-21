# firing at a wall-clock time

A timer today is a delay: a handler asks for one, the effect loop sleeps that long, and the firing comes back as an event. That covers everything a layer needs, because a layer's timeouts are all "so many milliseconds from now".

A job that runs at 09:00 on weekdays is not a delay. It is a time, and turning it into a delay requires knowing what time it is now, which no handler may read.

This is that timer. A schedule is a value in the model, the model holds a guard for the firing it is waiting for, and each firing carries the wall-clock time it happened at, which is what the handler computes the next one from. The model never holds "what time it is": it holds "what I am waiting for", and the answer to "what time is it" arrives with the firing that needed it.

## Why a source and not an effect

`schedule_timer` performs a delay by spawning a task that sleeps. A wall-clock deadline cannot be performed that way, for two reasons that are the same reason:

`Instant` on macOS does not advance while the machine is asleep. A sleep of eight hours, started at 01:00 on a machine that suspends for three of them, fires at 12:00. So a deadline has to be re-derived from the wall clock whenever the machine wakes and whenever the clock is set.

And a deadline that passed while the machine was asleep has to fire once, not once per occurrence it slept through. That means the firing has to carry the time it actually fired at, so the handler's next computation starts from now rather than from a Tuesday three weeks ago.

Both are facts about the outside world observed as they happen, which is a source. So `freddie_schedule` is a source with a sink: `arm` is the sink, and the firing is the event.

## Change 1: the schedule, and when it next fires

`crates/freddie/src/schedule.rs`. Calendar arithmetic and nothing else — no threads, no OS, and every function here is pure, so the whole file is a test table.

`jiff` for the calendar. Weekday arithmetic and the local zone are its job; the workspace gains `jiff = "0.2"`.

```rust
use jiff::civil::{Date, DateTime, Time, Weekday};

/// The days of the week a schedule fires on.
///
/// A set rather than a list, so a day cannot appear twice, and built only from the constructors
/// below, so it cannot be empty: a schedule that fires on no day is a schedule that does nothing,
/// and there is no way to spell one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Weekdays(u8);

impl Weekdays {
    pub const EVERY_DAY: Self = Self(0b0111_1111);
    pub const WEEKDAYS: Self = Self(0b0001_1111);
    pub const WEEKENDS: Self = Self(0b0110_0000);

    /// One day.
    #[must_use]
    pub const fn just(day: Weekday) -> Self {
        Self(1 << (day.to_monday_zero_offset() as u8))
    }

    /// Both, so several `just`s compose.
    #[must_use]
    pub const fn and(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[must_use]
    pub const fn contains(self, day: Weekday) -> bool {
        self.0 & (1 << (day.to_monday_zero_offset() as u8)) != 0
    }
}

/// A recurring time of day: 09:00 on weekdays.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Recurring {
    pub days: Weekdays,
    pub at: Time,
}

/// When something fires.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Schedule {
    Recurring(Recurring),
    /// One local time, once.
    Once(DateTime),
}

impl Schedule {
    /// The first firing strictly after `now`, in local civil time.
    ///
    /// `None` only for a [`Once`](Self::Once) whose time has passed, which is a schedule that will
    /// never fire again. A recurring schedule always has a next one, because [`Weekdays`] cannot be
    /// empty.
    ///
    /// Strictly after, so a firing that lands exactly on its own time computes tomorrow's rather
    /// than its own, which is what stops a handler from re-arming a deadline that is already due.
    #[must_use]
    pub fn next_after(&self, now: DateTime) -> Option<DateTime> {
        match self {
            Self::Once(at) => (*at > now).then_some(*at),
            Self::Recurring(recurring) => (0..=7).find_map(|days| {
                let date: Date = now.date().checked_add(jiff::Span::new().days(days)).ok()?;
                let at = date.at(recurring.at.hour(), recurring.at.minute(), 0, 0);
                (at > now && recurring.days.contains(date.weekday())).then_some(at)
            }),
        }
    }
}
```

`0..=7` and not `0..7`: a daily schedule whose time has already passed today has its next firing on the same weekday next week when that is the only day it fires on, and seven days ahead is that day.

The tests:

```rust
#[test]
fn a_recurring_schedule_finds_its_next_day() {
    let nine = Recurring { days: Weekdays::WEEKDAYS, at: time(9, 0, 0, 0) };
    let schedule = Schedule::Recurring(nine);
    for (now, want) in [
        // Tuesday morning, before nine: today.
        (date(2026, 7, 21).at(8, 59, 0, 0), date(2026, 7, 21).at(9, 0, 0, 0)),
        // Exactly nine: tomorrow, not this instant again.
        (date(2026, 7, 21).at(9, 0, 0, 0), date(2026, 7, 22).at(9, 0, 0, 0)),
        // Friday afternoon: Monday.
        (date(2026, 7, 24).at(17, 0, 0, 0), date(2026, 7, 27).at(9, 0, 0, 0)),
        // Saturday: Monday.
        (date(2026, 7, 25).at(1, 0, 0, 0), date(2026, 7, 27).at(9, 0, 0, 0)),
    ] {
        assert_eq!(schedule.next_after(now), Some(want), "{now}");
    }
}

#[test]
fn a_single_weekday_wraps_to_the_next_week() {
    let schedule = Schedule::Recurring(Recurring {
        days: Weekdays::just(Weekday::Tuesday),
        at: time(9, 0, 0, 0),
    });
    // Tuesday, after nine.
    assert_eq!(
        schedule.next_after(date(2026, 7, 21).at(9, 1, 0, 0)),
        Some(date(2026, 7, 28).at(9, 0, 0, 0))
    );
}

#[test]
fn a_once_that_has_passed_never_fires_again() {
    let schedule = Schedule::Once(date(2026, 7, 21).at(9, 0, 0, 0));
    assert_eq!(schedule.next_after(date(2026, 7, 21).at(8, 0, 0, 0)), Some(date(2026, 7, 21).at(9, 0, 0, 0)));
    assert_eq!(schedule.next_after(date(2026, 7, 21).at(9, 0, 0, 0)), None);
}
```

## Change 2: the guard, the firing, and the trigger

The same shape `timer.rs` has, in the same file, because a schedule is cancelled by dropping what holds it exactly as a timer is. `TimerId` and `drop_guard` are shared; what differs is that the firing carries a time.

```rust
/// A scheduled firing, carrying which schedule it was and the local time it happened at.
///
/// The time is when it fired, not when it was due. They differ when the machine was asleep at the
/// due time, and the difference is the point: a handler computing the next firing from this skips
/// every occurrence that was slept through instead of working through them.
#[derive(Debug)]
pub struct ScheduleFired {
    pub id: TimerId,
    pub at: DateTime,
}

/// Two firings compare equal under `testing` whatever their ids, for the reason [`TimerFired`]'s
/// do: a test rebuilding an expected event cannot know an id. The time is compared.
#[cfg(feature = "testing")]
impl PartialEq for ScheduleFired {
    fn eq(&self, other: &Self) -> bool {
        self.at == other.at
    }
}

#[cfg(feature = "testing")]
impl Eq for ScheduleFired {}

/// Matches only the firing of the schedule it was built from.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ScheduleTrigger(TimerId);

impl EventTrigger for ScheduleTrigger {
    type Event = ScheduleFired;
    fn is_matching(&self, ev: &ScheduleFired) -> bool {
        self.0 == ev.id
    }
}

/// Dropping it cancels the firing it is waiting for.
#[must_use = "dropping the guard cancels the schedule immediately"]
#[derive(Debug)]
pub struct ScheduleGuard {
    id: TimerId,
    #[expect(dead_code)]
    guard: DropGuard,
}

impl ScheduleGuard {
    /// The trigger matching this schedule's own firing, and no other.
    #[must_use]
    pub const fn trigger(&self) -> ScheduleTrigger {
        ScheduleTrigger(self.id)
    }
}

/// The arming half: the local time to fire at, the event to fire, and the cancel channel.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct ScheduleEffect<E> {
    pub at: DateTime,
    pub event: E,
    pub cancel: AlwaysEqual<oneshot::Receiver<()>>,
}

/// Build a linked guard and effect that fires at `at`.
pub fn schedule_effect_and_guard<E>(
    at: DateTime,
    event: impl FnOnce(TimerId) -> E,
) -> (ScheduleGuard, ScheduleEffect<E>) {
    let (guard, receiver) = drop_guard();
    let id = TimerId::mint();
    (
        ScheduleGuard { id, guard },
        ScheduleEffect {
            at,
            event: event(id),
            cancel: AlwaysEqual(receiver),
        },
    )
}
```

`event` is built at arming time and carries the id, so what reaches the model is `ScheduleFired { id, at }` with `at` filled in by the source at the moment it fires. So `event` is `impl FnOnce(TimerId) -> E` where mercury's `E` is a closure over the id that the source completes with the time; in practice mercury passes a constructor and the source calls it:

```rust
pub type ScheduleEvent<E> = Box<dyn FnOnce(DateTime) -> E + Send>;
```

and `ScheduleEffect::event` is that boxed constructor rather than a finished event. Under `testing` it compares through `AlwaysEqual`, the way the cancel receiver does: what a test asserts about an arming is the time it was armed for.

## Change 3: the source

`crates/freddie_schedule/src/lib.rs`. One thread holding every pending deadline, because they all have to be re-derived by the same two events and waking one task per schedule to do it is the same work done many times.

```rust
/// What the thread is told.
enum Message {
    Arm(Armed),
    Cancel(TimerId),
    /// The wall clock moved under us: the machine woke, or the clock was set. Every deadline is
    /// re-derived, because the duration to each of them just changed.
    Recompute,
}
```

The loop:

```rust
/// Wait for the earliest deadline, fire it, repeat.
///
/// `recv_timeout` is the select: the timeout is the earliest deadline, a message is an arming, a
/// cancellation, or the clock moving, and a disconnect is the sink going away. Nothing here wakes
/// to ask whether there is anything to do.
fn run(messages: &Receiver<Message>, on_fire: impl Fn(TimerFiring)) {
    let mut pending: HashMap<TimerId, Armed> = HashMap::new();
    loop {
        let now = Zoned::now();
        // Everything due fires now, in time order, each carrying the time it actually fired at.
        // A deadline that passed while the machine slept lands here on the first wake.
        let mut due: Vec<_> = pending
            .values()
            .filter(|armed| armed.at <= now.datetime())
            .map(|armed| armed.id)
            .collect();
        due.sort_unstable_by_key(|id| pending[id].at);
        for id in due {
            let armed = pending.remove(&id).expect("just read");
            on_fire(TimerFiring { id: armed.id, at: now.datetime(), event: armed.event });
        }
        let wait = pending
            .values()
            .map(|armed| until(&now, armed.at))
            .min()
            .unwrap_or(Duration::MAX);
        match messages.recv_timeout(wait) {
            Ok(Message::Arm(armed)) => { pending.insert(armed.id, armed); }
            Ok(Message::Cancel(id)) => { pending.remove(&id); }
            Ok(Message::Recompute) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// How long until `at` in local time, saturating at zero for one that has passed.
fn until(now: &Zoned, at: DateTime) -> Duration {
    let zoned = match at.to_zoned(now.time_zone().clone()) {
        Ok(zoned) => zoned,
        // A local time that does not exist, which is the hour a spring-forward skips. jiff's
        // compatible resolution moves it forward out of the gap, which is when it should run.
        Err(_) => return Duration::from_secs(60),
    };
    (&zoned - now).try_into().unwrap_or(Duration::ZERO)
}
```

The two things that move the wall clock under a monotonic wait are observed with the notification machinery `freddie_windows` already has, registered on the main thread in `watch` and each sending one `Message::Recompute`:

- `NSWorkspaceDidWakeNotification`, on `NSWorkspace`'s own centre. `objc2-app-kit`, `NSWorkspace.rs:724`.
- `NSSystemClockDidChangeNotification`, on the default centre. `objc2-foundation`, `NSDate.rs:14`.

The public shape follows `freddie_windows`: a watcher to hold and a sink to arm through.

```rust
/// Start scheduling. Dropping the watcher cancels everything pending.
#[must_use = "dropping the watcher cancels every schedule"]
pub fn watch(on_fire: impl Fn(ScheduleFired) + Send + 'static) -> (Watcher, ScheduleSink);

impl ScheduleSink {
    /// Arm one firing. The guard that cancels it is the caller's, minted with the effect.
    pub fn arm(&self, at: DateTime, id: TimerId, cancel: oneshot::Receiver<()>);
}
```

`arm` is a channel send and returns immediately; a cancel arrives by the receiver closing, which the thread learns on its next pass.

## Change 4: mercury holds the schedules

A schedule outlives every layer, so it hangs off the root. `Scheduled` is one job: the schedule, and the guard for the firing it is waiting for.

`crates/mercury/src/state/schedule.rs`:

```rust
/// One job: when it fires, and the firing it is waiting for.
///
/// `waiting` is `None` only for a [`Schedule::Once`] that has fired, which is a job with nothing
/// left to wait for. Its trigger is then `None`, and `Option<T>: EventTrigger` matches nothing, so
/// the binding written against it is simply not bound.
#[derive(Debug)]
pub struct Scheduled {
    schedule: Schedule,
    waiting: Option<ScheduleGuard>,
}

impl Scheduled {
    /// Arm the first firing after `now`, returning the job and the effect that arms it.
    #[must_use]
    pub fn new(schedule: Schedule, now: DateTime) -> (Self, Vec<MercuryEffect>) {
        let mut job = Self { schedule, waiting: None };
        let effects = job.arm(now);
        (job, effects)
    }

    /// The trigger matching this job's own firing.
    #[must_use]
    pub fn trigger(&self) -> Option<ScheduleTrigger> {
        self.waiting.as_ref().map(ScheduleGuard::trigger)
    }

    /// Wait for the first firing after `now`, dropping whatever was pending.
    ///
    /// Called on construction and again on every firing, with the time that firing carried. A job
    /// that slept through three of its occurrences arms the next one after now, not the next one
    /// after the occurrence it missed.
    #[must_use]
    pub fn arm(&mut self, now: DateTime) -> Vec<MercuryEffect> {
        let Some(at) = self.schedule.next_after(now) else {
            self.waiting = None;
            return Vec::new();
        };
        let (guard, effect) = schedule_effect_and_guard(at, |id| {
            Box::new(move |at| MercuryEvent::Schedule(ScheduleFired { id, at }))
        });
        self.waiting = Some(guard);
        vec![MercuryEffect::Schedule(effect)]
    }
}
```

Assigning `self.waiting` drops the previous guard, which cancels whatever was pending. So a re-arm cannot leave two firings outstanding, and there is no cancel step to forget.

The jobs live in a struct of their own, one field per job, so a binding names the one it is for:

```rust
/// Every job mercury runs on the clock. One field per job; a new one is a field, a binding, and a
/// handler.
#[derive(Debug)]
pub struct Schedules {
    pub morning: Scheduled,
}

impl Schedules {
    #[must_use]
    pub fn new(now: DateTime) -> (Self, Vec<MercuryEffect>) {
        let (morning, effects) = Scheduled::new(
            Schedule::Recurring(Recurring {
                days: Weekdays::WEEKDAYS,
                at: jiff::civil::time(9, 0, 0, 0),
            }),
            now,
        );
        (Self { morning }, effects)
    }
}
```

`Mercury` gains the field and the bind, before:

```rust
    |mercury_path| mercury_path.overlay_timer().map(TimerGuard::trigger) => hide_overlay,
```

after:

```rust
    |mercury_path| mercury_path.overlay_timer().map(TimerGuard::trigger) => hide_overlay,
    |mercury_path| mercury_path.schedules.morning.trigger() => morning,
```

which is the same closure-trigger shape the timers use, and matches nothing while the job has nothing pending.

The handler re-arms and does the job. `crates/mercury/src/handlers/schedule.rs`:

```rust
/// The weekday-morning job: re-arm for the next one, and do the work.
pub(crate) fn morning(ev: &ScheduleFired, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    let mut effects = root.schedules.morning.arm(ev.at);
    effects.push(/* what the job does */);
    effects
}
```

Re-arming first, so a job whose work panics has still scheduled tomorrow's.

## Change 5: the effect and boot

`MercuryEffect`, before:

```rust
    /// Arm a timer. The effect loop schedules it; it fires its event after the delay unless the
    /// guard held by the state that asked for it drops first.
    Timer(TimerEffect<MercuryEvent>),
```

after:

```rust
    /// Arm a timer. The effect loop schedules it; it fires its event after the delay unless the
    /// guard held by the state that asked for it drops first.
    Timer(TimerEffect<MercuryEvent>),
    /// Arm a firing at a wall-clock time, on the same terms.
    Schedule(ScheduleEffect<MercuryEvent>),
```

performed by handing it to the sink:

```rust
        MercuryEffect::Schedule(effect) => match schedules {
            Some(sink) => sink.arm(effect.at, effect.id, effect.cancel.0),
            None => debug!(at = %effect.at, "no schedule sink: nothing to arm through"),
        },
```

`Boot` gains the sink and the time the model is built at:

```rust
struct Boot {
    front_app: App,
    windows: Windows,
    window_sink: Option<WindowSink>,
    schedule_sink: ScheduleSink,
    now: DateTime,
}
```

with the source registered before the seed is read, by `seed-at-construction.md`'s rule. Nothing here is idempotent in that doc's sense and nothing has to be: a schedule armed during construction is armed once, by construction, and no watcher can report one twice.

`Mercury::new` takes `now`, builds `Schedules` from it, and returns the arming effects alongside itself, which is new: construction has never produced effects before.

```rust
    /// The model, and the effects that arm what it is already waiting for.
    pub fn new(front_app: App, windows: Windows, now: DateTime) -> (Self, Vec<MercuryEffect>)
```

The daemon performs them before entering the loop, the same way it performs any others.

## Tests

The calendar is Change 1's table. The model's half is that a firing re-arms and does not accumulate:

```rust
#[test]
fn a_firing_arms_the_next_one() {
    let (mut m, armed) = Mercury::new(App::Finder, Windows::default(), date(2026, 7, 21).at(8, 0, 0, 0));
    assert_eq!(armed, vec![scheduled_at(date(2026, 7, 21).at(9, 0, 0, 0))]);

    let effects = m.handle(&fired(date(2026, 7, 21).at(9, 0, 0, 0))).expect("bound");
    assert_eq!(effects.first(), Some(&scheduled_at(date(2026, 7, 22).at(9, 0, 0, 0))));
}

#[test]
fn a_job_slept_through_arms_from_now_and_not_from_when_it_was_due() {
    let (mut m, _) = Mercury::new(App::Finder, Windows::default(), date(2026, 7, 21).at(8, 0, 0, 0));
    // Woke on Thursday afternoon, three occurrences later. One firing, and the next is Friday.
    let effects = m.handle(&fired(date(2026, 7, 23).at(14, 30, 0, 0))).expect("bound");
    assert_eq!(effects.first(), Some(&scheduled_at(date(2026, 7, 24).at(9, 0, 0, 0))));
}

#[test]
fn a_firing_that_is_not_this_jobs_is_not_bound() {
    let (mut m, _) = Mercury::new(App::Finder, Windows::default(), date(2026, 7, 21).at(8, 0, 0, 0));
    assert_eq!(m.handle(&MercuryEvent::Schedule(other_schedules_firing())), None);
}
```
