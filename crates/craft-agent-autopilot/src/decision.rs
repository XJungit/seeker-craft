//! 归档 + 决策

#[derive(Debug)]
pub enum NextAction {
    Continue,
    AdvancePhase,
    FocusOnIssues,
    GlobalReview,
    CelebrateAndStop,
}

pub struct DecisionEngine;

impl DecisionEngine {
    pub fn decide(
        round_result: &super::orchestrator::RoundResult,
        history: &[super::orchestrator::RoundResult],
    ) -> NextAction {
        if round_result.ender_dragon_defeated {
            return NextAction::CelebrateAndStop;
        }

        let no_progress_streak = history.iter().rev().take_while(|r| !r.has_progress).count();
        if no_progress_streak >= 5 {
            return NextAction::GlobalReview;
        }

        if round_result.current_phase_complete {
            return NextAction::AdvancePhase;
        }

        if round_result.open_issues > 10 {
            return NextAction::FocusOnIssues;
        }

        NextAction::Continue
    }
}
