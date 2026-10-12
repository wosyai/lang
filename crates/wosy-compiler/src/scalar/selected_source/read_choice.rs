use super::*;
mod original_output;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceReadRegistration {
    Binding(EvaluatedValueId),
    Place(ScalarReadCfgPoint),
    Field { value: EvaluatedValueId, place: ScalarPlace },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceReadDestination {
    Concrete(SourceValueViewRef),
    Formal(SourceFormalValueRef),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceReadPointerObservation {
    Published(JointSelectedPortFact),
    Pending,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceReadFormalGrantInput {
    Parameter {
        binding: ReferenceBindingId,
        ordinal: usize,
        introduction: SourceEventId,
        origin: ReferenceOriginId,
        loan: ReferenceLoanId,
    },
    Boundary {
        port: BoundaryFormalPortRef,
        declaration: SourceSpan,
        mode: ScalarReferenceMutability,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceReadAssociation {
    DirectBinding(ReferencePlaceId),
    FormalPointer {
        pointer: EvaluatedValueId,
        coordinate: SourceSlotRef,
        view: SourceValueViewRef,
        template: SourceFormalValueRef,
        member: SourceShapeMemberPort,
        grants: Vec<SourceGrantClassRef>,
        original: SourceReadFormalGrantInput,
    },
    SelectedCheckedReferent {
        pointer: EvaluatedValueId,
        coordinate: SourceSlotRef,
        view: SourceValueViewRef,
        observation: SourceReadPointerObservation,
        referent: SourceCheckedReferentView,
        suffix: ScalarPlace,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceReadProjectionStep {
    Field {
        member: SourceShapeMemberPort,
        field: ScalarStructFieldId,
    },
    FixedIndex {
        member: SourceShapeMemberPort,
        length: u64,
        index: BigInt,
        selector: EvaluatedValueId,
    },
    DynamicIndex {
        member: SourceShapeMemberPort,
        selector: EvaluatedValueId,
    },
    CheckedDereference {
        member: SourceShapeMemberPort,
        pointer: EvaluatedValueId,
        mode: ScalarReferenceMutability,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceReadPosition {
    pub value: EvaluatedValueId,
    pub steps: Vec<SourceReadProjectionStep>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceReadChoice {
    pub event: SourceEventId,
    pub registration: SourceReadRegistration,
    pub input: SourceConfigurationRef,
    pub source: SourceSlotRef,
    pub view: SourceValueViewRef,
    pub association: SourceReadAssociation,
    pub destination: SourceReadDestination,
    pub position: SourceReadPosition,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::scalar) struct OriginalReadSelector {
    pub registration: ScalarIndexCfgPoint,
    pub value: EvaluatedValueId,
    pub expression: ScalarExpression,
}

fn indexes(place: &ScalarPlace) -> Vec<(ScalarExpression, ByteSpan)> {
    match place {
        ScalarPlace::Name { .. } | ScalarPlace::Dereference { .. } => Vec::new(),
        ScalarPlace::Field { base, .. } => indexes(base),
        ScalarPlace::Index { base, index, index_span, .. } => {
            let mut selectors = indexes(base);
            selectors.push((index.as_ref().clone(), *index_span));
            selectors
        },
    }
}

pub(in crate::scalar) fn original_selectors(
    tables: &EvaluatedValueTables,
    cfg: &ScalarReferenceCfg,
    structs: &[ScalarStruct],
    point: &ReferenceFlowPoint,
) -> Result<(SourceReadRegistration, Vec<OriginalReadSelector>), ReferenceAnalysisError> {
    let fail = || invalid(&point.source_span);
    let original = cfg.initial_points.get(point.id.0).ok_or_else(fail)?;
    if original.id != point.id || original.source_span != point.source_span || original.scope != point.scope { return Err(fail()); }
    let immutable = match (&original.kind, &point.kind) {
        (CfgPointKind::Read { binding }, CfgPointKind::Read { binding: actual }) => binding == actual,
        (CfgPointKind::ReadPlace { place, pointer_point, .. }, CfgPointKind::ReadPlace { place: actual, pointer_point: actual_pointer, .. }) => place == actual && pointer_point == actual_pointer,
        _ => false,
    };
    if !immutable { return Err(fail()); }
    let values = tables.values.iter().filter(|value| value.id.point == original.id && value.id.resolved.producer == cfg.producer_owner && value.id.resolved.output == 0).collect::<Vec<_>>();
    let [value] = values.as_slice() else { return Err(fail()); };
    if value.id.resolved.occurrence != original.source_span {
        return Err(fail());
    }
    let actual = cfg.read_points.iter().filter(|registration| registration.point == original.id).collect::<Vec<_>>();
    match &original.kind {
        CfgPointKind::Read { binding } => {
            if !actual.is_empty() || binding.source != point.source_span.source {
                return Err(fail());
            }
            let names = tables.source_name_reads.iter().filter(|row| row.value == value.id && row.event.point == original.id && row.event.owner == cfg.producer_owner).collect::<Vec<_>>();
            let [name] = names.as_slice() else { return Err(fail()); };
            let mut candidate = cfg.clone();
            candidate.evaluated_values = tables.clone();
            reference::name_resolution::validate(&candidate, structs)?;
            if name.binding != *binding { return Err(fail()); }
            Ok((SourceReadRegistration::Binding(value.id.clone()), Vec::new()))
        },
        CfgPointKind::ReadPlace { place, .. } => {
            let expressions = indexes(place);
            let [registration] = actual.as_slice() else {
                if actual.is_empty() && expressions.is_empty() && matches!(place, ScalarPlace::Field { .. }) {
                    return Ok((SourceReadRegistration::Field { value: value.id.clone(), place: place.clone() }, Vec::new()));
                }
                return Err(fail());
            };
            if registration.occurrence.source_span != point.source_span
                || registration.occurrence.output_projection != value.id.resolved.output_projection
                || registration.function_span != cfg.initial_points[0].source_span.range
                || registration.indices.len() != expressions.len() {
                return Err(fail());
            }
            let mut selectors = Vec::new();
            for (registered, (expression, index_span)) in registration.indices.iter().zip(expressions) {
                let selector = cfg.initial_points.get(registered.point.0).ok_or_else(fail)?;
                if selector.id != registered.point || selector.id.0 >= point.id.0
                    || selector.source_span != registered.source_span || selector.source_span.source != point.source_span.source
                    || registered.source_span.range != index_span || typecheck::expression_span(&expression) != index_span
                    || selectors.last().is_some_and(|known: &OriginalReadSelector| known.registration.point >= registered.point) {
                    return Err(fail());
                }
                let value = original_output::validate(cfg, tables, structs, &expression, registered)?;
                selectors.push(OriginalReadSelector { registration: registered.clone(), value, expression });
            }
            Ok((SourceReadRegistration::Place((*registration).clone()), selectors))
        },
        _ => Err(fail()),
    }
}
