use war_protocol::{Action, ActionBatch, MouseButton, NormalizedPoint, Target};

#[test]
fn compact_action_json_round_trips() {
    let batch = ActionBatch {
        expected_session_id: Some("session-7".into()),
        expected_epoch: Some(7),
        timeout_ms: Some(5_000),
        actions: vec![
            Action::Invoke {
                target: Target::Ref(12),
            },
            Action::SetValue {
                target: Target::Ref(18),
                value: "hello".into(),
            },
            Action::PointerGesture {
                target: Target::Ref(21),
                button: MouseButton::Left,
                points: vec![
                    NormalizedPoint { x: 0.1, y: 0.2 },
                    NormalizedPoint { x: 0.9, y: 0.8 },
                ],
                duration_ms: 250,
            },
        ],
        precondition: None,
        postcondition: None,
        stop_on_error: true,
    };
    let json = serde_json::to_string(&batch).unwrap();
    assert!(json.contains(r#"{"invoke":"@12"}"#));
    assert!(json.contains(r#""target":"@18""#));
    assert!(json.contains(r#""pointer_gesture""#));
    assert_eq!(serde_json::from_str::<ActionBatch>(&json).unwrap(), batch);
}

#[test]
fn accepts_documented_batch_shape() {
    let json = r#"{"actions":[{"invoke":"@12"},{"set_value":{"target":"@18","value":"hello"}}],"postcondition":{"type":"gone","target":"@17"},"stop_on_error":true}"#;
    let batch: ActionBatch = serde_json::from_str(json).unwrap();
    assert_eq!(batch.actions.len(), 2);
    assert!(matches!(
        batch.actions[0],
        Action::Invoke {
            target: Target::Ref(12)
        }
    ));
}
