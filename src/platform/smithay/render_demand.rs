//! Visual generations are independent of policy reconciliation and callback work.

use smithay::reexports::calloop::ping::Ping;

#[derive(Clone, Copy)]
pub(super) enum Reason {
    Initial,
    Commit,
    Policy,
    Input,
    Metadata,
    Lifecycle,
    Exposure,
    Resize,
    Capture,
    Diagnostic,
    UncertainCommit,
}

pub(super) struct RenderDemand {
    generation: u64,
    submitted: u64,
    reasons: u16,
    ping: Option<Ping>,
}

impl Default for RenderDemand {
    fn default() -> Self {
        Self {
            generation: 1,
            submitted: 0,
            reasons: 1 << Reason::Initial as u16,
            ping: None,
        }
    }
}

impl RenderDemand {
    pub fn set_ping(&mut self, ping: Ping) {
        self.ping = Some(ping);
    }

    /// Invalidation is retained until its exact scene generation is submitted.
    pub fn invalidate(&mut self, reason: Reason) {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("render generation exhausted");
        self.reasons |= 1 << reason as u16;
        self.wake();
    }

    /// Wake maintenance/callback scheduling without demanding identical pixels.
    pub fn wake(&self) {
        if let Some(ping) = &self.ping {
            ping.ping();
        }
    }

    pub fn visual_pending(&self) -> bool {
        self.generation != self.submitted
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn has_reason(&self, reason: Reason) -> bool {
        self.reasons & (1 << reason as u16) != 0
    }
    pub fn is_nondiagnostic(&self) -> bool {
        self.reasons & !(1 << Reason::Diagnostic as u16) != 0
    }

    pub fn is_scene_work(&self) -> bool {
        let mask = [
            Reason::Commit,
            Reason::Policy,
            Reason::Input,
            Reason::Metadata,
            Reason::Lifecycle,
            Reason::UncertainCommit,
        ]
        .into_iter()
        .fold(0, |mask, reason| mask | (1 << reason as u16));
        self.reasons & mask != 0
    }

    pub fn requires_full_frame(&self) -> bool {
        let mask = [
            Reason::Initial,
            Reason::Exposure,
            Reason::Resize,
            Reason::Capture,
            Reason::Policy,
            Reason::UncertainCommit,
        ]
        .into_iter()
        .fold(0, |mask, reason| mask | (1 << reason as u16));
        self.reasons & mask != 0
    }

    pub fn submitted(&mut self, generation: u64) {
        self.submitted = generation;
        if generation == self.generation {
            self.reasons = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_invalidation_survives_submission_of_earlier_scene() {
        let mut demand = RenderDemand::default();
        let rendered = demand.generation();
        demand.invalidate(Reason::Policy);
        demand.submitted(rendered);
        assert!(demand.visual_pending());
        demand.submitted(demand.generation());
        assert!(!demand.visual_pending());
    }

    #[test]
    fn diagnostic_work_does_not_restart_visual_activity() {
        let mut demand = RenderDemand::default();
        demand.submitted(demand.generation());
        demand.invalidate(Reason::Diagnostic);
        assert!(demand.visual_pending());
        assert!(!demand.is_nondiagnostic());
        demand.invalidate(Reason::Commit);
        assert!(demand.is_nondiagnostic());
    }

    #[test]
    fn policy_and_recovery_force_full_frames_but_client_damage_can_be_incremental() {
        let mut demand = RenderDemand::default();
        assert!(demand.requires_full_frame());
        demand.submitted(demand.generation());
        demand.invalidate(Reason::Commit);
        assert!(demand.is_scene_work());
        assert!(!demand.requires_full_frame());
        demand.invalidate(Reason::Policy);
        assert!(demand.requires_full_frame());
        demand.submitted(demand.generation());
        demand.invalidate(Reason::UncertainCommit);
        assert!(demand.requires_full_frame());
        assert!(demand.is_scene_work());
    }
}
