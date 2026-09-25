//! Question broker: submit/answer lifecycle, validation, and the plan-review port.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use daemon::{
    AnswerError, QuestionAnswer, QuestionBroker, QuestionBrokerConfig, QuestionItem, QuestionOption,
};
use parking_lot::Mutex;
use plan_mode::{ReviewError, ReviewOutcome};

const MAX_CUSTOM: usize = 4 * 1024;

fn prompt_question() -> QuestionItem {
    QuestionItem::prompt(
        "Pick one",
        None,
        vec![QuestionOption::new("Yes"), QuestionOption::new("No")],
    )
    .unwrap()
}

#[test]
fn submit_registers_pending_and_fires_hook() {
    let broker = QuestionBroker::new();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let hook_seen = Arc::clone(&seen);
    broker.set_on_submit(move |item| hook_seen.lock().push(item.id.clone()));

    let question = prompt_question();
    let ticket = broker.submit(question.clone());

    assert_eq!(broker.pending().len(), 1);
    assert_eq!(broker.pending()[0].id, question.id);
    assert_eq!(seen.lock().as_slice(), [question.id.as_str()]);
    assert_eq!(ticket.id(), question.id);
}

#[test]
fn answer_resolves_the_ticket_and_clears_pending() {
    let broker = QuestionBroker::new();
    let question = prompt_question();
    let ticket = broker.submit(question.clone());

    broker
        .answer(&question.id, QuestionAnswer::Select { index: 1 })
        .unwrap();

    assert_eq!(
        ticket.wait_timeout(Duration::from_secs(1)).unwrap(),
        QuestionAnswer::Select { index: 1 }
    );
    assert!(broker.pending().is_empty());
}

#[test]
fn second_answer_is_unknown_not_silent() {
    let broker = QuestionBroker::new();
    let question = prompt_question();
    let _ticket = broker.submit(question.clone());
    broker
        .answer(&question.id, QuestionAnswer::Select { index: 0 })
        .unwrap();

    let err = broker.answer(&question.id, QuestionAnswer::Select { index: 1 });
    assert!(matches!(err, Err(AnswerError::UnknownQuestion(_))));
}

#[test]
fn answer_validation_rejects_bad_payloads() {
    let broker = QuestionBroker::new();
    let question = prompt_question();
    let _ticket = broker.submit(question.clone());

    assert!(matches!(
        broker.answer(&question.id, QuestionAnswer::Select { index: 9 }),
        Err(AnswerError::IndexOutOfBounds)
    ));
    assert!(matches!(
        broker.answer(&question.id, QuestionAnswer::Custom { text: "  ".into() }),
        Err(AnswerError::EmptyCustomAnswer)
    ));
    assert!(matches!(
        broker.answer(
            &question.id,
            QuestionAnswer::Custom {
                text: "x".repeat(MAX_CUSTOM + 1)
            }
        ),
        Err(AnswerError::CustomAnswerTooLong)
    ));
    // Failed answers leave the question pending.
    assert_eq!(broker.pending().len(), 1);
}

#[test]
fn plan_review_options_are_approve_reject_dismiss() {
    let question = QuestionItem::plan_review("# The plan", None);
    let labels: Vec<&str> = question.options.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(labels, ["Approve", "Reject", "Dismiss"]);
    assert_eq!(question.intent, daemon::QuestionIntent::PlanReview);
}

#[test]
fn review_port_approves_from_another_thread() {
    let broker = Arc::new(QuestionBroker::with_config(QuestionBrokerConfig {
        plan_review_timeout: Duration::from_secs(5),
    }));
    let port = broker.review_port();

    let answerer = Arc::clone(&broker);
    let handle = thread::spawn(move || {
        // Wait for the question to appear, then approve it.
        for _ in 0..100 {
            if let Some(q) = answerer.pending().first() {
                answerer
                    .answer(&q.id, QuestionAnswer::Select { index: 0 })
                    .unwrap();
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("question never appeared");
    });

    assert_eq!(port.review("# plan").unwrap(), ReviewOutcome::Approved);
    handle.join().unwrap();
    assert!(broker.pending().is_empty());
}

#[test]
fn review_port_reject_carries_feedback() {
    let broker = Arc::new(QuestionBroker::with_config(QuestionBrokerConfig {
        plan_review_timeout: Duration::from_secs(5),
    }));
    let port = broker.review_port();

    let answerer = Arc::clone(&broker);
    let handle = thread::spawn(move || {
        for _ in 0..100 {
            if let Some(q) = answerer.pending().first() {
                answerer
                    .answer(
                        &q.id,
                        QuestionAnswer::Custom {
                            text: "add a rollback step".into(),
                        },
                    )
                    .unwrap();
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("question never appeared");
    });

    match port.review("# plan").unwrap() {
        ReviewOutcome::Rejected { feedback } => assert_eq!(feedback, "add a rollback step"),
        other => panic!("expected Rejected, got {other:?}"),
    }
    handle.join().unwrap();
}

#[test]
fn review_port_timeout_cancels_and_clears_pending() {
    let broker = Arc::new(QuestionBroker::with_config(QuestionBrokerConfig {
        plan_review_timeout: Duration::from_millis(50),
    }));
    let port = broker.review_port();

    let err = port.review("# plan").unwrap_err();
    assert_eq!(err, ReviewError::Cancelled);
    // The timed-out question is gone: an answer for it is now unknown.
    assert!(broker.pending().is_empty());
}
