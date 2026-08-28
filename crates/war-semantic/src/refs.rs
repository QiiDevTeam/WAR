use crate::{fingerprint, fingerprint_score, normalize, relevance, retained_nodes};
use std::collections::{HashMap, HashSet};
use war_core::ProviderNodeRef;
use war_protocol::{
    DesktopNode, NodeId, RawSnapshot, Rect, SemanticNode, SemanticSnapshot, WindowInfo,
};

#[derive(Debug, Clone)]
pub struct CompiledSnapshot {
    pub semantic: SemanticSnapshot,
    pub provider_refs: HashMap<NodeId, ProviderNodeRef>,
    pub bounds: HashMap<NodeId, Rect>,
}

#[derive(Debug, Default)]
pub struct SemanticCompiler {
    next_id: NodeId,
    previous: HashMap<NodeId, DesktopNode>,
    max_nodes: usize,
}

impl SemanticCompiler {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            previous: HashMap::new(),
            max_nodes: 256,
        }
    }
    pub fn with_max_nodes(max_nodes: usize) -> Self {
        Self {
            max_nodes,
            ..Self::new()
        }
    }

    /// Drops cross-snapshot identity candidates without recycling public refs.
    /// This is required when a session changes window/scope: an old `@ref`
    /// must never silently point at an unrelated node in a different app.
    pub fn reset_history(&mut self) {
        self.previous.clear();
    }

    pub fn compile(&mut self, raw: RawSnapshot, window: WindowInfo) -> CompiledSnapshot {
        let mut raw = normalize(raw);
        fingerprint(&mut raw);
        let retained = retained_nodes(&raw);
        let mut used = HashSet::new();
        let mut remap = HashMap::new();
        let mut provider_order = Vec::new();
        collect_ids(&raw, raw.root, &mut provider_order);
        for node in provider_order
            .into_iter()
            .filter_map(|id| raw.nodes.get(&id))
            .filter(|n| retained.contains(&n.id))
        {
            let best = self
                .previous
                .iter()
                .filter(|(id, _)| !used.contains(*id))
                .map(|(id, old)| (*id, fingerprint_score(old, node)))
                .max_by(|a, b| a.1.total_cmp(&b.1));
            let stable = match best {
                Some((id, score)) if score >= 0.75 => {
                    used.insert(id);
                    id
                }
                _ => {
                    let id = self.next_id;
                    self.next_id += 1;
                    id
                }
            };
            remap.insert(node.id, stable);
        }
        let mut ordered = Vec::new();
        walk(&raw, raw.root, 0, &retained, &remap, &mut ordered);
        let total_nodes = ordered.len();
        retain_budgeted_hierarchy(&raw, &mut ordered, self.max_nodes.max(1));
        let truncated = ordered.len() < total_nodes;
        let focused = ordered
            .iter()
            .find(|(n, _)| n.states.focused)
            .and_then(|(n, _)| remap.get(&n.id))
            .copied();
        let nodes = ordered
            .iter()
            .map(|(n, depth)| SemanticNode {
                id: remap[&n.id],
                role: n.role,
                name: n.name.clone(),
                automation_id: n.automation_id.clone(),
                value: n.value.clone(),
                states: n.states.clone(),
                capabilities: n.capabilities,
                depth: *depth,
            })
            .collect();
        let provider_refs = remap
            .iter()
            .map(|(provider, stable)| (*stable, ProviderNodeRef { id: *provider }))
            .collect();
        let bounds = remap
            .iter()
            .filter_map(|(provider, stable)| {
                raw.nodes
                    .get(provider)
                    .and_then(|node| node.bounds)
                    .map(|bounds| (*stable, bounds))
            })
            .collect();
        self.previous = raw
            .nodes
            .values()
            .filter_map(|node| remap.get(&node.id).map(|id| (*id, node.clone())))
            .collect();
        CompiledSnapshot {
            semantic: SemanticSnapshot {
                session_id: String::new(),
                epoch: raw.epoch,
                window,
                nodes,
                total_nodes,
                truncated,
                focused,
            },
            provider_refs,
            bounds,
        }
    }
}

fn retain_budgeted_hierarchy(
    raw: &RawSnapshot,
    ordered: &mut Vec<(DesktopNode, u8)>,
    max_nodes: usize,
) {
    if ordered.len() <= max_nodes {
        return;
    }
    let eligible: HashSet<_> = ordered.iter().map(|(node, _)| node.id).collect();
    let mut ranked: Vec<_> = ordered.iter().map(|(node, _)| node.id).collect();
    ranked
        .sort_by_key(|id| std::cmp::Reverse(raw.nodes.get(id).map(relevance).unwrap_or(i32::MIN)));
    let mut chosen = HashSet::from([raw.root]);
    for id in ranked {
        let mut chain = Vec::new();
        let mut cursor = Some(id);
        while let Some(current) = cursor {
            if chosen.contains(&current) {
                break;
            }
            if eligible.contains(&current) {
                chain.push(current);
            }
            cursor = raw.nodes.get(&current).and_then(|node| node.parent);
        }
        if chosen.len() + chain.len() <= max_nodes {
            chosen.extend(chain);
        }
    }
    ordered.retain(|(node, _)| chosen.contains(&node.id));
}

fn collect_ids(raw: &RawSnapshot, id: NodeId, out: &mut Vec<NodeId>) {
    let Some(node) = raw.nodes.get(&id) else {
        return;
    };
    out.push(id);
    for child in &node.children {
        collect_ids(raw, *child, out);
    }
}

fn walk(
    raw: &RawSnapshot,
    id: NodeId,
    depth: u8,
    retained: &HashSet<NodeId>,
    remap: &HashMap<NodeId, NodeId>,
    out: &mut Vec<(DesktopNode, u8)>,
) {
    let Some(node) = raw.nodes.get(&id) else {
        return;
    };
    if retained.contains(&id) && remap.contains_key(&id) {
        out.push((node.clone(), depth));
    }
    for child in &node.children {
        walk(raw, *child, depth.saturating_add(1), retained, remap, out);
    }
}
