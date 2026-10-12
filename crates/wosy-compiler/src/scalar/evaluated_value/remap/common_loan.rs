use super::*;

impl ValueSchemaRemap {
    pub(super) fn common_target(
        &self,
        owner: &ProducerCfgOwnerId,
        target: &CommonLoanTarget,
    ) -> Result<CommonLoanTarget, ValueSchemaRemapError> {
        Ok(match target {
            CommonLoanTarget::Storage {
                binding,
                projection,
            } => CommonLoanTarget::Storage {
                binding: self.binding(owner, binding)?,
                projection: self.projections(projection)?,
            },
            CommonLoanTarget::Through { pointer, place } => CommonLoanTarget::Through {
                pointer: *pointer,
                place: place.clone(),
            },
        })
    }
    pub(super) fn common_pending(
        &self,
        pending: &CommonLoanPending,
    ) -> Result<CommonLoanPending, ValueSchemaRemapError> {
        Ok(CommonLoanPending {
            event: self.source_event(&pending.event)?,
            source_span: pending.source_span.clone(),
            reason: match &pending.reason {
                CommonLoanPendingReason::Shape(shape) => {
                    CommonLoanPendingReason::Shape(self.shape(shape)?)
                }
                CommonLoanPendingReason::MissingBindingInput(binding) => {
                    CommonLoanPendingReason::MissingBindingInput(
                        self.binding(&pending.event.owner, binding)?,
                    )
                }
                CommonLoanPendingReason::AmbiguousOutputs(values) => {
                    CommonLoanPendingReason::AmbiguousOutputs(
                        values
                            .iter()
                            .map(|value| self.value(value))
                            .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                    )
                }
                reason => reason.clone(),
            },
        })
    }
    pub(super) fn common_loans(
        &self,
        tables: &EvaluatedValueTables,
    ) -> Result<Vec<CommonLoanAnalysis>, ValueSchemaRemapError> {
        tables
            .common_loans
            .iter()
            .map(|original| {
                let mut row = original.clone();
                let owner = &original.input.owner;
                row.input.owner = self.producer(owner)?;
                for output in &mut row.input.outputs {
                    output.value = self.value(&output.value)?;
                    output.shape = self.resolved_shape(&output.shape)?;
                }
                for binding in &mut row.input.binding_shapes {
                    binding.binding = self.binding(owner, &binding.binding)?;
                    binding.shape = self.shape(&binding.shape)?;
                }
                row.input.final_inputs = row
                    .input
                    .final_inputs
                    .iter()
                    .map(|input| self.final_input(input))
                    .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?;
                for role in &mut row.input.roles {
                    role.owner = self.producer(&role.owner)?;
                    role.projection = self.projections(&role.projection)?;
                    role.source = match &role.source {
                        CommonLoanSourceRole::Binding(binding) => {
                            CommonLoanSourceRole::Binding(self.binding(owner, binding)?)
                        }
                        CommonLoanSourceRole::Evaluated(value) => {
                            CommonLoanSourceRole::Evaluated(self.value(value)?)
                        }
                        CommonLoanSourceRole::Frozen(event) => {
                            CommonLoanSourceRole::Frozen(self.source_event(event)?)
                        }
                        CommonLoanSourceRole::Saved(event) => {
                            CommonLoanSourceRole::Saved(self.source_event(event)?)
                        }
                        CommonLoanSourceRole::Result(event) => {
                            CommonLoanSourceRole::Result(self.source_event(event)?)
                        }
                        CommonLoanSourceRole::Boundary(role) => {
                            CommonLoanSourceRole::Boundary(self.boundary_role(owner, role)?)
                        }
                    };
                }
                for site in &mut row.input.sites {
                    site.event = self.source_event(&site.event)?;
                    site.target = self.common_target(owner, &site.target)?;
                }
                for event in &mut row.input.events {
                    event.event = self.source_event(&event.event)?;
                    event.successors = event
                        .successors
                        .iter()
                        .map(|event| self.source_event(event))
                        .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?;
                    for fact in &mut event.facts {
                        fact.event = self.source_event(&fact.event)?;
                        fact.kind = match &fact.kind {
                            CommonLoanFactKind::Use {
                                role,
                                mode,
                                target,
                                kind,
                            } => CommonLoanFactKind::Use {
                                role: *role,
                                mode: *mode,
                                target: self.common_target(owner, target)?,
                                kind: *kind,
                            },
                            CommonLoanFactKind::NoLoan { outputs } => CommonLoanFactKind::NoLoan {
                                outputs: outputs
                                    .iter()
                                    .map(|value| self.value(value))
                                    .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?,
                            },
                            kind => kind.clone(),
                        };
                    }
                }
                row.input.pending = row
                    .input
                    .pending
                    .iter()
                    .map(|pending| self.common_pending(pending))
                    .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?;
                row.pending = row
                    .pending
                    .iter()
                    .map(|pending| self.common_pending(pending))
                    .collect::<Result<Vec<_>, ValueSchemaRemapError>>()?;
                for point in &mut row.points {
                    point.event = self.source_event(&point.event)?;
                }
                for candidate in &mut row.candidates {
                    candidate.event = self.source_event(&candidate.event)?;
                    candidate.target = self.common_target(owner, &candidate.target)?;
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
    fn common_loan_remap_preserves_dense_links_and_qualified_incidence() {
        let cfg = crate::scalar::common_loan::tests::cfg("%%start\nunit() inspect = fn { u32 value = 1; *u32 root = &value; *u32 child = &(*root); *child; };\n%%end", ScalarTargetLayout::NATIVE64, false);
        let owner = cfg.producer_owner.clone();
        let imported = ProducerCfgOwnerId::Module(SourceIdentity::new(
            "imported-common".into(),
            "app".into(),
            "src/import.w".into(),
            "r1".into(),
        ));
        let bindings = cfg
            .evaluated_values
            .source_binding_shapes
            .iter()
            .map(|fact| {
                let mut binding = fact.binding.clone();
                binding.source = SourceIdentity::new(
                    "imported-common".into(),
                    "app".into(),
                    "src/import.w".into(),
                    "r1".into(),
                );
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
        let rows = remap
            .common_loans(&cfg.evaluated_values)
            .expect("total common remap");
        let [original] = cfg.evaluated_values.common_loans.as_slice() else {
            panic!("one original epoch");
        };
        let [mapped] = rows.as_slice() else {
            panic!("one mapped epoch");
        };
        assert_eq!(mapped.input.owner, imported);
        assert_eq!(original.subset, mapped.subset);
        for (original, mapped) in original.input.roles.iter().zip(&mapped.input.roles) {
            assert_eq!(mapped.owner, imported);
            if let CommonLoanSourceRole::Binding(binding) = &original.source {
                assert_eq!(
                    mapped.source,
                    CommonLoanSourceRole::Binding(
                        remap.binding(&owner, binding).expect("qualified binding")
                    )
                );
            }
            if let CommonLoanSourceRole::Evaluated(value) = &original.source {
                assert_eq!(
                    mapped.source,
                    CommonLoanSourceRole::Evaluated(
                        remap.value(value).expect("qualified evaluated output")
                    )
                );
            }
        }
        for (original, mapped) in original.points.iter().zip(&mapped.points) {
            assert_eq!(
                mapped.event,
                remap.source_event(&original.event).expect("event")
            );
            assert_eq!(original.incoming, mapped.incoming);
            assert_eq!(original.outgoing, mapped.outgoing);
            assert_eq!(original.may_live, mapped.may_live);
        }
        for (original, mapped) in original.input.sites.iter().zip(&mapped.input.sites) {
            assert_eq!(
                mapped.event,
                remap.source_event(&original.event).expect("qualified site")
            );
            assert_eq!(
                mapped.target,
                remap
                    .common_target(&owner, &original.target)
                    .expect("qualified target")
            );
            assert_eq!(original.source_span, mapped.source_span);
        }
        for (original, mapped) in original.candidates.iter().zip(&mapped.candidates) {
            assert_eq!(
                mapped.event,
                remap
                    .source_event(&original.event)
                    .expect("qualified candidate")
            );
            assert_eq!(mapped.sites, original.sites);
            assert_eq!(mapped.role, original.role);
        }
    }
}
