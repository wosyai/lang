use super::*;

fn close(members: &mut BTreeSet<CommonLoanMembership>, subset: &[CommonLoanSubset]) {
    let mut ready = std::collections::VecDeque::from_iter(members.iter().copied());
    while let Some(member) = ready.pop_front() {
        for edge in subset.iter().filter(|edge| edge.source == member.role) {
            let next = CommonLoanMembership {
                role: edge.target,
                site: member.site,
            };
            if members.insert(next) {
                ready.push_back(next);
            }
        }
    }
}
fn apply(members: &mut BTreeSet<CommonLoanMembership>, fact: &CommonLoanFactKind) {
    match fact {
        CommonLoanFactKind::Issue { output, site } => {
            members.insert(CommonLoanMembership {
                role: *output,
                site: *site,
            });
        }
        CommonLoanFactKind::KillSlot(role) => members.retain(|member| member.role != *role),
        CommonLoanFactKind::Flow { source, target, .. } => {
            let sites = members
                .iter()
                .filter(|member| member.role == *source)
                .map(|member| member.site)
                .collect::<Vec<_>>();
            for site in sites {
                members.insert(CommonLoanMembership {
                    role: *target,
                    site,
                });
            }
        }
        _ => {}
    }
}
fn subset(input: &CommonLoanInput) -> Vec<CommonLoanSubset> {
    let mut edges = (0..input.roles.len())
        .map(|role| CommonLoanSubset {
            source: CommonLoanRoleId(role),
            target: CommonLoanRoleId(role),
        })
        .collect::<Vec<_>>();
    for event in &input.events {
        for fact in &event.facts {
            if let CommonLoanFactKind::Flow { source, target, .. } = fact.kind {
                let edge = CommonLoanSubset { source, target };
                if !edges.contains(&edge) {
                    edges.push(edge);
                }
            }
        }
    }
    let mut cursor = 0;
    while cursor < edges.len() {
        let current = edges[cursor].clone();
        for edge in edges.clone() {
            for next in [
                (current.target == edge.source).then_some(CommonLoanSubset {
                    source: current.source,
                    target: edge.target,
                }),
                (edge.target == current.source).then_some(CommonLoanSubset {
                    source: edge.source,
                    target: current.target,
                }),
            ]
            .into_iter()
            .flatten()
            {
                if !edges.contains(&next) {
                    edges.push(next);
                }
            }
        }
        cursor += 1;
    }
    edges
}

pub(super) fn overlaps(left: &CommonLoanTarget, right: &CommonLoanTarget) -> bool {
    let (
        CommonLoanTarget::Storage {
            binding: left_binding,
            projection: left,
        },
        CommonLoanTarget::Storage {
            binding: right_binding,
            projection: right,
        },
    ) = (left, right)
    else {
        return true;
    };
    if left_binding != right_binding {
        return false;
    }
    for (left, right) in left.iter().zip(right) {
        match (left, right) {
            (ValueProjection::Field(left), ValueProjection::Field(right)) if left != right => {
                return false
            }
            (ValueProjection::FixedIndex(left), ValueProjection::FixedIndex(right))
                if left != right =>
            {
                return false
            }
            _ => {}
        }
    }
    true
}

pub(super) fn solve(input: CommonLoanInput) -> CommonLoanAnalysis {
    let subset = subset(&input);
    let count = input.events.len();
    let mut incoming = vec![BTreeSet::<CommonLoanMembership>::new(); count];
    let mut outgoing = incoming.clone();
    let mut reached = vec![false; count];
    let mut queued = vec![false; count];
    let mut ready = std::collections::VecDeque::from([0]);
    reached[0] = true;
    queued[0] = true;
    while let Some(index) = ready.pop_front() {
        queued[index] = false;
        let event = &input.events[index];
        let mut next = incoming[index].clone();
        for fact in &event.facts {
            apply(&mut next, &fact.kind);
        }
        close(&mut next, &subset);
        outgoing[index] = next.clone();
        for successor in &event.successors {
            let fresh = !reached[successor.point.0];
            let before = incoming[successor.point.0].len();
            incoming[successor.point.0].extend(next.iter().copied());
            reached[successor.point.0] = true;
            if (fresh || incoming[successor.point.0].len() != before) && !queued[successor.point.0]
            {
                ready.push_back(successor.point.0);
                queued[successor.point.0] = true;
            }
        }
    }
    let mut parents = vec![BTreeSet::<CommonLoanSiteId>::new(); input.sites.len()];
    let mut parent_roles = vec![BTreeSet::<CommonLoanRoleId>::new(); input.sites.len()];
    let mut pending = input.pending.clone();
    for (index, event) in input
        .events
        .iter()
        .enumerate()
        .filter(|(index, _)| reached[*index])
    {
        for fact in &event.facts {
            if let CommonLoanFactKind::ParentSite { child, parent } = fact.kind {
                parent_roles[child.0].insert(parent);
                parents[child.0].extend(
                    incoming[index]
                        .iter()
                        .filter(|member| member.role == parent)
                        .map(|member| member.site),
                );
                if parents[child.0].is_empty() {
                    let coverage = CommonLoanPending {
                        event: event.event.clone(),
                        source_span: fact.source_span.clone(),
                        reason: CommonLoanPendingReason::UnresolvedParent(child),
                    };
                    if !pending.contains(&coverage) {
                        pending.push(coverage);
                    }
                }
            }
        }
    }
    let mut demand_in = vec![BTreeSet::<CommonLoanRoleId>::new(); count];
    let mut demand_out = demand_in.clone();
    loop {
        let mut changed = false;
        for index in (0..count).rev().filter(|index| reached[*index]) {
            let event = &input.events[index];
            let mut after = BTreeSet::new();
            for successor in &event.successors {
                after.extend(demand_in[successor.point.0].iter().copied());
            }
            let mut before = after.clone();
            for fact in event.facts.iter().rev() {
                match &fact.kind {
                    CommonLoanFactKind::Use { role, .. } => {
                        before.insert(*role);
                    }
                    CommonLoanFactKind::Flow { source, target, .. } if before.contains(target) => {
                        before.insert(*source);
                    }
                    CommonLoanFactKind::KillSlot(role) => {
                        before.remove(role);
                    }
                    CommonLoanFactKind::Issue { output, .. } => {
                        before.remove(output);
                    }
                    _ => {}
                }
            }
            let mut live = incoming[index]
                .iter()
                .filter(|member| before.contains(&member.role))
                .chain(
                    outgoing[index]
                        .iter()
                        .filter(|member| after.contains(&member.role)),
                )
                .map(|member| member.site)
                .collect::<BTreeSet<_>>();
            let mut parent_ready = std::collections::VecDeque::from_iter(live.iter().copied());
            while let Some(site) = parent_ready.pop_front() {
                before.extend(parent_roles[site.0].iter().copied());
                after.extend(parent_roles[site.0].iter().copied());
                for parent in &parents[site.0] {
                    if live.insert(*parent) {
                        parent_ready.push_back(*parent);
                    }
                }
            }
            let prior_in = demand_in[index].len();
            let prior_out = demand_out[index].len();
            demand_in[index].extend(before);
            demand_out[index].extend(after);
            changed |= demand_in[index].len() != prior_in || demand_out[index].len() != prior_out;
        }
        if !changed {
            break;
        }
    }
    let mut points = Vec::new();
    let mut candidates = Vec::new();
    for (index, event) in input.events.iter().enumerate() {
        let mut live = incoming[index]
            .iter()
            .filter(|member| demand_in[index].contains(&member.role))
            .chain(
                outgoing[index]
                    .iter()
                    .filter(|member| demand_out[index].contains(&member.role)),
            )
            .map(|member| member.site)
            .collect::<BTreeSet<_>>();
        let mut ready = std::collections::VecDeque::from_iter(live.iter().copied());
        while let Some(site) = ready.pop_front() {
            for parent in &parents[site.0] {
                if live.insert(*parent) {
                    ready.push_back(*parent);
                }
            }
        }
        let issued = if reached[index] {
            event
                .facts
                .iter()
                .filter_map(|fact| {
                    if let CommonLoanFactKind::Issue { site, .. } = fact.kind {
                        Some(site)
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        points.push(CommonLoanPoint {
            event: event.event.clone(),
            incoming: incoming[index].iter().copied().collect(),
            outgoing: outgoing[index].iter().copied().collect(),
            demand_in: demand_in[index].iter().copied().collect(),
            demand_out: demand_out[index].iter().copied().collect(),
            issued,
            may_live: live.iter().copied().collect(),
        });
        if !reached[index] {
            continue;
        }
        let mut current = incoming[index].clone();
        for fact in &event.facts {
            if let CommonLoanFactKind::Use {
                role,
                mode,
                target,
                kind,
            } = &fact.kind
            {
                let sites = if *kind == CommonLoanUseKind::Escape {
                    current
                        .iter()
                        .filter(|member| member.role == *role)
                        .map(|member| member.site)
                        .collect::<BTreeSet<_>>()
                } else {
                    live.iter()
                        .filter(|site| {
                            current.iter().any(|member| member.site == **site)
                                && (*mode == ScalarReferenceMutability::Mutable
                                    || input.sites[site.0].mode
                                        == ScalarReferenceMutability::Mutable)
                                && overlaps(target, &input.sites[site.0].target)
                        })
                        .copied()
                        .collect::<BTreeSet<_>>()
                };
                if !sites.is_empty() {
                    candidates.push(CommonLoanCandidate {
                        event: event.event.clone(),
                        source_span: fact.source_span.clone(),
                        role: *role,
                        mode: *mode,
                        target: target.clone(),
                        sites: sites.into_iter().collect(),
                        reason: if *kind == CommonLoanUseKind::Escape {
                            CommonLoanCandidateReason::EscapeOutlives
                        } else {
                            CommonLoanCandidateReason::PossibleConflict
                        },
                    });
                }
            }
            apply(&mut current, &fact.kind);
        }
    }
    CommonLoanAnalysis {
        input,
        subset,
        points,
        candidates,
        pending,
    }
}
