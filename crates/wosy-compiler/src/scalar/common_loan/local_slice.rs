use super::*;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocalLoanCandidateRef {
    pub owner: ProducerCfgOwnerId,
    pub ordinal: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocalLoanPoint {
    pub event: CommonLoanEvent,
    pub incoming_subset: Vec<CommonLoanSubset>,
    pub outgoing_subset: Vec<CommonLoanSubset>,
    pub incoming: Vec<CommonLoanMembership>,
    pub outgoing: Vec<CommonLoanMembership>,
    pub demand_in: Vec<CommonLoanRoleId>,
    pub demand_out: Vec<CommonLoanRoleId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocalLoanSlice {
    pub candidate: LocalLoanCandidateRef,
    pub roles: Vec<CommonLoanRoleId>,
    pub sites: Vec<CommonLoanSiteId>,
    pub points: Vec<LocalLoanPoint>,
    pub coverage: Vec<CommonLoanPending>,
}

fn relevant(
    kind: &CommonLoanFactKind,
    roles: &BTreeSet<CommonLoanRoleId>,
    sites: &BTreeSet<CommonLoanSiteId>,
) -> bool {
    match kind {
        CommonLoanFactKind::Issue { output, site } => {
            roles.contains(output) || sites.contains(site)
        }
        CommonLoanFactKind::Flow { source, target, .. } => {
            roles.contains(source) || roles.contains(target)
        }
        CommonLoanFactKind::KillSlot(role) | CommonLoanFactKind::Use { role, .. } => {
            roles.contains(role)
        }
        CommonLoanFactKind::ParentSite { child, parent } => {
            sites.contains(child) || roles.contains(parent)
        }
        CommonLoanFactKind::NoLoan { .. } => false,
    }
}

pub(in crate::scalar) fn solve(analysis: &CommonLoanAnalysis, ordinal: usize) -> LocalLoanSlice {
    let candidate = &analysis.candidates[ordinal];
    let input = &analysis.input;
    let mut roles = BTreeSet::from([candidate.role]);
    let mut sites = BTreeSet::from_iter(candidate.sites.iter().copied());
    match &candidate.target {
        CommonLoanTarget::Through { pointer, .. } => {
            roles.insert(*pointer);
        }
        CommonLoanTarget::Storage {
            binding,
            projection,
        } => {
            for (index, role) in input.roles.iter().enumerate() {
                if role.source == CommonLoanSourceRole::Binding(binding.clone())
                    && role.projection == *projection
                {
                    roles.insert(CommonLoanRoleId(index));
                }
            }
        }
    }
    loop {
        let before = (roles.len(), sites.len());
        for site in sites.clone() {
            match &input.sites[site.0].target {
                CommonLoanTarget::Through { pointer, .. } => {
                    roles.insert(*pointer);
                }
                CommonLoanTarget::Storage {
                    binding,
                    projection,
                } => {
                    for (index, role) in input.roles.iter().enumerate() {
                        if role.source == CommonLoanSourceRole::Binding(binding.clone())
                            && role.projection == *projection
                        {
                            roles.insert(CommonLoanRoleId(index));
                        }
                    }
                }
            }
        }
        for event in &input.events {
            for fact in &event.facts {
                match &fact.kind {
                    CommonLoanFactKind::Flow { source, target, .. }
                        if roles.contains(source) || roles.contains(target) =>
                    {
                        roles.insert(*source);
                        roles.insert(*target);
                    }
                    CommonLoanFactKind::Issue { output, site }
                        if roles.contains(output) || sites.contains(site) =>
                    {
                        roles.insert(*output);
                        sites.insert(*site);
                    }
                    CommonLoanFactKind::ParentSite { child, parent }
                        if sites.contains(child) || roles.contains(parent) =>
                    {
                        sites.insert(*child);
                        roles.insert(*parent);
                        let common = &analysis.points[event.event.point.0];
                        sites.extend(
                            common
                                .incoming
                                .iter()
                                .filter(|member| member.role == *parent)
                                .map(|member| member.site),
                        );
                    }
                    CommonLoanFactKind::Use {
                        role,
                        target: CommonLoanTarget::Through { pointer, .. },
                        ..
                    } if roles.contains(role) => {
                        roles.insert(*pointer);
                    }
                    _ => {}
                }
            }
        }
        if before == (roles.len(), sites.len()) {
            break;
        }
    }
    let count = input.events.len();
    let facts = input
        .events
        .iter()
        .map(|event| {
            let mut event = event.clone();
            event
                .facts
                .retain(|fact| relevant(&fact.kind, &roles, &sites));
            event
        })
        .collect::<Vec<_>>();
    let mut incoming = vec![BTreeSet::new(); count];
    let mut outgoing = incoming.clone();
    let mut incoming_subset = vec![Vec::<CommonLoanSubset>::new(); count];
    let mut outgoing_subset = incoming_subset.clone();
    let mut reached = vec![false; count];
    let mut ready = std::collections::VecDeque::from([0]);
    reached[0] = true;
    while let Some(index) = ready.pop_front() {
        let event = &facts[index];
        let mut members = incoming[index].clone();
        let mut edges = incoming_subset[index].clone();
        for fact in &event.facts {
            match fact.kind {
                CommonLoanFactKind::KillSlot(role) => {
                    members.retain(|member: &CommonLoanMembership| member.role != role);
                    edges.retain(|edge| edge.source != role && edge.target != role);
                }
                CommonLoanFactKind::Issue { output, site } => {
                    members.retain(|member| member.role != output);
                    members.insert(CommonLoanMembership { role: output, site });
                    edges.retain(|edge| edge.target != output);
                }
                CommonLoanFactKind::Flow { source, target, .. } => {
                    let transported = members
                        .iter()
                        .filter(|member| member.role == source)
                        .map(|member| member.site)
                        .collect::<Vec<_>>();
                    members.retain(|member| member.role != target);
                    members.extend(
                        transported
                            .into_iter()
                            .map(|site| CommonLoanMembership { role: target, site }),
                    );
                    let mut transferred = edges
                        .iter()
                        .filter(|edge| edge.target == source)
                        .map(|edge| CommonLoanSubset {
                            source: edge.source,
                            target,
                        })
                        .collect::<Vec<_>>();
                    transferred.push(CommonLoanSubset { source, target });
                    edges.retain(|edge| edge.target != target);
                    for edge in transferred {
                        if !edges.contains(&edge) {
                            edges.push(edge);
                        }
                    }
                }
                _ => {}
            }
        }
        outgoing[index] = members.clone();
        outgoing_subset[index] = edges.clone();
        for successor in &event.successors {
            let next = successor.point.0;
            let mut changed = !reached[next];
            reached[next] = true;
            let prior = incoming[next].len();
            incoming[next].extend(members.iter().copied());
            changed |= incoming[next].len() != prior;
            for edge in &edges {
                if !incoming_subset[next].contains(edge) {
                    incoming_subset[next].push(edge.clone());
                    changed = true;
                }
            }
            if changed && !ready.contains(&next) {
                ready.push_back(next);
            }
        }
    }
    let mut demand_in = vec![BTreeSet::new(); count];
    let mut demand_out = demand_in.clone();
    loop {
        let mut changed = false;
        for index in (0..count).rev().filter(|index| reached[*index]) {
            let event = &facts[index];
            let mut after = BTreeSet::new();
            for successor in &event.successors {
                after.extend(demand_in[successor.point.0].iter().copied());
            }
            let mut before = after.clone();
            for fact in event.facts.iter().rev() {
                match fact.kind {
                    CommonLoanFactKind::Use { role, .. } => {
                        before.insert(role);
                    }
                    CommonLoanFactKind::Flow { source, target, .. } if before.contains(&target) => {
                        before.remove(&target);
                        before.insert(source);
                    }
                    CommonLoanFactKind::KillSlot(role) => {
                        before.remove(&role);
                    }
                    CommonLoanFactKind::Issue { output, .. } => {
                        before.remove(&output);
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
            loop {
                let prior = live.len();
                for parent_event in &facts {
                    for fact in &parent_event.facts {
                        if let CommonLoanFactKind::ParentSite { child, parent } = fact.kind {
                            if live.contains(&child) {
                                live.extend(
                                    incoming[parent_event.event.point.0]
                                        .iter()
                                        .filter(|member| member.role == parent)
                                        .map(|member| member.site),
                                );
                            }
                        }
                    }
                }
                if live.len() == prior {
                    break;
                }
            }
            for member in &incoming[index] {
                if live.contains(&member.site) {
                    before.insert(member.role);
                }
            }
            for member in &outgoing[index] {
                if live.contains(&member.site) {
                    after.insert(member.role);
                }
            }
            let prior = (demand_in[index].len(), demand_out[index].len());
            demand_in[index].extend(before);
            demand_out[index].extend(after);
            changed |= prior != (demand_in[index].len(), demand_out[index].len());
        }
        if !changed {
            break;
        }
    }
    let mut included = facts
        .iter()
        .filter(|event| !event.facts.is_empty())
        .map(|event| event.event.point.0)
        .collect::<BTreeSet<_>>();
    included.insert(candidate.event.point.0);
    loop {
        let prior = included.len();
        for event in &facts {
            if event
                .successors
                .iter()
                .any(|successor| included.contains(&successor.point.0))
            {
                included.insert(event.event.point.0);
            }
        }
        if prior == included.len() {
            break;
        }
    }
    let coverage = analysis
        .pending
        .iter()
        .filter(|pending| included.contains(&pending.event.point.0))
        .cloned()
        .collect();
    LocalLoanSlice {
        candidate: LocalLoanCandidateRef {
            owner: input.owner.clone(),
            ordinal,
        },
        roles: roles.into_iter().collect(),
        sites: sites.into_iter().collect(),
        points: facts
            .into_iter()
            .enumerate()
            .filter(|(index, _)| reached[*index] && included.contains(index))
            .map(|(index, mut event)| {
                event
                    .successors
                    .retain(|successor| included.contains(&successor.point.0));
                LocalLoanPoint {
                    event,
                    incoming_subset: incoming_subset[index].clone(),
                    outgoing_subset: outgoing_subset[index].clone(),
                    incoming: incoming[index].iter().copied().collect(),
                    outgoing: outgoing[index].iter().copied().collect(),
                    demand_in: demand_in[index].iter().copied().collect(),
                    demand_out: demand_out[index].iter().copied().collect(),
                }
            })
            .collect(),
        coverage,
    }
}
