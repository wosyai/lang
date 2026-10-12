use super::memory::EvaluatedCallableSelection;
use super::*;

pub(super) enum FrozenAssignmentDestination<'ctx> {
    Unit,
    Storage {
        address: PointerValue<'ctx>,
        selection: FrozenAssignmentSelection<'ctx>,
    },
}

pub(super) enum FrozenAssignmentSelection<'ctx> {
    Ordinary,
    Callable(EvaluatedCallableSelection<'ctx>),
}

pub(super) fn freeze_assignment_destination<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    assignment: &ScalarAssignment,
    position: usize,
    ty: &ScalarType,
    project: Option<ProjectCallScope<'_>>,
) -> Result<FrozenAssignmentDestination<'ctx>, String> {
    let target = &assignment.targets[position];
    if *ty == ScalarType::Unit {
        return Ok(FrozenAssignmentDestination::Unit);
    }
    if matches!(
        (&target.place, ty),
        (
            crate::ScalarPlace::Index { .. },
            ScalarType::Callable { .. }
        )
    ) {
        let (selected, index_point) = state
            .holder_events
            .selected_assignment(assignment, position)?;
        let binding = state
            .holder_events
            .assignment_binding(assignment, position)?;
        let selection = evaluate_callable_selection(
            context,
            state,
            &target.place,
            &binding,
            selected,
            index_point,
            project,
        )?;
        ownership::begin_graph_mutation(&state.builder, state.module)?;
        let address = resolve_callable_selection(context, state, &selection)?;
        ownership::end_graph_mutation(&state.builder, state.module)?;
        return Ok(FrozenAssignmentDestination::Storage {
            address,
            selection: FrozenAssignmentSelection::Callable(selection),
        });
    }
    if let Some(project) = project {
        if let Some(receiver) = target.receiver.as_ref().filter(|receiver| {
            project
                .module
                .namespace_bindings
                .iter()
                .any(|namespace| namespace.binding == **receiver)
        }) {
            let target_module =
                project_namespace_target(project.module, project.modules, receiver)?;
            let (global, _) = project_member_global(state, target_module, &target.target)?;
            let address = global
                .ok_or_else(|| format!("unknown LLVM member storage {}", target.target))?
                .as_pointer_value();
            return Ok(FrozenAssignmentDestination::Storage {
                address,
                selection: FrozenAssignmentSelection::Ordinary,
            });
        }
    }
    let address = match &target.place {
        crate::ScalarPlace::Name { name, .. } => {
            match state.storage.get(name).map(|(slot, _)| *slot).or_else(|| {
                state
                    .globals
                    .get(name)
                    .map(|(global, _)| global.as_pointer_value())
            }) {
                Some(destination) => destination,
                None => {
                    let slot = entry_alloca(
                        state,
                        storage_type(context, ty, state.structs, state.target_layout)?,
                        name,
                    )?;
                    state.storage.insert(name.clone(), (slot, ty.clone()));
                    slot
                }
            }
        }
        place => place_pointer(context, state, place, project)?,
    };
    Ok(FrozenAssignmentDestination::Storage {
        address,
        selection: FrozenAssignmentSelection::Ordinary,
    })
}

#[cfg(test)]
mod tests;
