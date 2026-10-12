use super::*;

struct ResourceChildren {
    owner: ProducerCfgOwnerId,
    references: Vec<BoundaryResourceRef>,
}

impl ResourceChildren {
    fn push(&mut self, key: BoundaryResourceKey) {
        self.references.push(BoundaryResourceRef { owner: self.owner.clone(), key });
    }

    fn projections(&mut self, projections: &[ReferencePlaceProjection]) {
        for projection in projections {
            match projection {
                ReferencePlaceProjection::Field(_) | ReferencePlaceProjection::Index(_) => {}
                ReferencePlaceProjection::Dereference(loan) => self.push(BoundaryResourceKey::Loan(*loan.clone())),
            }
        }
    }

    fn holder_place(&mut self, place: &CallableHolderPlace) {
        match place {
            CallableHolderPlace::Temporary(_) | CallableHolderPlace::Output(_) => {}
            CallableHolderPlace::Binding(binding) => self.push(BoundaryResourceKey::Binding(binding.clone())),
            CallableHolderPlace::Projection(place) => self.push(BoundaryResourceKey::Place(place.clone())),
            CallableHolderPlace::OutputProjection { projections, .. } => self.projections(projections),
        }
    }

    fn length(&mut self, length: &RuntimeArrayLengthFact) {
        match length {
            RuntimeArrayLengthFact::Fixed { .. } | RuntimeArrayLengthFact::Evaluated { .. } => {}
            RuntimeArrayLengthFact::Parameter { binding } => self.push(BoundaryResourceKey::Binding(binding.clone())),
        }
    }

    fn input(&mut self, input: &CallableCaptureInput) {
        self.push(BoundaryResourceKey::Binding(input.binding.clone()));
        for origin in &input.origins { self.push(BoundaryResourceKey::Origin(*origin)); }
        for environment in &input.environments { self.push(BoundaryResourceKey::Environment(environment.clone())); }
        for projected in &input.projected {
            self.push(BoundaryResourceKey::Place(projected.place.clone()));
            for environment in &projected.environments { self.push(BoundaryResourceKey::Environment(environment.clone())); }
        }
        for backing in &input.backings { self.push(BoundaryResourceKey::Backing(backing.clone())); }
        for projected in &input.projected_backings {
            self.push(BoundaryResourceKey::Place(projected.place.clone()));
            self.push(BoundaryResourceKey::Backing(projected.backing.clone()));
        }
    }

    fn paths(&mut self, paths: &[Vec<CallableCaptureInput>]) {
        for path in paths {
            for input in path { self.input(input); }
        }
    }

    fn index_key(&mut self, key: &CallableIndexKeyFact) {
        match key {
            CallableIndexKeyFact::Constant(_) | CallableIndexKeyFact::Evaluated { .. } => {}
            CallableIndexKeyFact::LivePlace(place) => self.push(BoundaryResourceKey::Place(place.clone())),
        }
    }

    fn equality(&mut self, equality: &CallableIndexEqualityFact) {
        self.index_key(&equality.left);
        self.index_key(&equality.right);
    }

    fn cell(&mut self, cell: &CallableCellCaseFact) {
        match cell {
            CallableCellCaseFact::Uninitialized => {}
            CallableCellCaseFact::Initialized { environment_origin, instance, holder_aliases, backing, owner, capture_paths, loans, .. } => {
                self.push(BoundaryResourceKey::Environment(environment_origin.clone()));
                match instance {
                    CallableEnvironmentInstanceClassFact::FreshPerCell | CallableEnvironmentInstanceClassFact::ImportedMayAlias => {}
                    CallableEnvironmentInstanceClassFact::SharedWith(holder) => self.holder_place(holder),
                }
                for holder in holder_aliases { self.holder_place(holder); }
                self.push(BoundaryResourceKey::Runtime(backing.clone()));
                self.push(BoundaryResourceKey::Binding(owner.clone()));
                self.paths(capture_paths);
                for loan in loans { self.push(BoundaryResourceKey::Origin(loan.origin)); }
            }
        }
    }

    fn domain(&mut self, domain: &CallableCellDomainFact) {
        for cell in &domain.alternatives { self.cell(cell); }
    }

    fn element(&mut self, element: &CallableArrayElementFact) {
        self.push(BoundaryResourceKey::Place(element.place.clone()));
        self.push(BoundaryResourceKey::Environment(element.environment.clone()));
        for holder in &element.aliases { self.holder_place(holder); }
        for loan in &element.loans { self.push(BoundaryResourceKey::Origin(loan.origin)); }
    }

    fn family(&mut self, family: &CallableArrayFamilyFact) {
        for element in &family.elements { self.element(element); }
        for partition in &family.partitions {
            for representative in &partition.representatives {
                self.index_key(&representative.key);
                for anchor in &representative.anchors {
                    match anchor {
                        CallableIndexAnchorFact::ValuePlace(place) => self.push(BoundaryResourceKey::Place(place.clone())),
                        CallableIndexAnchorFact::ActiveLoan { loan, backing, owner, length, .. } => {
                            self.push(BoundaryResourceKey::Loan(loan.clone()));
                            self.push(BoundaryResourceKey::Runtime(backing.clone()));
                            self.push(BoundaryResourceKey::Binding(owner.clone()));
                            self.length(length);
                        }
                        CallableIndexAnchorFact::EscapedHolder(holder) => self.holder_place(holder),
                        CallableIndexAnchorFact::InFlightSelection(_) => {}
                    }
                }
                self.domain(&representative.cell);
            }
            for equality in &partition.relations { self.equality(equality); }
            self.domain(&partition.remainder);
        }
    }

    fn selected_holder(&mut self, holder: &CallableArraySelectedHolderFact) {
        self.push(BoundaryResourceKey::Runtime(holder.value.clone()));
        self.push(BoundaryResourceKey::Binding(holder.owner.clone()));
        self.length(&holder.length);
        self.push(BoundaryResourceKey::Place(holder.place.clone()));
    }

    fn action(&mut self, action: &CallableHolderAction) {
        match action {
            CallableHolderAction::SelectElement { target, selected, .. } => {
                self.push(BoundaryResourceKey::Place(target.clone()));
                self.push(BoundaryResourceKey::Place(selected.clone()));
            }
            CallableHolderAction::DropSelected { holder, previous, old, guard, .. } => {
                self.selected_holder(holder);
                for element in previous { self.element(element); }
                self.cell(old);
                for equality in guard { self.equality(equality); }
            }
            CallableHolderAction::SkipUninitializedSelected { holder, guard, .. } => {
                self.selected_holder(holder);
                for equality in guard { self.equality(equality); }
            }
            CallableHolderAction::RetainCheckedSelected { holder, old, guard, to, .. } => {
                self.selected_holder(holder);
                self.cell(old);
                for equality in guard { self.equality(equality); }
                self.holder_place(to);
            }
            CallableHolderAction::RetainSelected { environment, from, to, .. }
            | CallableHolderAction::MoveSelected { environment, from, to, .. } => {
                self.push(BoundaryResourceKey::Environment(environment.clone()));
                self.holder_place(from);
                self.selected_holder(to);
            }
            CallableHolderAction::Retain { environment, from, to, .. }
            | CallableHolderAction::Move { environment, from, to, .. }
            | CallableHolderAction::ReturnTransfer { environment, from, to, .. } => {
                self.push(BoundaryResourceKey::Environment(environment.clone()));
                self.holder_place(from);
                self.holder_place(to);
            }
            CallableHolderAction::Drop { environment, holder, .. } => {
                self.push(BoundaryResourceKey::Environment(environment.clone()));
                self.holder_place(holder);
            }
            CallableHolderAction::DropArrayElements { backing, .. } => self.push(BoundaryResourceKey::Backing(backing.clone())),
            CallableHolderAction::DropCapturedValueCellOwner { binding, .. } => self.push(BoundaryResourceKey::Binding(binding.clone())),
            CallableHolderAction::DropCaptureCellOwner { binding, backing, .. } => {
                self.push(BoundaryResourceKey::Binding(binding.clone()));
                self.push(BoundaryResourceKey::Backing(backing.clone()));
            }
            CallableHolderAction::ReplaceCaptureCellValue { binding, previous, incoming, .. } => {
                self.push(BoundaryResourceKey::Binding(binding.clone()));
                self.push(BoundaryResourceKey::Backing(previous.clone()));
                self.push(BoundaryResourceKey::Backing(incoming.clone()));
            }
            CallableHolderAction::RetainArrayElements { source, backing, .. } => {
                self.push(BoundaryResourceKey::Backing(source.clone()));
                self.push(BoundaryResourceKey::Backing(backing.clone()));
            }
        }
    }

    fn producer(&mut self, producer: &HolderProducerRef) {
        match producer {
            HolderProducerRef::CaptureCreation { environment, .. } => self.push(BoundaryResourceKey::Environment(environment.clone())),
            HolderProducerRef::CallCreation { environment, selection, .. } => {
                self.push(BoundaryResourceKey::Environment(environment.clone()));
                self.push(BoundaryResourceKey::Environment(selection.destination.clone()));
                self.projections(&selection.projection);
                match &selection.target {
                    ResolvedCallTarget::LocalCallable(binding) => self.push(BoundaryResourceKey::Binding(binding.clone())),
                    ResolvedCallTarget::CallableField { receiver_place, .. } => self.push(BoundaryResourceKey::Place(receiver_place.clone())),
                    ResolvedCallTarget::Core(_) | ResolvedCallTarget::ModuleCallable { .. } | ResolvedCallTarget::ExternCallable { .. } => {}
                }
            }
            HolderProducerRef::InstalledRead { environment, action, .. } => {
                self.push(BoundaryResourceKey::Environment(environment.clone()));
                self.action(action);
            }
            HolderProducerRef::GlobalCallsiteInput { holder, environment, .. } => {
                self.holder_place(holder);
                self.push(BoundaryResourceKey::Environment(environment.clone()));
            }
            HolderProducerRef::Forward { environment, action, .. } => {
                self.push(BoundaryResourceKey::Environment(environment.clone()));
                self.action(action);
            }
        }
    }

    fn value(&mut self, value: &BoundaryResourceValue) {
        match value {
            BoundaryResourceValue::Origin { fact, .. } => match fact {
                ReferenceOrigin::Fresh { loan, .. } | ReferenceOrigin::BorrowedFrom { loan, .. } => self.push(BoundaryResourceKey::Loan(loan.clone())),
                ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => {}
            },
            BoundaryResourceValue::Binding(binding) => match &binding.kind {
                ReferenceBindingKind::ImportedCapture { producer, .. } => self.push(BoundaryResourceKey::Binding(*producer.clone())),
                ReferenceBindingKind::Declared | ReferenceBindingKind::BorrowedReferent { .. }
                | ReferenceBindingKind::CallOutput { .. } | ReferenceBindingKind::AssignmentPrevious { .. }
                | ReferenceBindingKind::Captured { .. } | ReferenceBindingKind::Projected { .. } => {}
            },
            BoundaryResourceValue::Place(place) => {
                self.push(BoundaryResourceKey::Binding(place.binding.clone()));
                self.projections(&place.projections);
            }
            BoundaryResourceValue::Loan(loan) => {
                self.push(BoundaryResourceKey::Place(loan.origin_place.clone()));
                if let Some(parent) = &loan.parent { self.push(BoundaryResourceKey::Loan(*parent.clone())); }
            }
            BoundaryResourceValue::Runtime(runtime) => match &runtime.position {
                RuntimeValuePosition::CapturedCall { lineage, .. } | RuntimeValuePosition::CallOutput { lineage, .. } => self.push(BoundaryResourceKey::Runtime(*lineage.clone())),
                RuntimeValuePosition::Local | RuntimeValuePosition::Parameter { .. } | RuntimeValuePosition::Argument { .. } => {}
            },
            BoundaryResourceValue::Environment { fact, .. } => {
                for origin in &fact.origins { self.push(BoundaryResourceKey::Origin(*origin)); }
                for owner in &fact.owners { self.push(BoundaryResourceKey::Binding(owner.clone())); }
                for loan in &fact.loans { self.push(BoundaryResourceKey::Origin(loan.origin)); }
                for backing in &fact.backings { self.push(BoundaryResourceKey::Backing(backing.clone())); }
                for input in &fact.inputs { self.input(input); }
                self.paths(&fact.capture_paths);
            }
            BoundaryResourceValue::Backing(backing) => {
                self.push(BoundaryResourceKey::Runtime(backing.value.clone()));
                self.push(BoundaryResourceKey::Binding(backing.owner.clone()));
                self.length(&backing.length);
                for family in &backing.callable_family { self.family(family); }
            }
            BoundaryResourceValue::Holder(holder) => {
                self.holder_place(&holder.holder);
                self.push(BoundaryResourceKey::Environment(holder.environment.clone()));
            }
        }
    }
}

pub(super) fn references(owner: &ProducerCfgOwnerId, value: &BoundaryResourceValue) -> Vec<BoundaryResourceRef> {
    let mut visitor = ResourceChildren { owner: owner.clone(), references: Vec::new() };
    visitor.value(value);
    visitor.references
}

pub(super) fn producer_references(owner: &ProducerCfgOwnerId, producer: &HolderProducerRef) -> Vec<BoundaryResourceRef> {
    let mut visitor = ResourceChildren { owner: owner.clone(), references: Vec::new() };
    visitor.producer(producer);
    visitor.references
}

pub(super) fn action_references(owner: &ProducerCfgOwnerId, action: &CallableHolderAction) -> Vec<BoundaryResourceRef> {
    let mut visitor = ResourceChildren { owner: owner.clone(), references: Vec::new() };
    visitor.action(action);
    visitor.references
}

pub(super) fn family_references(owner: &ProducerCfgOwnerId, family: &CallableArrayFamilyFact) -> Vec<BoundaryResourceRef> {
    let mut visitor = ResourceChildren { owner: owner.clone(), references: Vec::new() };
    visitor.family(family);
    visitor.references
}

pub(super) fn holder_place_references(owner: &ProducerCfgOwnerId, holder: &CallableHolderPlace) -> Vec<BoundaryResourceRef> {
    let mut visitor = ResourceChildren { owner: owner.clone(), references: Vec::new() };
    visitor.holder_place(holder);
    visitor.references
}
