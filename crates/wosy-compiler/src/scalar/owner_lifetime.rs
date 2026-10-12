use super::*;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OwnerParent {
    Root,
    Nested(OwnerCapabilityId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OwnerRole {
    Module(ReferenceBindingId),
    Scope(ScalarBlockOccurrence),
    Binding(ReferenceBindingId),
    Parameter(ReferenceBindingId),
    Result(ValueConsumerId),
    Discard(ValueConsumerId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OwnerEndpointKind {
    ScopeExit { block: ScalarBlockOccurrence },
    CheckedDiscard { consumer: ValueConsumerId },
    ParameterExit { binding: ReferenceBindingId },
    ResultHandoff { consumer: ValueConsumerId },
    ModuleTerminal { binding: ReferenceBindingId },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OwnerEndpointFact {
    pub event: SourceEventId,
    pub frame: FrameVariableRef,
    pub kind: OwnerEndpointKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OwnerCapabilityFact {
    pub id: OwnerCapabilityId,
    pub frame: FrameVariableRef,
    pub role: OwnerRole,
    pub parent: OwnerParent,
    pub introduction: SourceEventId,
    pub endpoints: Vec<OwnerEndpointFact>,
}

fn invalid(source: &SourceSpan) -> ReferenceAnalysisError {
    ReferenceAnalysisError::MissingContext {
        source_span: source.clone(),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    }
}

fn reaches(cfg: &ScalarReferenceCfg, start: CfgPointId, end: CfgPointId) -> bool {
    let mut pending = vec![start];
    let mut visited = BTreeSet::new();
    while let Some(point) = pending.pop() {
        if point == end {
            return true;
        }
        if visited.insert(point) {
            pending.extend(&cfg.points[point.0].successors);
        }
    }
    false
}

fn event(cfg: &ScalarReferenceCfg, point: CfgPointId) -> SourceEventId {
    SourceEventId {
        owner: cfg.producer_owner.clone(),
        point,
    }
}

pub(super) fn register(cfg: &mut ScalarReferenceCfg) -> Result<(), ReferenceAnalysisError> {
    if !cfg.evaluated_values.owner_capabilities.is_empty() {
        return Err(invalid(&cfg.points[0].source_span));
    }
    let frame = FrameVariableRef {
        owner: cfg.producer_owner.clone(),
        variable: SupportedInstanceVariable(0),
    };
    let mut owners = Vec::new();
    for exit in &cfg.block_scope_exits {
        let endpoint = &cfg.points[exit.point.0];
        let entries = cfg
            .points
            .iter()
            .filter(|point| {
                matches!(point.kind, CfgPointKind::ScopeEnter)
                    && point.scope == endpoint.scope
                    && point.source_span == endpoint.source_span
            })
            .collect::<Vec<_>>();
        let [entry] = entries.as_slice() else {
            return Err(invalid(&endpoint.source_span));
        };
        if !matches!(endpoint.kind, CfgPointKind::ScopeExit) || !reaches(cfg, entry.id, endpoint.id)
        {
            return Err(invalid(&endpoint.source_span));
        }
        owners.push(OwnerCapabilityFact {
            id: OwnerCapabilityId {
                producer: cfg.producer_owner.clone(),
                introduction: entry.id,
            },
            frame: frame.clone(),
            role: OwnerRole::Scope(exit.occurrence.clone()),
            parent: OwnerParent::Root,
            introduction: event(cfg, entry.id),
            endpoints: vec![OwnerEndpointFact {
                event: event(cfg, endpoint.id),
                frame: frame.clone(),
                kind: OwnerEndpointKind::ScopeExit {
                    block: exit.occurrence.clone(),
                },
            }],
        });
    }
    for index in 0..owners.len() {
        let entry = &cfg.points[owners[index].introduction.point.0];
        let parent = owners
            .iter()
            .filter(|owner| {
                let outer = &cfg.points[owner.introduction.point.0];
                outer.id != entry.id
                    && outer.source_span.source == entry.source_span.source
                    && outer.source_span.range.start <= entry.source_span.range.start
                    && outer.source_span.range.end >= entry.source_span.range.end
                    && outer.source_span.range != entry.source_span.range
                    && reaches(cfg, outer.id, entry.id)
                    && reaches(cfg, entry.id, owner.endpoints[0].event.point)
            })
            .min_by_key(|owner| {
                let range = cfg.points[owner.introduction.point.0].source_span.range;
                range.end - range.start
            });
        if let Some(parent) = parent {
            owners[index].parent = OwnerParent::Nested(parent.id.clone());
        }
    }
    for point in &cfg.points {
        let CfgPointKind::Bind { binding } = &point.kind else {
            continue;
        };
        let scopes = owners
            .iter()
            .filter(|owner| {
                matches!(owner.role, OwnerRole::Scope(_))
                    && cfg.points[owner.introduction.point.0].source_span.source == binding.source
                    && cfg.points[owner.introduction.point.0].source_span.range
                        == binding.block_span
            })
            .collect::<Vec<_>>();
        let [scope] = scopes.as_slice() else {
            return Err(invalid(&point.source_span));
        };
        if !reaches(cfg, point.id, scope.endpoints[0].event.point) {
            return Err(invalid(&point.source_span));
        }
        let parameter = cfg.reference_parameters.contains(binding);
        let endpoints = if parameter {
            scope
                .endpoints
                .iter()
                .map(|endpoint| OwnerEndpointFact {
                    event: endpoint.event.clone(),
                    frame: frame.clone(),
                    kind: OwnerEndpointKind::ParameterExit {
                        binding: binding.clone(),
                    },
                })
                .collect()
        } else {
            scope.endpoints.clone()
        };
        owners.push(OwnerCapabilityFact {
            id: OwnerCapabilityId {
                producer: cfg.producer_owner.clone(),
                introduction: point.id,
            },
            frame: frame.clone(),
            role: if parameter {
                OwnerRole::Parameter(binding.clone())
            } else {
                OwnerRole::Binding(binding.clone())
            },
            parent: OwnerParent::Nested(scope.id.clone()),
            introduction: event(cfg, point.id),
            endpoints,
        });
    }
    for input in &cfg.evaluated_values.final_inputs {
        let consumer = &input.consumer;
        owners.push(OwnerCapabilityFact {
            id: OwnerCapabilityId {
                producer: cfg.producer_owner.clone(),
                introduction: consumer.point,
            },
            frame: frame.clone(),
            role: OwnerRole::Result(consumer.clone()),
            parent: OwnerParent::Root,
            introduction: event(cfg, consumer.point),
            endpoints: vec![OwnerEndpointFact {
                event: event(cfg, consumer.point),
                frame: frame.clone(),
                kind: OwnerEndpointKind::ResultHandoff {
                    consumer: consumer.clone(),
                },
            }],
        });
    }
    cfg.evaluated_values.owner_capabilities = owners;
    validate(cfg)
}

pub(super) fn validate(cfg: &ScalarReferenceCfg) -> Result<(), ReferenceAnalysisError> {
    reference::parameter_input::validate_owners(cfg)?;
    for (index, owner) in cfg.evaluated_values.owner_capabilities.iter().enumerate() {
        let point = cfg
            .points
            .get(owner.introduction.point.0)
            .ok_or_else(|| invalid(&cfg.points[0].source_span))?;
        if owner.id.producer != cfg.producer_owner
            || owner.introduction.owner != cfg.producer_owner
            || owner.id.introduction != point.id
            || owner.frame.owner != cfg.producer_owner
            || owner.endpoints.is_empty()
            || cfg.evaluated_values.owner_capabilities[..index]
                .iter()
                .any(|previous| previous.id == owner.id)
        {
            return Err(invalid(&point.source_span));
        }
        if let OwnerParent::Nested(parent) = &owner.parent {
            let parents = cfg
                .evaluated_values
                .owner_capabilities
                .iter()
                .filter(|row| &row.id == parent)
                .collect::<Vec<_>>();
            let [parent] = parents.as_slice() else {
                return Err(invalid(&point.source_span));
            };
            let start = &cfg.points[parent.introduction.point.0];
            if parent.id == owner.id
                || parent.frame != owner.frame
                || !matches!(parent.role, OwnerRole::Scope(_))
            {
                return Err(invalid(&point.source_span));
            }
            let connected = match &owner.role {
                OwnerRole::Scope(_) => {
                    start.source_span.source == point.source_span.source
                        && start.source_span.range != point.source_span.range
                        && start.source_span.range.start <= point.source_span.range.start
                        && start.source_span.range.end >= point.source_span.range.end
                        && reaches(cfg, start.id, point.id)
                        && parent
                            .endpoints
                            .iter()
                            .all(|endpoint| reaches(cfg, point.id, endpoint.event.point))
                }
                OwnerRole::Binding(binding) | OwnerRole::Parameter(binding) => {
                    start.source_span.source == binding.source
                        && start.source_span.range == binding.block_span
                        && owner.endpoints.iter().all(|endpoint| {
                            parent
                                .endpoints
                                .iter()
                                .any(|outer| outer.event == endpoint.event)
                        })
                }
                _ => false,
            };
            if !connected {
                return Err(invalid(&point.source_span));
            }
        } else if matches!(owner.role, OwnerRole::Binding(_) | OwnerRole::Parameter(_)) {
            return Err(invalid(&point.source_span));
        }
        for endpoint in &owner.endpoints {
            let end = cfg
                .points
                .get(endpoint.event.point.0)
                .ok_or_else(|| invalid(&point.source_span))?;
            if endpoint.event.owner != cfg.producer_owner
                || endpoint.frame != owner.frame
                || !reaches(cfg, point.id, end.id)
            {
                return Err(invalid(&point.source_span));
            }
            let connected = match (&owner.role, &endpoint.kind, &point.kind, &end.kind) {
                (
                    OwnerRole::Scope(block),
                    OwnerEndpointKind::ScopeExit { block: actual },
                    CfgPointKind::ScopeEnter,
                    CfgPointKind::ScopeExit,
                ) => {
                    block == actual
                        && point.scope == end.scope
                        && point.source_span == end.source_span
                        && cfg
                            .block_scope_exits
                            .iter()
                            .any(|exit| exit.point == end.id && &exit.occurrence == block)
                }
                (
                    OwnerRole::Binding(binding),
                    OwnerEndpointKind::ScopeExit { block },
                    CfgPointKind::Bind { binding: actual },
                    CfgPointKind::ScopeExit,
                ) => {
                    binding == actual
                        && binding.source == end.source_span.source
                        && binding.block_span == end.source_span.range
                        && cfg
                            .block_scope_exits
                            .iter()
                            .any(|exit| exit.point == end.id && &exit.occurrence == block)
                }
                (
                    OwnerRole::Parameter(binding),
                    OwnerEndpointKind::ParameterExit { binding: actual },
                    CfgPointKind::Bind {
                        binding: introduced,
                    },
                    CfgPointKind::ScopeExit,
                ) => {
                    binding == actual
                        && binding == introduced
                        && cfg.reference_parameters.contains(binding)
                        && binding.source == end.source_span.source
                        && binding.block_span == end.source_span.range
                }
                (
                    OwnerRole::Result(consumer),
                    OwnerEndpointKind::ResultHandoff { consumer: actual },
                    CfgPointKind::ReturnOutput { .. },
                    CfgPointKind::ReturnOutput { .. },
                ) => {
                    consumer == actual
                        && consumer.producer == cfg.producer_owner
                        && consumer.point == point.id
                        && point.id == end.id
                        && cfg
                            .evaluated_values
                            .final_inputs
                            .iter()
                            .any(|input| &input.consumer == consumer)
                }
                _ => false,
            };
            if !connected {
                return Err(invalid(&point.source_span));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
