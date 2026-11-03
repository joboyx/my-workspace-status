//! Latest-only job slots for the TUI interpreter.
//!
//! Some jobs only matter for the newest request: a cursor move asks for a
//! new line blame, a typed key asks for a new file score. A
//! [`LatestOnly`] slot keeps the newest waiting request of one job kind and
//! queues [`UserTag`] slots on the [`Scheduler`] so that each waiting
//! request gets a worker, but never one slot per request.
//!
//! The [`super::effect::Interpreter`] owns one slot per job kind. A new
//! latest-only kind needs one field built with its tag and policy.

use super::scheduler::{Scheduler, UserTag};

/// How a [`LatestOnly`] slot shares the worker pool between its jobs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SlotPolicy {
    /// At most one job of the kind is queued or running. A request that
    /// arrives while a job runs waits until [`LatestOnly::finish`].
    Serial,
    /// At most one scheduler slot of the kind is queued. A running job does
    /// not hold it, so a newer request starts next to a running one.
    Overlap,
}

/// The newest waiting request of one latest-only job kind, plus the
/// scheduler slot it holds.
///
/// A newer request replaces a waiting one before a worker takes it. The
/// interpreter offers requests with [`Self::offer`], takes one at spawn
/// with [`Self::take_if`] or [`Self::take`], and, for a
/// [`SlotPolicy::Serial`] kind, calls [`Self::finish`] when the job's
/// answer lands.
#[derive(Debug)]
pub(crate) struct LatestOnly<T> {
    /// Scheduler tag of this kind's slots.
    tag: UserTag,
    /// How this kind shares the pool.
    policy: SlotPolicy,
    /// The newest request that no worker took yet.
    waiting: Option<T>,
    /// Serial: a job is queued or running. Overlap: a slot is queued.
    held: bool,
}

impl<T> LatestOnly<T> {
    /// Empty slot with the [`SlotPolicy::Serial`] policy.
    pub(crate) fn serial(tag: UserTag) -> Self {
        Self::new(tag, SlotPolicy::Serial)
    }

    /// Empty slot with the [`SlotPolicy::Overlap`] policy.
    pub(crate) fn overlap(tag: UserTag) -> Self {
        Self::new(tag, SlotPolicy::Overlap)
    }

    /// Empty slot for `tag` with `policy`.
    fn new(tag: UserTag, policy: SlotPolicy) -> Self {
        Self {
            tag,
            policy,
            waiting: None,
            held: false,
        }
    }

    /// Keep `job` as the newest request; it replaces a waiting one.
    ///
    /// Queues one scheduler slot when the kind holds none: for
    /// [`SlotPolicy::Serial`] no job is queued or running, for
    /// [`SlotPolicy::Overlap`] no slot is queued.
    pub(crate) fn offer(&mut self, job: T, sched: &mut Scheduler) {
        self.waiting = Some(job);
        if !self.held {
            self.held = true;
            sched.enqueue_user(self.tag);
        }
    }

    /// Drop the waiting request, if any. A queued slot stays and spawns
    /// with nothing to take.
    pub(crate) fn clear(&mut self) {
        self.waiting = None;
    }

    /// Take the waiting request for a worker.
    ///
    /// Call this when a slot of this kind spawns. See [`Self::take_if`].
    pub(crate) fn take(&mut self) -> Option<T> {
        self.take_if(|_| true)
    }

    /// Take the waiting request for a worker when `keep` accepts it; a
    /// rejected request is dropped.
    ///
    /// Call this when a slot of this kind spawns. The slot is no longer
    /// queued. For [`SlotPolicy::Serial`], a job that runs keeps the kind
    /// busy until [`Self::finish`]; no job (`None`) ends the busy state at
    /// once. The caller still ends a spawn that runs nothing on the
    /// scheduler.
    pub(crate) fn take_if(&mut self, keep: impl FnOnce(&T) -> bool) -> Option<T> {
        let job = self.waiting.take().filter(keep);
        if self.policy == SlotPolicy::Overlap || job.is_none() {
            self.held = false;
        }
        job
    }

    /// End the running job of a [`SlotPolicy::Serial`] kind, and queue the
    /// next slot when a newer request waits.
    pub(crate) fn finish(&mut self, sched: &mut Scheduler) {
        self.held = false;
        if self.waiting.is_some() {
            self.held = true;
            sched.enqueue_user(self.tag);
        }
    }

    /// The waiting request, if any.
    #[cfg(test)]
    pub(crate) fn waiting(&self) -> Option<&T> {
        self.waiting.as_ref()
    }

    /// True while the kind holds a scheduler slot: [`SlotPolicy::Serial`],
    /// a job is queued or running; [`SlotPolicy::Overlap`], a slot is
    /// queued.
    #[cfg(test)]
    pub(crate) fn holds_slot(&self) -> bool {
        self.held
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAG: UserTag = UserTag::LineBlame;

    /// Pop every ready slot, as the interpreter's spawn does, and return
    /// how many popped.
    fn spawn_slots(sched: &mut Scheduler) -> usize {
        sched.spawn_ready().len()
    }

    #[test]
    fn the_newest_offer_replaces_the_waiting_one_for_both_policies() {
        for mut slot in [LatestOnly::serial(TAG), LatestOnly::overlap(TAG)] {
            let mut sched = Scheduler::new(4);
            slot.offer(1, &mut sched);
            slot.offer(2, &mut sched);
            assert_eq!(slot.waiting(), Some(&2));
            assert_eq!(sched.queued_user_tag(TAG), 1, "one slot for both");
            assert_eq!(slot.take(), Some(2));
            assert_eq!(slot.take(), None, "taken once");
        }
    }

    #[test]
    fn serial_queues_one_slot_while_a_job_is_queued_or_running() {
        let mut sched = Scheduler::new(4);
        let mut slot = LatestOnly::serial(TAG);
        slot.offer(1, &mut sched);
        assert_eq!(spawn_slots(&mut sched), 1);
        assert_eq!(slot.take(), Some(1));
        assert!(slot.holds_slot(), "running");

        // Offers while the job runs queue nothing.
        slot.offer(2, &mut sched);
        slot.offer(3, &mut sched);
        assert_eq!(sched.queued_user_tag(TAG), 0);

        // The answer lands: the newest request gets the next slot.
        slot.finish(&mut sched);
        assert_eq!(sched.queued_user_tag(TAG), 1);
        assert!(slot.holds_slot());
        assert_eq!(spawn_slots(&mut sched), 1);
        assert_eq!(slot.take(), Some(3));

        // Nothing waits when it lands: no slot, and the next offer queues.
        slot.finish(&mut sched);
        assert_eq!(sched.queued_user_tag(TAG), 0);
        assert!(!slot.holds_slot());
        slot.offer(4, &mut sched);
        assert_eq!(sched.queued_user_tag(TAG), 1);
    }

    #[test]
    fn serial_ends_the_busy_state_when_the_spawn_takes_nothing() {
        let mut sched = Scheduler::new(4);
        let mut slot = LatestOnly::serial(TAG);

        // Cleared before the spawn: nothing to take.
        slot.offer(1, &mut sched);
        slot.clear();
        assert_eq!(spawn_slots(&mut sched), 1);
        assert_eq!(slot.take(), None);
        assert!(!slot.holds_slot());

        // Rejected at the spawn: dropped, and the next offer queues.
        slot.offer(2, &mut sched);
        assert_eq!(spawn_slots(&mut sched), 1);
        assert_eq!(slot.take_if(|job| *job > 2), None);
        assert!(!slot.holds_slot());
        assert_eq!(slot.waiting(), None, "dropped");
        slot.offer(3, &mut sched);
        assert_eq!(sched.queued_user_tag(TAG), 1);
    }

    #[test]
    fn overlap_queues_one_slot_and_a_running_job_does_not_hold_it() {
        let mut sched = Scheduler::new(4);
        let mut slot = LatestOnly::overlap(TAG);
        slot.offer(1, &mut sched);
        slot.offer(2, &mut sched);
        assert_eq!(sched.queued_user_tag(TAG), 1);
        assert_eq!(spawn_slots(&mut sched), 1);
        assert_eq!(slot.take(), Some(2));
        assert!(!slot.holds_slot(), "a running job holds no slot");

        // A new request while that job runs gets its own slot.
        slot.offer(3, &mut sched);
        assert_eq!(sched.queued_user_tag(TAG), 1);
        assert!(slot.holds_slot());

        // A rejected request frees the slot too.
        assert_eq!(spawn_slots(&mut sched), 1);
        assert_eq!(slot.take_if(|job| *job > 3), None);
        assert!(!slot.holds_slot());
        slot.offer(4, &mut sched);
        assert_eq!(sched.queued_user_tag(TAG), 1);
    }
}
