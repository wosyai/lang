use super::*;

pub(super) struct CopyResourceConstruction<'a> {
    remap: &'a ValueSchemaRemap,
    owner: &'a ProducerCfgOwnerId,
    source: &'a SourceIdentity,
    destination: SourceIdentity,
    forest: &'a SourceCaptureConstructionForest,
    mode: ResourceReferenceConstruction,
}

enum ResourceReferenceConstruction { Copied, Paired }

pub(super) fn lexical(owner: &ProducerCfgOwnerId) -> &SourceIdentity {
    match owner {
        ProducerCfgOwnerId::Module(source) | ProducerCfgOwnerId::Function { source, .. } => source,
        ProducerCfgOwnerId::Literal { enclosing, .. } => lexical(enclosing),
    }
}

impl<'a> CopyResourceConstruction<'a> {
    pub(super) fn new(remap: &'a ValueSchemaRemap, owner: &'a ProducerCfgOwnerId, destination: &ProducerCfgOwnerId, forest: &'a SourceCaptureConstructionForest) -> Self {
        Self { remap, owner, source: lexical(owner), destination: lexical(destination).clone(), forest, mode: ResourceReferenceConstruction::Copied }
    }

    pub(super) fn paired(remap: &'a ValueSchemaRemap, owner: &'a ProducerCfgOwnerId, destination: &ProducerCfgOwnerId, forest: &'a SourceCaptureConstructionForest) -> Self {
        Self { remap, owner, source: lexical(owner), destination: lexical(destination).clone(), forest, mode: ResourceReferenceConstruction::Paired }
    }

    fn source_identity(&self, source: &SourceIdentity) -> SourceIdentity {
        if source == self.source { self.destination.clone() } else { source.clone() }
    }

    fn canonical_source(&self, source: &SourceIdentity) -> SourceIdentity {
        match self.mode { ResourceReferenceConstruction::Copied => source.clone(), ResourceReferenceConstruction::Paired => self.source_identity(source) }
    }

    fn canonical_span(&self, span: &SourceSpan) -> SourceSpan {
        SourceSpan { source: self.canonical_source(&span.source), range: span.range }
    }

    fn instance(&self, instance: &CallableEnvironmentInstance) -> CallableEnvironmentInstance {
        let mut mapped = instance.clone();
        if let CallableEnvironmentInstance::CallOutput { call, .. } = &mut mapped { *call = self.canonical_span(call); }
        mapped
    }

    fn binding(&self, binding: &ReferenceBindingId) -> Result<ReferenceBindingId, ValueSchemaRemapError> {
        if !self.remap.producers.iter().any(|row| row.0 == *self.owner) { return Ok(binding.clone()); }
        match self.mode {
            ResourceReferenceConstruction::Copied => self.binding_constructor(binding),
            ResourceReferenceConstruction::Paired => self.remap.binding(self.owner, binding),
        }
    }

    fn binding_constructor(&self, binding: &ReferenceBindingId) -> Result<ReferenceBindingId, ValueSchemaRemapError> {
        let mut mapped = binding.clone();
        mapped.source = self.source_identity(&binding.source);
        mapped.kind = match &binding.kind {
            ReferenceBindingKind::CallOutput { point, output } => ReferenceBindingKind::CallOutput { point: self.point(*point)?, output: *output },
            ReferenceBindingKind::AssignmentPrevious { snapshot, output } => ReferenceBindingKind::AssignmentPrevious { snapshot: self.point(*snapshot)?, output: *output },
            ReferenceBindingKind::ImportedCapture { call_start, call_end, output, producer } => ReferenceBindingKind::ImportedCapture {
                call_start: *call_start, call_end: *call_end, output: *output, producer: Box::new(self.binding(producer)?),
            },
            ReferenceBindingKind::Declared | ReferenceBindingKind::BorrowedReferent { .. }
            | ReferenceBindingKind::Captured { .. } | ReferenceBindingKind::Projected { .. } => binding.kind.clone(),
        };
        Ok(mapped)
    }

    fn point(&self, point: CfgPointId) -> Result<CfgPointId, ValueSchemaRemapError> {
        if !self.remap.producers.iter().any(|row| row.0 == *self.owner) { return Ok(point); }
        Ok(self.remap.boundary_point(&SourceEventId { owner: self.owner.clone(), point })?.point)
    }

    fn projections(&self, projections: &[ReferencePlaceProjection]) -> Result<Vec<ReferencePlaceProjection>, ValueSchemaRemapError> {
        projections.iter().map(|projection| Ok(match projection {
            ReferencePlaceProjection::Field(_) | ReferencePlaceProjection::Index(_) => projection.clone(),
            ReferencePlaceProjection::Dereference(loan) => ReferencePlaceProjection::Dereference(Box::new(self.loan(loan)?)),
        })).collect()
    }

    fn place(&self, place: &ReferencePlaceId) -> Result<ReferencePlaceId, ValueSchemaRemapError> {
        if !self.remap.producers.iter().any(|row| row.0 == *self.owner) { return Ok(place.clone()); }
        match self.mode {
            ResourceReferenceConstruction::Copied => self.place_constructor(place),
            ResourceReferenceConstruction::Paired => self.remap.place(self.owner, place),
        }
    }

    fn place_constructor(&self, place: &ReferencePlaceId) -> Result<ReferencePlaceId, ValueSchemaRemapError> {
        let mut mapped = place.clone();
        mapped.source = self.source_identity(&place.source);
        mapped.binding = self.binding(&place.binding)?;
        mapped.projections = self.projections(&place.projections)?;
        Ok(mapped)
    }

    fn loan(&self, loan: &ReferenceLoanId) -> Result<ReferenceLoanId, ValueSchemaRemapError> {
        if !self.remap.producers.iter().any(|row| row.0 == *self.owner) { return Ok(loan.clone()); }
        match self.mode {
            ResourceReferenceConstruction::Copied => self.loan_constructor(loan),
            ResourceReferenceConstruction::Paired => self.remap.checked_loan(self.owner, loan),
        }
    }

    fn loan_constructor(&self, loan: &ReferenceLoanId) -> Result<ReferenceLoanId, ValueSchemaRemapError> {
        Ok(ReferenceLoanId {
            mode: loan.mode, origin_place: self.place(&loan.origin_place)?, creation_source: self.canonical_source(&loan.creation_source),
            creation_span: loan.creation_span, parent: loan.parent.as_ref().map(|parent| self.loan(parent).map(Box::new)).transpose()?,
        })
    }

    fn runtime(&self, runtime: &RuntimeValueId) -> Result<RuntimeValueId, ValueSchemaRemapError> {
        let owner = creators::runtime_owner(self.owner, runtime, self.forest)?;
        if owner != self.owner {
            let destination = allocation::qualified_owner(self.remap, owner)?;
            let construction = match self.mode {
                ResourceReferenceConstruction::Copied => Self::new(self.remap, owner, &destination, self.forest),
                ResourceReferenceConstruction::Paired => Self::paired(self.remap, owner, &destination, self.forest),
            };
            return construction.runtime(runtime);
        }
        if !self.remap.producers.iter().any(|row| row.0 == *owner) { return Ok(runtime.clone()); }
        match self.mode {
            ResourceReferenceConstruction::Copied => self.runtime_constructor(runtime),
            ResourceReferenceConstruction::Paired => self.remap.runtime(self.owner, runtime),
        }
    }

    fn runtime_constructor(&self, runtime: &RuntimeValueId) -> Result<RuntimeValueId, ValueSchemaRemapError> {
        let mut mapped = runtime.clone();
        mapped.source = self.source_identity(&runtime.source);
        mapped.position = match &runtime.position {
            RuntimeValuePosition::CapturedCall { call, output, lineage } => RuntimeValuePosition::CapturedCall {
                call: self.canonical_span(call), output: *output, lineage: Box::new(self.runtime(lineage)?),
            },
            RuntimeValuePosition::CallOutput { call, output, target_source, target_declaration, origin, action, lineage } => RuntimeValuePosition::CallOutput {
                call: *call, output: *output, target_source: self.runtime_target_source(runtime, target_source, *target_declaration)?, target_declaration: *target_declaration,
                origin: self.canonical_span(origin), action: action.clone(), lineage: Box::new(self.runtime(lineage)?),
            },
            RuntimeValuePosition::Local | RuntimeValuePosition::Parameter { .. } | RuntimeValuePosition::Argument { .. } => runtime.position.clone(),
        };
        Ok(mapped)
    }

    fn runtime_target_source(&self, runtime: &RuntimeValueId, source: &SourceIdentity, declaration: ByteSpan) -> Result<SourceIdentity, ValueSchemaRemapError> {
        if matches!(self.mode, ResourceReferenceConstruction::Copied) { return Ok(source.clone()); }
        let mut destination = source.clone();
        let mut matched = false;
        for (original, target) in &self.remap.producers {
            if !matches!(original, ProducerCfgOwnerId::Function { source: known, declaration: span, .. } if known == source && *span == declaration) { continue; }
            if self.forest.sources.iter().filter(|row| row.producer == *original).count() != 1 { return Err(ValueSchemaRemapError::MissingRuntime(runtime.clone())); }
            let mapped = lexical(target);
            if matched && destination != *mapped { return Err(ValueSchemaRemapError::MissingRuntime(runtime.clone())); }
            destination = mapped.clone(); matched = true;
        }
        Ok(destination)
    }

    fn environment(&self, environment: &CallableEnvironmentId) -> Result<CallableEnvironmentId, ValueSchemaRemapError> {
        if !self.remap.producers.iter().any(|row| row.0 == *self.owner) { return Ok(environment.clone()); }
        match self.mode {
            ResourceReferenceConstruction::Copied => Ok(environment.clone()),
            ResourceReferenceConstruction::Paired => self.remap.checked_environment(self.owner, environment),
        }
    }

    fn origin(&self, origin: ReferenceOriginId) -> Result<ReferenceOriginId, ValueSchemaRemapError> {
        if !self.remap.producers.iter().any(|row| row.0 == *self.owner) { return Ok(origin); }
        match self.mode {
            ResourceReferenceConstruction::Copied => Ok(origin),
            ResourceReferenceConstruction::Paired => self.remap.origin(self.owner, origin),
        }
    }

    fn holder_place(&self, holder: &CallableHolderPlace) -> Result<CallableHolderPlace, ValueSchemaRemapError> {
        Ok(match holder {
            CallableHolderPlace::Temporary(point) => CallableHolderPlace::Temporary(self.point(*point)?),
            CallableHolderPlace::Binding(binding) => CallableHolderPlace::Binding(self.binding(binding)?),
            CallableHolderPlace::Projection(place) => CallableHolderPlace::Projection(self.place(place)?),
            CallableHolderPlace::Output(output) => CallableHolderPlace::Output(*output),
            CallableHolderPlace::OutputProjection { output, projections } => CallableHolderPlace::OutputProjection { output: *output, projections: self.projections(projections)? },
        })
    }

    fn length(&self, length: &RuntimeArrayLengthFact) -> Result<RuntimeArrayLengthFact, ValueSchemaRemapError> {
        Ok(match length {
            RuntimeArrayLengthFact::Fixed { value, origin } => RuntimeArrayLengthFact::Fixed { value: *value, origin: self.canonical_span(origin) },
            RuntimeArrayLengthFact::Evaluated { expression, occurrence, point, function_span } => RuntimeArrayLengthFact::Evaluated {
                expression: expression.clone(), occurrence: self.canonical_span(occurrence), point: self.point(*point)?, function_span: *function_span,
            },
            RuntimeArrayLengthFact::Parameter { binding } => RuntimeArrayLengthFact::Parameter { binding: self.binding(binding)? },
        })
    }

    fn input(&self, input: &CallableCaptureInput) -> Result<CallableCaptureInput, ValueSchemaRemapError> {
        Ok(CallableCaptureInput {
            binding: self.binding(&input.binding)?, origins: input.origins.iter().map(|origin| self.origin(*origin)).collect::<Result<BTreeSet<_>, _>>()?,
            environments: input.environments.iter().map(|id| self.environment(id)).collect::<Result<BTreeSet<_>, _>>()?,
            projected: input.projected.iter().map(|fact| Ok(CallableProjectedEnvironment {
                place: self.place(&fact.place)?, environments: fact.environments.iter().map(|id| self.environment(id)).collect::<Result<BTreeSet<_>, _>>()?,
            })).collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
            backings: input.backings.iter().map(|fact| self.backing(fact)).collect::<Result<Vec<_>, _>>()?,
            projected_backings: input.projected_backings.iter().map(|fact| Ok(RuntimeProjectedBackingFact {
                place: self.place(&fact.place)?, backing: self.backing(&fact.backing)?,
            })).collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
        })
    }

    fn paths(&self, paths: &[Vec<CallableCaptureInput>]) -> Result<Vec<Vec<CallableCaptureInput>>, ValueSchemaRemapError> {
        paths.iter().map(|path| path.iter().map(|input| self.input(input)).collect()).collect()
    }

    fn index(&self, index: &CallableArrayIndexFact) -> Result<CallableArrayIndexFact, ValueSchemaRemapError> {
        Ok(match index {
            CallableArrayIndexFact::Constant(_) => index.clone(),
            CallableArrayIndexFact::Selected { point, occurrence, function_span } => CallableArrayIndexFact::Selected { point: self.point(*point)?, occurrence: self.canonical_span(occurrence), function_span: *function_span },
        })
    }

    fn key(&self, key: &CallableIndexKeyFact) -> Result<CallableIndexKeyFact, ValueSchemaRemapError> {
        Ok(match key {
            CallableIndexKeyFact::Constant(_) => key.clone(),
            CallableIndexKeyFact::LivePlace(place) => CallableIndexKeyFact::LivePlace(self.place(place)?),
            CallableIndexKeyFact::Evaluated { point, occurrence, function_span } => CallableIndexKeyFact::Evaluated { point: self.point(*point)?, occurrence: self.canonical_span(occurrence), function_span: *function_span },
        })
    }

    fn equality(&self, fact: &CallableIndexEqualityFact) -> Result<CallableIndexEqualityFact, ValueSchemaRemapError> {
        let mut mapped = fact.clone();
        mapped.left = self.key(&fact.left)?;
        mapped.right = self.key(&fact.right)?;
        Ok(mapped)
    }

    fn cell(&self, cell: &CallableCellCaseFact) -> Result<CallableCellCaseFact, ValueSchemaRemapError> {
        let mut mapped = cell.clone();
        match &mut mapped {
            CallableCellCaseFact::Uninitialized => {}
            CallableCellCaseFact::Initialized { environment_origin, instance, holder_aliases, backing, owner, capture_paths, loans, origin, .. } => {
                *environment_origin = self.environment(environment_origin)?;
                *origin = self.canonical_span(origin);
                if let CallableEnvironmentInstanceClassFact::SharedWith(holder) = instance { *holder = self.holder_place(holder)?; }
                *holder_aliases = holder_aliases.iter().map(|holder| self.holder_place(holder)).collect::<Result<Vec<_>, _>>()?;
                *backing = self.runtime(backing)?;
                *owner = self.binding(owner)?;
                *capture_paths = self.paths(capture_paths)?;
                *loans = loans.iter().map(|loan| Ok(CallableLoanId { origin: self.origin(loan.origin)? })).collect::<Result<BTreeSet<_>, ValueSchemaRemapError>>()?;
            }
        }
        Ok(mapped)
    }

    fn domain(&self, domain: &CallableCellDomainFact) -> Result<CallableCellDomainFact, ValueSchemaRemapError> {
        let mut mapped = domain.clone();
        mapped.alternatives = domain.alternatives.iter().map(|cell| self.cell(cell)).collect::<Result<Vec<_>, _>>()?;
        Ok(mapped)
    }

    fn element(&self, fact: &CallableArrayElementFact) -> Result<CallableArrayElementFact, ValueSchemaRemapError> {
        let mut mapped = fact.clone();
        mapped.place = self.place(&fact.place)?;
        mapped.index = self.index(&fact.index)?;
        mapped.environment = self.environment(&fact.environment)?;
        mapped.aliases = fact.aliases.iter().map(|holder| self.holder_place(holder)).collect::<Result<Vec<_>, _>>()?;
        mapped.loans = fact.loans.iter().map(|loan| Ok(CallableLoanId { origin: self.origin(loan.origin)? })).collect::<Result<BTreeSet<_>, ValueSchemaRemapError>>()?;
        Ok(mapped)
    }

    fn family(&self, family: &CallableArrayFamilyFact) -> Result<CallableArrayFamilyFact, ValueSchemaRemapError> {
        let mut mapped = family.clone();
        mapped.elements = family.elements.iter().map(|fact| self.element(fact)).collect::<Result<Vec<_>, _>>()?;
        for relation in &mut mapped.index_relations {
            relation.left = self.index(&relation.left)?;
            relation.right = self.index(&relation.right)?;
        }
        mapped.unselected.selected = family.unselected.selected.iter().map(|index| self.index(index)).collect::<Result<Vec<_>, _>>()?;
        for partition in &mut mapped.partitions {
            partition.relations = partition.relations.iter().map(|fact| self.equality(fact)).collect::<Result<Vec<_>, _>>()?;
            partition.remainder = self.domain(&partition.remainder)?;
            for representative in &mut partition.representatives {
                representative.key = self.key(&representative.key)?;
                representative.cell = self.domain(&representative.cell)?;
                for anchor in &mut representative.anchors {
                    match anchor {
                        CallableIndexAnchorFact::ValuePlace(place) => *place = self.place(place)?,
                        CallableIndexAnchorFact::ActiveLoan { loan, backing, owner, length, origin } => {
                            *loan = self.loan(loan)?; *backing = self.runtime(backing)?;
                            *owner = self.binding(owner)?; *length = self.length(length)?;
                            *origin = self.canonical_span(origin);
                        }
                        CallableIndexAnchorFact::EscapedHolder(holder) => *holder = self.holder_place(holder)?,
                        CallableIndexAnchorFact::InFlightSelection(point) => *point = self.point(*point)?,
                    }
                }
            }
        }
        Ok(mapped)
    }

    fn backing(&self, backing: &RuntimeBackingFact) -> Result<RuntimeBackingFact, ValueSchemaRemapError> {
        if !self.remap.producers.iter().any(|row| row.0 == *self.owner) { return Ok(backing.clone()); }
        match self.mode {
            ResourceReferenceConstruction::Copied => self.backing_constructor(backing),
            ResourceReferenceConstruction::Paired => self.remap.checked_backing(self.owner, backing),
        }
    }

    fn backing_constructor(&self, backing: &RuntimeBackingFact) -> Result<RuntimeBackingFact, ValueSchemaRemapError> {
        let mut mapped = backing.clone();
        mapped.value = self.runtime(&backing.value)?;
        mapped.origin = self.canonical_span(&backing.origin);
        mapped.owner = self.binding(&backing.owner)?;
        mapped.length = self.length(&backing.length)?;
        mapped.callable_family = backing.callable_family.iter().map(|family| self.family(family)).collect::<Result<Vec<_>, _>>()?;
        Ok(mapped)
    }

    fn selected(&self, holder: &CallableArraySelectedHolderFact) -> Result<CallableArraySelectedHolderFact, ValueSchemaRemapError> {
        Ok(CallableArraySelectedHolderFact {
            value: self.runtime(&holder.value)?, owner: self.binding(&holder.owner)?, length: self.length(&holder.length)?,
            index_point: self.point(holder.index_point)?, occurrence: self.canonical_span(&holder.occurrence), place: self.place(&holder.place)?,
        })
    }

    fn action(&self, action: &CallableHolderAction) -> Result<CallableHolderAction, ValueSchemaRemapError> {
        let mut mapped = action.clone();
        match &mut mapped {
            CallableHolderAction::SelectElement { .. } => {}
            CallableHolderAction::DropSelected { span, .. } | CallableHolderAction::SkipUninitializedSelected { span, .. }
            | CallableHolderAction::RetainCheckedSelected { span, .. } | CallableHolderAction::RetainSelected { span, .. }
            | CallableHolderAction::MoveSelected { span, .. } | CallableHolderAction::Retain { span, .. }
            | CallableHolderAction::Move { span, .. } | CallableHolderAction::Drop { span, .. }
            | CallableHolderAction::DropArrayElements { span, .. } | CallableHolderAction::RetainArrayElements { span, .. }
            | CallableHolderAction::ReturnTransfer { span, .. } => *span = self.canonical_span(span),
            CallableHolderAction::DropCapturedValueCellOwner { cell, span, .. } | CallableHolderAction::DropCaptureCellOwner { cell, span, .. }
            | CallableHolderAction::ReplaceCaptureCellValue { cell, span, .. } => { *cell = self.canonical_span(cell); *span = self.canonical_span(span); }
        }
        match &mut mapped {
            CallableHolderAction::SelectElement { target, index_point, selected, .. } => { *target = self.place(target)?; *index_point = self.point(*index_point)?; *selected = self.place(selected)?; }
            CallableHolderAction::DropSelected { holder, previous, old, guard, .. } => {
                *holder = self.selected(holder)?; *previous = previous.iter().map(|fact| self.element(fact)).collect::<Result<Vec<_>, _>>()?;
                *old = self.cell(old)?; *guard = guard.iter().map(|fact| self.equality(fact)).collect::<Result<Vec<_>, _>>()?;
            }
            CallableHolderAction::SkipUninitializedSelected { holder, guard, .. } => {
                *holder = self.selected(holder)?; *guard = guard.iter().map(|fact| self.equality(fact)).collect::<Result<Vec<_>, _>>()?;
            }
            CallableHolderAction::RetainCheckedSelected { holder, old, guard, to, .. } => {
                *holder = self.selected(holder)?; *old = self.cell(old)?;
                *guard = guard.iter().map(|fact| self.equality(fact)).collect::<Result<Vec<_>, _>>()?; *to = self.holder_place(to)?;
            }
            CallableHolderAction::RetainSelected { environment, from, to, .. } | CallableHolderAction::MoveSelected { environment, from, to, .. } => {
                *environment = self.environment(environment)?; *from = self.holder_place(from)?; *to = self.selected(to)?;
            }
            CallableHolderAction::Retain { environment, from, to, .. } | CallableHolderAction::Move { environment, from, to, .. }
            | CallableHolderAction::ReturnTransfer { environment, from, to, .. } => {
                *environment = self.environment(environment)?; *from = self.holder_place(from)?; *to = self.holder_place(to)?;
            }
            CallableHolderAction::Drop { environment, holder, .. } => { *environment = self.environment(environment)?; *holder = self.holder_place(holder)?; }
            CallableHolderAction::DropArrayElements { backing, .. } => *backing = self.backing(backing)?,
            CallableHolderAction::DropCapturedValueCellOwner { binding, .. } => *binding = self.binding(binding)?,
            CallableHolderAction::DropCaptureCellOwner { binding, backing, .. } => { *binding = self.binding(binding)?; *backing = self.backing(backing)?; }
            CallableHolderAction::ReplaceCaptureCellValue { binding, previous, incoming, .. } => { *binding = self.binding(binding)?; *previous = self.backing(previous)?; *incoming = self.backing(incoming)?; }
            CallableHolderAction::RetainArrayElements { source, backing, .. } => { *source = self.backing(source)?; *backing = self.backing(backing)?; }
        }
        Ok(mapped)
    }

    fn target(&self, target: &ResolvedCallTarget) -> Result<ResolvedCallTarget, ValueSchemaRemapError> {
        Ok(match target {
            ResolvedCallTarget::LocalCallable(binding) => ResolvedCallTarget::LocalCallable(self.binding(binding)?),
            ResolvedCallTarget::CallableField { receiver_place, field } => ResolvedCallTarget::CallableField { receiver_place: self.place(receiver_place)?, field: field.clone() },
            ResolvedCallTarget::Core(_) | ResolvedCallTarget::ModuleCallable { .. } | ResolvedCallTarget::ExternCallable { .. } => target.clone(),
        })
    }

    fn producer(&self, producer: &HolderProducerRef) -> Result<HolderProducerRef, ValueSchemaRemapError> {
        let actual = creators::constructor(self.owner, producer, self.forest)?;
        let destination = allocation::qualified_owner(self.remap, &actual.producer)?;
        let construction = match self.mode {
            ResourceReferenceConstruction::Copied => Self::new(self.remap, &actual.producer, &destination, self.forest),
            ResourceReferenceConstruction::Paired => Self::paired(self.remap, &actual.producer, &destination, self.forest),
        };
        construction.producer_local(producer)
    }

    fn producer_local(&self, producer: &HolderProducerRef) -> Result<HolderProducerRef, ValueSchemaRemapError> {
        Ok(match producer {
            HolderProducerRef::CaptureCreation { point, environment } => HolderProducerRef::CaptureCreation { point: self.point(*point)?, environment: self.environment(environment)? },
            HolderProducerRef::CallCreation { point, environment, selection } => {
                let mut mapped = selection.clone();
                mapped.call = self.point(selection.call)?;
                mapped.target = self.target(&selection.target)?;
                mapped.destination = self.environment(&selection.destination)?;
                mapped.projection = self.projections(&selection.projection)?;
                let rows = self.remap.producers.iter().filter(|row| row.0 == selection.source.table.owner).collect::<Vec<_>>();
                match rows.as_slice() {
                    [] => {}
                    [(_, owner)] => {
                        mapped.source.table.owner = owner.clone();
                        mapped.source.return_point = self.remap.boundary_point(&SourceEventId { owner: selection.source.table.owner.clone(), point: selection.source.return_point })?.point;
                    }
                    _ => return Err(ValueSchemaRemapError::MissingProducer(selection.source.table.owner.clone())),
                }
                let makers = self.forest.sources.iter().filter(|row| row.producer == selection.source.table.owner).collect::<Vec<_>>();
                let [maker] = makers.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(selection.source.table.owner.clone())); };
                let returns = maker.resource_construction.original().producer_return_cases.iter().filter(|row| row.table == selection.source.table && row.return_point == selection.source.return_point
                    && row.output == selection.source.output && row.alternative == selection.source.alternative).collect::<Vec<_>>();
                let [returned] = returns.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(selection.source.table.owner.clone())); };
                let maker_owner = allocation::qualified_owner(self.remap, &maker.producer)?;
                let maker_construction = match self.mode {
                    ResourceReferenceConstruction::Copied => Self::new(self.remap, &maker.producer, &maker_owner, self.forest),
                    ResourceReferenceConstruction::Paired => Self::paired(self.remap, &maker.producer, &maker_owner, self.forest),
                };
                mapped.source.alternative.source_index = maker_construction.environment(&returned.environment)?.index;
                mapped.source.alternative.source_instance = maker_construction.instance(&selection.source.alternative.source_instance);
                HolderProducerRef::CallCreation { point: self.point(*point)?, environment: self.environment(environment)?, selection: mapped }
            }
            HolderProducerRef::InstalledRead { point, environment, action } => HolderProducerRef::InstalledRead { point: self.point(*point)?, environment: self.environment(environment)?, action: self.action(action)? },
            HolderProducerRef::GlobalCallsiteInput { call, holder, environment } => HolderProducerRef::GlobalCallsiteInput { call: self.canonical_span(call), holder: self.holder_place(holder)?, environment: self.environment(environment)? },
            HolderProducerRef::Forward { point, environment, action, previous } => HolderProducerRef::Forward {
                point: self.point(*point)?, environment: self.environment(environment)?, action: self.action(action)?, previous: Box::new(self.producer(previous)?),
            },
        })
    }

    pub(super) fn value(&self, value: &BoundaryResourceValue) -> Result<BoundaryResourceValue, ValueSchemaRemapError> {
        Ok(match value {
            BoundaryResourceValue::Origin { id, fact } => BoundaryResourceValue::Origin { id: self.origin(*id)?, fact: match fact {
                ReferenceOrigin::Fresh { allocation, loan } => {
                    let mut mapped = allocation.clone(); mapped.source = self.canonical_source(&allocation.source);
                    ReferenceOrigin::Fresh { allocation: mapped, loan: self.loan(loan)? }
                },
                ReferenceOrigin::BorrowedFrom { parameter, loan } => ReferenceOrigin::BorrowedFrom { parameter: *parameter, loan: self.loan(loan)? },
                ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => fact.clone(),
            } },
            BoundaryResourceValue::Binding(binding) => BoundaryResourceValue::Binding(self.binding_constructor(binding)?),
            BoundaryResourceValue::Place(place) => BoundaryResourceValue::Place(self.place_constructor(place)?),
            BoundaryResourceValue::Loan(loan) => BoundaryResourceValue::Loan(self.loan_constructor(loan)?),
            BoundaryResourceValue::Runtime(runtime) => BoundaryResourceValue::Runtime(self.runtime_constructor(runtime)?),
            BoundaryResourceValue::Environment { id, fact } => {
                let mut mapped = fact.clone();
                mapped.literal = self.canonical_span(&fact.literal);
                mapped.instance = self.instance(&fact.instance);
                mapped.origins = fact.origins.iter().map(|origin| self.origin(*origin)).collect::<Result<BTreeSet<_>, _>>()?;
                mapped.loans = fact.loans.iter().map(|loan| Ok(CallableLoanId { origin: self.origin(loan.origin)? })).collect::<Result<BTreeSet<_>, ValueSchemaRemapError>>()?;
                mapped.owners = fact.owners.iter().map(|owner| self.binding(owner)).collect::<Result<BTreeSet<_>, _>>()?;
                mapped.backings = fact.backings.iter().map(|backing| self.backing(backing)).collect::<Result<Vec<_>, _>>()?;
                mapped.inputs = fact.inputs.iter().map(|input| self.input(input)).collect::<Result<Vec<_>, _>>()?;
                mapped.capture_paths = self.paths(&fact.capture_paths)?;
                BoundaryResourceValue::Environment { id: self.environment(id)?, fact: mapped }
            }
            BoundaryResourceValue::Backing(backing) => BoundaryResourceValue::Backing(self.backing_constructor(backing)?),
            BoundaryResourceValue::Holder(holder) => BoundaryResourceValue::Holder(ConnectedCaptureHolderFact {
                holder: self.holder_place(&holder.holder)?, environment: self.environment(&holder.environment)?, producer: self.producer(&holder.producer)?,
            }),
        })
    }
}
