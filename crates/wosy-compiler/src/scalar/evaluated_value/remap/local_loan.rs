use super::*;

impl ValueSchemaRemap {
    fn local_point(
        &self,
        owner: &ProducerCfgOwnerId,
        point: &LocalLoanPoint,
    ) -> Result<LocalLoanPoint, ValueSchemaRemapError> {
        let mut point = point.clone();
        point.event.event = self.source_event(&point.event.event)?;
        point.event.successors = point
            .event
            .successors
            .iter()
            .map(|event| self.source_event(event))
            .collect::<Result<Vec<_>, _>>()?;
        for fact in &mut point.event.facts {
            fact.event = self.source_event(&fact.event)?;
            match &mut fact.kind {
                CommonLoanFactKind::Use { target, .. } => {
                    *target = self.common_target(owner, target)?;
                }
                CommonLoanFactKind::NoLoan { outputs } => {
                    *outputs = outputs
                        .iter()
                        .map(|value| self.value(value))
                        .collect::<Result<Vec<_>, _>>()?;
                }
                _ => {}
            }
        }
        Ok(point)
    }
    fn local_candidate(
        &self,
        id: &LocalLoanCandidateRef,
    ) -> Result<LocalLoanCandidateRef, ValueSchemaRemapError> {
        Ok(LocalLoanCandidateRef {
            owner: self.producer(&id.owner)?,
            ordinal: id.ordinal,
        })
    }

    fn local_pending(
        &self,
        owner: &ProducerCfgOwnerId,
        reason: &LocalLoanPendingReason,
    ) -> Result<LocalLoanPendingReason, ValueSchemaRemapError> {
        Ok(match reason {
            LocalLoanPendingReason::Coverage(pending) => {
                LocalLoanPendingReason::Coverage(self.common_pending(pending)?)
            }
            LocalLoanPendingReason::MissingContributor(id) => {
                LocalLoanPendingReason::MissingContributor(self.local_candidate(id)?)
            }
            LocalLoanPendingReason::MissingMonitor(id) => {
                LocalLoanPendingReason::MissingMonitor(self.source_tuple(id)?)
            }
            LocalLoanPendingReason::Unavailable { role, state } => {
                LocalLoanPendingReason::Unavailable {
                    role: *role,
                    state: self.availability(owner, state)?,
                }
            }
            LocalLoanPendingReason::CheckedState { role, state } => {
                LocalLoanPendingReason::CheckedState {
                    role: *role,
                    state: match state {
                        CheckedValueState::Live => CheckedValueState::Live,
                        CheckedValueState::Null { source } => CheckedValueState::Null {
                            source: source.clone(),
                        },
                        CheckedValueState::Expired {
                            origin,
                            effect_point,
                        } => CheckedValueState::Expired {
                            origin: self.origin(owner, *origin)?,
                            effect_point: self.point(owner, *effect_point)?,
                        },
                    },
                }
            }
            reason => reason.clone(),
        })
    }

    fn local_grant(
        &self,
        grant: &SourceGrantClassFact,
        correspondence: &BoundaryArenaCorrespondence,
    ) -> Result<SourceGrantClassFact, ValueSchemaRemapError> {
        Ok(SourceGrantClassFact {
            class: self.grant_class(&grant.class)?,
            site: self.source_event(&grant.site)?,
            referent: self.source_instance(&grant.referent)?,
            coordinate: self.boundary_slot(&grant.coordinate, correspondence)?,
            projection: self.projections(&grant.projection)?,
            mode: grant.mode,
        })
    }

    fn local_support(
        &self,
        support: &LocalLoanSupport,
        correspondence: &BoundaryArenaCorrespondence,
    ) -> Result<LocalLoanSupport, ValueSchemaRemapError> {
        Ok(LocalLoanSupport {
            role: support.role,
            site: support.site,
            selected: self.selected_port(&support.selected, correspondence)?,
            grant: self.local_grant(&support.grant, correspondence)?,
        })
    }

    pub(super) fn staged_loan_queries(
        &self,
        tables: &EvaluatedValueTables,
        correspondence: &BoundaryArenaCorrespondence,
    ) -> Result<Vec<StagedLoanQueryAnalysis>, ValueSchemaRemapError> {
        let common = self.common_loans(tables)?;
        tables
            .staged_loan_queries
            .iter()
            .map(|original| {
                let epochs = tables
                    .common_loans
                    .iter()
                    .enumerate()
                    .filter(|(_, common)| common.input == original.epoch)
                    .collect::<Vec<_>>();
                let [(index, _)] = epochs.as_slice() else {
                    return Err(ValueSchemaRemapError::MissingProducer(
                        original.id.owner.clone(),
                    ));
                };
                let owner = &original.id.owner;
                let mut row = original.clone();
                row.id.owner = self.producer(owner)?;
                row.epoch = common[*index].input.clone();
                row.coverage = row
                    .coverage
                    .iter()
                    .map(|pending| self.common_pending(pending))
                    .collect::<Result<Vec<_>, _>>()?;
                if let StagedLoanQueryResult::Candidates {
                    slices,
                    queries,
                    pending,
                } = &mut row.result
                {
                    for slice in slices {
                        slice.candidate = self.local_candidate(&slice.candidate)?;
                        slice.coverage = slice
                            .coverage
                            .iter()
                            .map(|pending| self.common_pending(pending))
                            .collect::<Result<Vec<_>, _>>()?;
                        for point in &mut slice.points {
                            point.event.event = self.source_event(&point.event.event)?;
                            point.event.successors = point
                                .event
                                .successors
                                .iter()
                                .map(|event| self.source_event(event))
                                .collect::<Result<Vec<_>, _>>()?;
                            for fact in &mut point.event.facts {
                                fact.event = self.source_event(&fact.event)?;
                                match &mut fact.kind {
                                    CommonLoanFactKind::Use { target, .. } => {
                                        *target = self.common_target(owner, target)?;
                                    }
                                    CommonLoanFactKind::NoLoan { outputs } => {
                                        *outputs = outputs
                                            .iter()
                                            .map(|value| self.value(value))
                                            .collect::<Result<Vec<_>, _>>()?;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    let equations = self.source_port_equations(tables, correspondence)?;
                    for query in queries {
                        let originals = tables
                            .source_port_equations
                            .iter()
                            .enumerate()
                            .filter(|(_, equation)| **equation == query.equation)
                            .collect::<Vec<_>>();
                        let [(index, _)] = originals.as_slice() else {
                            return Err(ValueSchemaRemapError::MissingProducer(owner.clone()));
                        };
                        query.equation = equations[*index].clone();
                        query.candidate = self.local_candidate(&query.candidate)?;
                        query.contributor = self.source_tuple(&query.contributor)?;
                        query.input = self.source_configuration(&query.input)?;
                        query.output = self.source_configuration(&query.output)?;
                        query.frame = self.source_frame(&query.frame)?;
                        query.remaining_uses =
                            self.source_use_observation(&query.remaining_uses)?;
                        query.point = self.local_point(owner, &query.point)?;
                        query.access = match &query.access {
                            LocalLoanAccess::DirectStorage {
                                referent,
                                coordinate,
                                projection,
                            } => LocalLoanAccess::DirectStorage {
                                referent: self.source_instance(referent)?,
                                coordinate: self.boundary_slot(coordinate, correspondence)?,
                                projection: self.projections(projection)?,
                            },
                            LocalLoanAccess::Through(support) => {
                                LocalLoanAccess::Through(self.local_support(support, correspondence)?)
                            }
                            LocalLoanAccess::Pending => LocalLoanAccess::Pending,
                        };
                        for pair in &mut query.pairs {
                            pair.support = self.local_support(&pair.support, correspondence)?;
                            pair.protected = self.local_grant(&pair.protected, correspondence)?;
                        }
                        query.pending = query
                            .pending
                            .iter()
                            .map(|reason| self.local_pending(owner, reason))
                            .collect::<Result<Vec<_>, _>>()?;
                    }
                    *pending = pending
                        .iter()
                        .map(|reason| self.local_pending(owner, reason))
                        .collect::<Result<Vec<_>, _>>()?;
                }
                Ok(row)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_loan_remap_preserves_tuple_and_dense_source_links() {
        let cfg = crate::scalar::common_loan::tests::cfg("%%start\nunit() inspect = fn { u32 value = 1; *!u32 root = &!value; *!u32 child = &!(*root); *child; *root; };\n%%end", ScalarTargetLayout::NATIVE64, false);
        let owner = cfg.producer_owner.clone();
        let identity = SourceIdentity::new(
            "imported-local".into(),
            "app".into(),
            "src/import.w".into(),
            "r1".into(),
        );
        let imported = ProducerCfgOwnerId::Module(identity.clone());
        let bindings = cfg
            .evaluated_values
            .source_binding_shapes
            .iter()
            .map(|fact| {
                let mut binding = fact.binding.clone();
                binding.source = identity.clone();
                (fact.binding.clone(), binding)
            })
            .collect();
        let remap = ValueSchemaRemap {
            producers: vec![(owner.clone(), imported.clone())],
            points: cfg
                .initial_points
                .iter()
                .map(|point| ValuePointRemap {
                    producer: owner.clone(),
                    point: point.id,
                    imported: CfgPointId(point.id.0 + 17),
                })
                .collect(),
            shapes: cfg
                .evaluated_values
                .shapes
                .iter()
                .map(|shape| {
                    (
                        shape.id.clone(),
                        ValueShapeId {
                            producer: imported.clone(),
                            index: shape.id.index + 23,
                        },
                    )
                })
                .collect(),
            imports: vec![ValueImportCorrespondence {
                producer: owner.clone(),
                bindings,
                places: Vec::new(),
                origins: Vec::new(),
                loans: Vec::new(),
                runtime_values: Vec::new(),
                environments: Vec::new(),
                backings: Vec::new(),
                holders: Vec::new(),
            }],
        };
        let context = SourceCapturePolicyContext { structs: Vec::new(), enums: Vec::new(), generic_scope: owner.clone(), generic_declarations: Vec::new(), generic_parameters: BTreeSet::new() };
        let sources = capture_table_sources(&cfg, &context).expect("original staged constructor closure");
        let forest = capture_root_construction_forest(&cfg, &context).expect("actual original staged construction forest");
        let destination = remap.stage_boundary_sources(&sources, &forest).expect("actual selective staged headers");
        let checked = remap
            .staged_loan_queries_into(&owner, &cfg.evaluated_values, &sources, &forest, &destination)
            .expect("qualified staged remap");
        let correspondence = &checked.correspondence;
        let mapped = &checked.queries;
        let originals = cfg
            .evaluated_values
            .staged_loan_queries
            .iter()
            .filter(|row| row.id.owner == owner)
            .collect::<Vec<_>>();
        let [original] = originals.as_slice() else {
            panic!("one original stage");
        };
        let [mapped] = mapped.as_slice() else {
            panic!("one mapped stage");
        };
        assert_eq!(mapped.id.owner, imported);
        assert_eq!(mapped.epoch.owner, imported);
        let StagedLoanQueryResult::Candidates {
            slices: original_slices,
            queries: original_queries,
            ..
        } = &original.result
        else {
            panic!("actual local candidates");
        };
        let StagedLoanQueryResult::Candidates {
            slices, queries, ..
        } = &mapped.result
        else {
            panic!("same candidate stage");
        };
        assert_eq!(slices.len(), original_slices.len());
        assert_eq!(queries.len(), original_queries.len());
        assert!(!queries.is_empty());
        for (original, mapped) in original_slices.iter().zip(slices) {
            assert_eq!(mapped.candidate.owner, imported);
            assert_eq!(mapped.candidate.ordinal, original.candidate.ordinal);
            assert_eq!(mapped.roles, original.roles);
            assert_eq!(mapped.sites, original.sites);
            for (original, mapped) in original.points.iter().zip(&mapped.points) {
                assert_eq!(
                    mapped.event.event,
                    remap.source_event(&original.event.event).expect("event")
                );
                assert_eq!(mapped.event.source_span, original.event.source_span);
                assert_eq!(mapped.incoming, original.incoming);
                assert_eq!(mapped.outgoing, original.outgoing);
                assert_eq!(mapped.incoming_subset, original.incoming_subset);
                assert_eq!(mapped.outgoing_subset, original.outgoing_subset);
                assert_eq!(mapped.demand_in, original.demand_in);
                assert_eq!(mapped.demand_out, original.demand_out);
            }
        }
        let mut paired = 0;
        for (original, mapped) in original_queries.iter().zip(queries) {
            assert_eq!(mapped.candidate.ordinal, original.candidate.ordinal);
            assert_eq!(
                mapped.contributor,
                remap
                    .source_tuple(&original.contributor)
                    .expect("contributor")
            );
            assert_eq!(
                mapped.input,
                remap.source_configuration(&original.input).expect("input")
            );
            assert_eq!(
                mapped.output,
                remap
                    .source_configuration(&original.output)
                    .expect("output")
            );
            assert_eq!(
                mapped.equation.input,
                remap
                    .source_monitor(&original.equation.input)
                    .expect("input monitor")
            );
            assert_eq!(
                mapped.equation.output,
                remap
                    .source_monitor(&original.equation.output)
                    .expect("output monitor")
            );
            assert_eq!(
                mapped.point.event.event,
                remap
                    .source_event(&original.point.event.event)
                    .expect("candidate point")
            );
            for (original, mapped) in original.pairs.iter().zip(&mapped.pairs) {
                assert_eq!(mapped.support.role, original.support.role);
                assert_eq!(mapped.support.site, original.support.site);
                assert_eq!(
                    mapped.support.selected,
                    remap
                        .selected_port(&original.support.selected, correspondence)
                        .expect("exact selected port")
                );
                assert_eq!(
                    mapped.support.grant,
                    remap
                        .local_grant(&original.support.grant, correspondence)
                        .expect("support grant")
                );
                assert_eq!(
                    mapped.protected,
                    remap
                        .local_grant(&original.protected, correspondence)
                        .expect("protected class")
                );
                assert_eq!(mapped.outcome, original.outcome);
                paired += 1;
            }
            assert!(mapped
                .pending
                .contains(&LocalLoanPendingReason::CompleteFiniteNormalization));
        }
        assert!(paired > 0);
    }
}
