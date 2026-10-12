use super::*;
use crate::scalar::{
    RuntimeBackingFact, RuntimeBackingTransfer, RuntimeCallArgumentFact,
    RuntimeProjectedBackingFact, RuntimeReturnBackingAction, ScalarReferenceCfg,
};

pub(super) fn descriptor_cell_pointer<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    environment: PointerValue<'ctx>,
    offset: u64,
) -> Result<PointerValue<'ctx>, String> {
    let address = unsafe {
        state.builder.build_in_bounds_gep(
            context.i8_type(),
            environment,
            &[context.i64_type().const_int(offset, false)],
            "environment_capture_cell_address",
        )
    }
    .map_err(builder_error)?;
    Ok(state
        .builder
        .build_load(
            context.ptr_type(AddressSpace::default()),
            address,
            "environment_capture_cell",
        )
        .map_err(builder_error)?
        .into_pointer_value())
}

pub(super) fn distinct_environment_cell<'ctx, 'module>(
    context: &'ctx Context,
    state: &EmitState<'ctx, 'module>,
    cell: PointerValue<'ctx>,
    seen: &[PointerValue<'ctx>],
) -> Result<inkwell::values::IntValue<'ctx>, String> {
    let mut distinct = context.bool_type().const_int(1, false);
    for previous in seen {
        let equal = state
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                state
                    .builder
                    .build_ptr_to_int(
                        cell,
                        pointer_integer_type(context, state.target_layout),
                        "environment_cell_identity",
                    )
                    .map_err(builder_error)?,
                state
                    .builder
                    .build_ptr_to_int(
                        *previous,
                        pointer_integer_type(context, state.target_layout),
                        "environment_prior_cell_identity",
                    )
                    .map_err(builder_error)?,
                "environment_cell_alias",
            )
            .map_err(builder_error)?;
        distinct = state
            .builder
            .build_and(
                distinct,
                state
                    .builder
                    .build_not(equal, "environment_cell_new")
                    .map_err(builder_error)?,
                "environment_cell_distinct",
            )
            .map_err(builder_error)?;
    }
    Ok(distinct)
}

#[derive(Clone)]
pub(super) struct ValidatedCapture {
    pub binding: crate::scalar::ReferenceBindingId,
    pub backings: Vec<RuntimeBackingFact>,
    pub projected: Vec<RuntimeProjectedBackingFact>,
}

#[derive(Clone)]
pub(super) struct ValidatedEscape {
    pub literal: wosy_syntax::SourceSpan,
    pub binding: crate::scalar::ReferenceBindingId,
    pub backings: Vec<RuntimeBackingFact>,
}

pub(super) struct ValidatedFieldReceiving {
    pub event: crate::scalar::SourceEventId,
    pub source: crate::scalar::CallableFieldSource,
    pub target: crate::scalar::ReferenceBindingId,
}

#[derive(Default)]
pub(super) struct ValidatedBackingMaps {
    pub transfers: BTreeMap<(u32, u32), (RuntimeBackingTransfer, Vec<RuntimeBackingFact>, bool)>,
    pub places: BTreeMap<(u32, u32), (RuntimeBackingTransfer, Vec<RuntimeBackingFact>)>,
    pub copies: BTreeMap<(u32, u32), ValidatedFieldReceiving>,
    pub returns: BTreeMap<usize, RuntimeReturnBackingAction>,
    pub return_facts: BTreeMap<usize, Vec<RuntimeBackingFact>>,
    pub call_facts: BTreeMap<(u32, u32, usize), Vec<RuntimeBackingFact>>,
    pub call_paths: BTreeMap<(u32, u32), Vec<Vec<RuntimeCallArgumentFact>>>,
    pub input_paths: Vec<Vec<RuntimeBackingFact>>,
    pub captures: BTreeMap<(u32, u32), Vec<ValidatedCapture>>,
    pub escaping: Vec<ValidatedEscape>,
}

pub(super) fn validated_backing_maps(cfg: &ScalarReferenceCfg) -> ValidatedBackingMaps {
    let mut maps = ValidatedBackingMaps::default();
    maps.input_paths = cfg.backing_input_paths.clone();
    for point in &cfg.points {
        let span = (point.source_span.range.start, point.source_span.range.end);
        match &point.kind {
            CfgPointKind::Backing {
                transfer,
                materialize,
                ..
            } => {
                maps.transfers.insert(
                    span,
                    (
                        transfer.clone(),
                        point.possible_backings.clone(),
                        *materialize,
                    ),
                );
            }
            CfgPointKind::BackingPlace { transfer, .. } => {
                maps.places
                    .insert(span, (transfer.clone(), point.possible_backings.clone()));
            }
            CfgPointKind::CopyCallableFields { source, target } => {
                maps.copies.insert(span, ValidatedFieldReceiving {
                    event: crate::scalar::SourceEventId {
                        owner: cfg.producer_owner.clone(),
                        point: point.id,
                    },
                    source: source.clone(),
                    target: target.clone(),
                });
            }
            CfgPointKind::ReturnOutput {
                output,
                backing_action,
                ..
            } => {
                maps.returns.insert(*output, backing_action.clone());
                maps.return_facts
                    .insert(*output, point.possible_backings.clone());
            }
            CfgPointKind::Call { output, .. } => {
                let facts = point
                    .possible_backings
                    .iter()
                    .filter(|fact| {
                        matches!(&fact.owner.kind, ReferenceBindingKind::CallOutput { point: origin, output: result } if *origin == point.id && *result == *output)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if !facts.is_empty() {
                    maps.call_facts.insert((span.0, span.1, *output), facts);
                }
                if !point.backing_argument_paths.is_empty() {
                    maps.call_paths
                        .insert(span, point.backing_argument_paths.clone());
                }
            }
            CfgPointKind::Capture {
                literal,
                environment,
                captures,
                ..
            } => {
                let inputs = cfg
                    .environments
                    .get(environment.index)
                    .map(|fact| fact.inputs.clone())
                    .unwrap_or_default();
                let mut validated = Vec::new();
                for capture in captures {
                    let found = inputs.iter().find(|input| input.binding == capture.binding);
                    validated.push(ValidatedCapture {
                        binding: capture.binding.clone(),
                        backings: found
                            .map(|input| input.backings.clone())
                            .unwrap_or_default(),
                        projected: found
                            .map(|input| input.projected_backings.clone())
                            .unwrap_or_default(),
                    });
                }
                maps.captures
                    .insert((literal.start, literal.end), validated);
            }
            _ => {}
        }
    }
    let mut seen = BTreeSet::new();
    for point in &cfg.points {
        if !matches!(point.kind, CfgPointKind::ReturnOutput { .. }) {
            continue;
        }
        for environment in point.possible_environments.iter().chain(
            point
                .possible_projected_environments
                .iter()
                .flat_map(|projection| &projection.environments),
        ) {
            let Some(fact) = cfg.environments.get(environment.index) else {
                continue;
            };
            for input in &fact.inputs {
                if input.backings.is_empty() {
                    continue;
                }
                let key = (
                    fact.literal.range.start,
                    fact.literal.range.end,
                    input.binding.declaration_span.start,
                    input.binding.declaration_span.end,
                );
                if !seen.insert(key) {
                    continue;
                }
                maps.escaping.push(ValidatedEscape {
                    literal: fact.literal.clone(),
                    binding: input.binding.clone(),
                    backings: input.backings.clone(),
                });
            }
        }
    }
    maps
}
