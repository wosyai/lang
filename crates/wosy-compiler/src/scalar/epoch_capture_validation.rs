use super::*;

#[cfg(test)]
mod selected_proofs {
    use super::*;

    #[test]
    fn template_actual_deferred_and_selected_concrete_policy_retains_template_identity() {
        let source = SourceIdentity::new("template".into(), "selected".into(), "main.w".into(), "r1".into());
        let parsed = crate::parse_source(source, "%%start\ngeneric T;\nunit(T) run = fn(item) { unit() read = fn { item; }; read(); };\nunit() invoke = fn { run<u32>(1); };\n%%end".into(), &[]);
        assert!(parsed.diagnostics.is_empty());
        let (validated, trace) = derive_scalar_program_observed_test(&parsed.result, ScalarTargetLayout::NATIVE64);
        assert!(validated.diagnostics.is_empty(), "{:?}", validated.diagnostics);
        let module = ScalarModule::from_program(validated.program, Vec::new());
        let forest = source_boundary::module_preflight_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums, SourceCaptureConstructionPhase::Complete).unwrap();
        ValueSchemaRemap::validate_construction_forest(&forest).unwrap();
        let generic = forest.sources.iter().find(|row| row.policy_context.generic_parameters.contains("T") && !row.tables.source_capture_productions.is_empty()).unwrap();
        assert!(generic.tables.source_capture_productions.iter().flat_map(|row| &row.inputs).any(|port| port.policy == CopyPolicy::Deferred));
        let original_templates = generic.tables.shapes.iter().filter(|row| matches!(row.payload, ValuePayloadShape::TemplateParameter { .. })).collect::<Vec<_>>();
        assert!(!original_templates.is_empty());
        let pass = trace.passes.iter().find(|pass| matches!(pass.role, ReferenceTransferTestRole::SpecializedCopy { .. }) && !pass.cfg_at_entry.evaluated_values.source_capture_policy_instances.is_empty()).unwrap();
        source_boundary::validate_capture_preflight(&pass.cfg_at_entry, &pass.context.capture_input).unwrap();
        assert!(pass.cfg_at_entry.evaluated_values.source_capture_policy_instances.iter().any(|instance| instance.ty == ScalarType::U32 && instance.policy == CopyPolicy::Copy && instance.substitutions.get("T") == Some(&ScalarType::U32)));
        for template in original_templates {
            assert!(pass.cfg_at_entry.evaluated_values.shapes.iter().any(|row| row.ty == template.ty && row.payload == template.payload));
        }
    }
}

impl SourceEpoch {
    pub(super) fn validate_template_graph(&self, contexts: &[SourceCapturePolicyContext]) -> Result<(), SourceEpochError> {
        for shape in &self.shapes {
            let context = Self::lexical_context(&shape.id.producer, contexts)?;
            evaluated_value::template::validate_graph_node(shape, &self.shapes, context)
                .map_err(|_| SourceEpochError::UnresolvedShapeType { shape: shape.id.clone(), ty: shape.ty.clone() })?;
        }
        Ok(())
    }

    pub(in crate::scalar) fn lexical_context<'a>(owner: &ProducerCfgOwnerId, contexts: &'a [SourceCapturePolicyContext]) -> Result<&'a SourceCapturePolicyContext, SourceEpochError> {
        let mut scope = owner;
        while let ProducerCfgOwnerId::Literal { enclosing, .. } = scope { scope = enclosing; }
        let rows = contexts.iter().filter(|row| row.generic_scope == *scope).collect::<Vec<_>>();
        let [context] = rows.as_slice() else { return Err(SourceEpochError::MissingLexicalOwner {
            event: SourceEventId { owner: owner.clone(), point: CfgPointId(0) }, owner: scope.clone(),
        }); };
        if !evaluated_value::template::valid_context(context) { return Err(SourceEpochError::MissingLexicalOwner {
            event: SourceEventId { owner: owner.clone(), point: CfgPointId(0) }, owner: scope.clone(),
        }); }
        Ok(context)
    }

    pub(super) fn validate_capture_closure(&self, structs: &[ScalarStruct], enums: &[ScalarEnum], contexts: &[SourceCapturePolicyContext]) -> Result<(), SourceEpochError> {
        for capture in &self.captures {
            let context = Self::lexical_context(&capture.event.owner, contexts)?;
            let mut roots = vec![&capture.signature];
            for port in &capture.inputs { roots.extend([&port.value_shape, &port.place_shape, &port.formal_shape]); }
            roots.extend(capture.parameters.iter().map(|port| &port.shape));
            let mut visited = Vec::new();
            for root in roots {
                let shape = self.shapes.iter().find(|row| row.id == *root)
                    .ok_or_else(|| SourceEpochError::MissingShapeReference { shape: root.clone(), target: root.clone() })?;
                Self::canonical_shape(&self.shapes, root, &shape.ty, context, structs, enums, &mut visited)?;
            }
        }
        Ok(())
    }

    pub(in crate::scalar) fn canonical_shape(shapes: &[ResolvedValueShape], id: &ValueShapeId, ty: &ScalarType, context: &SourceCapturePolicyContext, structs: &[ScalarStruct], enums: &[ScalarEnum], visited: &mut Vec<(ValueShapeId, ScalarType)>) -> Result<(), SourceEpochError> {
        let fail = || SourceEpochError::UnresolvedShapeType { shape: id.clone(), ty: ty.clone() };
        let rows = shapes.iter().filter(|row| row.id == *id).collect::<Vec<_>>();
        let [shape] = rows.as_slice() else { return Err(fail()); };
        if shape.ty != *ty { return Err(fail()); }
        if visited.iter().any(|(known, expected)| known == id && expected == ty) { return Ok(()); }
        visited.push((id.clone(), ty.clone()));
        match (ty, &shape.payload) {
            (ScalarType::Named { .. }, ValuePayloadShape::TemplateParameter { .. }) => {
                if !evaluated_value::template::valid_node(shape, context) { return Err(fail()); }
            }
            (ScalarType::RawPointer(inner), ValuePayloadShape::Raw { pointee }) => Self::canonical_child(shapes, id, pointee, inner, context, structs, enums, visited)?,
            (ScalarType::CheckedReference { mutability, inner }, ValuePayloadShape::Checked { mutability: actual, referent }) if actual == mutability => Self::canonical_child(shapes, id, referent, inner, context, structs, enums, visited)?,
            (ScalarType::Array { element: inner, length, .. }, ValuePayloadShape::FixedArray { element, length: actual }) if actual == length => Self::canonical_child(shapes, id, element, inner, context, structs, enums, visited)?,
            (ScalarType::RuntimeArray { element: inner, .. }, ValuePayloadShape::RuntimeArray { element }) => Self::canonical_child(shapes, id, element, inner, context, structs, enums, visited)?,
            (ScalarType::Callable { parameters, outputs }, ValuePayloadShape::Callable { signature }) if signature == ty => {
                for ty in parameters.iter().chain(outputs.outputs.iter().map(|row| &row.ty)) {
                    let rows = shapes.iter().filter(|row| row.id.producer == id.producer && row.ty == *ty).collect::<Vec<_>>();
                    let [child] = rows.as_slice() else { return Err(fail()); };
                    Self::canonical_shape(shapes, &child.id, ty, context, structs, enums, visited)?;
                }
            }
            (ScalarType::Struct(id), ValuePayloadShape::Struct { structure, fields }) if id == structure => {
                let supplied = structs.iter().filter(|row| row.id == *id).collect::<Vec<_>>();
                let originals = context.structs.iter().filter(|row| row.id == *id).collect::<Vec<_>>();
                let ([declared], [original]) = (supplied.as_slice(), originals.as_slice()) else { return Err(fail()); };
                if declared != original || fields.len() != declared.fields.len() { return Err(fail()); }
                for (field, declared) in fields.iter().zip(&declared.fields) {
                    if field.field != declared.id || declared.id.structure != *id
                        || fields.iter().filter(|row| row.field == field.field).count() != 1 { return Err(fail()); }
                    Self::canonical_child(shapes, &shape.id, &field.shape, &declared.ty, context, structs, enums, visited)?;
                }
            }
            (ScalarType::Enum(id), ValuePayloadShape::Enum { enumeration, variants }) if id == enumeration => {
                let supplied = enums.iter().filter(|row| row.id == *id).collect::<Vec<_>>();
                let originals = context.enums.iter().filter(|row| row.id == *id).collect::<Vec<_>>();
                let ([declared], [original]) = (supplied.as_slice(), originals.as_slice()) else { return Err(fail()); };
                if declared != original || variants.len() != declared.variants.len() { return Err(fail()); }
                for (variant, declared) in variants.iter().zip(&declared.variants) {
                    if variant.ordinal != declared.tag as usize || !variant.fields.is_empty()
                        || variants.iter().filter(|row| row.ordinal == variant.ordinal).count() != 1 { return Err(fail()); }
                }
            }
            (ty, ValuePayloadShape::Plain(kind)) if matches!((ty, kind),
                (ScalarType::Unit, PlainValueKind::Unit) | (ScalarType::Bool, PlainValueKind::Bool) | (ScalarType::Char, PlainValueKind::Char) | (ScalarType::ArtifactId, PlainValueKind::ArtifactId)
                | (ScalarType::I8, PlainValueKind::I8) | (ScalarType::I16, PlainValueKind::I16) | (ScalarType::I32, PlainValueKind::I32) | (ScalarType::I64, PlainValueKind::I64) | (ScalarType::I128, PlainValueKind::I128)
                | (ScalarType::U8, PlainValueKind::U8) | (ScalarType::U16, PlainValueKind::U16) | (ScalarType::U32, PlainValueKind::U32) | (ScalarType::U64, PlainValueKind::U64) | (ScalarType::U128, PlainValueKind::U128)
                | (ScalarType::F32, PlainValueKind::F32) | (ScalarType::F64, PlainValueKind::F64)) => {}
            _ => return Err(fail()),
        }
        Ok(())
    }

    fn canonical_child(shapes: &[ResolvedValueShape], parent: &ValueShapeId, child: &ValueShapeId, ty: &ScalarType, context: &SourceCapturePolicyContext, structs: &[ScalarStruct], enums: &[ScalarEnum], visited: &mut Vec<(ValueShapeId, ScalarType)>) -> Result<(), SourceEpochError> {
        if parent.producer != child.producer {
            return Err(SourceEpochError::UnresolvedShapeType { shape: child.clone(), ty: ty.clone() });
        }
        Self::canonical_shape(shapes, child, ty, context, structs, enums, visited)
    }
}
