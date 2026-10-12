use super::ReferenceEdgeState;

pub(super) fn admit(states: &mut Vec<ReferenceEdgeState>, incoming: &ReferenceEdgeState) -> bool {
    if states.contains(incoming) {
        return false;
    }
    states.push(incoming.clone());
    true
}
