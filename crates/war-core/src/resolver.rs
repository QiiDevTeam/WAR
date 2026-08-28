use war_protocol::{
    Capabilities, NodeId, SemanticNode, SemanticSnapshot, SemanticTarget, Target, WarError,
    WarResult,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedNode {
    pub id: NodeId,
    pub confidence: f32,
}

pub fn resolve_target(snapshot: &SemanticSnapshot, target: &Target) -> WarResult<ResolvedNode> {
    match target {
        Target::Ref(id) => snapshot
            .nodes
            .iter()
            .any(|n| n.id == *id)
            .then_some(ResolvedNode {
                id: *id,
                confidence: 1.0,
            })
            .ok_or_else(|| WarError::TargetNotFound(format!("@{id}"))),
        Target::Semantic(query) => resolve_semantic(snapshot, query),
        Target::Coordinates(point) => snapshot
            .nodes
            .iter()
            .filter(|n| n.states.enabled)
            .find(|_| point.x.is_finite() && point.y.is_finite())
            .map(|n| ResolvedNode {
                id: n.id,
                confidence: 1.0,
            })
            .ok_or_else(|| {
                WarError::TargetNotFound(format!("coordinates {},{}", point.x, point.y))
            }),
    }
}

fn resolve_semantic(
    snapshot: &SemanticSnapshot,
    query: &SemanticTarget,
) -> WarResult<ResolvedNode> {
    let mut candidates: Vec<(&SemanticNode, f32)> = snapshot
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| {
            node.capabilities.contains(query.required_capabilities)
                && (!query.required_capabilities.is_empty() && node.states.enabled
                    || query.required_capabilities.is_empty())
        })
        .map(|(index, node)| {
            let mut score = 0.0;
            let mut possible = 0.0;
            if let Some(role) = query.role {
                possible += 0.30;
                if node.role == role {
                    score += 0.30;
                }
            }
            if let Some(name) = &query.name {
                possible += 0.45;
                let actual = node.name.as_deref().unwrap_or_default();
                if actual.eq_ignore_ascii_case(name) {
                    score += 0.45;
                } else if actual.to_lowercase().contains(&name.to_lowercase()) {
                    score += 0.30;
                }
            }
            if let Some(id) = &query.automation_id {
                possible += 0.25;
                if node.automation_id.as_deref() == Some(id) {
                    score += 0.25;
                }
            }
            if let Some(ancestor) = &query.ancestor {
                possible += 0.35;
                if matching_ancestor(snapshot, index, ancestor) {
                    score += 0.35;
                }
            }
            let confidence = if possible == 0.0 {
                0.0
            } else {
                score / possible
            };
            (node, confidence)
        })
        .filter(|(_, score)| *score > 0.0)
        .collect();
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
    let Some((node, confidence)) = candidates.first() else {
        return Err(WarError::TargetNotFound(format!(
            "semantic target {query:?}"
        )));
    };
    if *confidence < 0.75 {
        return Err(WarError::LowConfidence(*confidence));
    }
    if candidates
        .get(1)
        .is_some_and(|second| (second.1 - confidence).abs() < 0.05)
    {
        return Err(WarError::LowConfidence(*confidence * 0.7));
    }
    Ok(ResolvedNode {
        id: node.id,
        confidence: *confidence,
    })
}

/// Returns ancestors from the snapshot root through the node itself.
pub fn ancestor_chain(snapshot: &SemanticSnapshot, id: NodeId) -> WarResult<Vec<&SemanticNode>> {
    let node_index = snapshot
        .nodes
        .iter()
        .position(|node| node.id == id)
        .ok_or_else(|| WarError::TargetNotFound(format!("@{id}")))?;
    let mut chain = vec![&snapshot.nodes[node_index]];
    let mut child_depth = snapshot.nodes[node_index].depth;
    for index in (0..node_index).rev() {
        let candidate = &snapshot.nodes[index];
        if candidate.depth >= child_depth {
            continue;
        }
        chain.push(candidate);
        child_depth = candidate.depth;
        if child_depth == 0 {
            break;
        }
    }
    chain.reverse();
    Ok(chain)
}

/// Finds the node itself or its nearest ancestor supporting every requested capability.
pub fn nearest_capable_ancestor(
    snapshot: &SemanticSnapshot,
    id: NodeId,
    required: Capabilities,
) -> WarResult<&SemanticNode> {
    ancestor_chain(snapshot, id)?
        .into_iter()
        .rev()
        .find(|node| node.states.enabled && node.capabilities.contains(required))
        .ok_or_else(|| {
            WarError::CapabilityUnavailable(format!(
                "@{id} and its ancestors do not support {required:?}"
            ))
        })
}

fn matching_ancestor(
    snapshot: &SemanticSnapshot,
    node_index: usize,
    query: &SemanticTarget,
) -> bool {
    let mut child_depth = snapshot.nodes[node_index].depth;
    for index in (0..node_index).rev() {
        let candidate = &snapshot.nodes[index];
        if candidate.depth >= child_depth {
            continue;
        }
        child_depth = candidate.depth;
        if semantic_confidence(snapshot, index, query) >= 0.75 {
            return true;
        }
        if child_depth == 0 {
            break;
        }
    }
    false
}

fn semantic_confidence(snapshot: &SemanticSnapshot, index: usize, query: &SemanticTarget) -> f32 {
    let node = &snapshot.nodes[index];
    if !node.capabilities.contains(query.required_capabilities) {
        return 0.0;
    }
    let mut score = 0.0;
    let mut possible = 0.0;
    if let Some(role) = query.role {
        possible += 0.30;
        if node.role == role {
            score += 0.30;
        }
    }
    if let Some(name) = &query.name {
        possible += 0.45;
        let actual = node.name.as_deref().unwrap_or_default();
        if actual.eq_ignore_ascii_case(name) {
            score += 0.45;
        } else if actual.to_lowercase().contains(&name.to_lowercase()) {
            score += 0.30;
        }
    }
    if let Some(id) = &query.automation_id {
        possible += 0.25;
        if node.automation_id.as_deref() == Some(id) {
            score += 0.25;
        }
    }
    if let Some(ancestor) = &query.ancestor {
        possible += 0.35;
        if matching_ancestor(snapshot, index, ancestor) {
            score += 0.35;
        }
    }
    if possible == 0.0 {
        0.0
    } else {
        score / possible
    }
}
