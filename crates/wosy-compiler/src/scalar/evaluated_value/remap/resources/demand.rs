use super::*;

struct DemandVisitor {
    roots: Vec<BoundaryResourceRef>,
}

impl DemandVisitor {
    fn push(&mut self, owner: &ProducerCfgOwnerId, key: BoundaryResourceKey) {
        self.roots.push(BoundaryResourceRef { owner: owner.clone(), key });
    }

    fn consumer(&mut self, consumer: &ValueConsumerId) {
        if let ValueConsumerKind::Binding { binding, .. } = &consumer.kind {
            self.push(&consumer.producer, BoundaryResourceKey::Binding(binding.clone()));
        }
    }

    fn availability(&mut self, owner: &ProducerCfgOwnerId, availability: &ValueAvailability) {
        if let ValueAvailability::Moved { place, .. } = availability { self.push(owner, BoundaryResourceKey::Place(place.clone())); }
    }

    fn state(&mut self, owner: &ProducerCfgOwnerId, state: &CheckedValueState) {
        if let CheckedValueState::Expired { origin, .. } = state { self.push(owner, BoundaryResourceKey::Origin(*origin)); }
    }

    fn raw(&mut self, owner: &ProducerCfgOwnerId, provenance: &RawValueProvenance) {
        if let RawValueProvenance::CompilerStorage { source_place, .. } = provenance { self.push(owner, BoundaryResourceKey::Place(source_place.clone())); }
    }

    fn dependency(&mut self, owner: &ProducerCfgOwnerId, dependency: &EvaluatedDependencyFact) {
        match &dependency.source {
            DependencySource::Allocation(_) | DependencySource::Manual { .. } => {}
            DependencySource::Environment(environment) => self.push(owner, BoundaryResourceKey::Environment(environment.clone())),
            DependencySource::BindingCell(binding) => self.push(owner, BoundaryResourceKey::Binding(binding.clone())),
            DependencySource::FixedReferent { origin, loan } => {
                self.push(owner, BoundaryResourceKey::Origin(*origin)); self.push(owner, BoundaryResourceKey::Loan(loan.clone()));
            }
            DependencySource::Region { instance, membership } => {
                self.push(owner, BoundaryResourceKey::Binding(instance.clone())); self.push(owner, BoundaryResourceKey::Runtime(membership.clone()));
            }
        }
    }

    fn formal(&mut self, tables: &EvaluatedValueTables, port: &BoundaryFormalPortRef, view: &SourceFormalValueRef) {
        for signature in tables.boundary_signatures.iter().filter(|signature| signature.owner == port.owner) {
            for (ordinal, input) in signature.ports.iter().enumerate().filter(|(ordinal, _)| *ordinal == port.port) {
                let _ = ordinal;
                self.boundary(&signature.owner, &input.role);
            }
        }
        let mut pending = vec![view.clone()];
        let mut visited = Vec::new();
        while let Some(id) = pending.pop() {
            if visited.contains(&id) { continue; }
            visited.push(id.clone());
            for row in tables.source_formal_values.iter().filter(|row| row.id == id) {
                match &row.owner {
                    SourceFormalOwnerRole::Parameter { binding, .. } | SourceFormalOwnerRole::Capture { binding, .. }
                    | SourceFormalOwnerRole::Global { binding, .. } => self.push(&row.id.owner, BoundaryResourceKey::Binding(binding.clone())),
                    SourceFormalOwnerRole::Manual { .. } => {},
                }
                pending.extend(row.members.iter().map(|edge| edge.value.clone()));
            }
        }
    }

    fn case(&mut self, tables: &EvaluatedValueTables, owner: &ProducerCfgOwnerId, case: &EvaluatedValueCase) {
        self.availability(owner, &case.availability);
        match &case.payload {
            EvaluatedPayloadFact::Plain | EvaluatedPayloadFact::FixedAggregate { .. } => {}
            EvaluatedPayloadFact::Formal { port, view } => self.formal(tables, port, view),
            EvaluatedPayloadFact::Checked { origins, state } => {
                for origin in origins { self.push(owner, BoundaryResourceKey::Origin(*origin)); }
                self.state(owner, state);
            }
            EvaluatedPayloadFact::Raw { provenance } => self.raw(owner, provenance),
            EvaluatedPayloadFact::Callable { producers } => {
                for holder in producers { self.push(owner, BoundaryResourceKey::Holder(holder.clone())); }
            }
            EvaluatedPayloadFact::RuntimeBacking { backing, elements } => {
                self.push(owner, BoundaryResourceKey::Backing(backing.clone()));
                self.push(owner, BoundaryResourceKey::Runtime(elements.backing.clone()));
            }
        }
        for dependency in &case.dependencies { self.dependency(owner, dependency); }
    }

    fn boundary(&mut self, owner: &ProducerCfgOwnerId, role: &BoundaryPortRole) {
        match role {
            BoundaryPortRole::Parameter { binding, .. } | BoundaryPortRole::Capture { binding, .. } | BoundaryPortRole::Global(binding) => self.push(owner, BoundaryResourceKey::Binding(binding.clone())),
            BoundaryPortRole::Effect { binding, projection } => {
                self.push(owner, BoundaryResourceKey::Binding(binding.clone()));
                for projection in projection {
                    if let ReferencePlaceProjection::Dereference(loan) = projection { self.push(owner, BoundaryResourceKey::Loan(*loan.clone())); }
                }
            }
            BoundaryPortRole::Output(_) => {}
        }
    }

    fn coordinate(&mut self, owner: &ProducerCfgOwnerId, role: &SourceCoordinateRole) {
        if let SourceCoordinateRole::Binding(binding) = role { self.push(owner, BoundaryResourceKey::Binding(binding.clone())); }
    }

    fn target(&mut self, owner: &ProducerCfgOwnerId, target: &ResolvedCallTarget) {
        match target {
            ResolvedCallTarget::LocalCallable(binding) => self.push(owner, BoundaryResourceKey::Binding(binding.clone())),
            ResolvedCallTarget::CallableField { receiver_place, .. } => self.push(owner, BoundaryResourceKey::Place(receiver_place.clone())),
            ResolvedCallTarget::Core(_) | ResolvedCallTarget::ModuleCallable { .. } | ResolvedCallTarget::ExternCallable { .. } => {}
        }
    }

    fn endpoint(&mut self, endpoint: &OwnerEndpointFact) {
        match &endpoint.kind {
            OwnerEndpointKind::ParameterExit { binding } | OwnerEndpointKind::ModuleTerminal { binding } => self.push(&endpoint.event.owner, BoundaryResourceKey::Binding(binding.clone())),
            OwnerEndpointKind::CheckedDiscard { consumer } | OwnerEndpointKind::ResultHandoff { consumer } => self.consumer(consumer),
            OwnerEndpointKind::ScopeExit { .. } => {}
        }
    }

    fn receiving(&mut self, receiving: &OwnerReceivingTransitionFact) {
        for selection in &receiving.dependencies { self.dependency(&receiving.event.owner, &selection.dependency); }
        for endpoint in &receiving.endpoints { self.endpoint(endpoint); }
    }

    fn disposition(&mut self, disposition: &ValueDisposition) {
        match &disposition.action {
            ValueDispositionAction::CopyTo(consumer) | ValueDispositionAction::MoveTo(consumer) | ValueDispositionAction::BorrowFor(consumer) => self.consumer(consumer),
            ValueDispositionAction::DiscardAt(_) => {}
        }
    }

    fn owner_consumer(&mut self, row: &EvaluatedOwnerConsumer) {
        self.consumer(&row.consumer);
        for selection in &row.dependencies { self.dependency(&row.consumer.producer, &selection.dependency); }
        self.disposition(&row.disposition);
    }

    fn common_target(&mut self, owner: &ProducerCfgOwnerId, target: &CommonLoanTarget) {
        if let CommonLoanTarget::Storage { binding, .. } = target { self.push(owner, BoundaryResourceKey::Binding(binding.clone())); }
    }

    fn common_pending(&mut self, row: &CommonLoanPending) {
        if let CommonLoanPendingReason::MissingBindingInput(binding) = &row.reason { self.push(&row.event.owner, BoundaryResourceKey::Binding(binding.clone())); }
    }

    fn common_fact(&mut self, row: &CommonLoanFact) {
        if let CommonLoanFactKind::Use { target, .. } = &row.kind { self.common_target(&row.event.owner, target); }
    }

    fn common_input(&mut self, input: &CommonLoanInput) {
        for row in &input.binding_shapes { self.push(&input.owner, BoundaryResourceKey::Binding(row.binding.clone())); }
        for row in &input.final_inputs { self.consumer(&row.consumer); }
        for role in &input.roles {
            match &role.source {
                CommonLoanSourceRole::Binding(binding) => self.push(&role.owner, BoundaryResourceKey::Binding(binding.clone())),
                CommonLoanSourceRole::Boundary(role_kind) => self.boundary(&role.owner, role_kind),
                CommonLoanSourceRole::Evaluated(_) | CommonLoanSourceRole::Frozen(_) | CommonLoanSourceRole::Saved(_) | CommonLoanSourceRole::Result(_) => {}
            }
        }
        for site in &input.sites { self.common_target(&site.event.owner, &site.target); }
        for event in &input.events { for fact in &event.facts { self.common_fact(fact); } }
        for pending in &input.pending { self.common_pending(pending); }
    }

    fn common(&mut self, tables: &EvaluatedValueTables) {
        for row in &tables.common_loans {
            self.common_input(&row.input);
            for candidate in &row.candidates { self.common_target(&candidate.event.owner, &candidate.target); }
            for pending in &row.pending { self.common_pending(pending); }
        }
    }

    fn uses(&mut self, tables: &EvaluatedValueTables) {
        self.common(tables);
        for row in &tables.owner_receiving_transitions { self.receiving(row); }
        for graph in &tables.source_use_graphs {
            for coverage in &graph.coverage {
                match &coverage.reason {
                    SourceUseCoverageReason::Unavailable { state, .. } => self.availability(&graph.id.owner, state),
                    SourceUseCoverageReason::CheckedState { state, .. } => self.state(&graph.id.owner, state),
                    SourceUseCoverageReason::Projection(place) => self.push(&graph.id.owner, BoundaryResourceKey::Place(place.clone())),
                    SourceUseCoverageReason::MissingInput(role) => self.coordinate(&graph.id.owner, role),
                    _ => {}
                }
            }
            for selected in &graph.uses {
                if let SourceUseInput::Storage(slot) = &selected.input { self.availability(&graph.id.owner, &slot.availability); }
            }
            for returned in &graph.returns { self.consumer(&returned.consumer); }
        }
    }

    fn local_pending(&mut self, owner: &ProducerCfgOwnerId, pending: &LocalLoanPendingReason) {
        match pending {
            LocalLoanPendingReason::Coverage(row) => self.common_pending(row),
            LocalLoanPendingReason::Unavailable { state, .. } => self.availability(owner, state),
            LocalLoanPendingReason::CheckedState { state, .. } => self.state(owner, state),
            _ => {}
        }
    }

    fn stages(&mut self, tables: &EvaluatedValueTables) {
        self.common(tables);
        for row in &tables.staged_loan_queries {
            for pending in &row.coverage { self.common_pending(pending); }
            if let StagedLoanQueryResult::Candidates { slices, queries, pending } = &row.result {
                for slice in slices {
                    for pending in &slice.coverage { self.common_pending(pending); }
                    for point in &slice.points { for fact in &point.event.facts { self.common_fact(fact); } }
                }
                for query in queries {
                    for fact in &query.point.event.facts { self.common_fact(fact); }
                    for pending in &query.pending { self.local_pending(&row.id.owner, pending); }
                }
                for pending in pending { self.local_pending(&row.id.owner, pending); }
            }
        }
    }

    fn destination(&mut self, row: &EvaluatedAssignmentDestination) {
        let owner = &row.id.operation.producer;
        match &row.storage {
            EvaluatedDestinationStorage::Direct(place) => self.push(owner, BoundaryResourceKey::Place(place.clone())),
            EvaluatedDestinationStorage::Inferred(binding) => self.push(owner, BoundaryResourceKey::Binding(binding.clone())),
            EvaluatedDestinationStorage::Through { cases, .. } => {
                for case in cases {
                    self.push(owner, BoundaryResourceKey::Place(case.place.clone()));
                    if let ReferenceBorrowAccess::Through { parent_origin, .. } = &case.access { self.push(owner, BoundaryResourceKey::Origin(*parent_origin)); }
                }
            }
        }
    }

    fn snapshot(&mut self, tables: &EvaluatedValueTables, row: &AssignmentSnapshot) {
        match &row.state {
            SnapshotState::ElidedDiscard { cases } | SnapshotState::Unavailable { cases } => {
                for case in cases { self.case(tables, &row.destination.operation.producer, case); }
            }
            SnapshotState::Saved(_) | SnapshotState::NoPriorDeclaration => {}
        }
    }

    fn previous(&mut self, owner: &ProducerCfgOwnerId, row: &AssignmentPreviousPlan) {
        if let PreviousOutputPlan::InferredDeclaration { binding } = &row.previous { self.push(owner, BoundaryResourceKey::Binding(binding.clone())); }
    }

    fn full(&mut self, owner: &ProducerCfgOwnerId, tables: &EvaluatedValueTables) {
        self.uses(tables); self.stages(tables);
        for row in &tables.source_origin_derivations {
            self.push(&row.origin().owner, BoundaryResourceKey::Origin(row.origin().origin));
        }
        for slot in &tables.source_slots { self.coordinate(owner, &slot.role); }
        for signature in &tables.boundary_signatures { for port in &signature.ports { self.boundary(&signature.owner, &port.role); } }
        for row in &tables.call_argument_inputs { self.target(&row.event.owner, &row.target); }
        for value in &tables.values { for case in &value.cases { self.case(tables, &value.id.resolved.producer, case); } }
        for disposition in &tables.dispositions { self.disposition(disposition); }
        for row in &tables.disposition_inputs { self.consumer(&row.consumer); }
        for row in &tables.final_inputs { self.consumer(&row.consumer); }
        for row in &tables.source_binding_shapes { self.push(owner, BoundaryResourceKey::Binding(row.binding.clone())); }
        for row in &tables.source_backing_producers { self.push(&row.event.owner, BoundaryResourceKey::Binding(row.target.clone())); }
        for row in &tables.source_backing_observations { self.push(&row.view.owner, BoundaryResourceKey::Backing(row.backing.clone())); }
        for row in &tables.raw_address_inputs { self.push(&row.value.resolved.producer, BoundaryResourceKey::Place(row.place.clone())); }
        for row in &tables.source_move_inputs { self.push(&row.event.owner, BoundaryResourceKey::Place(row.place.clone())); }
        for row in &tables.source_move_witnesses { self.push(&row.event.owner, BoundaryResourceKey::Place(row.place.clone())); }
        for row in &tables.pending_source_moves {
            match &row.reason {
                SourceMovePendingReason::UnsupportedPlace(place) => self.push(&row.event.owner, BoundaryResourceKey::Place(place.clone())),
                SourceMovePendingReason::SelectedSource(input) => self.push(&input.event.owner, BoundaryResourceKey::Place(input.place.clone())),
                SourceMovePendingReason::Deferred | SourceMovePendingReason::Through => {}
            }
        }
        for row in &tables.source_saved_ports {
            self.push(&row.snapshot.owner, BoundaryResourceKey::Place(row.source_place.clone()));
            for duty in &row.duties { self.endpoint(&duty.endpoint); }
        }
        for row in &tables.source_views {
            match &row.payload {
                SourcePayloadView::Raw { provenance, .. } => self.raw(&row.id.owner, provenance),
                SourcePayloadView::Checked { state, .. } => self.state(&row.id.owner, state),
                _ => {}
            }
            for dependency in &row.dependencies { self.dependency(&row.id.owner, dependency); }
        }
        for row in &tables.source_configurations {
            for slot in &row.slots { self.availability(&row.id.owner, &slot.availability); }
            for saved in &row.saved { self.push(&row.id.owner, BoundaryResourceKey::Binding(saved.holder.clone())); self.availability(&row.id.owner, &saved.availability); }
            for owns in &row.owns { self.endpoint(&owns.endpoint); }
        }
        for row in &tables.pending_owner_receiving {
            if let OwnerReceivingPendingReason::DependencyAction { dependency, .. } = &row.reason { self.dependency(&row.event.owner, dependency); }
        }
        for row in &tables.owner_consumers { self.owner_consumer(row); }
        for row in &tables.pending_owner_consumers { self.owner_consumer(&row.candidate); }
        for row in &tables.owner_capabilities {
            match &row.role {
                OwnerRole::Module(binding) | OwnerRole::Binding(binding) | OwnerRole::Parameter(binding) => self.push(&row.id.producer, BoundaryResourceKey::Binding(binding.clone())),
                OwnerRole::Result(consumer) | OwnerRole::Discard(consumer) => self.consumer(consumer),
                OwnerRole::Scope(_) => {}
            }
            for endpoint in &row.endpoints { self.endpoint(endpoint); }
        }
        for row in &tables.assignment_plans {
            for target in &row.targets {
                match &target.storage {
                    ResolvedAssignmentStorage::Existing(place) => self.push(&row.operation.producer, BoundaryResourceKey::Place(place.clone())),
                    ResolvedAssignmentStorage::InferredDeclaration(binding) => self.push(&row.operation.producer, BoundaryResourceKey::Binding(binding.clone())),
                    ResolvedAssignmentStorage::Through { .. } => {}
                }
            }
            for previous in &row.previous_outputs { self.previous(&row.operation.producer, previous); }
            if let AssignmentConsumption::Received { outputs, .. } = &row.consumption { for consumer in outputs { self.consumer(consumer); } }
        }
        for previous in &tables.assignment_previous { self.previous(owner, previous); }
        for row in &tables.destinations { self.destination(row); }
        for row in &tables.snapshots { self.snapshot(tables, row); }
        for row in &tables.commits { for disposition in &row.dispositions { self.disposition(disposition); } }
        for row in &tables.results { for consumer in &row.consumers { self.consumer(consumer); } }
        for row in &tables.assignment_evaluations {
            for destination in &row.destinations { self.destination(destination); }
            for previous in &row.previous { self.snapshot(tables, previous); }
            for write in &row.writes { for disposition in &write.dispositions { self.disposition(disposition); } }
            for result in &row.results { for consumer in &result.consumers { self.consumer(consumer); } }
        }
    }

    fn captures(&mut self, source: &[ValueTableSource]) {
        for carrier in source {
            let cfg = carrier.resource_construction.original();
            for production in &cfg.evaluated_values.source_capture_productions {
                let paths = cfg.capture_input_path_facts.iter().filter(|path| path.owner == production.event.owner && path.capture == production.event.point).cloned().collect();
                self.roots.extend(capture_tuple_roots(&BoundaryCaptureResourceTuple { production: production.clone(), paths }));
            }
            for slot in &carrier.tables.source_slots { self.coordinate(&carrier.producer, &slot.role); }
            for signature in &carrier.tables.boundary_signatures { for port in &signature.ports { self.boundary(&signature.owner, &port.role); } }
        }
    }
}

pub(in crate::scalar::evaluated_value::remap) fn resource_demand_full(source: &[ValueTableSource]) -> BoundaryResourceDemand {
    let mut visitor = DemandVisitor { roots: Vec::new() };
    for carrier in source { visitor.full(&carrier.producer, &carrier.tables); }
    visitor.captures(source);
    BoundaryResourceDemand { roots: visitor.roots }
}

pub(super) fn table_roots(owner: &ProducerCfgOwnerId, tables: &EvaluatedValueTables, cut: &BoundaryResourceDemandCut) -> Vec<BoundaryResourceRef> {
    let mut visitor = DemandVisitor { roots: Vec::new() };
    match cut {
        BoundaryResourceDemandCut::Full => visitor.full(owner, tables),
        BoundaryResourceDemandCut::SourceUses { .. } => visitor.uses(tables),
        BoundaryResourceDemandCut::StagedLoans { .. } => visitor.stages(tables),
    }
    visitor.roots
}

pub(in crate::scalar::evaluated_value::remap) fn resource_demand_uses(_root: &ProducerCfgOwnerId, tables: &EvaluatedValueTables, source: &[ValueTableSource]) -> BoundaryResourceDemand {
    let mut visitor = DemandVisitor { roots: Vec::new() };
    visitor.uses(tables);
    visitor.captures(source);
    BoundaryResourceDemand { roots: visitor.roots }
}

pub(in crate::scalar::evaluated_value::remap) fn resource_demand_stages(_root: &ProducerCfgOwnerId, tables: &EvaluatedValueTables, source: &[ValueTableSource]) -> BoundaryResourceDemand {
    let mut visitor = DemandVisitor { roots: Vec::new() };
    visitor.stages(tables);
    visitor.captures(source);
    BoundaryResourceDemand { roots: visitor.roots }
}
