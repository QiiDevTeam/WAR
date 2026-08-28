use war_core::validate_batch;
use war_protocol::{Action, ActionBatch, MouseButton, NormalizedPoint, Target, WarError};

fn batch(points: Vec<NormalizedPoint>) -> ActionBatch {
    ActionBatch {
        expected_session_id: Some("session".into()),
        expected_epoch: Some(1),
        timeout_ms: Some(1_000),
        actions: vec![Action::PointerGesture {
            target: Target::Ref(7),
            button: MouseButton::Left,
            points,
            duration_ms: 100,
        }],
        precondition: None,
        postcondition: None,
        stop_on_error: true,
    }
}

#[test]
fn validates_normalized_pointer_gesture_before_global_input() {
    validate_batch(&batch(vec![
        NormalizedPoint { x: 0.0, y: 0.0 },
        NormalizedPoint { x: 1.0, y: 1.0 },
    ]))
    .unwrap();

    let error = validate_batch(&batch(vec![
        NormalizedPoint { x: 0.0, y: 0.0 },
        NormalizedPoint { x: 1.1, y: 0.5 },
    ]))
    .unwrap_err();
    assert!(matches!(error, WarError::InvalidRequest(_)));
}
