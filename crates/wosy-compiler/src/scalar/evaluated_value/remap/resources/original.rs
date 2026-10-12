use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct OriginalResourceVertex {
    pub(super) reference: BoundaryResourceRef,
    pub(super) value: BoundaryResourceValue,
    pub(super) dependencies: Vec<BoundaryResourceRef>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct OriginalResourceClosure {
    pub(super) vertices: Vec<OriginalResourceVertex>,
    pub(super) originals: Vec<BoundaryResourceConstructionEvidence>,
}

fn structural_value(key: &BoundaryResourceKey) -> BoundaryResourceValue {
    match key {
        BoundaryResourceKey::Binding(binding) => BoundaryResourceValue::Binding(binding.clone()),
        BoundaryResourceKey::Place(place) => BoundaryResourceValue::Place(place.clone()),
        BoundaryResourceKey::Loan(loan) => BoundaryResourceValue::Loan(loan.clone()),
        BoundaryResourceKey::Runtime(runtime) => BoundaryResourceValue::Runtime(runtime.clone()),
        BoundaryResourceKey::Backing(backing) => BoundaryResourceValue::Backing(backing.clone()),
        BoundaryResourceKey::Holder(holder) => BoundaryResourceValue::Holder(holder.clone()),
        BoundaryResourceKey::Origin(_) | BoundaryResourceKey::Environment(_) => unreachable!(),
    }
}

pub(super) fn missing(reference: &BoundaryResourceRef) -> ValueSchemaRemapError {
    match &reference.key {
        BoundaryResourceKey::Origin(origin) => ValueSchemaRemapError::MissingOrigin { producer: reference.owner.clone(), origin: *origin },
        BoundaryResourceKey::Binding(binding) => ValueSchemaRemapError::MissingBinding(binding.clone()),
        BoundaryResourceKey::Place(place) => ValueSchemaRemapError::MissingPlace(place.clone()),
        BoundaryResourceKey::Loan(loan) => ValueSchemaRemapError::MissingLoan(loan.clone()),
        BoundaryResourceKey::Runtime(runtime) => ValueSchemaRemapError::MissingRuntime(runtime.clone()),
        BoundaryResourceKey::Environment(environment) => ValueSchemaRemapError::MissingEnvironment(environment.clone()),
        BoundaryResourceKey::Backing(backing) => ValueSchemaRemapError::MissingBacking(backing.clone()),
        BoundaryResourceKey::Holder(holder) => ValueSchemaRemapError::MissingHolder(holder.clone()),
    }
}

fn add(keys: &mut Vec<BoundaryResourceKey>, key: BoundaryResourceKey) {
    if !keys.contains(&key) { keys.push(key); }
}

fn backing(keys: &mut Vec<BoundaryResourceKey>, fact: &RuntimeBackingFact) {
    add(keys, BoundaryResourceKey::Backing(fact.clone()));
}

fn input(keys: &mut Vec<BoundaryResourceKey>, fact: &CallableCaptureInput) {
    add(keys, BoundaryResourceKey::Binding(fact.binding.clone()));
    for projected in &fact.projected { add(keys, BoundaryResourceKey::Place(projected.place.clone())); }
    for fact in &fact.backings { backing(keys, fact); }
    for projected in &fact.projected_backings {
        add(keys, BoundaryResourceKey::Place(projected.place.clone()));
        backing(keys, &projected.backing);
    }
}

fn references(keys: &mut Vec<BoundaryResourceKey>, children: Vec<BoundaryResourceRef>) {
    for child in children {
        match child.key {
            BoundaryResourceKey::Origin(_) | BoundaryResourceKey::Environment(_) => {}
            key => add(keys, key),
        }
    }
}

fn candidate(keys: &mut Vec<BoundaryResourceKey>, candidate: &InvalidationCandidate) {
    add(keys, BoundaryResourceKey::Place(candidate.place.clone()));
    add(keys, BoundaryResourceKey::Loan(candidate.loan.clone()));
}

fn projections(keys: &mut Vec<BoundaryResourceKey>, projections: &[ReferencePlaceProjection]) {
    for projection in projections {
        if let ReferencePlaceProjection::Dereference(loan) = projection { add(keys, BoundaryResourceKey::Loan(*loan.clone())); }
    }
}

fn length(keys: &mut Vec<BoundaryResourceKey>, length: &RuntimeArrayLengthFact) {
    if let RuntimeArrayLengthFact::Parameter { binding } = length { add(keys, BoundaryResourceKey::Binding(binding.clone())); }
}

fn inventory(cfg: &ScalarReferenceCfg) -> Vec<BoundaryResourceKey> {
    let mut keys = Vec::new();
    for reference in demand::table_roots(&cfg.producer_owner, &cfg.evaluated_values, &BoundaryResourceDemandCut::Full) {
        if reference.owner != cfg.producer_owner { continue; }
        match reference.key {
            BoundaryResourceKey::Origin(_) | BoundaryResourceKey::Environment(_) => {}
            key => add(&mut keys, key),
        }
    }
    for binding in cfg.allocations.iter().chain(&cfg.reference_parameters).chain(&cfg.backing_parameters) {
        add(&mut keys, BoundaryResourceKey::Binding(binding.clone()));
    }
    for (binding, place) in &cfg.checked_bindings {
        add(&mut keys, BoundaryResourceKey::Binding(binding.clone()));
        add(&mut keys, BoundaryResourceKey::Place(place.clone()));
    }
    for fact in &cfg.facts {
        match fact {
            ReferenceOrigin::Fresh { loan, .. } | ReferenceOrigin::BorrowedFrom { loan, .. } => add(&mut keys, BoundaryResourceKey::Loan(loan.clone())),
            ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => {}
        }
    }
    for fact in &cfg.environments {
        for owner in &fact.owners { add(&mut keys, BoundaryResourceKey::Binding(owner.clone())); }
        for value in &fact.backings { backing(&mut keys, value); }
        for value in &fact.inputs { input(&mut keys, value); }
        for path in &fact.capture_paths { for value in path { input(&mut keys, value); } }
    }
    for value in &cfg.inputs { input(&mut keys, value); }
    for path in &cfg.capture_input_paths { for value in path { input(&mut keys, value); } }
    for path in &cfg.backing_input_paths { for value in path { backing(&mut keys, value); } }
    for path in &cfg.projected_backing_input_paths {
        for argument in path { projections(&mut keys, &argument.projections); backing(&mut keys, &argument.backing); }
    }
    for parameter in &cfg.callable_array_parameters { add(&mut keys, BoundaryResourceKey::Binding(parameter.binding.clone())); }
    for site in &cfg.backing_length_sites { add(&mut keys, BoundaryResourceKey::Binding(site.binding.clone())); length(&mut keys, &site.length); }
    for signature in &cfg.call_signatures { for binding in &signature.bindings { add(&mut keys, BoundaryResourceKey::Binding(binding.clone())); } }
    for contract in &cfg.backing_output_contracts {
        projections(&mut keys, &contract.projections);
        for source in &contract.sources {
            if let RuntimeOutputBackingSource::Created { lineage, length: len, callable_family, owner, .. } = source {
                add(&mut keys, BoundaryResourceKey::Runtime(lineage.clone()));
                add(&mut keys, BoundaryResourceKey::Binding(owner.clone()));
                length(&mut keys, len);
                for family in callable_family { references(&mut keys, children::family_references(&cfg.producer_owner, family)); }
            }
        }
    }
    for edge in &cfg.holder_edges { for action in &edge.actions { references(&mut keys, children::action_references(&cfg.producer_owner, action)); } }
    for exit in &cfg.holder_exits { for action in &exit.actions { references(&mut keys, children::action_references(&cfg.producer_owner, action)); } }
    for path in &cfg.global_callable_input_paths {
        for holder in &path.holders {
            references(&mut keys, children::holder_place_references(&cfg.producer_owner, &holder.holder));
            for relation in &holder.relations { for holder in &relation.holders { references(&mut keys, children::holder_place_references(&cfg.producer_owner, holder)); } }
        }
    }
    for exit in &cfg.global_call_exit_facts {
        for action in &exit.actions { references(&mut keys, children::action_references(&cfg.producer_owner, action)); }
        for holder in &exit.kills { references(&mut keys, children::holder_place_references(&cfg.producer_owner, holder)); }
        for relation in &exit.installs { for holder in &relation.holders { references(&mut keys, children::holder_place_references(&cfg.producer_owner, holder)); } }
        for holder in &exit.post_holders {
            references(&mut keys, children::holder_place_references(&cfg.producer_owner, &holder.holder));
            for relation in &holder.relations { for holder in &relation.holders { references(&mut keys, children::holder_place_references(&cfg.producer_owner, holder)); } }
        }
    }
    for holder in &cfg.nested_call_installs { references(&mut keys, children::holder_place_references(&cfg.producer_owner, holder)); }
    for fact in &cfg.capture_input_path_facts {
        for value in &fact.inputs {
            add(&mut keys, BoundaryResourceKey::Binding(value.binding.clone()));
            if let CaptureInputOriginFact::Callable { holders } = &value.origin {
                for holder in holders { add(&mut keys, BoundaryResourceKey::Holder(holder.clone())); }
            }
        }
    }
    for value in &cfg.producer_return_cases {
        add(&mut keys, BoundaryResourceKey::Holder(ConnectedCaptureHolderFact {
            holder: value.holder.clone(), environment: value.environment.clone(), producer: value.producer.clone(),
        }));
    }
    for point in cfg.points.iter().chain(&cfg.initial_points) {
        for value in &point.possible_backings { backing(&mut keys, value); }
        for projected in &point.possible_projected_backings {
            add(&mut keys, BoundaryResourceKey::Place(projected.place.clone()));
            backing(&mut keys, &projected.backing);
        }
        for projected in &point.possible_projected_environments { add(&mut keys, BoundaryResourceKey::Place(projected.place.clone())); }
        match &point.kind {
            CfgPointKind::Borrow { request: ReferenceBorrowRequest::DirectStorage { place, loan, .. } } => {
                add(&mut keys, BoundaryResourceKey::Place(place.clone())); add(&mut keys, BoundaryResourceKey::Loan(loan.clone()));
            }
            CfgPointKind::AllocationIdentityAddress { request } => candidate(&mut keys, &request.candidate),
            CfgPointKind::CopyCallableFields { source, target } => {
                add(&mut keys, BoundaryResourceKey::Binding(target.clone()));
                if let CallableFieldSource::Assignment(binding) = source { add(&mut keys, BoundaryResourceKey::Binding(binding.clone())); }
            }
            CfgPointKind::AssignmentDestination { storage, .. } => match storage {
                EvaluatedDestinationStorage::Direct(place) => add(&mut keys, BoundaryResourceKey::Place(place.clone())),
                EvaluatedDestinationStorage::Inferred(binding) => add(&mut keys, BoundaryResourceKey::Binding(binding.clone())),
                EvaluatedDestinationStorage::Through { cases, .. } => for case in cases { add(&mut keys, BoundaryResourceKey::Place(case.place.clone())); },
            },
            CfgPointKind::Bind { binding } | CfgPointKind::Read { binding }
            | CfgPointKind::AssignmentSnapshot { holder: binding, .. }
            | CfgPointKind::Backing { target: binding, .. } => add(&mut keys, BoundaryResourceKey::Binding(binding.clone())),
            CfgPointKind::BindArrayElement { target, .. } | CfgPointKind::BackingPlace { target, .. }
            | CfgPointKind::SelectArrayElement { target, .. } | CfgPointKind::Assign { target, .. }
            | CfgPointKind::CopyProjectedBackings { target, .. } => add(&mut keys, BoundaryResourceKey::Place(target.clone())),
            CfgPointKind::Move { place, owner, .. } | CfgPointKind::DeferredMove { place, owner, .. } => {
                add(&mut keys, BoundaryResourceKey::Place(place.clone()));
                add(&mut keys, BoundaryResourceKey::Binding(owner.clone()));
            }
            CfgPointKind::Capture { captures, .. } => {
                for capture in captures { add(&mut keys, BoundaryResourceKey::Binding(capture.binding.clone())); }
            }
            CfgPointKind::ReadPlace { candidates, .. } => {
                for place in candidates { add(&mut keys, BoundaryResourceKey::Place(place.clone())); }
            }
            CfgPointKind::AssignThrough { candidates, .. } | CfgPointKind::MoveThrough { candidates, .. }
            | CfgPointKind::DeferredMoveThrough { candidates, .. } | CfgPointKind::Invalidate { candidates, .. } => {
                for candidate in candidates {
                    add(&mut keys, BoundaryResourceKey::Place(candidate.place.clone()));
                    add(&mut keys, BoundaryResourceKey::Loan(candidate.loan.clone()));
                }
            }
            _ => {}
        }
        match &point.kind {
            CfgPointKind::Backing { transfer, .. } | CfgPointKind::BackingPlace { transfer, .. } => match transfer {
                RuntimeBackingTransfer::CopyFixed { source } | RuntimeBackingTransfer::Move { source } => add(&mut keys, BoundaryResourceKey::Binding(source.clone())),
                RuntimeBackingTransfer::CreateFixed | RuntimeBackingTransfer::Allocate => {}
            },
            CfgPointKind::CopyProjectedBackings { source, .. } => add(&mut keys, BoundaryResourceKey::Binding(source.clone())),
            CfgPointKind::Assign { previous_loans, .. } | CfgPointKind::AssignThrough { previous_loans, .. } => {
                for loan in previous_loans { add(&mut keys, BoundaryResourceKey::Loan(loan.clone())); }
            }
            CfgPointKind::Call { target, .. } => match target {
                ResolvedCallTarget::LocalCallable(binding) => add(&mut keys, BoundaryResourceKey::Binding(binding.clone())),
                ResolvedCallTarget::CallableField { receiver_place, .. } => add(&mut keys, BoundaryResourceKey::Place(receiver_place.clone())),
                ResolvedCallTarget::Core(_) | ResolvedCallTarget::ModuleCallable { .. } | ResolvedCallTarget::ExternCallable { .. } => {}
            },
            CfgPointKind::Release { target } => match target {
                ReleaseTarget::Raw { binding } => add(&mut keys, BoundaryResourceKey::Binding(binding.clone())),
                ReleaseTarget::Checked { candidates, .. } => for value in candidates { candidate(&mut keys, value); },
            },
            CfgPointKind::ReturnOutput { backing_action, .. } => match backing_action {
                RuntimeReturnBackingAction::CopyFixed { source } | RuntimeReturnBackingAction::MoveFixed { source } => add(&mut keys, BoundaryResourceKey::Binding(source.clone())),
                RuntimeReturnBackingAction::Transfer => {}
            },
            _ => {}
        }
    }
    for evaluation in &cfg.borrow_evaluations {
        for case in &evaluation.cases {
            if let ReferenceBorrowCase::Accepted { place, loan, .. } = case {
                add(&mut keys, BoundaryResourceKey::Place(place.clone()));
                add(&mut keys, BoundaryResourceKey::Loan(loan.clone()));
            }
        }
    }
    for evaluation in &cfg.checked_install_evaluations { add(&mut keys, BoundaryResourceKey::Place(evaluation.target.clone())); }
    for install in &cfg.checked_installs {
        if let ReferenceCheckedInstallInput::Parameter { binding, .. } = &install.input { add(&mut keys, BoundaryResourceKey::Binding(binding.clone())); }
    }
    for evaluation in &cfg.checked_install_evaluations {
        if let ReferenceCheckedInstallInput::Parameter { binding, .. } = &evaluation.input { add(&mut keys, BoundaryResourceKey::Binding(binding.clone())); }
    }
    for evaluation in &cfg.allocation_address_evaluations {
        for case in &evaluation.cases { if let ReferenceAllocationAddressCase::Accepted { candidate: value } = case { candidate(&mut keys, value); } }
    }
    for evaluation in &cfg.rebind_evaluations {
        candidate(&mut keys, &evaluation.candidate);
        for value in evaluation.previous_backings.iter().chain(&evaluation.resulting_backings) { backing(&mut keys, value); }
    }
    let mut next = 0;
    while next < keys.len() {
        let key = &keys[next];
        let children = children::references(&cfg.producer_owner, &structural_value(key));
        for child in children {
            match child.key {
                BoundaryResourceKey::Origin(_) | BoundaryResourceKey::Environment(_) => {}
                key => add(&mut keys, key),
            }
        }
        next += 1;
    }
    keys
}

fn resolve(reference: &BoundaryResourceRef, cfg: &ScalarReferenceCfg) -> Result<BoundaryResourceValue, ValueSchemaRemapError> {
    if cfg.producer_owner != reference.owner { return Err(missing(reference)); }
    match &reference.key {
        BoundaryResourceKey::Origin(id) => cfg.facts.get(id.0).map(|fact| BoundaryResourceValue::Origin { id: *id, fact: fact.clone() }).ok_or_else(|| missing(reference)),
        BoundaryResourceKey::Environment(id) => cfg.environments.get(id.index).filter(|fact| fact.literal == id.literal)
            .map(|fact| BoundaryResourceValue::Environment { id: id.clone(), fact: fact.clone() }).ok_or_else(|| missing(reference)),
        key => {
            if !inventory(cfg).contains(key) { return Err(missing(reference)); }
            Ok(structural_value(key))
        }
    }
}

pub(super) fn catalog(source: &[ValueTableSource]) -> Result<OriginalResourceClosure, ValueSchemaRemapError> {
    let mut vertices = Vec::new();
    let mut originals = Vec::new();
    for carrier in source {
        let cfg = carrier.resource_construction.original();
        if cfg.producer_owner != carrier.producer || originals.iter().any(|original: &BoundaryResourceConstructionEvidence| original.original().producer_owner == carrier.producer) {
            return Err(ValueSchemaRemapError::MissingProducer(carrier.producer.clone()));
        }
        let mut keys = cfg.facts.iter().enumerate().map(|(index, _)| BoundaryResourceKey::Origin(ReferenceOriginId(index))).collect::<Vec<_>>();
        keys.extend(cfg.environments.iter().enumerate().map(|(index, fact)| BoundaryResourceKey::Environment(CallableEnvironmentId { literal: fact.literal.clone(), index })));
        keys.extend(inventory(cfg));
        for key in keys {
            let reference = BoundaryResourceRef { owner: carrier.producer.clone(), key };
            let value = resolve(&reference, cfg)?;
            let dependencies = children::references(&reference.owner, &value);
            vertices.push(OriginalResourceVertex { reference, value, dependencies });
        }
        originals.push(carrier.resource_construction.clone());
    }
    Ok(OriginalResourceClosure { vertices, originals })
}

pub(super) fn closure(demand: &BoundaryResourceDemand, forest: &SourceCaptureConstructionForest) -> Result<OriginalResourceClosure, ValueSchemaRemapError> {
    let mut vertices: Vec<OriginalResourceVertex> = Vec::new();
    let mut originals = Vec::new();
    let mut pending = demand.roots.clone();
    let mut next = 0;
    while next < pending.len() {
        let reference = pending[next].clone();
        next += 1;
        if vertices.iter().any(|row| row.reference == reference) { continue; }
        let carriers = forest.sources.iter().filter(|row| row.producer == reference.owner).collect::<Vec<_>>();
        let [carrier] = carriers.as_slice() else { return Err(missing(&reference)); };
        let value = resolve(&reference, carrier.resource_construction.original())?;
        let mut dependencies = children::references(&reference.owner, &value);
        if let BoundaryResourceValue::Holder(holder) = &value {
            dependencies.extend(creators::dependencies(&reference.owner, holder, forest)?);
        }
        creators::qualify_runtime(&mut dependencies, forest)?;
        vertices.push(OriginalResourceVertex { reference: reference.clone(), value, dependencies: dependencies.clone() });
        if !originals.contains(&carrier.resource_construction) { originals.push(carrier.resource_construction.clone()); }
        if let BoundaryResourceKey::Holder(holder) = &reference.key {
            for creator in creators::validate(&reference.owner, holder, forest)? {
                if !originals.contains(&creator) { originals.push(creator); }
            }
        }
        pending.extend(dependencies);
    }
    Ok(OriginalResourceClosure { vertices, originals })
}
