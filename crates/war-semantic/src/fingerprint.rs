use std::hash::{Hash, Hasher};
use war_protocol::{DesktopNode, NodeFingerprint, NodeId, RawSnapshot};

pub fn fingerprint(raw: &mut RawSnapshot) {
    let root = raw.root;
    fingerprint_subtree(raw, root, 0);
}

fn fingerprint_subtree(raw: &mut RawSnapshot, id: NodeId, ancestor_hash: u64) {
    let Some(node) = raw.nodes.get(&id).cloned() else {
        return;
    };
    let own_hash = stable_hash(&(
        ancestor_hash,
        node.role,
        node.automation_id.as_deref(),
        node.name.as_deref(),
    ));
    if let Some(current) = raw.nodes.get_mut(&id) {
        current.fingerprint = NodeFingerprint {
            process_id: current.fingerprint.process_id,
            role: current.role,
            automation_id: current.automation_id.as_ref().map(stable_hash),
            name_hash: current.name.as_ref().map(stable_hash),
            ancestor_hash,
            sibling_hint: current.fingerprint.sibling_hint,
        };
    }
    for (index, child) in node.children.iter().enumerate() {
        if let Some(child_node) = raw.nodes.get_mut(child) {
            child_node.fingerprint.sibling_hint = index.min(u16::MAX as usize) as u16;
        }
        fingerprint_subtree(raw, *child, own_hash);
    }
}

pub fn stable_hash<T: Hash + ?Sized>(value: &T) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

pub fn fingerprint_score(old: &DesktopNode, new: &DesktopNode) -> f32 {
    let a = &old.fingerprint;
    let b = &new.fingerprint;
    if a.process_id != 0 && b.process_id != 0 && a.process_id != b.process_id {
        return 0.0;
    }
    let mut score = 0.0;
    let mut max = 0.0;
    max += 50.0;
    if old.source == new.source {
        score += 50.0;
    }
    max += 40.0;
    if old.role == new.role {
        score += 40.0;
    }
    if a.automation_id.is_some() && b.automation_id.is_some() {
        max += 100.0;
        if a.automation_id == b.automation_id {
            score += 100.0;
        }
    }
    if a.name_hash.is_some() && b.name_hash.is_some() {
        max += 35.0;
        if a.name_hash == b.name_hash {
            score += 35.0;
        }
    }
    max += 30.0;
    if a.ancestor_hash == b.ancestor_hash {
        score += 30.0;
    }
    max += 5.0;
    if a.sibling_hint == b.sibling_hint {
        score += 5.0;
    }
    if let (Some(a), Some(b)) = (old.bounds, new.bounds) {
        max += 10.0;
        let ac = a.center();
        let bc = b.center();
        if (ac.x - bc.x).abs() < 16.0 && (ac.y - bc.y).abs() < 16.0 {
            score += 10.0;
        }
    }
    score / max
}
