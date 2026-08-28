use std::collections::HashMap;
use war_protocol::*;
use war_semantic::{diff, render_delta, render_snapshot, SemanticCompiler};

fn node(
    id: NodeId,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    role: Role,
    name: &str,
    value: Option<&str>,
) -> DesktopNode {
    DesktopNode {
        id,
        source: NodeSource::Uia,
        role,
        name: Some(name.into()),
        automation_id: Some(format!("id-{id}")),
        value: value.map(str::to_owned),
        description: None,
        bounds: None,
        states: NodeStates {
            enabled: true,
            focused: role == Role::TextInput,
            focusable: true,
            ..Default::default()
        },
        capabilities: if role == Role::Button {
            Capabilities::INVOKE
        } else {
            Capabilities::SET_VALUE
        },
        parent,
        children,
        fingerprint: NodeFingerprint {
            process_id: 42,
            ..Default::default()
        },
    }
}

fn raw(epoch: u64, value: &str, include_dialog: bool) -> RawSnapshot {
    let mut nodes = HashMap::from([
        (
            10,
            node(
                10,
                None,
                if include_dialog {
                    vec![11, 12, 13]
                } else {
                    vec![11, 12]
                },
                Role::Window,
                "Editor",
                None,
            ),
        ),
        (
            11,
            node(
                11,
                Some(10),
                vec![],
                Role::TextInput,
                "Document",
                Some(value),
            ),
        ),
        (12, node(12, Some(10), vec![], Role::Button, "Save", None)),
    ]);
    if include_dialog {
        nodes.insert(13, node(13, Some(10), vec![], Role::Dialog, "Saved", None));
    }
    RawSnapshot {
        epoch,
        root: 10,
        nodes,
    }
}

#[test]
fn compiler_keeps_refs_stable_and_produces_small_delta() {
    let mut compiler = SemanticCompiler::new();
    let window = WindowInfo {
        id: 1,
        process_id: 42,
        app: Some("fixture.exe".into()),
        title: Some("Editor".into()),
    };
    let first = compiler
        .compile(raw(1, "before", false), window.clone())
        .semantic;
    let second = compiler.compile(raw(2, "after", true), window).semantic;
    let old_doc = first
        .nodes
        .iter()
        .find(|n| n.name.as_deref() == Some("Document"))
        .unwrap();
    let new_doc = second
        .nodes
        .iter()
        .find(|n| n.name.as_deref() == Some("Document"))
        .unwrap();
    assert_eq!(old_doc.id, new_doc.id);
    let delta = diff(&first, &second);
    assert_eq!(delta.added.len(), 1);
    assert_eq!(delta.changed.len(), 1);
    assert!(render_delta(&delta).contains("value=Some(\"after\")"));
    assert!(render_snapshot(&second).contains("button \"Save\" [invoke]"));
}

#[test]
fn reset_history_invalidates_old_refs_without_recycling_ids() {
    let mut compiler = SemanticCompiler::new();
    let first = compiler
        .compile(raw(1, "before", false), WindowInfo::default())
        .semantic;
    let old_ids: std::collections::HashSet<_> = first.nodes.iter().map(|node| node.id).collect();
    compiler.reset_history();
    let second = compiler
        .compile(raw(2, "after", false), WindowInfo::default())
        .semantic;
    assert!(second.nodes.iter().all(|node| !old_ids.contains(&node.id)));
}

#[test]
fn node_budget_keeps_ancestor_chain_and_reports_truncation() {
    let mut tree = raw(1, "value", false);
    tree.nodes.get_mut(&10).unwrap().children = vec![20, 30];
    tree.nodes.insert(
        20,
        DesktopNode {
            id: 20,
            source: NodeSource::Uia,
            role: Role::Group,
            name: None,
            automation_id: None,
            value: None,
            description: None,
            bounds: None,
            states: NodeStates::default(),
            capabilities: Capabilities::empty(),
            parent: Some(10),
            children: vec![21],
            fingerprint: NodeFingerprint {
                process_id: 42,
                ..Default::default()
            },
        },
    );
    tree.nodes.insert(
        21,
        node(21, Some(20), vec![], Role::Button, "Primary", None),
    );
    tree.nodes.insert(
        30,
        node(30, Some(10), vec![], Role::Button, "Secondary", None),
    );
    tree.nodes.remove(&11);
    tree.nodes.remove(&12);

    let snapshot = SemanticCompiler::with_max_nodes(3)
        .compile(tree, WindowInfo::default())
        .semantic;
    assert!(snapshot.truncated);
    assert_eq!(snapshot.total_nodes, 4);
    assert_eq!(snapshot.nodes.len(), 3);
    assert_eq!(
        snapshot
            .nodes
            .iter()
            .map(|node| node.depth)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert!(render_snapshot(&snapshot).contains("nodes=3/4 truncated"));
}

#[test]
fn hidden_unselected_content_is_not_exposed() {
    let mut tree = raw(1, "visible", false);
    let mut hidden = node(
        30,
        Some(10),
        vec![],
        Role::TabItem,
        "private-file.txt",
        None,
    );
    hidden.states.offscreen = true;
    hidden.states.selected = Some(false);
    tree.nodes.get_mut(&10).unwrap().children.push(30);
    tree.nodes.insert(30, hidden);

    let snapshot = SemanticCompiler::new()
        .compile(tree, WindowInfo::default())
        .semantic;
    assert!(!snapshot
        .nodes
        .iter()
        .any(|node| node.name.as_deref() == Some("private-file.txt")));
}
