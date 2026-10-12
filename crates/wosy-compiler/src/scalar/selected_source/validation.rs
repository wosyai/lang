use super::*;

fn instance(tables: &EvaluatedValueTables, id: &SourceInstanceRef) -> bool {
    match id {
        SourceInstanceRef::Supported(id) => tables
            .source_instances
            .get(id.index)
            .is_some_and(|row| row.id == *id),
        SourceInstanceRef::Residual(id) => tables
            .source_populations
            .get(id.index)
            .is_some_and(|row| row.id == *id),
    }
}

pub(in crate::scalar) fn validate(cfg: &ScalarReferenceCfg) -> Result<(), ReferenceAnalysisError> {
    backing_producer::validate(cfg)?;
    selected_grant::validate(cfg)?;
    source_use::validate(cfg)?;
    local_loan::validate(cfg)?;
    let tables = &cfg.evaluated_values;
    let source = &cfg.points[0].source_span;
    for row in &tables.source_configurations {
        if tables.source_configurations.get(row.id.index) != Some(row)
            || row.frame.owner != row.id.owner
        {
            return Err(invalid(source));
        }
        let mut coordinates = BTreeSet::new();
        for slot in &row.slots {
            let coordinate = tables
                .source_slots
                .get(slot.coordinate.index)
                .filter(|coordinate| coordinate.id == slot.coordinate)
                .ok_or_else(|| invalid(source))?;
            let view = tables
                .source_views
                .get(slot.view.index)
                .filter(|view| view.id == slot.view)
                .ok_or_else(|| invalid(source))?;
            if coordinate.shape != view.shape
                || coordinate.id.owner != row.id.owner
                || !coordinates.insert(slot.coordinate.index)
            {
                return Err(invalid(source));
            }
            match &view.payload {
                SourcePayloadView::Aggregate(members) => {
                    for member in members {
                        let child = tables
                            .source_views
                            .get(member.view.index)
                            .filter(|child| child.id == member.view)
                            .ok_or_else(|| invalid(source))?;
                        let value = tables
                            .values
                            .iter()
                            .find(|value| value.id == member.source)
                            .ok_or_else(|| invalid(source))?;
                        if child.shape != value.shape {
                            return Err(invalid(source));
                        }
                    }
                }
                SourcePayloadView::Raw { referent, .. }
                | SourcePayloadView::Backing {
                    instance: referent, ..
                } => {
                    if !row.instances.contains(referent) {
                        return Err(invalid(source));
                    }
                }
                SourcePayloadView::Plain
                | SourcePayloadView::Unwritten
                | SourcePayloadView::Formal(_)
                | SourcePayloadView::StaticRaw(_) => {}
                SourcePayloadView::Checked { referents, .. } => {
                    for referent in referents {
                        if !row.instances.contains(&referent.instance)
                            || !tables
                                .source_slots
                                .iter()
                                .any(|slot| slot.id == referent.coordinate)
                        {
                            return Err(invalid(source));
                        }
                    }
                }
            }
        }
        for id in &row.instances {
            if !instance(tables, id) {
                return Err(invalid(source));
            }
        }
        for saved in &row.saved {
            if saved.snapshot.owner != row.id.owner
                || saved.selection.application.frame != row.frame
                || !tables.source_views.iter().any(|view| view.id == saved.view)
                || !matches!(
                    cfg.initial_points[saved.snapshot.point.0].kind,
                    CfgPointKind::AssignmentSnapshot { .. }
                )
            {
                return Err(invalid(source));
            }
        }
        let mut variables = BTreeSet::new();
        for id in &row.instances {
            if let SourceInstanceRef::Supported(id) = id {
                let supported = &tables.source_instances[id.index];
                if supported.frame != row.frame || !variables.insert(supported.variable.0) {
                    return Err(invalid(source));
                }
            }
        }
        for owns in &row.owns {
            let capability = tables
                .owner_capabilities
                .iter()
                .find(|capability| capability.id == owns.capability)
                .ok_or_else(|| invalid(source))?;
            if !row.instances.contains(&owns.owner_instance)
                || !row.instances.contains(&owns.payload)
                || !capability.endpoints.contains(&owns.endpoint)
                || owns.endpoint.frame != row.frame
            {
                return Err(invalid(source));
            }
            match &owns.owner_instance {
                SourceInstanceRef::Supported(id)
                    if tables.source_instances[id.index].role
                        == SourceObjectRole::Owner(owns.capability.clone()) => {}
                _ => return Err(invalid(source)),
            }
        }
    }
    for row in &tables.source_contributors {
        let input = tables
            .source_configurations
            .get(row.input.index)
            .filter(|input| input.id == row.input)
            .ok_or_else(|| invalid(source))?;
        let output = tables
            .source_configurations
            .get(row.output.index)
            .filter(|output| output.id == row.output)
            .ok_or_else(|| invalid(source))?;
        if tables.source_contributors.get(row.id.index) != Some(row)
            || row.id.owner != row.event.owner
            || row.input.owner != row.id.owner
            || row.output.owner != row.id.owner
        {
            return Err(invalid(source));
        }
        for preserved in &row.preserved {
            if !input.instances.contains(preserved) || !output.instances.contains(preserved) {
                return Err(invalid(source));
            }
        }
        for fresh in &row.fresh {
            if input.instances.contains(fresh) || !output.instances.contains(fresh) {
                return Err(invalid(source));
            }
        }
        for child in &row.children {
            if child.configuration != row.input || child.event.owner != row.event.owner {
                return Err(invalid(source));
            }
        }
    }
    for path in &tables.source_paths {
        let configuration = tables
            .source_configurations
            .get(path.configuration.index)
            .filter(|configuration| configuration.id == path.configuration)
            .ok_or_else(|| invalid(source))?;
        let contributor = tables
            .source_contributors
            .get(path.contributor.index)
            .filter(|contributor| contributor.id == path.contributor)
            .ok_or_else(|| invalid(source))?;
        if tables.source_paths.get(path.id.ordinal) != Some(path)
            || contributor.output != path.configuration
            || contributor.event != path.event
            || configuration.frame != path.frame
            || path.id.producer != path.event.owner
        {
            return Err(invalid(source));
        }
        for output in &contributor.outputs {
            let slot = selected_slot(
                tables,
                configuration,
                &SourceCoordinateRole::Evaluated(output.clone()),
            )
            .map_err(|_| invalid(source))?;
            if let SourcePayloadView::Aggregate(members) =
                &tables.source_views[slot.view.index].payload
            {
                if matches!(
                    cfg.initial_points[path.event.point.0].kind,
                    CfgPointKind::Evaluate(
                        ScalarExpression::StructLiteral { .. }
                            | ScalarExpression::ArrayLiteral { .. }
                    )
                ) {
                    for member in members {
                        let child = selected_slot(
                            tables,
                            configuration,
                            &SourceCoordinateRole::Evaluated(member.source.clone()),
                        )
                        .map_err(|_| invalid(source))?;
                        if child.view != member.view {
                            return Err(invalid(source));
                        }
                    }
                }
            }
        }
    }
    for value in &tables.values {
        for case in &value.cases {
            if tables.source_paths.iter().any(|path| path.id == case.path) {
                let path = &tables.source_paths[case.path.ordinal];
                if value.cases.get(case.id.alternative) != Some(case)
                    || case.id.value != value.id
                    || path.event.point != value.id.point
                    || path.event.owner != value.id.resolved.producer
                {
                    return Err(invalid(&value.id.resolved.occurrence));
                }
            }
        }
    }
    for selected in &tables.source_case_views {
        let value = tables
            .values
            .iter()
            .find(|value| value.id == selected.case.value)
            .ok_or_else(|| invalid(source))?;
        let view = tables
            .source_views
            .get(selected.view.index)
            .filter(|view| view.id == selected.view)
            .ok_or_else(|| invalid(source))?;
        if value
            .cases
            .get(selected.case.alternative)
            .is_none_or(|case| case.id != selected.case)
            || value.shape != view.shape
        {
            return Err(invalid(source));
        }
    }
    for monitor in &tables.source_port_monitors {
        let requested = monitor
            .ports
            .iter()
            .map(|port| port.coordinate.clone())
            .collect::<Vec<_>>();
        port_monitor::query(tables, &monitor.id, &requested, source)?;
    }
    for saved in &tables.source_saved_ports {
        let selected = port_monitor::query(
            tables,
            &saved.monitor,
            std::slice::from_ref(&saved.selected.coordinate),
            source,
        )?;
        if selected != vec![saved.selected.clone()]
            || !matches!(&tables.source_slots[saved.selected.coordinate.index].role, SourceCoordinateRole::Saved(selection) if *selection == saved.selection)
        {
            return Err(invalid(source));
        }
        let monitor = &tables.source_port_monitors[saved.monitor.index];
        let rule = &tables.source_contributors[monitor.contributor.index];
        if rule.event != saved.snapshot
            || !tables.source_configurations[rule.output.index]
                .saved
                .iter()
                .any(|row| {
                    row.selection == saved.selection
                        && row.snapshot == saved.snapshot
                        && row.coordinate == saved.selected.coordinate
                })
        {
            return Err(invalid(source));
        }
    }
    for pending in &tables.pending_owner_consumers {
        if !pending
            .requirements
            .contains(&OwnerConsumerRequirement::StagedLoanValidation(
                SourceEventId {
                    owner: pending.candidate.consumer.producer.clone(),
                    point: pending.candidate.consumer.point,
                },
            ))
        {
            return Err(invalid(
                &cfg.initial_points[pending.candidate.consumer.point.0].source_span,
            ));
        }
        let tuple = tables
            .source_contributors
            .get(pending.candidate.contributor.index)
            .filter(|tuple| tuple.id == pending.candidate.contributor)
            .ok_or_else(|| invalid(source))?;
        let configuration = tables
            .source_configurations
            .get(tuple.output.index)
            .filter(|configuration| configuration.id == tuple.output)
            .ok_or_else(|| invalid(source))?;
        owner_consumer::validate_monitored_correspondence(
            tables,
            &cfg.initial_points,
            configuration,
            &pending.candidate,
            &pending.monitor,
        )?;
        let transition = tables
            .owner_receiving_transitions
            .iter()
            .find(|transition| {
                transition.output == configuration.id
                    && transition.receiving == pending.candidate.receiving
                    && transition.dependencies == pending.candidate.dependencies
            })
            .ok_or_else(|| invalid(source))?;
        if !pending
            .requirements
            .contains(&OwnerConsumerRequirement::RemainingDependencyUses(
                transition.input.clone(),
            ))
            || matches!(
                pending.candidate.disposition.action,
                ValueDispositionAction::MoveTo(_)
            ) && !pending
                .requirements
                .contains(&OwnerConsumerRequirement::SourceMoveWitness(
                    pending.candidate.input.value.clone(),
                ))
        {
            return Err(invalid(source));
        }
        if !pending.requirements.contains(
            &OwnerConsumerRequirement::CompleteSourcePredicateProduct(configuration.id.clone()),
        ) || !pending.requirements.contains(
            &OwnerConsumerRequirement::FiniteSupportNormalization(configuration.id.clone()),
        ) {
            return Err(invalid(source));
        }
    }
    for input in &tables.disposition_inputs {
        if tables
            .disposition_inputs
            .iter()
            .filter(|other| other.consumer == input.consumer)
            .count()
            != 1
        {
            return Err(invalid(source));
        }
        match &input.consumer.kind {
            ValueConsumerKind::FinalOutput { .. } => {
                if !tables.final_inputs.iter().any(|port| port.source == input.source && port.consumer == input.consumer) {
                    return Err(invalid(source));
                }
            }
            ValueConsumerKind::Binding { binding, projection } => {
                let ports = tables.binding_inputs.iter().filter(|port| port.producer == input.consumer.producer
                    && port.consumer == input.consumer.point && port.source == input.source).collect::<Vec<_>>();
                let [port] = ports.as_slice() else { return Err(invalid(source)); };
                if !projection.is_empty() || !matches!(&cfg.initial_points.get(port.consumer.0).ok_or_else(|| invalid(source))?.kind,
                    CfgPointKind::Bind { binding: actual } if actual == binding) { return Err(invalid(source)); }
                reference::binding_disposition(tables, port, binding)?;
            }
            _ => return Err(invalid(source)),
        }
    }
    for witness in &tables.source_move_witnesses {
        selected_move::validate(tables, &cfg.initial_points, &witness.id, source)?;
    }
    for row in &tables.source_configurations {
        for view in &row.owner_views {
            let slot = row
                .slots
                .iter()
                .find(|slot| slot.coordinate == view.coordinate)
                .ok_or_else(|| invalid(source))?;
            let instances = owner_consumer::selected_instances(tables, row, &slot.view, source)?;
            if !instances.contains(&(view.projection.clone(), view.payload.clone()))
                || !row.owns.iter().any(|owns| {
                    owns.payload == view.payload
                        && owns.capability == view.capability
                        && owns.owner_instance == view.owner_instance
                        && owns.state == SourceObligationState::Outstanding
                })
            {
                return Err(invalid(source));
            }
        }
    }
    Ok(())
}
