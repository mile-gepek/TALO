use chrono::Duration;

pub enum AfterResumeBehavior {
    /// Reschedule the break after resuming the system.
    Reschedule,
    /// Keep the break's remaining time, clamped to the lower bound [`min_wait_after_resume`].
    ///
    /// The system does not remove scheduled breaks on paused.
    /// Instead, it saves the timestamp of when it was paused,
    /// and uses it to calculate the time remaining until the break's deadline.
    /// That remaining time is clamped to [`min_wait_after_resume`],
    /// and becomes the delay until the break runs.
    ///
    /// [`min_wait_after_resume`]: AfterResumeConfig::min_wait_after_resume
    KeepTimeRemaining,
}

pub struct AfterResumeConfig {
    pub behavior: AfterResumeBehavior,
    /// Lower bound for the delay before firing a pending break
    /// after resuming when [`AfterResumeBehavior::OffsetBreak`] is used.
    pub min_wait_after_resume: Duration,
}

pub struct BreakConfig {
    pub break_interval: Duration,
    pub long_break_interval: usize,
    pub after_resume_config: AfterResumeConfig,
}

impl BreakConfig {
    pub fn new() -> Self {
        BreakConfig {
            break_interval: Duration::seconds(5),
            long_break_interval: 3,
            after_resume_config: AfterResumeConfig {
                behavior: AfterResumeBehavior::KeepTimeRemaining,
                min_wait_after_resume: Duration::seconds(5),
            },
        }
    }
}
