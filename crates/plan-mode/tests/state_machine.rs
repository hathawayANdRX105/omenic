//! Plan-mode command parsing and state-machine transitions.

use plan_mode::{MAX_PLAN_COMMAND_MESSAGE_BYTES, PlanModeCommand, PlanModeError, PlanModeRuntime};

#[test]
fn parse_plan_variants() {
    assert_eq!(
        PlanModeCommand::parse("/plan"),
        Some(Ok(PlanModeCommand::Enter { message: None }))
    );
    assert_eq!(
        PlanModeCommand::parse(" /plan inspect first "),
        Some(Ok(PlanModeCommand::Enter {
            message: Some("inspect first".to_owned())
        }))
    );
    assert_eq!(
        PlanModeCommand::parse("/plan off"),
        Some(Ok(PlanModeCommand::Off))
    );
}

#[test]
fn parse_rejects_non_plan_prefix() {
    assert_eq!(PlanModeCommand::parse("/planner"), None);
    assert_eq!(PlanModeCommand::parse("/planx something"), None);
    assert_eq!(PlanModeCommand::parse("hello"), None);
}

#[test]
fn parse_rejects_oversized_message() {
    let large = "x".repeat(MAX_PLAN_COMMAND_MESSAGE_BYTES + 1);
    let input = format!("/plan {}", large);
    assert_eq!(
        PlanModeCommand::parse(&input),
        Some(Err(PlanModeError::MessageTooLarge))
    );
}

#[test]
fn parse_handles_surrounding_whitespace() {
    assert_eq!(
        PlanModeCommand::parse("   /plan   "),
        Some(Ok(PlanModeCommand::Enter { message: None }))
    );
    assert_eq!(
        PlanModeCommand::parse("  /plan  hello  "),
        Some(Ok(PlanModeCommand::Enter {
            message: Some("hello".to_owned())
        }))
    );
}

#[test]
fn prepare_set_is_idempotent_and_commit_applies() {
    let rt = PlanModeRuntime::new(false);
    assert!(!rt.active().unwrap());

    let mutation = rt.prepare_set(true).unwrap().unwrap();
    assert!(mutation.change().active());
    assert!(
        !rt.active().unwrap(),
        "prepared mutation must not apply before commit"
    );
    mutation.commit().unwrap();
    assert!(rt.active().unwrap());

    assert!(
        rt.prepare_set(true).unwrap().is_none(),
        "same-state set is a no-op"
    );

    let exit = rt.prepare_set(false).unwrap().unwrap();
    assert!(!exit.change().active());
    exit.commit().unwrap();
    assert!(!rt.active().unwrap());
}

#[test]
fn pending_change_makes_further_prepare_set_stale() {
    let rt = PlanModeRuntime::new(true);
    // An approved exit parks pending=false while active stays true (the
    // boundary hook lands it later).
    rt.prepare_approved_exit().unwrap().commit().unwrap();
    assert!(
        rt.active().unwrap(),
        "parked exit leaves active until the boundary"
    );
    // Further prepares are stale while a change is parked.
    assert!(
        matches!(rt.prepare_set(false), Err(PlanModeError::Stale)),
        "a pending change makes further prepares stale"
    );
    // The boundary lands the parked exit.
    let boundary = rt.prepare_boundary().unwrap().unwrap();
    assert!(!boundary.change().active());
    boundary.commit().unwrap();
    assert!(!rt.active().unwrap());
    assert!(rt.prepare_boundary().unwrap().is_none());
}

#[test]
fn boundary_commits_pending_and_clears_it() {
    let rt = PlanModeRuntime::new(false);
    rt.prepare_set(true).unwrap().unwrap().commit().unwrap();
    assert!(rt.active().unwrap());

    let boundary = rt.prepare_boundary().unwrap();
    assert!(boundary.is_none(), "no pending change at the boundary");

    // Simulate a pending selection parked by an approved exit.
    let exit = rt.prepare_approved_exit().unwrap();
    exit.commit().unwrap();
    let boundary = rt.prepare_boundary().unwrap().unwrap();
    assert!(!boundary.change().active());
    boundary.commit().unwrap();
    assert!(!rt.active().unwrap());
    assert!(rt.prepare_boundary().unwrap().is_none());
}

#[test]
fn approved_exit_requires_active_state() {
    let rt = PlanModeRuntime::new(false);
    assert!(
        matches!(rt.prepare_approved_exit(), Err(PlanModeError::Inactive)),
        "approved exit requires active state"
    );

    rt.prepare_set(true).unwrap().unwrap().commit().unwrap();
    assert!(rt.prepare_approved_exit().is_ok());
}

#[test]
fn plan_policy_section_tracks_active_state() {
    let rt = PlanModeRuntime::new(false);
    assert_eq!(rt.plan_policy_section("section text"), "");

    rt.prepare_set(true).unwrap().unwrap().commit().unwrap();
    assert_eq!(rt.plan_policy_section("section text"), "section text");
}
