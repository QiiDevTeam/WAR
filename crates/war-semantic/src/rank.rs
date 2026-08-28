use war_protocol::{DesktopNode, Role};

pub fn relevance(node: &DesktopNode) -> i32 {
    let mut score = 0;
    if node.states.focused {
        score += 1000;
    }
    if !node.capabilities.is_empty() {
        score += 300;
    }
    if node.name.is_some() {
        score += 100;
    }
    if node.value.is_some() {
        score += 80;
    }
    if !node.states.offscreen {
        score += 50;
    }
    score
        + match node.role {
            Role::Window | Role::Dialog => 200,
            Role::Button | Role::TextInput | Role::Checkbox | Role::ComboBox => 150,
            Role::Pane | Role::Group => -50,
            _ => 0,
        }
}
