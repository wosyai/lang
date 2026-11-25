use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write;
use std::mem::ManuallyDrop;
use std::num::NonZeroU32;

use inkwell::builder::{Builder, BuilderError};
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, FunctionType, StructType};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValueEnum, FunctionValue, GlobalValue, PointerValue, StructValue,
    ValueKind,
};
use inkwell::AddressSpace;
use inkwell::{FloatPredicate, IntPredicate};
use serde::{Deserialize, Serialize};

use crate::scalar::control::{
    CallableBodyCallNodes, InitializerCallNodes, ModuleInitializationCallTarget,
    ModuleInitializationRegion, ModuleInitializationStepKind, NamespaceTarget,
};
use crate::scalar::layout::ScalarTargetLayouts;
use crate::scalar::{
    CfgPointKind, CoreOperationId, InvalidationCandidate, InvalidationInputs, ReferenceBindingKind,
    ReferenceOrigin, ReleaseTarget, ResolvedCallTarget, ScalarAllocationIdentity,
    ScalarAutomaticReturnResult, ScalarAutomaticReturnResultState, ScalarBindingOutput,
    ScalarBindingOutputOrigin, ScalarFieldReference, ScalarOverloadSelection, ScalarReferenceCfg,
};
use crate::{
    BinaryOperator, ScalarAssignment, ScalarBlock, ScalarBlockItem, ScalarBuildValidation,
    ScalarExpression, ScalarFunction, ScalarItem, ScalarModule, ScalarProject,
    ScalarProjectValidation, ScalarStruct, ScalarTargetLayout, ScalarType, ScalarValidation,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LlvmValueType {
    Void,
    Aggregate(Vec<LlvmValueType>),
    InlineBytes(u32),
    I1,
    I16,
    I32,
    I8,
    I64,
    I128,
    F32,
    F64,
    Char,
    ArtifactId,
    Pointer,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LlvmPartition {
    pub module_name: String,
    pub declarations: Vec<LlvmFunction>,
    pub functions: Vec<LlvmFunction>,
    text: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LlvmFunction {
    pub name: String,
    pub result: LlvmValueType,
    pub parameters: Vec<(String, LlvmValueType)>,
    pub attributes: Vec<LlvmFunctionAttributes>,
    pub body: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LlvmFunctionAttributes {
    pub group: u32,
    pub wasm_import_module: String,
    pub wasm_import_name: String,
}

#[derive(Clone)]
enum EmitValue<'ctx> {
    Unit,
    Basic(BasicValueEnum<'ctx>),
    Aggregate {
        value: StructValue<'ctx>,
        outputs: Vec<ScalarType>,
    },
}

type GlobalStorage<'ctx> = (GlobalValue<'ctx>, ScalarType);
struct GlobalNames<'ctx, 'module>(BTreeMap<&'module str, &'module GlobalStorage<'ctx>>);

impl<'ctx, 'module> GlobalNames<'ctx, 'module> {
    fn get(&self, name: &str) -> Option<&'module GlobalStorage<'ctx>> {
        self.0.get(name).copied()
    }
}

struct ModuleNames<'ctx, 'module> {
    globals: GlobalNames<'ctx, 'module>,
    unit_values: BTreeSet<&'module str>,
    members: BTreeMap<&'module str, MemberName<'module>>,
    namespaces: BTreeMap<&'module str, &'module wosy_syntax::SourceIdentity>,
    externs: BTreeMap<&'module str, &'module crate::scalar::ScalarExtern>,
}

struct MemberName<'module> {
    ty: &'module ScalarType,
    unsigned: bool,
}

struct CheckedAccessSymbols {
    non_null: String,
    in_extent: String,
}

struct ProjectNames<'ctx, 'module> {
    scopes: Vec<ModuleNames<'ctx, 'module>>,
    positions: HashMap<&'module wosy_syntax::SourceIdentity, usize>,
    checked_access: Option<CheckedAccessSymbols>,
    private_runtime_portable_symbols: std::collections::HashSet<String>,

    require_release_checked_access: bool,

}

impl<'ctx, 'module> ProjectNames<'ctx, 'module> {
    fn for_module(&self, module: &ScalarModule) -> &ModuleNames<'ctx, 'module> {
        &self.scopes[self.positions[&module.source]]
    }
}

#[derive(Clone, Copy)]
enum LocalRuntimeExtent<'ctx> {
    Construction {
        length: inkwell::values::IntValue<'ctx>,
        signed: Option<memory::SignedRuntimeAddressDomain>,
    },
    PortableConstruction {
        length: inkwell::values::IntValue<'ctx>,
        domain: memory::PortableRuntimeAddressDomain,
    },
    OrdinaryDescriptor {
        length: inkwell::values::IntValue<'ctx>,
        signed: Option<memory::SignedRuntimeAddressDomain>,
    },
    WidenedLiteral {
        length: inkwell::values::IntValue<'ctx>,
        signed: Option<memory::SignedRuntimeAddressDomain>,
    },
    OwningFormal(inkwell::values::IntValue<'ctx>),
    ManagedLengthSlot(PointerValue<'ctx>),
}

#[derive(Default)]
struct RuntimeBorrowIndex<'ctx> {
    sites: HashMap<usize, (wosy_syntax::ByteSpan, Option<wosy_syntax::ByteSpan>)>,
    portable_places: std::collections::HashSet<usize>,
    field_sites: HashMap<usize, crate::scalar::RuntimeArrayFieldSite>,
    owners: HashMap<wosy_syntax::ByteSpan, Option<PointerValue<'ctx>>>,
}

type StorageBinding<'ctx> = (PointerValue<'ctx>, ScalarType);

#[derive(Default)]
struct StorageBindings<'ctx> {
    entries: BTreeMap<String, StorageBinding<'ctx>>,
    undo: Vec<(String, Option<StorageBinding<'ctx>>)>,
    scoped: bool,
}

struct StorageScope {
    undo_start: usize,
    previously_scoped: bool,
}

impl<'ctx> StorageBindings<'ctx> {
    fn get(&self, name: &str) -> Option<&StorageBinding<'ctx>> {
        self.entries.get(name)
    }

    fn insert(&mut self, name: String, binding: StorageBinding<'ctx>) {
        if self.scoped {
            let undo_name = name.clone();
            let previous = self.entries.insert(name, binding);
            self.undo.push((undo_name, previous));
        } else {
            self.entries.insert(name, binding);
        }
    }

    fn enter_scope(&mut self) -> StorageScope {
        StorageScope {
            undo_start: self.undo.len(),
            previously_scoped: std::mem::replace(&mut self.scoped, true),
        }
    }

    fn leave_scope(&mut self, scope: StorageScope) {
        while self.undo.len() > scope.undo_start {
            let (name, previous) = self.undo.pop().expect("pending storage change");
            if let Some(binding) = previous {
                self.entries.insert(name, binding);
            } else {
                self.entries.remove(&name);
            }
        }
        self.scoped = scope.previously_scoped;
    }
}

struct EmitState<'ctx, 'module> {
    prepared_render: bool,
    require_release_checked_access: bool,

    aggregate: Option<aggregate_transfer::AggregateEmitter<'ctx, 'module>>,
    initializer_aggregate: Option<aggregate_transfer::InitializerEmitter<'ctx, 'module>>,
    initializer_nodes: Option<&'module InitializerCallNodes<'module>>,
    body_call_nodes: Option<&'module CallableBodyCallNodes<'module>>,
    local_callable_slots: HashMap<(u32, u32), local_callable::LocalCallableSlot<'ctx>>,
    local_callable_arrays: HashMap<wosy_syntax::ByteSpan, local_callable::LocalCallableArray<'ctx>>,
    guarded_scalar: Option<Box<ownership::GuardedScalarSites<'ctx>>>,
    initializer_symbols: Option<&'module InitializerSymbols<'module>>,
    builder: &'ctx Builder<'ctx>,
    module: &'module Module<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    call_targets: &'ctx BTreeMap<String, String>,
    signatures: &'ctx BTreeMap<String, ScalarType>,
    runtime_array_results: &'ctx BTreeMap<String, RuntimeArrayResultProvenance>,
    dynamic_owner_outputs: &'ctx BTreeMap<String, BTreeSet<usize>>,
    values: BTreeMap<String, EmitValue<'ctx>>,
    storage: StorageBindings<'ctx>,
    return_bindings: BTreeMap<String, ScalarExpression>,
    fresh_return_outputs: BTreeSet<usize>,
    branch_return_outputs: ownership::ReturnBranches,
    joined_return_bindings: BTreeMap<String, (PointerValue<'ctx>, ScalarType)>,
    joined_assignment_owners: BTreeMap<(u32, u32), CheckedAssignmentOwner>,
    return_output_types: Vec<ScalarType>,
    checked_call_owners: BTreeMap<(u32, u32, usize), CheckedCallOwner>,
    checked_owner_flags: BTreeMap<String, inkwell::values::IntValue<'ctx>>,
    call_owner_flags: BTreeMap<usize, inkwell::values::IntValue<'ctx>>,
    return_owner_flags: BTreeMap<usize, inkwell::values::IntValue<'ctx>>,
    return_owner_slots: BTreeMap<usize, PointerValue<'ctx>>,
    runtime_array_owners: BTreeMap<String, bool>,
    runtime_array_allocations: BTreeMap<String, bool>,
    runtime_array_lengths: BTreeMap<String, PointerValue<'ctx>>,
    // Exact emitted storage identity: shadowed/unproved bindings cannot
    // reuse this extent by name, and scoped restoration naturally selects
    // only the backing (or paired replacement slot) still installed.
    local_runtime_extents: HashMap<PointerValue<'ctx>, (wosy_syntax::ByteSpan, LocalRuntimeExtent<'ctx>, bool)>,
    indexed_runtime_allocations: BTreeSet<(u32, u32)>,
    portable_runtime_allocations: BTreeSet<(u32, u32)>,
    runtime_borrow_index: RuntimeBorrowIndex<'ctx>,
    observed_runtime_allocations: Option<Box<[(u32, u32)]>>,

    runtime_array_live: BTreeMap<String, PointerValue<'ctx>>,
    mutable_runtime_allocations: BTreeSet<(u32, u32)>,
    invalidations: BTreeMap<
        (u32, u32),
        (
            CoreOperationId,
            Vec<InvalidationCandidate>,
            InvalidationInputs,
        ),
    >,
    automatic_return_result_owners: BTreeMap<String, ScalarAutomaticReturnResult>,
    globals: &'module GlobalNames<'ctx, 'module>,
    all_globals: &'module BTreeMap<String, GlobalStorage<'ctx>>,
    unit_values: &'module BTreeSet<&'module str>,
    project_names: Option<&'module ProjectNames<'ctx, 'module>>,
    structs: &'module [&'module ScalarStruct],
    layouts: &'module crate::scalar::layout::ScalarLayoutIndex<'module>,
    target_layout: ScalarTargetLayout,
    next_literal: usize,
    next_block: usize,
}

#[derive(Clone, Copy)]
enum CheckedCallOwner {
    Fresh,
    Borrowed,
    Mixed,
}

#[derive(Clone, Copy)]
enum CheckedAssignmentOwner {
    Local,
    External,
    Borrowed,
}

fn portable_lexical_runtime_body(
    target: ScalarTargetLayout,
    release: bool,
    function: &ScalarFunction,
    reference_cfg: &ScalarReferenceCfg,
) -> bool {
    // Don't inspect the signature or classify an unrelated scalar Wasm body.
    target == ScalarTargetLayout::WASM32 && release && !reference_cfg.allocations.is_empty()
        && function.generic_parameters.is_empty()
        && matches!(&function.signature, ScalarType::Callable { parameters, outputs }
            if parameters.iter().all(memory::ordinary_scalar_storage)
                && outputs.outputs.iter().all(|output| output.ty == ScalarType::Unit
                    || memory::ordinary_scalar_storage(&output.ty)))
}

type RuntimeAllocationAccess<'ctx> = (
    BTreeSet<(u32, u32)>, BTreeSet<(u32, u32)>, RuntimeBorrowIndex<'ctx>, BTreeSet<(u32, u32)>,
);

fn portable_runtime_plan_allows_plain_storage(plan: &crate::scalar::ClosedBody) -> bool {
    !plan.fallible_allocation_leaf && !plan.guarded_array_descriptor_leaf
        && plan.literal_array.is_none()
}

fn runtime_allocation_access<'ctx>(
    reference_cfg: &ScalarReferenceCfg,
    body_nodes: Option<&CallableBodyCallNodes<'_>>,
    plan: Option<&crate::scalar::ClosedBody>,
    checked_access: bool,
    mut portable_body: bool,
    allow_private_pair_transport: bool,
) -> RuntimeAllocationAccess<'ctx> {
    let mut mutable = BTreeSet::new();
    let mut indexed = BTreeSet::new();
    let mut unproved_writes = BTreeSet::new();
    let mut not_portable = std::collections::HashSet::new();
    let key = |span: wosy_syntax::ByteSpan| (span.start, span.end);
    let mut reborrows = portable_body.then(|| portable_reborrow::Census::new(reference_cfg)).flatten();
    let portable_private_sites = (portable_body && allow_private_pair_transport).then(|| {
        let mut calls = HashMap::new();
        let mut borrows = HashMap::new();
        if let Some(plan) = plan {
            for site in plan.private_runtime_access_calls.values() {
                if let Some((call, borrow)) = site.portable_birth {
                    calls.insert(call, (borrow, site.owner));
                    borrows.insert(borrow, site.owner);
                }
            }
        }
        (calls, borrows)
    });
    let widened_rebind_points = plan.filter(|plan| !plan.widened_rebinds.is_empty())
        .map(|plan| plan.widened_rebinds.values().map(|rebind| rebind.point).collect::<std::collections::HashSet<_>>());
    let widened_owning_install = plan.and_then(|plan| plan.widened_owning_roundtrip).map(|(_, install, _)| install);
    let widened_join_replacements = plan.filter(|plan| !plan.widened_join_replacements.is_empty())
        .map(|plan| plan.widened_join_replacements.values().map(|(_, install, _)| *install)
            .collect::<std::collections::HashSet<_>>());
    let widened_owning_replacements = plan.filter(|plan| !plan.widened_owning_receives.is_empty())
        .map(|plan| plan.widened_owning_receives.values()
            .filter_map(|(_, _, replace)| replace.map(|(install, _)| install))
            .collect::<std::collections::HashSet<_>>());
    let mut proved_extent_writes = HashMap::new();
    if let (true, Some(nodes), Some(plan)) = (checked_access, body_nodes, plan) {
        // This exact install updates pointer and returned extent together.
        if plan.replacement_index_observation.is_some() {
            if let Some(crate::scalar::ClosedLiteralArray::Replacement { call, destination, unit, .. }) = plan.literal_array {
                for (call, destination) in std::iter::once((call, destination)).chain(
                    unit.and_then(|unit| unit.other_writer).map(|writer| (writer.call, writer.destination)))
                {
                    if plan.array_descriptor_calls.contains(&call) {
                        if let Some((producer, binding)) = nodes.existing_array_rebind_event(reference_cfg, call, destination) {
                            proved_extent_writes.insert(producer, binding);
                        }
                    }
                }
            }
        }
        for (&call, &destination) in &plan.same_array_rebinds {
            if let Some((producer, binding)) = nodes.existing_array_rebind_event(reference_cfg, call, destination) {
                proved_extent_writes.insert(producer, binding);
            }
        }
    }
    let record_index = |indexed: &mut BTreeSet<_>, place: &crate::scalar::ReferencePlaceId| {
        if checked_access && matches!(&place.place, crate::ScalarPlace::Index { base, .. }
            if matches!(base.as_ref(), crate::ScalarPlace::Name { .. }))
            && reference_cfg.allocations.contains(&place.binding)
        {
            indexed.insert(key(place.declaration_span));
        }
    };
    for point in &reference_cfg.points {
        // No portable authority is inferred from a type or given to an
        // independently lowered address. Only already sealed private pair
        // calls can transport it out of the exact standard owner.
        if portable_body {
            let exact_reborrow = reborrows.as_mut().is_some_and(|census|
                census.observe(reference_cfg, point, &mut not_portable));
            let mut exclude = |binding: &crate::scalar::ReferenceBindingId| {
                if reference_cfg.allocations.contains(binding) {
                    not_portable.insert(key(binding.declaration_span));
                }
            };
            let direct_index = |place: &crate::scalar::ReferencePlaceId| {
                matches!((&place.place, place.projections.as_slice()),
                    (crate::ScalarPlace::Index { base, .. }, [crate::scalar::ReferencePlaceProjection::Index(_)])
                        if matches!(base.as_ref(), crate::ScalarPlace::Name { .. }))
            };
            match &point.kind {
                _ if exact_reborrow => {}
                CfgPointKind::Read { binding } => exclude(binding),
                CfgPointKind::Assign { target, .. } => {
                    if !direct_index(target) { exclude(&target.binding); }
                }
                CfgPointKind::ReadPlace { pointer_point: None, candidates, .. } => {
                    for place in candidates {
                        if !direct_index(place) { exclude(&place.binding); }
                    }
                }
                CfgPointKind::Borrow { place, loan } => {
                    let exact_length = loan.mode == crate::scalar::ScalarReferenceMutability::Shared
                        && loan.parent.is_none() && place.projections.is_empty()
                        && matches!(place.place, crate::ScalarPlace::Name { .. })
                        && matches!(point.successors.as_slice(), [call] if matches!(
                            reference_cfg.points.get(call.0).map(|next| &next.kind),
                            Some(CfgPointKind::Call {
                                output: 0, target: ResolvedCallTarget::Core(CoreOperationId::ArrayLen),
                                argument_points, ..
                            }) if argument_points.as_slice() == [point.id]));
                    let exact_private = portable_private_sites.as_ref().and_then(|(_, borrows)| borrows.get(&point.id))
                        == Some(&place.binding.declaration_span)
                        && loan.parent.is_none() && place.projections.is_empty()
                        && matches!(place.place, crate::ScalarPlace::Name { .. });
                    if !exact_length && !exact_private { exclude(&place.binding); }
                }
                CfgPointKind::Call { argument_points, .. }
                    if portable_private_sites.as_ref().and_then(|(calls, _)| calls.get(&point.id))
                        .is_some_and(|(borrow, _)| argument_points.first() == Some(borrow)) => {}
                CfgPointKind::Call { target: ResolvedCallTarget::Core(CoreOperationId::ArrayLen), .. }
                | CfgPointKind::Entry | CfgPointKind::ScopeEnter | CfgPointKind::ScopeExit
                | CfgPointKind::Bind { .. } | CfgPointKind::Branch { .. } | CfgPointKind::Join
                | CfgPointKind::ReturnOutput { .. } | CfgPointKind::Return
                | CfgPointKind::ConditionalOutput { .. }
                | CfgPointKind::CaptureAssignmentPrevious { .. } => {}
                CfgPointKind::Evaluate(expression)
                    if !matches!(expression, ScalarExpression::RawAddress { .. }) => {}
                _ => portable_body = false,
            }
        }
        match &point.kind {
            CfgPointKind::Assign { target, rhs_point, .. } if target.projections.is_empty() => {
                if widened_rebind_points.as_ref().is_some_and(|points| points.contains(&point.id))
                    || widened_owning_install == Some(point.id)
                    || widened_join_replacements.as_ref().is_some_and(|points| points.contains(&point.id))
                    || widened_owning_replacements.as_ref().is_some_and(|points| points.contains(&point.id))
                {
                    // This dominating existing-owner transfer re-keys the
                    // original pointer and extent, rather than storing either
                    // into an independently mutable slot.
                    continue;
                }
                let id = key(target.declaration_span);
                mutable.insert(id);
                if checked_access && reference_cfg.allocations.contains(&target.binding)
                    && proved_extent_writes.get(rhs_point) != Some(&&target.binding)
                {
                    unproved_writes.insert(id);
                }
            }
            CfgPointKind::Release {
                target: ReleaseTarget::Checked { candidates, .. },
            }
            | CfgPointKind::Invalidate { candidates, .. } => {
                for candidate in candidates {
                    let id = key(candidate.place.declaration_span);
                    mutable.insert(id);
                    if checked_access && reference_cfg.allocations.contains(&candidate.place.binding) {
                        unproved_writes.insert(id);
                    }
                }
            }
            CfgPointKind::Assign { target, .. } => record_index(&mut indexed, target),
            CfgPointKind::Borrow { place, .. } => record_index(&mut indexed, place),
            CfgPointKind::ReadPlace { pointer_point: None, candidates, .. } => {
                for place in candidates { record_index(&mut indexed, place); }
            }
            _ => {}
        }
    }
    if let Some(plan) = plan {
        for &owner in &plan.private_runtime_access_owners {
            let id = key(owner);
            indexed.insert(id);
        }
        for site in plan.private_runtime_access_calls.values() {
            if !allow_private_pair_transport || site.portable_birth.is_none() {
                not_portable.insert(key(site.owner));
            }
        }
    }
    indexed.retain(|id| !unproved_writes.contains(id));
    let mut borrowed = RuntimeBorrowIndex::default();
    if checked_access {
        for (&point, site) in &reference_cfg.runtime_array_field_sites {
            if !reference_cfg.runtime_array_field_site(point, site) { continue; }
            let id = key(site.owner.declaration_span);
            if mutable.contains(&id) { continue; }
            indexed.insert(id);
            not_portable.insert(id);
            borrowed.field_sites.insert(site.leaf, site.clone());
        }
        for (&point, site) in &reference_cfg.runtime_array_read_sites {
            let Some(owner) = reference_cfg.runtime_array_read_origin(point) else { continue; };
            let declaration = owner.declaration_span;
            // A general mutable pointer slot is not an extent receipt.
            if mutable.contains(&key(declaration)) { continue; }
            indexed.insert(key(declaration));
            borrowed.sites.insert(site.occurrence, (declaration, site.handle));
            not_portable.insert(key(declaration));
            borrowed.owners.entry(declaration).or_insert(None);
        }
        for (&point, site) in &reference_cfg.runtime_array_exclusive_sites {
            let Some(owner) = reference_cfg.runtime_array_exclusive_origin(point, site) else { continue; };
            let declaration = owner.declaration_span;
            if mutable.contains(&key(declaration)) { continue; }
            indexed.insert(key(declaration));
            if reborrows.as_ref().is_some_and(|census| census.contains_direct(point)) {
                borrowed.portable_places.insert(site.occurrence);
            } else {
                not_portable.insert(key(declaration));
            }
            borrowed.sites.insert(site.occurrence, (declaration, Some(site.handle)));
            borrowed.owners.entry(declaration).or_insert(None);
        }
        for site in &reference_cfg.runtime_array_child_sites {
            let Some(owner) = reference_cfg.runtime_array_child_origin(site) else { continue; };
            let declaration = owner.declaration_span;
            if mutable.contains(&key(declaration)) { continue; }
            indexed.insert(key(declaration));
            if !reborrows.as_ref().is_some_and(|census| census.contains(reference_cfg, site)) {
                not_portable.insert(key(declaration));
            } else {
                borrowed.portable_places.insert(site.occurrence);
            }
            borrowed.sites.insert(site.occurrence, (declaration, Some(site.handle)));
            borrowed.owners.entry(declaration).or_insert(None);
        }
    }
    let portable = if portable_body {
        indexed.iter().copied().filter(|key| !mutable.contains(key) && !not_portable.contains(key)).collect()
    } else { BTreeSet::new() };
    (mutable, indexed, borrowed, portable)
}

fn resolved_invalidations(
    reference_cfg: &ScalarReferenceCfg,
) -> Result<
    BTreeMap<
        (u32, u32),
        (
            CoreOperationId,
            Vec<InvalidationCandidate>,
            InvalidationInputs,
        ),
    >,
    String,
> {
    let mut events = BTreeMap::new();
    for point in &reference_cfg.points {
        let (operation, candidates, inputs) = match &point.kind {
            CfgPointKind::Invalidate {
                operation,
                candidates,
                inputs,
                ..
            } => (*operation, candidates, inputs.clone()),
            CfgPointKind::Release {
                target: ReleaseTarget::Checked { candidates, .. },
            } => (CoreOperationId::Free, candidates, InvalidationInputs::Free),
            _ => continue,
        };
        if !reference_cfg.points.iter().any(|call| {
            call.source_span == point.source_span && matches!(&call.kind,
                CfgPointKind::Call { target: ResolvedCallTarget::Core(target), .. } if *target == operation)
        }) {
            return Err("core invalidation lacks a resolved call target".to_owned());
        }
        if candidates.is_empty() {
            return Err("core invalidation lacks a resolved allocation place".to_owned());
        }
        let range = point.source_span.range;
        events.insert(
            (range.start, range.end),
            (operation, candidates.clone(), inputs),
        );
    }
    Ok(events)
}

#[derive(Clone, Copy)]
enum RuntimeArrayResultProvenance {
    LocalOwner,
    ParameterAlias,
    FixedWidening,
    FreshCall,
    CheckedReceipt,
}

impl RuntimeArrayResultProvenance {
    fn owns(self) -> bool {
        matches!(self, Self::LocalOwner | Self::FreshCall)
    }
}

#[derive(Clone)]
struct RuntimeArrayAssignmentProvenance {
    result: RuntimeArrayResultProvenance,
    source_name: Option<String>,
}

#[derive(Clone, Copy)]
struct ProjectCallScope<'a> {
    module: &'a ScalarModule,
    modules: &'a [&'a ScalarModule],
}

struct LlvmExternIdentity {
    internal_name: String,
    import_module: String,
    import_name: String,
    signature: ScalarType,
}

const CORE_ALLOC_SYMBOL: &str = "__wosy_core_alloc";
const CORE_FREE_SYMBOL: &str = "__wosy_core_free";
const CORE_SYSTEM_PANIC_SYMBOL: &str = "__wosy_core_system_panic";

fn declare_core_runtime<'ctx>(context: &'ctx Context, module: &Module<'ctx>) {
    let pointer = context.ptr_type(AddressSpace::default());
    let u64_type = context.i64_type();
    module.add_function(
        CORE_ALLOC_SYMBOL,
        pointer.fn_type(&[u64_type.into(), u64_type.into()], false),
        None,
    );
    module.add_function(
        CORE_FREE_SYMBOL,
        context.void_type().fn_type(&[pointer.into()], false),
        None,
    );
    module.add_function(
        CORE_SYSTEM_PANIC_SYMBOL,
        context.void_type().fn_type(&[], false),
        None,
    );
}

#[derive(Clone, Copy)]
enum PartitionMetadata {
    Full,
    ExternsOnly,
}

pub fn emit_scalar_llvm(validation: &ScalarValidation) -> Result<LlvmPartition, String> {
    emit_scalar_llvm_with_metadata(validation, PartitionMetadata::Full)
}

fn emit_scalar_llvm_with_metadata(
    validation: &ScalarValidation,
    metadata: PartitionMetadata,
) -> Result<LlvmPartition, String> {
    if !validation.diagnostics.is_empty() {
        return Err("cannot emit LLVM for an invalid scalar program".into());
    }
    let prepared = crate::scalar::PreparedProgram::rebuild_raw(&validation.program)?;
    let program = prepared.program();
    emit_single_module_llvm(
        &program.source,
        &program.items,
        prepared.initialization_region(),
        &program.structs,
        &program.layouts,
        program.target_layout,
        GenericInitializerInput::RawStandalone(&prepared),
        &|reservations| prepared.aggregate(reservations),
        &|_, _, function| Some(&function.reference_cfg),
        metadata,
        &[],
    )
}

pub fn emit_scalar_build_single_module_llvm(
    validation: &ScalarBuildValidation<'_>,
) -> Result<LlvmPartition, String> {
    emit_scalar_build_single_module_with_metadata(validation, PartitionMetadata::Full, &[])
}

pub fn emit_scalar_build_single_module_llvm_text(
    validation: &ScalarBuildValidation<'_>,
) -> Result<String, String> {
    Ok(emit_scalar_build_single_module_artifact_llvm(validation)?.into_text())
}

pub fn emit_scalar_build_single_module_artifact_llvm(
    validation: &ScalarBuildValidation<'_>,
) -> Result<LlvmPartition, String> {
    emit_scalar_build_single_module_with_metadata(validation, PartitionMetadata::ExternsOnly, &[])
}

struct WasmRootExport<'a> {
    label: &'a str,
    symbol: String,
    signature: &'a ScalarType,
}

pub fn emit_scalar_build_wasm_root_exports_llvm_text(
    validation: &ScalarBuildValidation<'_>,
    root: &wosy_syntax::SourceIdentity,
    requested: &BTreeMap<String, String>,
) -> Result<String, String> {
    Ok(emit_scalar_build_wasm_root_exports_artifact_llvm(validation, root, requested)?.into_text())
}

pub fn emit_scalar_build_wasm_root_exports_artifact_llvm(
    validation: &ScalarBuildValidation<'_>,
    root: &wosy_syntax::SourceIdentity,
    requested: &BTreeMap<String, String>,
) -> Result<LlvmPartition, String> {
    let modules = &validation.prepared().project().modules;
    let singleton = modules.len() == 1;
    let root_module = modules.iter().find(|module| &module.source == root)
        .ok_or_else(|| format!("missing selected Wasm export root {}", root.path))?;
    if root_module.target_layout != ScalarTargetLayout::WASM32 {
        return Err("Wasm root exports require a Wasm artifact".into());
    }
    let direct = |ty: &ScalarType| matches!(
        ty, ScalarType::I32 | ScalarType::U32 | ScalarType::I64 | ScalarType::U64
            | ScalarType::F32 | ScalarType::F64
    );
    let mut root_functions = requested
        .values()
        .map(|name| (name.as_str(), None))
        .collect::<BTreeMap<_, _>>();
    for item in &root_module.items {
        if let ScalarItem::Function(function) = item {
            if let Some(entry) = root_functions.get_mut(function.name.as_str()) {
                match entry {
                    Some((_, ambiguous)) => *ambiguous = true,
                    None => *entry = Some((function, false)),
                }
            }
        }
    }
    let mut exports = Vec::with_capacity(requested.len());
    for (label, authored) in requested {
        let mut bytes = label.bytes();
        if !bytes.next().is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
            || !bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_')
            || matches!(label.as_str(), "main" | "_start" | "memory")
            || label.starts_with("__") || label.starts_with("wosy_")
        {
            return Err(format!("invalid or reserved Wasm root export label {label:?}"));
        }
        let (function, ambiguous) = root_functions[authored.as_str()].ok_or_else(|| {
            format!("Wasm root export {label:?} names no root function {authored:?}")
        })?;
        if ambiguous || !function.generic_parameters.is_empty()
            || !function.overload_arms.is_empty()
        {
            return Err(format!("Wasm root export {label:?} needs an unambiguous concrete declaration"));
        }
        let ScalarType::Callable { outputs, parameters } = &function.signature else {
            return Err(format!("Wasm root export {label:?} is not callable"));
        };
        if !parameters.iter().all(direct) || !match outputs.outputs.as_slice() {
            [] => true,
            [output] => output.ty == ScalarType::Unit || direct(&output.ty),
            _ => false,
        } {
            return Err(format!("Wasm root export {label:?} needs only direct scalar ABI values"));
        }
        exports.push(WasmRootExport {
            label,
            symbol: if singleton { authored.clone() } else { project_function_name(root, authored) },
            signature: &function.signature,
        });
    }
    if singleton {
        emit_scalar_build_single_module_with_metadata(validation, PartitionMetadata::ExternsOnly, &exports)
    } else {
        emit_scalar_build_project_with_policy(
            validation, EmissionPolicy::Prepared, PartitionMetadata::ExternsOnly, &exports,
        )
    }
}

fn emit_wasm_root_export_wrappers<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    functions: &BTreeMap<String, FunctionValue<'ctx>>,
    exports: &[WasmRootExport<'_>],
) -> Result<(), String> {
    for export in exports {
        let implementation = functions.get(&export.symbol).copied().ok_or_else(|| {
            format!("missing emitted implementation of Wasm root export {:?}", export.label)
        })?;
        let expected = function_type(context, export.signature, ScalarTargetLayout::WASM32)?;
        if implementation.get_type() != expected || implementation.get_call_conventions() != 0 {
            return Err(format!("Wasm root export {:?} has a non-direct ABI", export.label));
        }
        if export.label == export.symbol && implementation.get_linkage() != Linkage::External {
            return Err(format!("Wasm root export {:?} has a private implementation symbol", export.label));
        }
        if module.get_global(export.label).is_some()
            || (export.label != export.symbol && module.get_function(export.label).is_some())
        {
            return Err(format!("Wasm root export {:?} collides with an emitted symbol", export.label));
        }
    }
    if exports.is_empty() {
        return Ok(());
    }
    let builder = context.create_builder();
    for export in exports {
        if export.label == export.symbol {
            continue;
        }
        let implementation = functions[&export.symbol];
        let wrapper = module.add_function(export.label, implementation.get_type(), Some(Linkage::External));
        builder.position_at_end(context.append_basic_block(wrapper, "entry"));
        let arguments = wrapper.get_param_iter().map(Into::into).collect::<Vec<BasicMetadataValueEnum<'ctx>>>();
        let call = builder.build_call(implementation, &arguments, "root_export_call")
            .map_err(builder_error)?;
        let result = call.try_as_basic_value().basic();
        builder.build_return(result.as_ref().map(|v| v as &dyn inkwell::values::BasicValue))
            .map_err(builder_error)?;
    }
    Ok(())
}

fn emit_scalar_build_single_module_with_metadata(
    validation: &ScalarBuildValidation<'_>,
    metadata: PartitionMetadata,
    exports: &[WasmRootExport<'_>],
) -> Result<LlvmPartition, String> {
    if !validation.diagnostics().is_empty() {
        return Err("cannot emit LLVM for an invalid scalar program".into());
    }
    let prepared = validation.prepared();
    let modules = &prepared.project().modules;
    let [module] = modules.as_slice() else {
        return Err(format!(
            "single-module LLVM emission requires exactly one prepared module, found {}",
            modules.len()
        ));
    };
    let initialization_region = prepared.initialization_region(0).ok_or_else(|| {
        format!(
            "internal error: missing module initialization region for {:?} (module 0)",
            module.source,
        )
    })?;
    emit_single_module_llvm(
        &module.source,
        &module.items,
        initialization_region,
        &module.structs,
        &module.layouts,
        module.target_layout,
        GenericInitializerInput::PreparedRows(prepared),
        &|reservations| validation.aggregate(reservations),
        &|item, arm, _| validation.reference_cfg(0, item, arm),
        metadata,
        exports,
    )
}

fn emit_single_module_llvm<'a>(
    source: &'a wosy_syntax::SourceIdentity,
    items: &'a [ScalarItem],
    initialization_region: &'a ModuleInitializationRegion,
    structs: &[ScalarStruct],
    layouts: &ScalarTargetLayouts,
    target_layout: ScalarTargetLayout,
    generic_input: GenericInitializerInput<'a>,
    close_aggregates: &dyn Fn(&crate::scalar::AggregateReservations) -> Result<crate::scalar::ClosedAggregates, String>,
    reference_cfg: &dyn Fn(
        usize,
        Option<usize>,
        &'a ScalarFunction,
    ) -> Option<&'a ScalarReferenceCfg>,
    metadata: PartitionMetadata,
    exports: &[WasmRootExport<'_>],
) -> Result<LlvmPartition, String> {
    let prepared_render = generic_input.prepared_render();
    let initializer_nodes = Some(initialization_region.bind_call_nodes(items)?);
    let require_cfg = |item_ordinal, arm_index, function: &'a ScalarFunction| {
        reference_cfg(item_ordinal, arm_index, function).ok_or_else(|| {
            let span = function.span;
            format!(
                "internal error: missing reference CFG for {} in {source:?} at {}..{} (module 0, item {item_ordinal}, arm {arm_index:?})",
                function.name, span.start, span.end,
            )
        })
    };
    let context = Context::create();
    let module = ManuallyDrop::new(context.create_module(&source.path));
    let builder = context.create_builder();
    let layouts = crate::scalar::layout::ScalarLayoutIndex::program(layouts);
    let structs = structs.iter().collect::<Vec<_>>();
    let inline_results = InlineResultShapes::new(&structs);
    let mut globals = BTreeMap::new();
    for item in items {
        if let ScalarItem::Binding(binding) = item {
            for (name, ty) in binding_receivers(binding).iter() {
                if *ty == ScalarType::Unit {
                    continue;
                }
                let (llvm_ty, alignment) =
                    storage_type_and_alignment(&context, ty, &layouts, target_layout)?;
                let global = module.add_global(llvm_ty, None, name);
                if let Some(alignment) = storage_alignment(alignment)? {
                    global.set_alignment(alignment);
                }
                global.set_linkage(Linkage::Internal);
                global.set_initializer(&llvm_ty.const_zero());
                globals.insert(name.clone(), (global, ty.clone()));
            }
        }
    }
    let mut signatures = items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Binding(binding) => {
                Some((binding.name.clone(), binding.declared_type.clone()))
            }
            ScalarItem::Function(function) => {
                Some((function.name.clone(), function.signature.clone()))
            }
            ScalarItem::Namespace(_) | ScalarItem::Extern(_) | ScalarItem::Executable(_) => None,
        })
        .collect::<BTreeMap<_, _>>();
    let mut externs = Vec::new();
    let mut functions = BTreeMap::new();
    let mut call_targets = BTreeMap::new();
    let mut initializer_symbols = InitializerSymbols::default();
    let mut definitions = Vec::new();
    for (item_ordinal, item) in items.iter().enumerate() {
        if prepared_render {
            if let ScalarItem::Binding(binding) = item {
                initializer_symbols.callable_binding(source, binding);
            }
        }
        let ScalarItem::Function(function) = item else {
            continue;
        };
        initializer_symbols.function(source, function, function.name.clone());
        if !function.generic_parameters.is_empty() || !function.overload_arms.is_empty() {
            continue;
        }
        let cfg = require_cfg(item_ordinal, None, function)?;
        if prepared_render { initializer_symbols.scalar_local_target(source, function, cfg,
            generic_input.has_closed_shared_scalar_reader(0, function.name_span)); }
        definitions.push((function, cfg, function.name.as_str(), item_ordinal, None));
    }
    let ordinary_count = definitions.len();
    let specializations = generic_specializations(
        std::iter::once((0, source, items, initialization_region)),
        |_, name| name.to_owned(),
        prepared_render,
    )?
    .remove(&0)
    .expect("single module has a specialization set");
    let mut prepared_overloads = prepared_initializer_overloads(
        initializer_nodes
            .iter()
            .filter(|_| prepared_render)
            .map(|_| initialization_region),
        &initializer_symbols,
    )?;
    collect_body_overload_reservations(
        generic_input,
        0,
        items,
        initialization_region,
        &initializer_symbols,
        &mut prepared_overloads,
    )?;
    let overloads = selected_overload_specializations(
        items,
        str::to_owned,
        prepared_overloads.remove(source).into_iter().flatten(),
    );
    let mut reservations = crate::scalar::AggregateReservations {
        checked_access: matches!(generic_input, GenericInitializerInput::PreparedRows(prepared) if prepared.checked_access().is_some()),
        target_layout: Some(target_layout),
        ..Default::default()
    };
    reservations.modules.push(0);
    for &(_, _, _, item, arm) in &definitions {
        reservations.bodies.insert((0, item, arm), true);
    }
    for specialization in specializations.values().chain(overloads.values()) {
        reservations.bodies.insert((0, specialization.item_ordinal, specialization.arm_index), matches!(specialization.function, Cow::Borrowed(_)));
    }
    let aggregates = close_aggregates(&reservations)?;
    for &(function, cfg, _, item_ordinal, _) in &definitions {
        let value = module.add_function(
            &function.name,
            aggregate_transfer::definition_type(
                &context,
                function,
                cfg,
                target_layout,
                &inline_results,
                &layouts,
                aggregates.body(0, item_ordinal, None),
            )?,
            aggregate_transfer::linkage(aggregates.body(0, item_ordinal, None)),
        );
        for (index, name) in function.parameters.iter().enumerate() {
            if let Some(parameter) = value.get_nth_param(index as u32) {
                parameter.set_name(name);
            }
        }
        functions.insert(function.name.clone(), value);
        call_targets.insert(function.name.clone(), function.name.clone());
    }
    for (
        lookup,
        LlvmSpecialization {
            function,
            name,
            template,
            item_ordinal,
            arm_index,
        },
    ) in specializations.iter().chain(&overloads)
    {
        let cfg = require_cfg(*item_ordinal, *arm_index, template)?;
        let value = module.add_function(
            name,
            aggregate_transfer::definition_type(&context, function, cfg, target_layout, &inline_results, &layouts, aggregates.body(0, *item_ordinal, *arm_index))?,
            aggregate_transfer::linkage(aggregates.body(0, *item_ordinal, *arm_index)),
        );
        for (index, parameter) in function.parameters.iter().enumerate() {
            if let Some(argument) = value.get_nth_param(index as u32) {
                argument.set_name(parameter);
            }
        }
        call_targets.insert(lookup.clone(), name.clone());
        signatures.insert(name.clone(), function.signature.clone());
        functions.insert(name.clone(), value);
        definitions.push((
            function.as_ref(),
            cfg,
            name.as_str(),
            *item_ordinal,
            *arm_index,
        ));
    }
    for item in items {
        if let ScalarItem::Extern(extern_decl) = item {
            for function in &extern_decl.functions {
                let key = format!("{}.{}", extern_decl.binding, function.name);
                let crate::scalar::ScalarExternModule::Valid(import_module) =
                    &extern_decl.actual_module
                else {
                    continue;
                };
                let identity = llvm_extern_identity(
                    source,
                    &extern_decl.binding,
                    import_module,
                    &function.name,
                    &function.signature,
                );
                let value = module.add_function(
                    &identity.internal_name,
                    function_type(&context, &function.signature, target_layout)?,
                    None,
                );
                if prepared_render {
                    initializer_symbols.external(
                        source,
                        function.name_span,
                        &identity.internal_name,
                    );
                }
                call_targets.insert(key, identity.internal_name.clone());
                signatures.insert(identity.internal_name.clone(), function.signature.clone());
                functions.insert(identity.internal_name.clone(), value);
                externs.push(identity);
            }
        }
    }
    functions.insert(
        "main".into(),
        module.add_function("main", context.i32_type().fn_type(&[], false), None),
    );
    let mut runtime_array_results = BTreeMap::new();
    let mut dynamic_owner_outputs = BTreeMap::new();
    for (item_ordinal, item) in items.iter().enumerate() {
        let ScalarItem::Function(function) = item else {
            continue;
        };
        if let Some(target) = call_targets.get(&function.name) {
            let cfg = require_cfg(item_ordinal, None, function)?;
            insert_function_result_provenance(
                &mut runtime_array_results,
                &mut dynamic_owner_outputs,
                target,
                function,
                cfg,
            );
        }
    }
    for &(function, cfg, name, _, _) in &definitions[ordinary_count..] {
        insert_function_result_provenance(
            &mut runtime_array_results,
            &mut dynamic_owner_outputs,
            name,
            function,
            cfg,
        );
    }
    let names = ModuleNames {
        globals: GlobalNames(
            globals
                .iter()
                .map(|(name, global)| (name.as_str(), global))
                .collect(),
        ),
        unit_values: unit_values(items),
        members: BTreeMap::new(),
        namespaces: BTreeMap::new(),
        externs: BTreeMap::new(),
    };
    for (function, cfg, name, item, arm) in definitions {
        let nodes = generic_input.bind_body(0, item, arm, &function.body)?;
        emit_function(
            prepared_render,
            generic_input.requires_release_checked_access(),
            aggregates.body(0, item, arm),
            &context,
            &builder,
            &functions,
            &call_targets,
            &signatures,
            &runtime_array_results,
            &dynamic_owner_outputs,
            &globals,
            function,
            cfg,
            nodes.as_ref(),
            &initializer_symbols,
            name,
            &names,
            &module,
            &structs,
            &layouts,
            target_layout,
        )?;
    }
    emit_main(
        prepared_render,
        generic_input.requires_release_checked_access(),
        aggregates.initializer(source),
        &context,
        &builder,
        &functions,
        &call_targets,
        &signatures,
        &runtime_array_results,
        &dynamic_owner_outputs,
        &globals,
        &names,
        items,
        initialization_region,
        initializer_nodes.as_ref(),
        &initializer_symbols,
        &module,
        &structs,
        &layouts,
        target_layout,
    )?;
    if !exports.is_empty() {
        emit_wasm_root_export_wrappers(&context, &module, &functions, exports)?;
    }
    finish_partition(
        ManuallyDrop::into_inner(module),
        source.path.clone(),
        &functions,
        &signatures,
        &externs,
        metadata,
    )
}

pub fn emit_scalar_project_llvm(
    validation: &ScalarProjectValidation,
) -> Result<LlvmPartition, String> {
    if !validation.diagnostics.is_empty() {
        return Err("cannot emit LLVM for an invalid scalar project".into());
    }
    let prepared = crate::PreparedScalarProject::new(validation.project.clone())
        .map_err(|_| "cannot emit LLVM for an invalid scalar project".to_owned())?;
    let build = crate::analyze_scalar_build(&prepared);
    emit_scalar_build_project_with_policy(
        &build,
        EmissionPolicy::RawCompatibility,
        PartitionMetadata::Full,
        &[],
    )
}

pub fn emit_scalar_build_project_llvm(
    validation: &ScalarBuildValidation<'_>,
) -> Result<LlvmPartition, String> {
    emit_scalar_build_project_with_policy(
        validation,
        EmissionPolicy::Prepared,
        PartitionMetadata::Full,
        &[],
    )
}

pub fn emit_scalar_build_project_llvm_text(
    validation: &ScalarBuildValidation<'_>,
) -> Result<String, String> {
    Ok(emit_scalar_build_project_artifact_llvm(validation)?.into_text())
}

pub fn emit_scalar_build_project_artifact_llvm(
    validation: &ScalarBuildValidation<'_>,
) -> Result<LlvmPartition, String> {
    emit_scalar_build_project_with_policy(
        validation,
        EmissionPolicy::Prepared,
        PartitionMetadata::ExternsOnly,
        &[],
    )
}

fn emit_scalar_build_project_with_policy(
    validation: &ScalarBuildValidation<'_>,
    policy: EmissionPolicy,
    metadata: PartitionMetadata,
    exports: &[WasmRootExport<'_>],
) -> Result<LlvmPartition, String> {
    if !validation.diagnostics().is_empty() {
        return Err("cannot emit LLVM for an invalid scalar project".into());
    }
    let prepared = validation.prepared();
    emit_project_llvm(
        prepared.project(),
        &|module| prepared.initialization_region(module),
        match policy {
            EmissionPolicy::RawCompatibility => GenericInitializerInput::RawProject(prepared),
            EmissionPolicy::Prepared => GenericInitializerInput::PreparedRows(prepared),
        },
        &|reservations| validation.aggregate(reservations),
        &|module, item, arm, _| validation.reference_cfg(module, item, arm),
        match policy {
            EmissionPolicy::RawCompatibility => None,
            EmissionPolicy::Prepared => Some(validation),
        },
        metadata,
        exports,
    )
}

fn emit_project_llvm<'a>(
    project: &'a ScalarProject,
    initialization_region: &dyn Fn(usize) -> Option<&'a ModuleInitializationRegion>,
    generic_input: GenericInitializerInput<'a>,
    close_aggregates: &dyn Fn(&crate::scalar::AggregateReservations) -> Result<crate::scalar::ClosedAggregates, String>,
    reference_cfg: &dyn Fn(
        usize,
        usize,
        Option<usize>,
        &'a ScalarFunction,
    ) -> Option<&'a ScalarReferenceCfg>,
    project_build: Option<&'a ScalarBuildValidation<'a>>,
    metadata: PartitionMetadata,
    exports: &[WasmRootExport<'_>],
) -> Result<LlvmPartition, String> {
    let mut module_ordinals = HashMap::with_capacity(project.modules.len());
    for (ordinal, module) in project.modules.iter().enumerate() {
        module_ordinals.entry(&module.source).or_insert(ordinal);
    }
    let ordered_modules = project
        .initialization_order
        .iter()
        .map(|source| {
            module_ordinals
                .get(source)
                .map(|&ordinal| (ordinal, &project.modules[ordinal]))
                .ok_or_else(|| format!("missing module {}", source.path))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let modules = ordered_modules
        .iter()
        .map(|&(_, module)| module)
        .collect::<Vec<_>>();
    let regions = ordered_modules
        .iter()
        .map(|&(ordinal, module)| {
            initialization_region(ordinal).ok_or_else(|| {
                format!(
                    "internal error: missing module initialization region for {:?} (module {ordinal})",
                    module.source,
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let prepared_render = generic_input.prepared_render();
    let initializer_nodes = Some(
        modules
            .iter()
            .zip(&regions)
            .map(|(module, region)| region.bind_call_nodes(&module.items))
            .collect::<Result<Vec<_>, _>>()?,
    );
    let require_cfg = |module_ordinal, item_ordinal, arm_index, function: &'a ScalarFunction| {
        reference_cfg(module_ordinal, item_ordinal, arm_index, function).ok_or_else(|| {
            let source = &project.modules[module_ordinal].source;
            let span = function.span;
            format!(
                "internal error: missing reference CFG for {} in {source:?} at {}..{} (module {module_ordinal}, item {item_ordinal}, arm {arm_index:?})",
                function.name, span.start, span.end,
            )
        })
    };
    let all_structs = modules
        .iter()
        .flat_map(|module| module.structs.iter())
        .collect::<Vec<_>>();
    let inline_results = InlineResultShapes::new(&all_structs);
    let layouts = crate::scalar::layout::ScalarLayoutIndex::project(&project.modules);
    let module_name = modules
        .first()
        .ok_or_else(|| "project has no reachable modules".to_owned())?
        .source
        .path
        .clone();
    let context = Context::create();
    let module = ManuallyDrop::new(context.create_module(&module_name));
    let builder = context.create_builder();
    let target_layout = project
        .modules
        .first()
        .ok_or_else(|| "project has no reachable modules".to_owned())?
        .target_layout;
    let mut globals = BTreeMap::new();
    let mut functions = BTreeMap::new();
    let mut signatures = BTreeMap::new();
    let mut externs = Vec::new();
    let mut call_targets = BTreeMap::new();
    let mut initializer_symbols = InitializerSymbols::default();
    enum Declaration<'a> {
        Ordinary(usize),
        Extern(LlvmExternIdentity, String, &'a ScalarType),
    }
    let mut declarations = Vec::new();
    let mut definitions = Vec::new();
    for &(module_ordinal, source_module) in &ordered_modules {
        for (item_ordinal, item) in source_module.items.iter().enumerate() {
            if let ScalarItem::Binding(binding) = item {
                if prepared_render {
                    initializer_symbols.callable_binding(&source_module.source, binding);
                }
                for (name, ty) in binding_receivers(binding).iter() {
                    if *ty == ScalarType::Unit {
                        continue;
                    }
                    let name = project_global_name(&source_module.source, name);
                    let (llvm_ty, alignment) =
                        storage_type_and_alignment(&context, ty, &layouts, target_layout)?;
                    let global = module.add_global(llvm_ty, None, &name);
                    if let Some(alignment) = storage_alignment(alignment)? {
                        global.set_alignment(alignment);
                    }
                    global.set_linkage(Linkage::Internal);
                    global.set_initializer(&llvm_ty.const_zero());
                    globals.insert(name, (global, ty.clone()));
                }
            }
            if let ScalarItem::Function(function) = item {
                let name = project_function_name(&source_module.source, &function.name);
                initializer_symbols.function(&source_module.source, function, name.clone());
                if !function.generic_parameters.is_empty() || !function.overload_arms.is_empty() {
                    continue;
                }
                let cfg = require_cfg(module_ordinal, item_ordinal, None, function)?;
                if prepared_render { initializer_symbols.scalar_local_target(&source_module.source, function, cfg,
                    generic_input.has_closed_shared_scalar_reader(module_ordinal, function.name_span)); }
                declarations.push(Declaration::Ordinary(definitions.len()));
                definitions.push((
                    module_ordinal,
                    source_module,
                    function,
                    cfg,
                    name,
                    item_ordinal,
                    None,
                ));
            }
            if let ScalarItem::Extern(extern_decl) = item {
                let module_name = match &extern_decl.actual_module {
                    crate::scalar::ScalarExternModule::Valid(name) => name,
                    crate::scalar::ScalarExternModule::Invalid { .. } => continue,
                };
                for function in &extern_decl.functions {
                    let identity = llvm_extern_identity(
                        &source_module.source,
                        &extern_decl.binding,
                        module_name,
                        &function.name,
                        &function.signature,
                    );
                    let key = project_extern_lookup_key(
                        &source_module.source,
                        &extern_decl.binding,
                        module_name,
                        &function.name,
                    );
                    if prepared_render {
                        initializer_symbols.external(
                            &source_module.source,
                            function.name_span,
                            &identity.internal_name,
                        );
                    }
                    declarations.push(Declaration::Extern(identity, key, &function.signature));
                }
            }
        }
    }
    let mut prepared_specializations = generic_specializations(
        ordered_modules
            .iter()
            .zip(&regions)
            .map(|(&(ordinal, module), &region)| {
                (ordinal, &module.source, module.items.as_slice(), region)
            }),
        project_function_name,
        prepared_render,
    )?;
    let mut project_overloads = Vec::new();
    let mut prepared_overloads = prepared_initializer_overloads(
        initializer_nodes
            .iter()
            .filter(|_| prepared_render)
            .flat_map(|_| regions.iter().copied()),
        &initializer_symbols,
    )?;
    for (&(ordinal, source_module), &region) in ordered_modules.iter().zip(&regions) {
        collect_body_overload_reservations(
            generic_input,
            ordinal,
            &source_module.items,
            region,
            &initializer_symbols,
            &mut prepared_overloads,
        )?;
    }
    let mut overload_reservations = BTreeMap::new();
    for &(ordinal, source_module) in &ordered_modules {
        overload_reservations.insert(ordinal, selected_overload_specializations(
            &source_module.items,
            |name| project_function_name(&source_module.source, name),
            prepared_overloads
                .remove(&source_module.source)
                .into_iter()
                .flatten(),
        ));
    }
    let mut reservations = crate::scalar::AggregateReservations {
        checked_access: matches!(generic_input, GenericInitializerInput::PreparedRows(prepared) if prepared.checked_access().is_some()),
        target_layout: Some(target_layout),
        ..Default::default()
    };
    reservations.modules.extend(ordered_modules.iter().map(|&(ordinal, _)| ordinal));
    for &(ordinal, _, _, _, _, item, arm) in &definitions {
        reservations.bodies.insert((ordinal, item, arm), true);
    }
    for (&ordinal, rows) in prepared_specializations.iter().chain(&overload_reservations) {
        for specialization in rows.values() {
            reservations.bodies.insert((ordinal, specialization.item_ordinal, specialization.arm_index), matches!(specialization.function, Cow::Borrowed(_)));
        }
    }
    let snapshot_struct = |id: &crate::scalar::ScalarStructId| {
        let &module = module_ordinals.get(&id.source)?;
        let definition = project.modules.get(module)?.structs.get(id.index)?;
        (&definition.id == id && layouts.aggregate(id).is_some()).then_some(definition)
    };
    if prepared_render && reservations.modules.len() > 1 {
        reservations.snapshot_struct = Some(&snapshot_struct);
    }
    let mut aggregates = close_aggregates(&reservations)?;
    let mut planned_runtime_access = HashMap::new();
    if target_layout == ScalarTargetLayout::WASM32 && reservations.checked_access
        && !aggregates.private_runtime_pair_members().is_empty()
    {
        // Only exact member owners enter the pre-definition census. The
        // result is consumed at body emission instead of walking those CFGs
        // twice. Unrelated ordinary and managed bodies stay on their path.
        let mut portable_calls = HashMap::new();
        for &key @ (ordinal, item, arm) in aggregates.private_runtime_pair_members() {
            let plan = aggregates.body(ordinal, item, arm).expect("indexed private runtime member");
            if !plan.private_runtime_access_calls.values().any(|site| site.portable_birth.is_some())
                || !portable_runtime_plan_allows_plain_storage(plan)
                || !plan.fallible_relay_allocations.is_empty() || !plan.same_array_rebinds.is_empty()
                || plan.replacement_index_observation.is_some()
            {
                continue;
            }
            let Some(ScalarItem::Function(function)) = project.modules[ordinal].items.get(item) else {
                return Err("portable runtime owner body lost its prepared function".into());
            };
            let cfg = require_cfg(ordinal, item, arm, function)?;
            if !portable_lexical_runtime_body(target_layout, true, function, cfg) {
                continue;
            }
            // This exact class has no managed rebind requiring body nodes
            // in runtime_allocation_access; every private Call/Borrow is
            // authenticated against its sealed CFG points instead.
            let mut access = runtime_allocation_access(cfg, None, Some(plan), true, true, true);
            for owner in access.2.owners.keys() {
                access.3.remove(&(owner.start, owner.end));
            }
            let admitted: std::collections::HashSet<_> = plan.private_runtime_access_calls.iter().filter_map(|(&call, site)|
                access.3.contains(&(site.owner.start, site.owner.end)).then_some(call)).collect();
            if !admitted.is_empty() { portable_calls.insert(key, admitted); }
            planned_runtime_access.insert(key, access);
        }
        aggregates.mark_private_runtime_portable(portable_calls)?;
    }
    let mut private_runtime_portable_symbols = std::collections::HashSet::new();
    for declaration in declarations {
        match declaration {
            Declaration::Ordinary(index) => {
                let (module_ordinal, _, function, cfg, ref name, item_ordinal, _) = definitions[index];
                let plan = aggregates.body(module_ordinal, item_ordinal, None);
                if plan.is_some_and(|plan| plan.private_runtime_portable_formal) {
                    private_runtime_portable_symbols.insert(name.clone());
                }
                let value = module.add_function(
                    &name,
                    aggregate_transfer::definition_type(&context, function, cfg, target_layout, &inline_results, &layouts, plan)?,
                    aggregate_transfer::linkage(plan),
                );
                for (index, parameter) in function.parameters.iter().enumerate() {
                    if let Some(argument) = value.get_nth_param(index as u32) {
                        argument.set_name(parameter);
                    }
                }
                functions.insert(name.clone(), value);
                call_targets.insert(name.clone(), name.clone());
                signatures.insert(name.clone(), function.signature.clone());
            }
            Declaration::Extern(identity, key, signature) => {
                let value = module.add_function(&identity.internal_name, function_type(&context, signature, target_layout)?, None);
                call_targets.insert(key, identity.internal_name.clone());
                signatures.insert(identity.internal_name.clone(), signature.clone());
                functions.insert(identity.internal_name.clone(), value);
                externs.push(identity);
            }
        }
    }
    let mut project_specializations = Vec::new();
    for &(module_ordinal, source_module) in &ordered_modules {
        let specializations = prepared_specializations
            .remove(&module_ordinal)
            .expect("project module has a specialization set");
        for (lookup, specialization) in specializations {
            let LlvmSpecialization {
                function,
                name,
                template,
                item_ordinal,
                arm_index,
            } = specialization;
            let cfg = require_cfg(module_ordinal, item_ordinal, arm_index, template)?;
            let value = module.add_function(
                &name,
                definition_function_type(&context, &function, cfg, target_layout, &inline_results, &layouts)?,
                None,
            );
            for (index, parameter) in function.parameters.iter().enumerate() {
                if let Some(argument) = value.get_nth_param(index as u32) {
                    argument.set_name(parameter);
                }
            }
            call_targets.insert(lookup, name.clone());
            signatures.insert(name.clone(), function.signature.clone());
            functions.insert(name.clone(), value);
            project_specializations.push((
                module_ordinal,
                source_module,
                function,
                cfg,
                name,
                item_ordinal,
                arm_index,
            ));
        }
    }
    for &(module_ordinal, source_module) in &ordered_modules {
        for (lookup, specialization) in overload_reservations.remove(&module_ordinal).expect("project module has an overload reservation set") {
            let LlvmSpecialization {
                function,
                name,
                template,
                item_ordinal,
                arm_index,
            } = specialization;
            let cfg = require_cfg(module_ordinal, item_ordinal, arm_index, template)?;
            let value = module.add_function(
                &name,
                aggregate_transfer::definition_type(&context, &function, cfg, target_layout, &inline_results, &layouts, aggregates.body(module_ordinal, item_ordinal, arm_index))?,
                aggregate_transfer::linkage(aggregates.body(module_ordinal, item_ordinal, arm_index)),
            );
            for (index, parameter) in function.parameters.iter().enumerate() {
                if let Some(argument) = value.get_nth_param(index as u32) {
                    argument.set_name(parameter);
                }
            }
            call_targets.insert(lookup, name.clone());
            signatures.insert(name.clone(), function.signature.clone());
            functions.insert(name.clone(), value);
            project_overloads.push((
                module_ordinal,
                source_module,
                function,
                cfg,
                name,
                item_ordinal,
                arm_index,
            ));
        }
    }
    let mut anonymous_definitions = Vec::new();
    if let Some(build) = project_build {
        let prepared = build.prepared();
        let mut reachable = None;
        let mut bound_owner: Option<((usize, usize), CallableBodyCallNodes<'_>)> = None;
        for (owner, arm) in prepared.inline_arms() {
            let reachable = reachable.get_or_insert_with(|| ordered_modules.iter().map(|&(module, _)| module)
                .collect::<std::collections::HashSet<_>>());
            if !reachable.contains(&owner.module) {
                continue;
            }
            let owner_key = (owner.module, owner.item);
            if bound_owner.as_ref().is_none_or(|(key, _)| *key != owner_key) {
                let ScalarItem::Function(function) = &project.modules[owner.module].items[owner.item] else {
                    return Err("private inline callable owner changed".into());
                };
                bound_owner = Some((owner_key, prepared.bind_callable_body_call_nodes(
                    owner.module, owner.item, None, &function.body,
                )?));
            }
            let nodes = &bound_owner.as_ref().expect("bound owner").1;
            let value = crate::scalar::anonymous_callable::actual_inline_element(prepared, owner, nodes)?;
            let name = format!("__wosy_private_inline_array_{}_{}_{}_{}_{}",
                owner.module, owner.item, owner.block, owner.binding, owner.ordinal);
            if functions.contains_key(&name) || module.get_function(&name).is_some()
                || initializer_symbols.inline_array_arms.insert(std::ptr::from_ref(value), name.clone()).is_some()
            { return Err("duplicate private inline callable".into()); }
            let signature = &arm.function().signature;
            let function = module.add_function(&name, function_type(&context, signature, target_layout)?, Some(Linkage::Private));
            signatures.insert(name.clone(), signature.clone());
            functions.insert(name.clone(), function);
            anonymous_definitions.push((owner.module, name, arm));
        }
    }
    functions.insert(
        "main".into(),
        module.add_function("main", context.i32_type().fn_type(&[], false), None),
    );
    let mut runtime_array_results = BTreeMap::new();
    let mut dynamic_owner_outputs = BTreeMap::new();
    for (_, _, function, cfg, name, _, _) in &definitions {
        insert_function_result_provenance(
            &mut runtime_array_results,
            &mut dynamic_owner_outputs,
            name,
            function,
            cfg,
        );
    }
    for (_, _, function, cfg, name, _, _) in
        project_specializations.iter().chain(&project_overloads)
    {
        insert_function_result_provenance(
            &mut runtime_array_results,
            &mut dynamic_owner_outputs,
            name,
            function,
            cfg,
        );
    }
    let scopes = modules
        .iter()
        .map(|module| project_module_names(module, &globals))
        .collect();

    let mut module_positions = HashMap::with_capacity(modules.len());
    let mut ordered_positions = vec![0; project.modules.len()];
    for (position, &(ordinal, module)) in ordered_modules.iter().enumerate() {
        let first_position = *module_positions.entry(&module.source).or_insert(position);
        ordered_positions[ordinal] = first_position;
    }
    let names = ProjectNames {
        scopes,
        positions: module_positions,
        require_release_checked_access: generic_input.requires_release_checked_access(),
        private_runtime_portable_symbols,
        checked_access: project_build.and_then(|build| build.prepared().checked_access()).map(|access| {
            CheckedAccessSymbols {
                non_null: project_function_name(&access.source, &access.non_null),
                in_extent: project_function_name(&access.source, &access.in_extent),
            }
        }),
    };
    for (ordinal, name, arm) in &anonymous_definitions {
        let nodes = arm.typed_nodes()?;
        emit_function(
            true, false, None, &context, &builder, &functions, &call_targets, &signatures,
            &runtime_array_results, &dynamic_owner_outputs, &globals, arm.function(), arm.cfg(),
            Some(&nodes), &initializer_symbols, name, &names.scopes[ordered_positions[*ordinal]],
            &module, &all_structs, &layouts, target_layout,
        )?;
    }
    for (ordinal, source_module, function, cfg, name, item, arm, original) in definitions
        .iter()
        .map(|(ordinal, module, function, cfg, name, item, arm)| {
            (*ordinal, *module, *function, *cfg, name, *item, *arm, true)
        })
        .chain(
            project_specializations
                .iter()
                .chain(&project_overloads)
                .map(|(ordinal, module, function, cfg, name, item, arm)| {
                    (
                        *ordinal,
                        *module,
                        function.as_ref(),
                        *cfg,
                        name,
                        *item,
                        *arm,
                        false,
                    )
                }),
        )
    {
        let nodes = match project_build {
            Some(build) if original && arm.is_none() => {
                Some(build.bind_original_project_body(ordinal, item, function)?)
            }
            _ => generic_input.bind_body(ordinal, item, arm, &function.body)?,
        };
        emit_project_function(
            prepared_render,
            aggregates.body(ordinal, item, arm),
            &context,
            &builder,
            &functions,
            &call_targets,
            &signatures,
            &runtime_array_results,
            &dynamic_owner_outputs,
            function,
            cfg,
            nodes.as_ref(),
            &initializer_symbols,
            source_module,
            &modules,
            &globals,
            &names,
            &names.scopes[ordered_positions[ordinal]],
            name,
            &module,
            &all_structs,
            &layouts,
            target_layout,
            planned_runtime_access.remove(&(ordinal, item, arm)),
        )?;
    }
    emit_project_main(
        prepared_render,
        &aggregates,
        &context,
        &builder,
        &functions,
        &call_targets,
        &signatures,
        &runtime_array_results,
        &dynamic_owner_outputs,
        &modules,
        &regions,
        &names,
        initializer_nodes.as_deref(),
        &initializer_symbols,
        &globals,
        &module,
        &all_structs,
        &layouts,
        target_layout,
    )?;
    if !exports.is_empty() {
        emit_wasm_root_export_wrappers(&context, &module, &functions, exports)?;
    }
    finish_partition(
        ManuallyDrop::into_inner(module),
        module_name,
        &functions,
        &signatures,
        &externs,
        metadata,
    )
}

pub fn emit_scalar_llvm_text(validation: &ScalarValidation) -> Result<String, String> {
    Ok(emit_scalar_llvm_with_metadata(validation, PartitionMetadata::ExternsOnly)?.into_text())
}

impl LlvmPartition {
    fn into_text(mut self) -> String {
        if !self.declarations.is_empty() || self.text.contains("\r\n") {
            return self.to_text();
        }
        if let Some(index) = extra_source_filename_blank(&self.text) {
            let offset = self.text.match_indices('\n').nth(index)
                .expect("blank LLVM header line has a newline").0;
            self.text.remove(offset);
        }
        self.text
    }

    pub fn to_text(&self) -> String {
        let mut bytes = Vec::with_capacity(self.text.len());
        self.write_text(&mut bytes)
            .expect("writing to a Vec cannot fail");
        String::from_utf8(bytes).expect("LLVM partition text is UTF-8")
    }

    pub fn write_text(&self, output: &mut impl std::io::Write) -> std::io::Result<()> {
        if !self.declarations.is_empty() {
            let mut declarations = self
                .declarations
                .iter()
                .map(|declaration| (declaration.name.as_str(), None))
                .collect::<HashMap<_, _>>();
            let simple_names = declarations.keys().all(|name| !name.contains('('));
            let mut excluded = Vec::with_capacity(declarations.len());
            for (index, line) in self.text.lines().enumerate() {
                let mut matched = false;
                if let Some((_, after_at)) = line
                    .strip_prefix("declare ")
                    .and_then(|declaration| declaration.split_once('@'))
                {
                    if simple_names && !after_at.contains('@') {
                        if let Some(first) = after_at
                            .split_once('(')
                            .and_then(|(name, _)| declarations.get_mut(name))
                        {
                            first.get_or_insert(line);
                            matched = true;
                        }
                    } else {
                        for (&name, first) in &mut declarations {
                            if line.contains(&format!("@{name}(")) {
                                first.get_or_insert(line);
                                matched = true;
                            }
                        }
                    }
                }
                if index >= 2 && matched {
                    excluded.push(index);
                }
            }
            let mut lines = self.text.lines();
            if let Some(first) = lines.next() {
                output.write_all(first.as_bytes())?;
            }
            if let Some(second) = lines.next() {
                output.write_all(b"\n")?;
                output.write_all(second.as_bytes())?;
            }
            output.write_all(b"\n\n")?;
            for declaration in &self.declarations {
                let line = declarations[declaration.name.as_str()]
                    .expect("LLVM extern declaration is present in module text");
                writeln!(output, "{line} #{}", declaration.attributes[0].group)?;
            }
            output.write_all(b"\n")?;
            for attributes in declaration_attributes(&self.declarations) {
                writeln!(
                    output,
                    "attributes #{} = {{ \"wasm-import-module\"=\"{}\" \"wasm-import-name\"=\"{}\" }}",
                    attributes.group,
                    LlvmQuotedAttribute(&attributes.wasm_import_module),
                    LlvmQuotedAttribute(&attributes.wasm_import_name)
                )?;
            }
            let mut excluded = excluded.into_iter().peekable();
            let mut first = true;
            for (index, line) in lines.enumerate() {
                if excluded.peek() == Some(&(index + 2)) {
                    excluded.next();
                    continue;
                }
                if !std::mem::replace(&mut first, false) {
                    output.write_all(b"\n")?;
                }
                output.write_all(line.as_bytes())?;
            }
        } else {
            let excluded = extra_source_filename_blank(&self.text);
            if !self.text.contains("\r\n") {
                if let Some(index) = excluded {
                    let offset = self.text.match_indices('\n').nth(index)
                        .expect("blank LLVM header line has a newline").0;
                    output.write_all(&self.text.as_bytes()[..offset])?;
                    return output.write_all(&self.text.as_bytes()[offset + 1..]);
                }
                return output.write_all(self.text.as_bytes());
            }
            let mut first = true;
            for (index, line) in self.text.lines().enumerate() {
                if excluded == Some(index) {
                    continue;
                }
                if !std::mem::replace(&mut first, false) {
                    output.write_all(b"\n")?;
                }
                output.write_all(line.as_bytes())?;
            }
            if self.text.ends_with('\n') {
                output.write_all(b"\n")?;
            }
        }
        Ok(())
    }
}

fn extra_source_filename_blank(text: &str) -> Option<usize> {
    let mut lines = text.lines().enumerate();
    let (index, _) = lines.find(|(_, line)| line.starts_with("source_filename"))?;
    (matches!(lines.next(), Some((_, ""))) && matches!(lines.next(), Some((_, ""))))
        .then_some(index + 2)
}

struct LlvmQuotedAttribute<'a>(&'a str);

impl std::fmt::Display for LlvmQuotedAttribute<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut start = 0;
        for (index, ch) in self.0.char_indices() {
            if ch.is_ascii() && !ch.is_ascii_control() && !matches!(ch, '"' | '\\') {
                continue;
            }
            formatter.write_str(&self.0[start..index])?;
            let mut buffer = [0; 4];
            for byte in ch.encode_utf8(&mut buffer).bytes() {
                write!(formatter, "\\{byte:02X}")?;
            }
            start = index + ch.len_utf8();
        }
        formatter.write_str(&self.0[start..])
    }
}

fn declaration_attributes(declarations: &[LlvmFunction]) -> Vec<&LlvmFunctionAttributes> {
    let mut attributes = declarations
        .iter()
        .flat_map(|declaration| declaration.attributes.iter())
        .collect::<Vec<_>>();
    attributes.sort_by_key(|attribute| attribute.group);
    attributes
}

fn first_external_attribute_group(text: &str) -> u32 {
    const PREFIX: &str = "\nattributes #";
    text.match_indices(PREFIX)
        .filter_map(|(start, _)| {
            text[start + PREFIX.len()..]
                .split_once(" = ")
                .and_then(|(group, _)| group.parse::<u32>().ok())
        })
        .max()
        .map_or(0, |group| group + 1)
}

fn finish_partition<'ctx>(
    module: inkwell::module::Module<'ctx>,
    module_name: String,
    functions: &BTreeMap<String, FunctionValue<'ctx>>,
    signatures: &BTreeMap<String, ScalarType>,
    externs: &[LlvmExternIdentity],
    metadata: PartitionMetadata,
) -> Result<LlvmPartition, String> {
    module.verify().map_err(|error| error.to_string())?;
    let text = module.print_to_string().to_string();
    let metadata = match metadata {
        PartitionMetadata::ExternsOnly => Vec::new(),
        PartitionMetadata::Full => functions
            .iter()
            .filter_map(|(name, function)| {
                let (result, parameters) = if name == "main" {
                    (LlvmValueType::I32, &[][..])
                } else {
                    let ScalarType::Callable { outputs, parameters } = signatures.get(name)? else {
                        return None;
                    };
                    (match function.get_type().get_return_type() {
                        Some(BasicTypeEnum::ArrayType(bytes)) => LlvmValueType::InlineBytes(bytes.len()),
                        _ => function_value_result(outputs).ok()?,
                    }, parameters.as_slice())
                };
                let parameters = parameters
                    .iter()
                    .enumerate()
                    .map(|(index, ty)| {
                        let parameter = function.get_nth_param(index as u32)?;
                        let name = parameter.get_name();
                        Some((name.to_string_lossy().to_string(), value_type(ty).ok()?))
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(LlvmFunction {
                    name: name.clone(),
                    result,
                    parameters,
                    attributes: Vec::new(),
                    body: String::new(),
                })
            })
            .collect(),
    };
    let first_external_group = if externs.is_empty() {
        0
    } else {
        first_external_attribute_group(&text)
    };
    let declarations = externs
        .iter()
        .enumerate()
        .filter_map(|(group, extern_identity)| {
            let function = functions.get(&extern_identity.internal_name)?;
            let ScalarType::Callable {
                outputs,
                parameters,
            } = &extern_identity.signature
            else {
                return None;
            };
            let result = function_value_result(outputs).ok()?;
            let parameters = parameters
                .iter()
                .enumerate()
                .map(|(index, ty)| {
                    let value = function.get_nth_param(index as u32)?;
                    Some((
                        value.get_name().to_string_lossy().to_string(),
                        value_type(ty).ok()?,
                    ))
                })
                .collect::<Option<Vec<_>>>()?;
            Some(LlvmFunction {
                name: extern_identity.internal_name.clone(),
                result,
                parameters,
                attributes: vec![LlvmFunctionAttributes {
                    group: first_external_group + group as u32,
                    wasm_import_module: extern_identity.import_module.clone(),
                    wasm_import_name: extern_identity.import_name.clone(),
                }],
                body: String::new(),
            })
        })
        .collect();
    Ok(LlvmPartition {
        module_name,
        declarations,
        functions: metadata,
        text,
    })
}

fn function_type<'ctx>(
    context: &'ctx Context,
    signature: &ScalarType,
    target_layout: ScalarTargetLayout,
) -> Result<FunctionType<'ctx>, String> {
    let ScalarType::Callable {
        outputs,
        parameters,
    } = signature
    else {
        return Err("function has no callable signature".into());
    };
    let parameters = parameters
        .iter()
        .map(|ty| basic_type(context, ty, target_layout).map(Into::into))
        .collect::<Result<Vec<BasicMetadataTypeEnum>, _>>()?;
    Ok(match outputs.outputs.as_slice() {
        [] => context.void_type().fn_type(&parameters, false),
        [output] if output.ty == ScalarType::Unit => {
            context.void_type().fn_type(&parameters, false)
        }
        [output] => basic_type(context, &output.ty, target_layout)?.fn_type(&parameters, false),
        _ => aggregate_type(context, outputs, target_layout)?.fn_type(&parameters, false),
    })
}

fn function_value_result(
    outputs: &crate::ScalarOutputSequence,
) -> Result<LlvmValueType, String> {
    match outputs.outputs.as_slice() {
        [] => Ok(LlvmValueType::Void),
        [output] => value_type(&output.ty),
        _ => Ok(LlvmValueType::Aggregate(
            outputs
                .outputs
                .iter()
                .map(|output| {
                    if output.ty == ScalarType::Unit {
                        Ok(LlvmValueType::I8)
                    } else {
                        value_type(&output.ty)
                    }
                })
                .collect::<Result<Vec<_>, _>>()?,
        )),
    }
}

struct InlineResultShapes<'a> {
    definitions: HashMap<&'a crate::scalar::ScalarStructId, &'a ScalarStruct>,
    fixed: std::cell::RefCell<HashMap<&'a crate::scalar::ScalarStructId, bool>>,
}

impl<'a> InlineResultShapes<'a> {
    fn new(structs: &[&'a ScalarStruct]) -> Self {
        Self {
            definitions: structs.iter().map(|&decl| (&decl.id, decl)).collect(),
            fixed: Default::default(),
        }
    }

    fn fixed_inline(
        &self,
        ty: &ScalarType,
        memo: &mut HashMap<&'a crate::scalar::ScalarStructId, bool>,
    ) -> bool {
        match ty {
            ScalarType::Bool | ScalarType::Char | ScalarType::Enum(_) | ScalarType::F32 | ScalarType::F64 => true,
            ty if integer_width(ty).is_some() => true,
            ScalarType::Array { element, .. } => self.fixed_inline(element, memo),
            ScalarType::Struct(id) => {
                let Some(&decl) = self.definitions.get(id) else { return false; };
                if let Some(&fixed) = memo.get(&decl.id) { return fixed; }
                memo.insert(&decl.id, false);
                let fixed = decl.copy_policy == crate::scalar::CopyPolicy::Copy
                    && decl.fields.iter().all(|field| self.fixed_inline(&field.ty, memo));
                memo.insert(&decl.id, fixed);
                fixed
            }
            _ => false,
        }
    }

    fn result_type<'ctx>(
        &self,
        context: &'ctx Context,
        signature: &ScalarType,
        layouts: &crate::scalar::layout::ScalarLayoutIndex<'_>,
        target_layout: ScalarTargetLayout,
    ) -> Result<Option<BasicTypeEnum<'ctx>>, String> {
        let ScalarType::Callable { outputs, .. } = signature else { return Ok(None); };
        let [output] = outputs.outputs.as_slice() else { return Ok(None); };
        let ScalarType::Struct(id) = &output.ty else { return Ok(None); };
        let decl = self.definitions.get(id).ok_or_else(|| format!("unknown LLVM struct {}", id.index))?;
        if decl.copy_policy != crate::scalar::CopyPolicy::Copy { return Ok(None); }
        if !self.fixed_inline(&output.ty, &mut self.fixed.borrow_mut()) {
            return Err("copy struct result lacks a supported fixed inline data layout".into());
        }
        Ok(Some(storage_type(context, &output.ty, layouts, target_layout)?))
    }
}

fn definition_function_type<'ctx>(
    context: &'ctx Context,
    function: &ScalarFunction,
    reference_cfg: &ScalarReferenceCfg,
    target_layout: ScalarTargetLayout,
    inline_results: &InlineResultShapes<'_>,
    layouts: &crate::scalar::layout::ScalarLayoutIndex<'_>,
) -> Result<FunctionType<'ctx>, String> {
    let inline = inline_results.result_type(context, &function.signature, layouts, target_layout)?;
    if let Some(result) = inline {
        if !dynamic_return_outputs(function.span, reference_cfg).is_empty() {
            return Err("fixed copy callable result cannot have an ownership sidecar".into());
        }
        let original = function_type(context, &function.signature, target_layout)?;
        let operands = original.get_param_types().into_iter().map(Into::into).collect::<Vec<_>>();
        return Ok(result.fn_type(&operands, false));
    }
    legacy_definition_function_type(context, function, reference_cfg, target_layout)
}

fn legacy_definition_function_type<'ctx>(
    context: &'ctx Context,
    function: &ScalarFunction,
    reference_cfg: &ScalarReferenceCfg,
    target_layout: ScalarTargetLayout,
) -> Result<FunctionType<'ctx>, String> {
    let owner_outputs = dynamic_return_outputs(function.span, reference_cfg);
    if owner_outputs.is_empty() {
        return function_type(context, &function.signature, target_layout);
    }
    let ScalarType::Callable {
        outputs,
        parameters,
    } = &function.signature
    else {
        return Err("function has no callable signature".to_owned());
    };
    let mut arguments = parameters
        .iter()
        .map(|ty| basic_type(context, ty, target_layout).map(Into::into))
        .collect::<Result<Vec<BasicMetadataTypeEnum>, _>>()?;
    for _ in owner_outputs {
        arguments.push(context.ptr_type(AddressSpace::default()).into());
    }
    match outputs.outputs.as_slice() {
        [output] => Ok(basic_type(context, &output.ty, target_layout)?.fn_type(&arguments, false)),
        _ => Ok(aggregate_type(context, outputs, target_layout)?.fn_type(&arguments, false)),
    }
}

mod aggregate_transfer;
mod local_callable;
pub(crate) use local_callable::closed_scalar_target;
mod portable_reborrow;
mod memory;
use memory::{
    aggregate_type, align_storage_alloca, allocation_layout, basic_type, build_aggregate, builder_error,
    emit_array_literal, emit_place_value, emit_place_value_in_project, emit_struct_literal,
    emit_typed_expression, emit_utf8_literal, entry_alloca, entry_storage_alloca, extract_output, float_constant,
    integer_constant, integer_scalar_type, integer_type, integer_width, place_pointer,
    pointer_integer_type, storage_alignment, storage_type, storage_type_and_alignment, store_value, struct_member_place, structure, take_basic,
    value_type,
};
fn emit_binding_value<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    binding: &crate::ScalarBinding,
) -> Result<EmitValue<'ctx>, String> {
    if let Some(value) = local_callable::binding_value(context, state, binding, None)? {
        return Ok(value);
    }
    if binding.output_sequence.outputs.len() == 1 {
        return emit_typed_expression(context, state, &binding.value, &binding.declared_type);
    }
    match binding.output_origin {
        ScalarBindingOutputOrigin::SingleExpression => {
            emit_expression(context, state, &binding.value)
        }
        ScalarBindingOutputOrigin::IndependentExpressions => {
            let values = binding
                .output_values
                .iter()
                .enumerate()
                .map(|(position, output)| {
                    let ScalarBindingOutput::Evaluate(expression) = binding.output_source(position)
                    else {
                        unreachable!("independent binding output evaluates its source");
                    };
                    emit_typed_expression(context, state, expression, &output.ty)
                })
                .collect::<Result<Vec<_>, _>>()?;
            build_aggregate(context, state, &binding.output_sequence, values)
        }
    }
}

fn store_binding_outputs<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    binding: &crate::ScalarBinding,
    value: EmitValue<'ctx>,
) -> Result<(), String> {
    if local_callable::store_binding(context, state, binding, &value)? {
        return Ok(());
    }
    if binding.is_allocation {
        let receiver = binding
            .receivers
            .first()
            .ok_or_else(|| "runtime allocation has no receiver".to_owned())?;
        let ScalarType::RuntimeArray { element, .. } = &receiver.ty else {
            return Err("allocation binding requires a runtime array type".to_owned());
        };
        let length = receiver
            .allocation_length
            .as_ref()
            .ok_or_else(|| "runtime allocation has no length".to_owned())?;
        let length = match aggregate_transfer::checked_array_formal_extent(state, binding, length)? {
            Some(length) => length,
            None => take_basic(emit_typed_expression(context, state, length, &ScalarType::U64)?)?
                .into_int_value(),
        };
        return store_runtime_allocation(context, state, binding, receiver, element, length, false, |_, _| Ok(()));
    }
    if aggregate_transfer::store_received_binding(context, state, binding, &value)? {
        return Ok(());
    }
    if aggregate_transfer::install_ordinary_array_descriptor(context, state, binding, &value)?
        || aggregate_transfer::install_widened_owning_receiver(context, state, binding, &value)? {
        return Ok(());
    }
    if store_widened_join(context, state, binding, &value)? {
        return Ok(());
    }
    if store_widened_move(state, binding, &value)? {
        return Ok(());
    }
    let receivers = binding_receivers(binding);
    for (position, (name, ty)) in receivers.iter().enumerate() {
        let value = extract_output(state, &value, position)?;
        if *ty == ScalarType::Unit {
            state.values.insert(name.clone(), EmitValue::Unit);
            continue;
        }
        let slot = entry_storage_alloca(context, state, ty, name)?;
        store_value(context, state, slot, ty, value)?;
        state.storage.insert(name.clone(), (slot, ty.clone()));
        transition_runtime_array_binding_initialization(state, name, ty, &binding.value)?;
        transition_automatic_return_result_binding_initialization(
            state,
            name,
            ty,
            &binding.value,
            position,
        )?;
    }
    Ok(())
}

fn store_widened_join<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    binding: &crate::ScalarBinding,
    value: &EmitValue<'ctx>,
) -> Result<bool, String> {
    let Some(plan) = state.aggregate.as_ref().map(|aggregate| aggregate.plan())
        .filter(|plan| !plan.widened_join_receivers.is_empty()) else { return Ok(false) };
    let [receiver] = binding.receivers.as_slice() else { return Ok(false) };
    if !plan.widened_join_receivers.contains(&(receiver.name_span.start, receiver.name_span.end)) {
        return Ok(false);
    }
    install_widened_join(context, state, (&receiver.name, receiver.name_span, &receiver.ty), false, (false, None), value)?;
    Ok(true)
}

fn install_widened_join<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    (name, declaration, ty): (&str, wosy_syntax::ByteSpan, &ScalarType),
    replace: bool,
    (nullable, signed): (bool, Option<memory::SignedRuntimeAddressDomain>),
    value: &EmitValue<'ctx>,
) -> Result<(), String> {
    let EmitValue::Basic(BasicValueEnum::StructValue(pair)) = value else {
        return Err("missing checked widened array join descriptor".into());
    };
    if pair.get_type() != aggregate_transfer::array_descriptor_type(context) {
        return Err("invalid checked widened array join descriptor".into());
    }
    let backing = state.builder.build_extract_value(*pair, 0, "widened_join_backing")
        .map_err(builder_error)?.into_pointer_value();
    let count = state.builder.build_extract_value(*pair, 1, "widened_join_extent")
        .map_err(builder_error)?.into_int_value();
    if nullable {
        aggregate_transfer::require_ordinary_array_backing(context, state, backing)?;
    }
    if replace {
        let (previous, _) = state.storage.get(name).cloned().ok_or("missing widened join predecessor")?;
        if state.runtime_array_owners.get(name) != Some(&true)
            || state.runtime_array_allocations.get(name) != Some(&true)
            || !matches!(state.local_runtime_extents.get(&previous),
                Some((owner, LocalRuntimeExtent::WidenedLiteral { .. }, true)) if *owner == declaration)
        { return Err("widened join predecessor lost its exact owner".into()); }
        ownership::consume_runtime_array_release(context, state,
            ownership::RuntimeArrayRelease { pointer: previous.into(), live: None })?;
        state.local_runtime_extents.remove(&previous);
    }
    install_runtime_array_binding_owner(state, name, true, true, None);
    state.storage.insert(name.to_owned(), (backing, ty.clone()));
    state.local_runtime_extents.insert(backing, (declaration, LocalRuntimeExtent::WidenedLiteral { length: count, signed }, true));
    Ok(())
}

fn pack_widened_join_arm<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    expression: &ScalarExpression,
    value: &EmitValue<'ctx>,
) -> Result<Option<EmitValue<'ctx>>, String> {
    let Some(plan) = state.aggregate.as_ref().map(|aggregate| aggregate.plan())
        .filter(|plan| !plan.widened_join_arms.is_empty()) else { return Ok(None) };
    let nodes = state.body_call_nodes.ok_or("missing prepared widened array join")?;
    let Some(declaration) = nodes.expression_ordinal(expression).and_then(|ordinal| plan.widened_join_arms.get(&ordinal))
    else { return Ok(None) };
    let ScalarExpression::Name { name, .. } = expression else { return Err("invalid widened join arm".into()) };
    let EmitValue::Basic(BasicValueEnum::PointerValue(backing)) = value else {
        return Err("missing widened join arm backing".into());
    };
    let Some(&(owner, LocalRuntimeExtent::WidenedLiteral { length: count, .. }, true)) = state.local_runtime_extents.get(backing)
    else { return Err("missing widened join arm extent".into()) };
    if owner != *declaration || nodes.lexical_expression_declaration(expression) != Some(owner)
        || state.runtime_array_owners.get(name) != Some(&true)
        || state.runtime_array_allocations.get(name) != Some(&true)
    { return Err("widened join arm lacks its original owner".into()); }
    let pair = state.builder.build_insert_value(aggregate_transfer::array_descriptor_type(context).get_undef(),
        *backing, 0, "widened_arm_backing").map_err(builder_error)?.into_struct_value();
    let pair = state.builder.build_insert_value(pair, count, 1, "widened_arm_extent").map_err(builder_error)?;
    // The packed value now carries the sole cleanup obligation. An ordinary
    // block result's inferred output type is not the transfer authority here.
    ownership::retire_named_runtime_array_return(state, name);
    state.local_runtime_extents.remove(backing);
    Ok(Some(EmitValue::Basic(pair.into_struct_value().into())))
}

fn store_widened_move<'ctx>(
    state: &mut EmitState<'ctx, '_>,
    binding: &crate::ScalarBinding,
    value: &EmitValue<'ctx>,
) -> Result<bool, String> {
    if !matches!(binding.declared_type, ScalarType::RuntimeArray { .. }) { return Ok(false); }
    let ScalarExpression::Name { name: source, .. } = &binding.value else { return Ok(false) };
    let Some((backing, _)) = state.storage.get(source) else { return Ok(false) };
    let backing = *backing;
    let Some(&(declaration, extent @ LocalRuntimeExtent::WidenedLiteral { .. }, checked)) =
        state.local_runtime_extents.get(&backing)
    else { return Ok(false) };
    let [receiver] = binding.receivers.as_slice() else {
        return Err("widened array move requires one exact destination".into());
    };
    let Some(nodes) = state.body_call_nodes else { return Err("missing prepared widened array move".into()) };
    if nodes.lexical_expression_declaration(&binding.value) != Some(declaration)
        || nodes.binding_position(binding).is_none()
        || !matches!(receiver.ty, ScalarType::RuntimeArray { .. })
        || !matches!(value, EmitValue::Basic(BasicValueEnum::PointerValue(actual)) if *actual == backing)
        || state.runtime_array_owners.get(source) != Some(&true)
        || state.runtime_array_allocations.get(source) != Some(&true)
    {
        return Err("widened array move changed its prepared owner".into());
    }
    // The ordinary ownership transition retires the source. Re-key the
    // existing extent under the sole receiving declaration, with no load,
    // store, new backing, or independent runtime owner.
    install_runtime_array_binding_owner(state, &receiver.name, true, true, Some(source));
    state.storage.insert(receiver.name.clone(), (backing, receiver.ty.clone()));
    state.local_runtime_extents.insert(backing, (receiver.name_span, extent, checked));
    Ok(true)
}

fn store_runtime_allocation<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    binding: &crate::ScalarBinding,
    receiver: &crate::ScalarOutputReceiver,
    element: &ScalarType,
    length: inkwell::values::IntValue<'ctx>,
    widened: bool,
    initialize: impl FnOnce(&mut EmitState<'ctx, 'module>, PointerValue<'ctx>) -> Result<(), String>,
) -> Result<(), String> {
    let declaration = (receiver.name_span.start, receiver.name_span.end);
    let mutable = state.mutable_runtime_allocations.contains(&declaration);
    if widened && mutable {
        return Err("widened array backing must remain in its original local binding".into());
    }
    let private_literal = aggregate_transfer::has_literal_array_plan(state);
    let prepared = state.body_call_nodes.is_some_and(|nodes| nodes.binding_position(binding).is_some());
    let checked_extent = widened || state.aggregate.as_ref().is_some_and(|aggregate| aggregate.plan().guarded_array_descriptor_leaf)
        || (prepared && state.require_release_checked_access && state.indexed_runtime_allocations.contains(&declaration));
    let tracked_extent = checked_extent || (prepared && state.observed_runtime_allocations.as_deref()
        .is_some_and(|owners| owners.binary_search(&declaration).is_ok()));
    aggregate_transfer::capture_borrowed_extent(state, receiver.name_span, length)?;
    let relay_site = state.aggregate.as_ref().and_then(|aggregate| {
        let owners = &aggregate.plan().fallible_relay_allocations;
        (!owners.is_empty()).then_some((owners.get(aggregate.fallible_relay_next).copied(),
            aggregate.fallible_relay_next + 1 < owners.len(), aggregate.fallible_relay_unwind))
    });
    if relay_site.is_some_and(|(expected, _, _)|
        expected != Some(receiver.name_span) || !prepared || mutable || widened || !tracked_extent)
    {
        return Err("invalid sealed fallible allocation relay backing".into());
    }
    let relay_unwind = match relay_site {
        None => None,
        Some((_, _, Some(unwind))) => Some(unwind),
        Some((_, _, None)) => Some(ownership::emit_relay_absence(context, state)?),
    };
    let portable = (prepared && checked_extent && !widened && !mutable && relay_unwind.is_none()
        && state.portable_runtime_allocations.contains(&declaration)
        && state.aggregate.as_ref().is_none_or(|aggregate| portable_runtime_plan_allows_plain_storage(aggregate.plan()))).then(|| memory::PortableRuntimeAddressDomain::for_construction(state.target_layout, element)).flatten();
    let allocation = ownership::emit_runtime_array_allocation_with_absence(
        context, state, element, length, checked_extent && portable.is_none(), relay_unwind,
    )?;
    let slot = allocation.backing;
    if let Some((target, producer_element)) = state.aggregate.as_ref()
        .filter(|aggregate| aggregate.plan().guarded_array_descriptor_leaf)
        .and_then(|aggregate| aggregate.plan().signed_array_descriptor_result.as_ref())
    {
        if *target != state.target_layout || producer_element != element
            || !allocation.signed.is_some_and(|domain| domain.matches_consumer(*target, producer_element))
        {
            return Err("private array descriptor lost producer address-domain authority".into());
        }
    }
    initialize(state, slot)?;
    aggregate_transfer::record_allocation(state, binding, &receiver.name, slot)?;
    if !mutable {
        if tracked_extent {
            let extent = if let Some(domain) = portable {
                LocalRuntimeExtent::PortableConstruction { length: allocation.length, domain }
            } else if widened {
                LocalRuntimeExtent::WidenedLiteral { length, signed: None }
            } else {
                LocalRuntimeExtent::Construction { length: allocation.length, signed: allocation.signed }
            };
            state.local_runtime_extents.insert(slot, (receiver.name_span, extent, checked_extent));
            if let Some(backing) = state.runtime_borrow_index.owners.get_mut(&receiver.name_span) {
                if widened || !checked_extent || backing.replace(slot).is_some() {
                    return Err("invalid checked borrow backing construction".into());
                }
            }
        }
        state
            .storage
            .insert(receiver.name.clone(), (slot, receiver.ty.clone()));
        if private_literal {
            aggregate_transfer::enroll_literal_array_allocation(state, binding, slot, length, None)?;
        } else {
            state.runtime_array_owners.insert(receiver.name.clone(), true);
        }
        state
            .runtime_array_allocations
            .insert(receiver.name.clone(), true);
        if let (Some((_, has_next, _)), Some(unwind)) = (relay_site, relay_unwind) {
            let unwind = if has_next {
                ownership::emit_relay_unwind(context, state, slot, unwind)?
            } else {
                unwind
            };
            let aggregate = state.aggregate.as_mut().ok_or("missing sealed fallible relay plan")?;
            aggregate.fallible_relay_next += 1;
            aggregate.fallible_relay_unwind = Some(unwind);
        }
        return Ok(());
    }
    let backing = entry_alloca(
        state,
        context.ptr_type(AddressSpace::default()).into(),
        &format!("{}_backing", receiver.name),
    )?;
    state
        .builder
        .build_store(backing, slot)
        .map_err(builder_error)?;
    let length_slot = entry_alloca(
        state,
        context.i64_type().into(),
        &format!("{}_length", receiver.name),
    )?;
    state
        .builder
        .build_store(length_slot, length)
        .map_err(builder_error)?;
    state
        .runtime_array_lengths
        .insert(receiver.name.clone(), length_slot);
    if private_literal {
        aggregate_transfer::enroll_literal_array_allocation(state, binding, slot, length, Some((backing, length_slot)))?;
    } else {
        let live = entry_alloca(
            state, context.bool_type().into(), &format!("{}_live", receiver.name),
        )?;
        state.builder.build_store(live, context.bool_type().const_int(1, false))
            .map_err(builder_error)?;
        aggregate_transfer::record_failure_allocation(state, binding, &receiver.name, slot, live)?;
        state.runtime_array_live.insert(receiver.name.clone(), live);
        state.runtime_array_owners.insert(receiver.name.clone(), true);
    }
    if tracked_extent {
        state.local_runtime_extents.insert(backing, (receiver.name_span, LocalRuntimeExtent::ManagedLengthSlot(length_slot), checked_extent));
    }
    state
        .storage
        .insert(receiver.name.clone(), (backing, receiver.ty.clone()));

    state
        .runtime_array_allocations
        .insert(receiver.name.clone(), false);
    Ok(())
}

fn store_widened_local_array<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    binding: &crate::ScalarBinding,
    project: Option<ProjectCallScope<'_>>,
) -> Result<bool, String> {
    if let Some((source, count)) = state.aggregate.as_ref()
        .and_then(|aggregate| {
            let [receiver] = binding.receivers.as_slice() else { return None };
            aggregate.plan().widened_fixed_copies.get(&(receiver.name_span.start, receiver.name_span.end)).copied()
        })
    {
        let [receiver] = binding.receivers.as_slice() else { unreachable!() };
        let ScalarExpression::Name { name, .. } = &binding.value else { return Err("widening requires an exact fixed Copy source".into()); };
        let nodes = state.body_call_nodes.ok_or("missing checked fixed Copy operand")?;
        if nodes.lexical_expression_declaration(&binding.value) != Some(source) || nodes.binding_position(binding).is_none() {
            return Err("fixed Copy widening changed its prepared binding".into());
        }
        let (source, fixed_ty) = state.storage.get(name).cloned().ok_or("missing fixed Copy storage")?;
        let (element, actual) = crate::scalar::widened_fixed_copy(binding, &fixed_ty)
            .ok_or("widened Copy backing must be fixed")?;
        if actual != count { return Err("fixed Copy widening changed its exact extent".into()); }
        let count = context.i64_type().const_int(count, false);
        store_runtime_allocation(context, state, binding, receiver, element, count, true, |state, backing|
            memory::copy_fixed_aggregate(context, state, backing, source, &fixed_ty))?;
        return Ok(true);
    }
    let Some((element, values, _)) = crate::scalar::widened_local_literal(binding) else {
        if matches!(binding.declared_type, ScalarType::RuntimeArray { .. })
            && matches!(&binding.value, ScalarExpression::Name { name, .. }
                if state.storage.get(name).is_some_and(|(_, ty)| crate::scalar::widened_fixed_copy(binding, ty).is_some()))
        {
            return Err("fixed Copy widening requires an exact prepared backing plan".into());
        }
        return Ok(false);
    };
    let [receiver] = binding.receivers.as_slice() else { return Err("invalid widened array receiver".into()) };
    let count = context.i64_type().const_int(values.len() as u64, false);
    let mut staged = memory::stage_ordinary_array_elements(context, state, values, element,
        match project {
            Some(project) => ExpressionPath::Project { module: project.module, modules: project.modules },
            None => ExpressionPath::Single,
        })?;
    store_runtime_allocation(context, state, binding, receiver, element, count, true, |state, backing| {
        let item = storage_type(context, element, state.layouts, state.target_layout)?;
        for (position, expression) in values.iter().enumerate() {
            let value = if let Some(staged) = staged.as_mut() {
                staged.next().expect("one staged value per widened array element")
            } else {
                match project {
                    Some(project) => emit_project_typed_expression(context, state, expression, element,
                        project.module, project.modules)?,
                    None => emit_typed_expression(context, state, expression, element)?,
                }
            };
            // This index comes from the exact same literal count supplied to
            // the successful allocation, not from an authored index.
            let destination = unsafe { state.builder.build_in_bounds_gep(
                item, backing, &[pointer_integer_type(context, state.target_layout).const_int(position as u64, false)],
                "widened_array_cell",
            ) }.map_err(builder_error)?;
            store_value(context, state, destination, element, value)?;
        }
        Ok(())
    })?;
    Ok(true)
}

fn transition_automatic_return_result_binding_initialization(
    state: &mut EmitState<'_, '_>,
    name: &str,
    ty: &ScalarType,
    value: &ScalarExpression,
    output: usize,
) -> Result<(), String> {
    let ScalarType::CheckedReference { inner, .. } = ty else {
        return Ok(());
    };
    let (source, owner_flag) = match value {
        ScalarExpression::Call { span, .. } => match state
            .checked_call_owners
            .get(&(span.start, span.end, output))
            .ok_or_else(|| format!("missing validated checked-call owner at {span:?}"))?
        {
            CheckedCallOwner::Fresh => (true, state.call_owner_flags.get(&output).copied()),
            CheckedCallOwner::Borrowed => (false, None),
            CheckedCallOwner::Mixed => (
                true,
                Some(
                    state
                        .call_owner_flags
                        .get(&output)
                        .copied()
                        .ok_or_else(|| format!("missing mixed-call owner flag at {span:?}"))?,
                ),
            ),
        },
        ScalarExpression::Name { name: source, .. } => (
            state.automatic_return_result_owners.contains_key(source),
            state.checked_owner_flags.get(source).copied(),
        ),
        _ => (false, None),
    };
    if source {
        if let Some(owner_flag) = owner_flag {
            state
                .checked_owner_flags
                .insert(name.to_owned(), owner_flag);
        }
        state.automatic_return_result_owners.insert(
            name.to_owned(),
            ScalarAutomaticReturnResult {
                aggregate_type: inner.as_ref().clone(),
                allocation_identity: ScalarAllocationIdentity {
                    binding: name.to_owned(),
                },
                state: ScalarAutomaticReturnResultState::Live,
            },
        );
        if let ScalarExpression::Name { name: source, .. } = value {
            state.automatic_return_result_owners.remove(source);
            state.checked_owner_flags.remove(source);
        }
    }
    Ok(())
}

fn release_automatic_return_result_owners<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    outer_owners: &BTreeMap<String, ScalarAutomaticReturnResult>,
) -> Result<(), String> {
    let owners = state
        .automatic_return_result_owners
        .keys()
        .filter(|name| !outer_owners.contains_key(*name))
        .cloned()
        .collect::<Vec<_>>();
    for name in owners {
        let (slot, ty) = state
            .storage
            .get(&name)
            .cloned()
            .ok_or_else(|| format!("missing automatic return-result storage for {name}"))?;
        let address = state
            .builder
            .build_load(
                basic_type(context, &ty, state.target_layout)?,
                slot,
                "return_result_owner",
            )
            .map_err(builder_error)?
            .into_int_value();
        let block = state
            .builder
            .get_insert_block()
            .ok_or_else(|| "missing return-result release block".to_owned())?;
        let function = block
            .get_parent()
            .ok_or_else(|| "missing return-result release function".to_owned())?;
        let free = context.append_basic_block(function, "return_result_release.live");
        let next = context.append_basic_block(function, "return_result_release.next");
        let mut is_live = state
            .builder
            .build_int_compare(
                IntPredicate::NE,
                address,
                address.get_type().const_zero(),
                "return_result_release_live",
            )
            .map_err(builder_error)?;
        if let Some(flag) = state.checked_owner_flags.get(&name) {
            is_live = state
                .builder
                .build_and(is_live, *flag, "return_result_owned")
                .map_err(builder_error)?;
        }
        state
            .builder
            .build_conditional_branch(is_live, free, next)
            .map_err(builder_error)?;
        state.builder.position_at_end(free);
        let pointer = state
            .builder
            .build_int_to_ptr(
                address,
                context.ptr_type(AddressSpace::default()),
                "return_result_release",
            )
            .map_err(builder_error)?;
        declare_core_runtime(context, state.module);
        let release = state
            .module
            .get_function(CORE_FREE_SYMBOL)
            .ok_or_else(|| "core free runtime declaration is missing".to_owned())?;
        state
            .builder
            .build_call(release, &[pointer.into()], "return_result_release")
            .map_err(builder_error)?;
        state
            .builder
            .build_unconditional_branch(next)
            .map_err(builder_error)?;
        state.builder.position_at_end(next);
        state.automatic_return_result_owners.remove(&name);
        state.checked_owner_flags.remove(&name);
    }
    Ok(())
}

fn transition_runtime_array_binding_initialization(
    state: &mut EmitState<'_, '_>,
    name: &str,
    ty: &ScalarType,
    value: &ScalarExpression,
) -> Result<(), String> {
    if !matches!(ty, ScalarType::RuntimeArray { .. }) {
        return Ok(());
    }
    let provenance = runtime_array_assignment_provenance(state, value)?;
    install_runtime_array_binding_owner(state, name, provenance.result.owns(), false, provenance.source_name.as_deref());
    Ok(())
}

fn install_runtime_array_binding_owner(
    state: &mut EmitState<'_, '_>,
    name: &str,
    owns: bool,
    direct_backing: bool,
    source: Option<&str>,
) {
    state.runtime_array_owners.insert(name.into(), owns);
    state.runtime_array_allocations.insert(name.into(), direct_backing);
    if owns {
        if let Some(source) = source.filter(|source| *source != name) {
            state.runtime_array_owners.insert(source.into(), false);
        }
    }
}

fn runtime_array_assignment_provenance(
    state: &EmitState<'_, '_>,
    expression: &ScalarExpression,
) -> Result<RuntimeArrayAssignmentProvenance, String> {
    Ok(match expression {
        ScalarExpression::Name { name, .. } => RuntimeArrayAssignmentProvenance {
            result: if state
                .runtime_array_owners
                .get(name)
                .copied()
                .unwrap_or(false)
            {
                RuntimeArrayResultProvenance::LocalOwner
            } else {
                RuntimeArrayResultProvenance::ParameterAlias
            },
            source_name: Some(name.clone()),
        },
        ScalarExpression::Call { .. } if state.has_prepared_calls() => {
            let target = state
                .prepared_call(expression)?
                .expect("prepared call node");
            RuntimeArrayAssignmentProvenance {
                result: target
                    .symbol()
                    .and_then(|target| state.runtime_array_results.get(target))
                    .copied()
                    .unwrap_or(RuntimeArrayResultProvenance::FreshCall),
                source_name: None,
            }
        }
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            ..
        } => {
            let qualified = receiver
                .as_ref()
                .map_or_else(|| name.clone(), |receiver| format!("{receiver}.{name}"));
            let target = state.raw_call_symbol(expression, &qualified, type_arguments)?;
            RuntimeArrayAssignmentProvenance {
                result: target
                    .and_then(|target| state.runtime_array_results.get(target))
                    .copied()
                    .unwrap_or(RuntimeArrayResultProvenance::FreshCall),
                source_name: None,
            }
        }
        _ => RuntimeArrayAssignmentProvenance {
            result: RuntimeArrayResultProvenance::FixedWidening,
            source_name: None,
        },
    })
}

fn insert_function_result_provenance(
    runtime_array_results: &mut BTreeMap<String, RuntimeArrayResultProvenance>,
    dynamic_owner_outputs: &mut BTreeMap<String, BTreeSet<usize>>,
    name: &str,
    function: &ScalarFunction,
    reference_cfg: &ScalarReferenceCfg,
) {
    if let Some(provenance) = function_runtime_array_result_provenance(function) {
        runtime_array_results.insert(name.into(), provenance);
    }
    dynamic_owner_outputs.insert(
        name.into(),
        dynamic_return_outputs(function.span, reference_cfg),
    );
}

mod ownership;
use ownership::{
    binding_receivers, checked_call_owners, dynamic_return_outputs,
    emit_runtime_array_allocation, fresh_return_outputs, function_runtime_array_result_provenance,
    initialize_joined_return_bindings, joined_assignment_owners, joined_return_binding_types,
    release_scope_runtime_array_owners, return_bindings, transfer_runtime_array_return,
};
fn emit_project_binding_value<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    binding: &crate::ScalarBinding,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    if matches!(binding.value, ScalarExpression::GeneratedArray { .. }) {
        return memory::emit_generated_fixed_scalar_array(context, state, binding);
    }
    if let Some(value) = local_callable::binding_value(context, state, binding, Some(ProjectCallScope { module, modules }))? {
        return Ok(value);
    }
    if binding.output_sequence.outputs.len() == 1 {
        return emit_project_typed_expression(
            context,
            state,
            &binding.value,
            &binding.declared_type,
            module,
            modules,
        );
    }
    match binding.output_origin {
        ScalarBindingOutputOrigin::SingleExpression => {
            emit_project_expression(context, state, &binding.value, module, modules)
        }
        ScalarBindingOutputOrigin::IndependentExpressions => {
            let values = binding
                .output_values
                .iter()
                .enumerate()
                .map(|(position, output)| {
                    let ScalarBindingOutput::Evaluate(expression) = binding.output_source(position)
                    else {
                        unreachable!("independent binding output evaluates its source");
                    };
                    emit_project_typed_expression(
                        context, state, expression, &output.ty, module, modules,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            build_aggregate(context, state, &binding.output_sequence, values)
        }
    }
}

fn emit_function<'ctx, 'module>(
    prepared_render: bool,
    require_release_checked_access: bool,
    aggregate_plan: Option<&'module crate::scalar::ClosedBody>,
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    call_targets: &'ctx BTreeMap<String, String>,
    signatures: &'ctx BTreeMap<String, ScalarType>,
    runtime_array_results: &'ctx BTreeMap<String, RuntimeArrayResultProvenance>,
    dynamic_owner_outputs: &'ctx BTreeMap<String, BTreeSet<usize>>,
    globals: &'module BTreeMap<String, GlobalStorage<'ctx>>,
    function: &ScalarFunction,
    reference_cfg: &ScalarReferenceCfg,
    body_call_nodes: Option<&'module CallableBodyCallNodes<'module>>,
    initializer_symbols: &'module InitializerSymbols<'module>,
    name: &str,
    names: &'module ModuleNames<'ctx, 'module>,
    module: &'module Module<'ctx>,
    structs: &'module [&'module ScalarStruct],
    layouts: &'module crate::scalar::layout::ScalarLayoutIndex<'module>,
    target_layout: ScalarTargetLayout,
) -> Result<(), String> {
    let value = *functions
        .get(name)
        .ok_or_else(|| format!("unknown LLVM function {name}"))?;
    let entry = context.append_basic_block(value, "entry");
    builder.position_at_end(entry);
    let array_descriptor_observer = aggregate_plan.and_then(|plan| plan.array_descriptor_observer);
    let array_descriptor_forward = aggregate_plan.and_then(|plan| plan.array_descriptor_forward);
    if array_descriptor_forward.is_none() && array_descriptor_observer.is_none()
        && aggregate_plan.is_some_and(|plan| plan.array_descriptor_identity) {
        return aggregate_transfer::emit_array_descriptor_identity(context, builder, value);
    }
    if let Some(slot) = aggregate_plan.and_then(|plan| plan.identity_relay) {
        return aggregate_transfer::emit_identity_relay(builder, value, slot);
    }
    let (fresh_return_outputs, branch_return_outputs) =
        fresh_return_outputs(function, reference_cfg)?;
    let joined_bindings = joined_return_binding_types(function, &branch_return_outputs)?;
    let joined_assignment_owners =
        joined_assignment_owners(function.span, reference_cfg, &joined_bindings)?;
    let (mutable_runtime_allocations, indexed_runtime_allocations, runtime_borrow_index, portable_runtime_allocations) =
        runtime_allocation_access(reference_cfg, body_call_nodes, aggregate_plan, require_release_checked_access,
            portable_lexical_runtime_body(target_layout, require_release_checked_access, function, reference_cfg), false);
    let mut state = EmitState {
        prepared_render,
        require_release_checked_access,
        aggregate: aggregate_transfer::AggregateEmitter::new(
            aggregate_plan,
            value,
            function.parameters.len(),
        )?,
        initializer_aggregate: None,
        initializer_nodes: None,
        body_call_nodes,
        local_callable_slots: HashMap::new(),
        local_callable_arrays: HashMap::new(),
        guarded_scalar: ownership::GuardedScalarSites::from_cfg(reference_cfg),
        initializer_symbols: Some(initializer_symbols),
        builder,
        module,
        functions,
        call_targets,
        signatures,
        runtime_array_results,
        dynamic_owner_outputs,
        values: BTreeMap::new(),
        storage: StorageBindings::default(),
        return_bindings: return_bindings(&function.body),
        fresh_return_outputs,
        branch_return_outputs,
        joined_return_bindings: BTreeMap::new(),
        joined_assignment_owners,
        return_output_types: match &function.signature {
            ScalarType::Callable { outputs, .. } => outputs
                .outputs
                .iter()
                .map(|output| output.ty.clone())
                .collect(),
            _ => return Err("function has no callable signature".to_owned()),
        },
        checked_call_owners: checked_call_owners(reference_cfg)?,
        checked_owner_flags: BTreeMap::new(),
        call_owner_flags: BTreeMap::new(),
        return_owner_flags: BTreeMap::new(),
        return_owner_slots: BTreeMap::new(),
        runtime_array_owners: BTreeMap::new(),
        runtime_array_allocations: BTreeMap::new(),
        runtime_array_lengths: BTreeMap::new(),
        local_runtime_extents: HashMap::new(),
        indexed_runtime_allocations,
        portable_runtime_allocations,
        runtime_borrow_index,
        observed_runtime_allocations: reference_cfg.observed_runtime_allocations.clone(),
        runtime_array_live: BTreeMap::new(),
        mutable_runtime_allocations,
        invalidations: resolved_invalidations(reference_cfg)?,
        automatic_return_result_owners: BTreeMap::new(),
        globals: &names.globals,
        all_globals: globals,
        unit_values: &names.unit_values,
        project_names: None,
        structs,
        layouts,
        target_layout,
        next_literal: 0,
        next_block: 0,
    };
    use aggregate_transfer::ArrayDescriptorForward;
    if let Some(call) = array_descriptor_forward {
        return aggregate_transfer::emit_array_descriptor_forward(context, &state, value, function, call, ArrayDescriptorForward::Owned);
    }
    if let Some(call) = aggregate_plan.and_then(|plan| plan.private_runtime_read_forward) {
        return aggregate_transfer::emit_array_descriptor_forward(context, &state, value, function, call, ArrayDescriptorForward::PrivateSharedIndex);
    }
    if let Some(call) = aggregate_plan.and_then(|plan| plan.borrowed_extent_forward) {
        return aggregate_transfer::emit_array_descriptor_forward(context, &state, value, function, call, ArrayDescriptorForward::Borrowed);
    }
    if let Some(call) = aggregate_plan.and_then(|plan| plan.guarded_array_descriptor_forward) {
        return aggregate_transfer::emit_array_descriptor_forward(context, &state, value, function, call, ArrayDescriptorForward::GuardedProducer);
    }
    initialize_joined_return_bindings(context, &mut state, joined_bindings)?;
    if let ScalarType::Callable { parameters, .. } = &function.signature {
        for (position, output) in dynamic_return_outputs(function.span, reference_cfg)
            .into_iter()
            .enumerate()
        {
            let slot = value
                .get_nth_param((parameters.len() + position) as u32)
                .ok_or_else(|| format!("missing mixed-return owner slot {output}"))?
                .into_pointer_value();
            state.return_owner_slots.insert(output, slot);
        }
        for (index, parameter) in parameters.iter().enumerate() {
            let argument = value
                .get_nth_param(index as u32)
                .ok_or_else(|| "missing function parameter".to_owned())?;
            let (storage, alignment) =
                storage_type_and_alignment(context, parameter, layouts, target_layout)?;
            let slot = builder
                .build_alloca(storage, &function.parameters[index])
                .map_err(builder_error)?;
            align_storage_alloca(slot, alignment)?;
            let argument = if index == 0 && aggregate_plan.is_some_and(|plan| plan.borrowed_extent_observer.is_some() || plan.private_runtime_access_formal.is_some()) {
                aggregate_transfer::extract_borrowed_extent_formal(context, &mut state, argument)?
            } else if let Some(declaration) = array_descriptor_observer.filter(|_| index == 0) {
                let (backing, extent) = aggregate_transfer::extract_observing_array_formal(
                    context, builder, argument,
                )?;
                state.local_runtime_extents.insert(slot, (declaration, LocalRuntimeExtent::OwningFormal(extent), false));
                backing
            } else {
                argument
            };
            store_value(
                context,
                &mut state,
                slot,
                parameter,
                EmitValue::Basic(argument),
            )?;
            state.storage.insert(
                function.parameters[index].clone(),
                (slot, parameter.clone()),
            );
            if matches!(parameter, ScalarType::RuntimeArray { .. }) {
                state
                    .runtime_array_owners
                    .insert(function.parameters[index].clone(), false);
                state
                    .runtime_array_allocations
                    .insert(function.parameters[index].clone(), false);
            }
        }
    }
    let outputs = match &function.signature {
        ScalarType::Callable { outputs, .. } => outputs,
        _ => return Err("function has no callable signature".into()),
    };
    if state.guarded_scalar.is_none() && !aggregate_transfer::requires_scoped_function_emission(aggregate_plan) && !value.get_type().get_return_type().is_some_and(|ty| ty.is_array_type()) && self_tail::eligible(context, &state, value, function, reference_cfg) {
        return self_tail::emit(context, &mut state, value, function, None);
    }
    let single_output = if function.body.items.len() == 1 {
        function.body.final_expressions().next()
    } else {
        None
    };
    let result = match single_output {
        Some((expression, _))
            if !aggregate_transfer::requires_scoped_function_emission(aggregate_plan) && !value.get_type().get_return_type().is_some_and(|ty| ty.is_array_type()) && canonical_tail_if(&function.body, BlockEmission::Return(outputs), false).is_none() =>
        {
            if outputs.outputs.len() == 1 {
                emit_typed_expression(context, &mut state, expression, &outputs.outputs[0].ty)?
            } else {
                let value = emit_expression(context, &mut state, expression)?;
                if outputs
                    .outputs
                    .iter()
                    .all(|output| output.ty == outputs.outputs[0].ty)
                {
                    build_aggregate(
                        context,
                        &mut state,
                        outputs,
                        vec![value; outputs.outputs.len()],
                    )?
                } else {
                    value
                }
            }
        }
        _ => emit_block_at(
            context,
            &mut state,
            &function.body,
            BlockEmission::Return(outputs),
        )?,
    };
    if let Some((expression, _)) = single_output {
        for output in state.return_owner_slots.keys().copied().collect::<Vec<_>>() {
            record_forwarded_return_owner(context, &mut state, expression, output)?;
        }
    }
    release_automatic_return_result_owners(context, &mut state, &BTreeMap::new())?;
    if array_descriptor_observer.is_some() {
        return aggregate_transfer::emit_array_descriptor_identity(context, builder, value);
    }
    if let Some(returned) =
        aggregate_transfer::finish_return(&mut state)?
    {
        return Ok(returned);
    }
    emit_return(context, &mut state, result, outputs)
}

fn emit_main<'ctx, 'module>(
    prepared_render: bool,
    require_release_checked_access: bool,
    initializer_plan: Option<&'module crate::scalar::ClosedInitializer>,
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    call_targets: &'ctx BTreeMap<String, String>,
    signatures: &'ctx BTreeMap<String, ScalarType>,
    runtime_array_results: &'ctx BTreeMap<String, RuntimeArrayResultProvenance>,
    dynamic_owner_outputs: &'ctx BTreeMap<String, BTreeSet<usize>>,
    globals: &'module BTreeMap<String, GlobalStorage<'ctx>>,
    names: &'module ModuleNames<'ctx, 'module>,
    items: &[ScalarItem],
    initialization_region: &ModuleInitializationRegion,
    initializer_nodes: Option<&'module InitializerCallNodes<'module>>,
    initializer_symbols: &'module InitializerSymbols<'module>,
    module: &'module Module<'ctx>,
    structs: &'module [&'module ScalarStruct],
    layouts: &'module crate::scalar::layout::ScalarLayoutIndex<'module>,
    target_layout: ScalarTargetLayout,
) -> Result<(), String> {
    let main = *functions
        .get("main")
        .ok_or_else(|| "missing main".to_owned())?;
    let entry = context.append_basic_block(main, "entry");
    builder.position_at_end(entry);
    let mut state = EmitState {
        prepared_render,
        require_release_checked_access,
        aggregate: None,
        initializer_aggregate: initializer_plan.map(aggregate_transfer::InitializerEmitter::new),
        initializer_nodes,
        body_call_nodes: None,
        local_callable_slots: HashMap::new(),
        local_callable_arrays: HashMap::new(),
        guarded_scalar: None,
        initializer_symbols: Some(initializer_symbols),
        builder,
        module,
        functions,
        call_targets,
        signatures,
        runtime_array_results,
        dynamic_owner_outputs,
        values: BTreeMap::new(),
        storage: StorageBindings::default(),
        return_bindings: BTreeMap::new(),
        fresh_return_outputs: BTreeSet::new(),
        branch_return_outputs: BTreeMap::new(),
        joined_return_bindings: BTreeMap::new(),
        joined_assignment_owners: BTreeMap::new(),
        return_output_types: Vec::new(),
        checked_call_owners: BTreeMap::new(),
        checked_owner_flags: BTreeMap::new(),
        call_owner_flags: BTreeMap::new(),
        return_owner_flags: BTreeMap::new(),
        return_owner_slots: BTreeMap::new(),
        runtime_array_owners: BTreeMap::new(),
        runtime_array_allocations: BTreeMap::new(),
        runtime_array_lengths: BTreeMap::new(),
        local_runtime_extents: HashMap::new(),
        indexed_runtime_allocations: BTreeSet::new(),
        portable_runtime_allocations: BTreeSet::new(),
        runtime_borrow_index: RuntimeBorrowIndex::default(),
        observed_runtime_allocations: None,
        runtime_array_live: BTreeMap::new(),
        mutable_runtime_allocations: BTreeSet::new(),
        invalidations: BTreeMap::new(),
        automatic_return_result_owners: BTreeMap::new(),
        globals: &names.globals,
        all_globals: globals,
        unit_values: &names.unit_values,
        project_names: None,
        structs,
        layouts,
        target_layout,
        next_literal: 0,
        next_block: 0,
    };
    for step in &initialization_region.steps {
        match (&step.kind, &items[step.item]) {
            (ModuleInitializationStepKind::BindingInitialization, ScalarItem::Binding(binding)) => {
                if aggregate_transfer::emit_initializer_binding(context, &mut state, binding, None)? {
                    continue;
                }
                if binding.is_allocation {
                    initialize_runtime_array_global(context, &mut state, binding, |state, length| {
                        emit_typed_expression(context, state, length, &ScalarType::U64)
                    })?;
                    continue;
                }
                let value = emit_binding_value(context, &mut state, binding)?;
                for (position, (name, ty)) in binding_receivers(binding).iter().enumerate() {
                    if *ty == ScalarType::Unit {
                        continue;
                    }
                    let value = extract_output(&mut state, &value, position)?;
                    let (global, _) = state
                        .globals
                        .get(name)
                        .cloned()
                        .ok_or_else(|| format!("unknown LLVM global {name}"))?;
                    store_value(context, &mut state, global.as_pointer_value(), ty, value)?;
                    if matches!(ty, ScalarType::RuntimeArray { .. }) {
                        state
                            .storage
                            .insert(name.clone(), (global.as_pointer_value(), ty.clone()));
                    }
                    transition_runtime_array_binding_initialization(
                        &mut state,
                        name,
                        ty,
                        &binding.value,
                    )?;
                }
            }
            (ModuleInitializationStepKind::ExecutableItem, ScalarItem::Executable(item)) => {
                emit_block_item(context, &mut state, item)?;
            }
            (ModuleInitializationStepKind::NamespaceDispatch { .. }, ScalarItem::Namespace(_)) => {}
            _ => panic!(
                "module initialization step disagrees with item {} in {:?}",
                step.item, initialization_region.source,
            ),
        }
    }
    aggregate_transfer::finish_initializer(&mut state)?;
    release_scope_runtime_array_owners(context, &mut state, &BTreeMap::new())?;
    release_automatic_return_result_owners(context, &mut state, &BTreeMap::new())?;
    builder
        .build_return(Some(&context.i32_type().const_zero()))
        .map_err(builder_error)?;
    Ok(())
}

fn emit_project_function<'ctx, 'module>(
    prepared_render: bool,
    aggregate_plan: Option<&'module crate::scalar::ClosedBody>,
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    call_targets: &'ctx BTreeMap<String, String>,
    signatures: &'ctx BTreeMap<String, ScalarType>,
    runtime_array_results: &'ctx BTreeMap<String, RuntimeArrayResultProvenance>,
    dynamic_owner_outputs: &'ctx BTreeMap<String, BTreeSet<usize>>,
    function: &ScalarFunction,
    reference_cfg: &ScalarReferenceCfg,
    body_call_nodes: Option<&'module CallableBodyCallNodes<'module>>,
    initializer_symbols: &'module InitializerSymbols<'module>,
    source_module: &ScalarModule,
    modules: &[&'module ScalarModule],
    globals: &'module BTreeMap<String, GlobalStorage<'ctx>>,
    project_names: &'module ProjectNames<'ctx, 'module>,
    names: &'module ModuleNames<'ctx, 'module>,
    name: &str,
    module: &'module Module<'ctx>,
    structs: &'module [&'module ScalarStruct],
    layouts: &'module crate::scalar::layout::ScalarLayoutIndex<'module>,
    target_layout: ScalarTargetLayout,
    planned_runtime_access: Option<RuntimeAllocationAccess<'ctx>>,
) -> Result<(), String> {
    let value = *functions
        .get(name)
        .ok_or_else(|| format!("unknown LLVM function {name}"))?;
    let entry = context.append_basic_block(value, "entry");
    builder.position_at_end(entry);
    let array_descriptor_observer = aggregate_plan.and_then(|plan| plan.array_descriptor_observer);
    let array_descriptor_forward = aggregate_plan.and_then(|plan| plan.array_descriptor_forward);
    if array_descriptor_forward.is_none() && array_descriptor_observer.is_none()
        && aggregate_plan.is_some_and(|plan| plan.array_descriptor_identity) {
        return aggregate_transfer::emit_array_descriptor_identity(context, builder, value);
    }
    if let Some(slot) = aggregate_plan.and_then(|plan| plan.identity_relay) {
        return aggregate_transfer::emit_identity_relay(builder, value, slot);
    }
    let (fresh_return_outputs, branch_return_outputs) =
        fresh_return_outputs(function, reference_cfg)?;
    let joined_bindings = joined_return_binding_types(function, &branch_return_outputs)?;
    let joined_assignment_owners =
        joined_assignment_owners(function.span, reference_cfg, &joined_bindings)?;
    let (mutable_runtime_allocations, indexed_runtime_allocations, runtime_borrow_index, portable_runtime_allocations) =
        planned_runtime_access.unwrap_or_else(|| runtime_allocation_access(reference_cfg, body_call_nodes, aggregate_plan,
            project_names.require_release_checked_access,
            portable_lexical_runtime_body(target_layout, project_names.require_release_checked_access, function, reference_cfg), false));
    let mut state = EmitState {
        prepared_render,
        require_release_checked_access: project_names.require_release_checked_access,
        aggregate: aggregate_transfer::AggregateEmitter::new(
            aggregate_plan,
            value,
            function.parameters.len(),
        )?,
        initializer_aggregate: None,
        initializer_nodes: None,
        body_call_nodes,
        local_callable_slots: HashMap::new(),
        local_callable_arrays: HashMap::new(),
        guarded_scalar: ownership::GuardedScalarSites::from_cfg(reference_cfg),
        initializer_symbols: Some(initializer_symbols),
        builder,
        module,
        functions,
        call_targets,
        signatures,
        runtime_array_results,
        dynamic_owner_outputs,
        values: BTreeMap::new(),
        storage: StorageBindings::default(),
        return_bindings: return_bindings(&function.body),
        fresh_return_outputs,
        branch_return_outputs,
        joined_return_bindings: BTreeMap::new(),
        joined_assignment_owners,
        return_output_types: match &function.signature {
            ScalarType::Callable { outputs, .. } => outputs
                .outputs
                .iter()
                .map(|output| output.ty.clone())
                .collect(),
            _ => return Err("function has no callable signature".to_owned()),
        },
        checked_call_owners: checked_call_owners(reference_cfg)?,
        checked_owner_flags: BTreeMap::new(),
        call_owner_flags: BTreeMap::new(),
        return_owner_flags: BTreeMap::new(),
        return_owner_slots: BTreeMap::new(),
        runtime_array_owners: BTreeMap::new(),
        runtime_array_allocations: BTreeMap::new(),
        runtime_array_lengths: BTreeMap::new(),
        local_runtime_extents: HashMap::new(),
        indexed_runtime_allocations,
        portable_runtime_allocations,
        runtime_borrow_index,
        observed_runtime_allocations: reference_cfg.observed_runtime_allocations.clone(),
        runtime_array_live: BTreeMap::new(),
        mutable_runtime_allocations,
        invalidations: resolved_invalidations(reference_cfg)?,
        automatic_return_result_owners: BTreeMap::new(),
        globals: &names.globals,
        all_globals: globals,
        unit_values: &names.unit_values,
        project_names: Some(project_names),
        structs,
        layouts,
        target_layout,
        next_literal: 0,
        next_block: 0,
    };
    use aggregate_transfer::ArrayDescriptorForward;
    if let Some(call) = array_descriptor_forward {
        return aggregate_transfer::emit_array_descriptor_forward(context, &state, value, function, call, ArrayDescriptorForward::Owned);
    }
    if let Some(call) = aggregate_plan.and_then(|plan| plan.private_runtime_read_forward) {
        return aggregate_transfer::emit_array_descriptor_forward(context, &state, value, function, call, ArrayDescriptorForward::PrivateSharedIndex);
    }
    if let Some(call) = aggregate_plan.and_then(|plan| plan.borrowed_extent_forward) {
        return aggregate_transfer::emit_array_descriptor_forward(context, &state, value, function, call, ArrayDescriptorForward::Borrowed);
    }
    if let Some(call) = aggregate_plan.and_then(|plan| plan.guarded_array_descriptor_forward) {
        return aggregate_transfer::emit_array_descriptor_forward(context, &state, value, function, call, ArrayDescriptorForward::GuardedProducer);
    }
    initialize_joined_return_bindings(context, &mut state, joined_bindings)?;
    if let ScalarType::Callable { parameters, .. } = &function.signature {
        for (position, output) in dynamic_return_outputs(function.span, reference_cfg)
            .into_iter()
            .enumerate()
        {
            let slot = value
                .get_nth_param((parameters.len() + position) as u32)
                .ok_or_else(|| format!("missing mixed-return owner slot {output}"))?
                .into_pointer_value();
            state.return_owner_slots.insert(output, slot);
        }
        for (index, parameter) in parameters.iter().enumerate() {
            let argument = value
                .get_nth_param(index as u32)
                .ok_or_else(|| "missing function parameter".to_owned())?;
            let (storage, alignment) =
                storage_type_and_alignment(context, parameter, layouts, target_layout)?;
            let slot = builder
                .build_alloca(storage, &function.parameters[index])
                .map_err(builder_error)?;
            align_storage_alloca(slot, alignment)?;
            let argument = if index == 0 && aggregate_plan.is_some_and(|plan| plan.borrowed_extent_observer.is_some() || plan.private_runtime_access_formal.is_some()) {
                aggregate_transfer::extract_borrowed_extent_formal(context, &mut state, argument)?
            } else if let Some(declaration) = array_descriptor_observer.filter(|_| index == 0) {
                let (backing, extent) = aggregate_transfer::extract_observing_array_formal(
                    context, builder, argument,
                )?;
                state.local_runtime_extents.insert(slot, (declaration, LocalRuntimeExtent::OwningFormal(extent), false));
                backing
            } else {
                argument
            };
            store_value(
                context,
                &mut state,
                slot,
                parameter,
                EmitValue::Basic(argument),
            )?;
            state.storage.insert(
                function.parameters[index].clone(),
                (slot, parameter.clone()),
            );
            if matches!(parameter, ScalarType::RuntimeArray { .. }) {
                state
                    .runtime_array_owners
                    .insert(function.parameters[index].clone(), false);
                state
                    .runtime_array_allocations
                    .insert(function.parameters[index].clone(), false);
            }
        }
    }
    let outputs = match &function.signature {
        ScalarType::Callable { outputs, .. } => outputs,
        _ => return Err("function has no callable signature".into()),
    };
    if state.guarded_scalar.is_none() && !aggregate_transfer::requires_scoped_function_emission(aggregate_plan) && !value.get_type().get_return_type().is_some_and(|ty| ty.is_array_type()) && self_tail::eligible(context, &state, value, function, reference_cfg) {
        return self_tail::emit(
            context,
            &mut state,
            value,
            function,
            Some(ProjectCallScope {
                module: source_module,
                modules,
            }),
        );
    }
    let single_output = if function.body.items.len() == 1 {
        function.body.final_expressions().next()
    } else {
        None
    };
    let result = match single_output {
        Some((expression, _))
            if !aggregate_transfer::requires_scoped_function_emission(aggregate_plan) && !value.get_type().get_return_type().is_some_and(|ty| ty.is_array_type()) && canonical_tail_if(&function.body, BlockEmission::Return(outputs), false).is_none() =>
        {
            if outputs.outputs.len() == 1 {
                emit_project_typed_expression(
                    context,
                    &mut state,
                    expression,
                    &outputs.outputs[0].ty,
                    source_module,
                    modules,
                )?
            } else {
                let value = emit_project_expression(
                    context,
                    &mut state,
                    expression,
                    source_module,
                    modules,
                )?;
                if outputs
                    .outputs
                    .iter()
                    .all(|output| output.ty == outputs.outputs[0].ty)
                {
                    build_aggregate(
                        context,
                        &mut state,
                        outputs,
                        vec![value; outputs.outputs.len()],
                    )?
                } else {
                    value
                }
            }
        }
        _ => emit_project_block_at(
            context,
            &mut state,
            &function.body,
            source_module,
            modules,
            BlockEmission::Return(outputs),
        )?,
    };
    if let Some((expression, _)) = single_output {
        for output in state.return_owner_slots.keys().copied().collect::<Vec<_>>() {
            record_forwarded_return_owner(context, &mut state, expression, output)?;
        }
    }
    release_automatic_return_result_owners(context, &mut state, &BTreeMap::new())?;
    if array_descriptor_observer.is_some() {
        return aggregate_transfer::emit_array_descriptor_identity(context, builder, value);
    }
    if let Some(returned) =
        aggregate_transfer::finish_return(&mut state)?
    {
        return Ok(returned);
    }
    emit_return(context, &mut state, result, outputs)
}

fn emit_project_main<'ctx, 'module>(
    prepared_render: bool,
    aggregates: &'module crate::scalar::ClosedAggregates,
    context: &'ctx Context,
    builder: &'ctx Builder<'ctx>,
    functions: &'ctx BTreeMap<String, FunctionValue<'ctx>>,
    call_targets: &'ctx BTreeMap<String, String>,
    signatures: &'ctx BTreeMap<String, ScalarType>,
    runtime_array_results: &'ctx BTreeMap<String, RuntimeArrayResultProvenance>,
    dynamic_owner_outputs: &'ctx BTreeMap<String, BTreeSet<usize>>,
    modules: &[&ScalarModule],
    regions: &[&ModuleInitializationRegion],
    names: &'module ProjectNames<'ctx, 'module>,
    initializer_nodes: Option<&'module [InitializerCallNodes<'module>]>,
    initializer_symbols: &'module InitializerSymbols<'module>,
    globals: &'module BTreeMap<String, GlobalStorage<'ctx>>,
    module: &'module Module<'ctx>,
    structs: &'module [&'module ScalarStruct],
    layouts: &'module crate::scalar::layout::ScalarLayoutIndex<'module>,
    target_layout: ScalarTargetLayout,
) -> Result<(), String> {
    let main = *functions
        .get("main")
        .ok_or_else(|| "missing main".to_owned())?;
    let entry = context.append_basic_block(main, "entry");
    builder.position_at_end(entry);
    let empty_globals = GlobalNames(BTreeMap::new());
    let empty_unit_values = BTreeSet::new();
    let mut state = EmitState {
        prepared_render,
        require_release_checked_access: names.require_release_checked_access,
        aggregate: None,
        initializer_aggregate: None,
        initializer_nodes: None,
        body_call_nodes: None,
        local_callable_slots: HashMap::new(),
        local_callable_arrays: HashMap::new(),
        guarded_scalar: None,
        initializer_symbols: Some(initializer_symbols),
        builder,
        module,
        functions,
        call_targets,
        signatures,
        runtime_array_results,
        dynamic_owner_outputs,
        values: BTreeMap::new(),
        storage: StorageBindings::default(),
        return_bindings: BTreeMap::new(),
        fresh_return_outputs: BTreeSet::new(),
        branch_return_outputs: BTreeMap::new(),
        joined_return_bindings: BTreeMap::new(),
        joined_assignment_owners: BTreeMap::new(),
        return_output_types: Vec::new(),
        checked_call_owners: BTreeMap::new(),
        checked_owner_flags: BTreeMap::new(),
        call_owner_flags: BTreeMap::new(),
        return_owner_flags: BTreeMap::new(),
        return_owner_slots: BTreeMap::new(),
        runtime_array_owners: BTreeMap::new(),
        runtime_array_allocations: BTreeMap::new(),
        runtime_array_lengths: BTreeMap::new(),
        local_runtime_extents: HashMap::new(),
        indexed_runtime_allocations: BTreeSet::new(),
        portable_runtime_allocations: BTreeSet::new(),
        runtime_borrow_index: RuntimeBorrowIndex::default(),
        observed_runtime_allocations: None,
        runtime_array_live: BTreeMap::new(),
        mutable_runtime_allocations: BTreeSet::new(),
        invalidations: BTreeMap::new(),
        automatic_return_result_owners: BTreeMap::new(),
        globals: &empty_globals,
        all_globals: globals,
        unit_values: &empty_unit_values,
        project_names: Some(names),
        structs,
        layouts,
        target_layout,
        next_literal: 0,
        next_block: 0,
    };
    initialize_project_module(
        context,
        &mut state,
        aggregates,
        0,
        modules,
        regions,
        names,
        initializer_nodes,
        &mut vec![false; modules.len()],
    )?;
    release_scope_runtime_array_owners(context, &mut state, &BTreeMap::new())?;
    release_automatic_return_result_owners(context, &mut state, &BTreeMap::new())?;
    builder
        .build_return(Some(&context.i32_type().const_zero()))
        .map_err(builder_error)?;
    Ok(())
}

fn initialize_runtime_array_global<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    binding: &crate::ScalarBinding,
    emit_length: impl FnOnce(
        &mut EmitState<'ctx, 'module>,
        &ScalarExpression,
    ) -> Result<EmitValue<'ctx>, String>,
) -> Result<(), String> {
    let receiver = binding
        .receivers
        .first()
        .ok_or_else(|| "runtime allocation has no receiver".to_owned())?;
    let ScalarType::RuntimeArray { element, .. } = &receiver.ty else {
        return Err("allocation binding requires a runtime array type".to_owned());
    };
    let length = receiver
        .allocation_length
        .as_ref()
        .ok_or_else(|| "runtime allocation has no length".to_owned())?;
    let length = take_basic(emit_length(state, length)?)?.into_int_value();
    let pointer = emit_runtime_array_allocation(context, state, element, length)?;
    let (global, _) = state
        .globals
        .get(&receiver.name)
        .ok_or_else(|| format!("unknown LLVM global {}", receiver.name))?;
    let storage = global.as_pointer_value();
    state.builder.build_store(storage, pointer).map_err(builder_error)?;
    state.storage.insert(receiver.name.clone(), (storage, receiver.ty.clone()));
    state.runtime_array_owners.insert(receiver.name.clone(), true);
    state.runtime_array_allocations.insert(receiver.name.clone(), false);
    Ok(())
}

fn initialize_project_module<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    aggregates: &'module crate::scalar::ClosedAggregates,
    position: usize,
    modules: &[&'module ScalarModule],
    regions: &[&ModuleInitializationRegion],
    names: &'module ProjectNames<'ctx, 'module>,
    initializer_nodes: Option<&'module [InitializerCallNodes<'module>]>,
    initialized: &mut [bool],
) -> Result<(), String> {
    let module = modules[position];
    let region = regions[position];
    assert_eq!(region.source, module.source);
    if std::mem::replace(&mut initialized[position], true) {
        return Ok(());
    }

    aggregate_transfer::initializer_namespace_edge(state)?;
    let previous_aggregate = std::mem::replace(&mut state.initializer_aggregate,
        aggregates.initializer(&module.source).map(aggregate_transfer::InitializerEmitter::new));
    let previous_nodes = state.initializer_nodes;
    state.initializer_nodes = initializer_nodes.map(|nodes| &nodes[position]);

    let values = std::mem::take(&mut state.values);
    let storage = std::mem::take(&mut state.storage);
    let runtime_array_owners = std::mem::take(&mut state.runtime_array_owners);
    let runtime_array_allocations = std::mem::take(&mut state.runtime_array_allocations);
    let runtime_array_lengths = std::mem::take(&mut state.runtime_array_lengths);
    let runtime_array_live = std::mem::take(&mut state.runtime_array_live);
    let automatic_return_result_owners = std::mem::take(&mut state.automatic_return_result_owners);
    let checked_owner_flags = std::mem::take(&mut state.checked_owner_flags);
    let current_globals = std::mem::replace(&mut state.globals, &names.scopes[position].globals);
    let current_unit_values =
        std::mem::replace(&mut state.unit_values, &names.scopes[position].unit_values);

    for step in &region.steps {
        match (&step.kind, &module.items[step.item]) {
            (
                ModuleInitializationStepKind::NamespaceDispatch {
                    binding,
                    binding_span,
                    target,
                },
                ScalarItem::Namespace(namespace),
            ) => {
                assert_eq!(binding, &namespace.binding);
                assert_eq!(*binding_span, namespace.binding_span);
                let NamespaceTarget::Bound(target) = target else {
                    return Err(format!("unknown namespace binding {binding}"));
                };
                let &position = names.positions
                    .get(target)
                    .ok_or_else(|| format!("missing namespace target {}", target.path))?;
                initialize_project_module(
                    context,
                    state,
                    aggregates,
                    position,
                    modules,
                    regions,
                    names,
                    initializer_nodes,
                    initialized,
                )?;
            }
            (ModuleInitializationStepKind::BindingInitialization, ScalarItem::Binding(binding)) => {
                if aggregate_transfer::emit_initializer_binding(context, state, binding, Some((module, modules)))? {
                    continue;
                }
                if binding.is_allocation {
                    initialize_runtime_array_global(context, state, binding, |state, length| {
                        emit_project_typed_expression(
                            context, state, length, &ScalarType::U64, module, modules,
                        )
                    })?;
                    continue;
                }
                aggregate_transfer::prepare_ordinary_initializer_binding(state, binding, &module.source)?;
                let value = emit_project_binding_value(context, state, binding, module, modules)?;
                for (position, (name, ty)) in binding_receivers(binding).iter().enumerate() {
                    if *ty == ScalarType::Unit {
                        continue;
                    }
                    let value = extract_output(state, &value, position)?;
                    let (global, _) = state
                        .globals
                        .get(name)
                        .ok_or_else(|| format!("unknown LLVM global {name}"))?;
                    store_value(context, state, global.as_pointer_value(), ty, value)?;
                    if matches!(ty, ScalarType::RuntimeArray { .. }) {
                        state
                            .storage
                            .insert(name.clone(), (global.as_pointer_value(), ty.clone()));
                    }
                    transition_runtime_array_binding_initialization(
                        state,
                        name,
                        ty,
                        &binding.value,
                    )?;
                }
            }
            (ModuleInitializationStepKind::ExecutableItem, ScalarItem::Executable(item)) => {
                emit_project_block_item(context, state, item, module, modules)?;
            }
            _ => panic!(
                "module initialization step disagrees with item {} in {:?}",
                step.item, module.source,
            ),
        }
    }

    aggregate_transfer::finish_initializer(state)?;
    state.initializer_aggregate = previous_aggregate;
    state.initializer_nodes = previous_nodes;
    state.globals = current_globals;
    state.unit_values = current_unit_values;
    state.values = values;
    let mut retained_storage = storage;
    retained_storage.entries.extend(
        std::mem::take(&mut state.storage.entries)
            .into_iter()
            .map(|(name, storage)| (project_runtime_array_name(&module.source, &name), storage)),
    );
    state.storage = retained_storage;
    let mut retained_owners = runtime_array_owners;
    retained_owners.extend(
        std::mem::take(&mut state.runtime_array_owners)
            .into_iter()
            .map(|(name, owner)| (project_runtime_array_name(&module.source, &name), owner)),
    );
    state.runtime_array_owners = retained_owners;
    let mut retained_allocations = runtime_array_allocations;
    retained_allocations.extend(
        std::mem::take(&mut state.runtime_array_allocations)
            .into_iter()
            .map(|(name, allocation)| {
                (
                    project_runtime_array_name(&module.source, &name),
                    allocation,
                )
            }),
    );
    state.runtime_array_allocations = retained_allocations;
    let mut retained_lengths = runtime_array_lengths;
    retained_lengths.extend(
        std::mem::take(&mut state.runtime_array_lengths)
            .into_iter()
            .map(|(name, slot)| (project_runtime_array_name(&module.source, &name), slot)),
    );
    state.runtime_array_lengths = retained_lengths;
    let mut retained_live = runtime_array_live;
    retained_live.extend(
        std::mem::take(&mut state.runtime_array_live)
            .into_iter()
            .map(|(name, slot)| (project_runtime_array_name(&module.source, &name), slot)),
    );
    state.runtime_array_live = retained_live;
    let mut retained_return_results = automatic_return_result_owners;
    retained_return_results.extend(std::mem::take(&mut state.automatic_return_result_owners));
    state.automatic_return_result_owners = retained_return_results;
    state.checked_owner_flags = checked_owner_flags;
    Ok(())
}

#[derive(Clone, Copy)]
enum BlockEmission<'a> {
    Ordinary,
    ContextualNumeric(&'a ScalarType),
    Return(&'a crate::ScalarOutputSequence),
    TailArm(&'a crate::ScalarOutputSequence),
}

impl<'a> BlockEmission<'a> {
    fn expected_numeric(self) -> Option<&'a ScalarType> {
        match self {
            Self::ContextualNumeric(ty) => Some(ty),
            Self::Return(outputs) | Self::TailArm(outputs) => match outputs.outputs.as_slice() {
                [output]
                    if integer_width(&output.ty).is_some()
                        || matches!(output.ty, ScalarType::F32 | ScalarType::F64) =>
                {
                    Some(&output.ty)
                }
                _ => None,
            },
            Self::Ordinary => None,
        }
    }

    fn return_outputs(self) -> Option<&'a crate::ScalarOutputSequence> {
        match self {
            Self::Return(outputs) | Self::TailArm(outputs) => Some(outputs),
            _ => None,
        }
    }

    fn ordered_outputs(self) -> Option<&'a crate::ScalarOutputSequence> {
        match self {
            Self::Return(outputs) | Self::TailArm(outputs) if outputs.outputs.len() > 1 => {
                Some(outputs)
            }
            _ => None,
        }
    }

    fn explicit_ordered_outputs(
        self,
        expression_count: usize,
    ) -> Result<Option<&'a crate::ScalarOutputSequence>, String> {
        let (outputs, site) = match self {
            Self::TailArm(outputs) if outputs.outputs.len() > 1 => (outputs, "conditional arm"),
            Self::Return(outputs) if expression_count > 1 && outputs.outputs.len() > 1 => {
                (outputs, "callable return")
            }
            _ => return Ok(None),
        };
        if expression_count != outputs.outputs.len() {
            return Err(format!("{site} has the wrong number of LLVM outputs"));
        }
        Ok(Some(outputs))
    }
}

struct CanonicalTailIf<'syntax, 'signature> {
    expression: &'syntax ScalarExpression,
    condition: &'syntax ScalarExpression,
    then_branch: &'syntax ScalarBlock,
    else_branch: &'syntax ScalarBlock,
    outputs: &'signature crate::ScalarOutputSequence,
}

fn canonical_tail_if<'syntax, 'signature>(
    block: &'syntax ScalarBlock,
    emission: BlockEmission<'signature>,
    inline: bool,
) -> Option<CanonicalTailIf<'syntax, 'signature>> {
    let outputs = if inline { emission.return_outputs()? } else { emission.ordered_outputs()? };
    if block.final_output_values.len() != 1 {
        return None;
    }
    let (expression, _) = block.final_expressions().next()?;
    match expression {
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => Some(CanonicalTailIf {
            expression,
            condition,
            then_branch,
            else_branch,
            outputs,
        }),
        _ => None,
    }
}

fn emit_block<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
) -> Result<EmitValue<'ctx>, String> {
    emit_block_at(context, state, block, BlockEmission::Ordinary)
}

fn emit_block_at<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
    emission: BlockEmission<'_>,
) -> Result<EmitValue<'ctx>, String> {
    let storage_scope = state.storage.enter_scope();
    let values = state.values.clone();
    let runtime_array_owners = state.runtime_array_owners.clone();
    let runtime_array_allocations = state.runtime_array_allocations.clone();
    let runtime_array_lengths = state.runtime_array_lengths.clone();
    let runtime_array_live = state.runtime_array_live.clone();
    let automatic_return_result_owners = state.automatic_return_result_owners.clone();
    let checked_owner_flags = state.checked_owner_flags.clone();
    let final_start = block.items.len() - block.final_output_values.len();
    for item in &block.items[..final_start] {
        emit_block_item(context, state, item)?;
    }
    let result = if final_start < block.items.len() || matches!(emission, BlockEmission::TailArm(_)) {
        let ancestors = matches!(emission, BlockEmission::Ordinary | BlockEmission::ContextualNumeric(_))
            .then_some(&runtime_array_owners);
        emit_final_outputs(context, state, block, emission, ancestors)?
    } else {
        EmitValue::Unit
    };
    let result = finish_block_return(context, state, block, result, emission)?;
    let result = aggregate_transfer::commit_named_result_arm(context, state, block, result)?;
    let result = if let BlockEmission::TailArm(outputs) = emission {
        aggregate_transfer::commit_mixed_branch_arm(context, state, block, result, outputs)?
    } else {
        result
    };
    if let BlockEmission::Return(outputs) = emission {
        aggregate_transfer::commit_return(context, state, block, &result, outputs)?;
    }
    ownership::release_guarded_scalar(context, state, block.span)?;
    aggregate_transfer::release_scope(context, state, block)?;
    release_scope_runtime_array_owners(context, state, &runtime_array_owners)?;
    if matches!(emission, BlockEmission::Return(_)) {
        aggregate_transfer::release_checked_backing(context, state)?;
    }
    release_automatic_return_result_owners(context, state, &automatic_return_result_owners)?;
    if matches!(emission, BlockEmission::TailArm(_)) {
        aggregate_transfer::release_failure_allocation(context, state, block)?;
        aggregate_transfer::publish_checked_backing_arm(state, block)?;
    }
    state.storage.leave_scope(storage_scope);
    state.values = values;
    state.runtime_array_owners = runtime_array_owners
        .into_iter()
        .map(|(name, owner)| {
            let updated = *state.runtime_array_owners.get(&name).unwrap_or(&owner);
            (name, updated)
        })
        .collect();
    state.runtime_array_allocations = runtime_array_allocations;
    state.runtime_array_lengths = runtime_array_lengths;
    state.runtime_array_live = runtime_array_live;
    state.automatic_return_result_owners = automatic_return_result_owners;
    state.checked_owner_flags = checked_owner_flags;
    Ok(result)
}

fn emit_project_block<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    emit_project_block_at(
        context,
        state,
        block,
        module,
        modules,
        BlockEmission::Ordinary,
    )
}

fn emit_project_block_at<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
    module: &ScalarModule,
    modules: &[&ScalarModule],
    emission: BlockEmission<'_>,
) -> Result<EmitValue<'ctx>, String> {
    let storage_scope = state.storage.enter_scope();
    let values = state.values.clone();
    let runtime_array_owners = state.runtime_array_owners.clone();
    let runtime_array_allocations = state.runtime_array_allocations.clone();
    let runtime_array_lengths = state.runtime_array_lengths.clone();
    let runtime_array_live = state.runtime_array_live.clone();
    let automatic_return_result_owners = state.automatic_return_result_owners.clone();
    let checked_owner_flags = state.checked_owner_flags.clone();
    let final_start = block.items.len() - block.final_output_values.len();
    for item in &block.items[..final_start] {
        match item {
            ScalarBlockItem::LocalBinding(binding) if binding.fallible_assignment.is_some() => {
                emit_project_block_item(context, state, item, module, modules)?;
            }
            ScalarBlockItem::LocalBinding(binding) => {
                if ownership::move_guarded_runtime_owner(context, state, binding)?
                    || store_widened_local_array(context, state, binding, Some(ProjectCallScope { module, modules }))? {
                    continue;
                }
                let value = emit_project_binding_value(context, state, binding, module, modules)?;
                store_binding_outputs(context, state, binding, value)?;
            }
            ScalarBlockItem::Expression(expression) => {
                emit_project_expression(context, state, expression, module, modules)?;
            }
            ScalarBlockItem::Assignment(assignment) => {
                emit_project_assignment(context, state, assignment, module, modules)?;
            }
        }
    }
    let result = if final_start < block.items.len() || matches!(emission, BlockEmission::TailArm(_)) {
        let ancestors = matches!(emission, BlockEmission::Ordinary | BlockEmission::ContextualNumeric(_))
            .then_some(&runtime_array_owners);
        emit_project_final_outputs(context, state, block, module, modules, emission, ancestors)?
    } else {
        EmitValue::Unit
    };
    let result = finish_block_return(context, state, block, result, emission)?;
    let result = aggregate_transfer::commit_named_result_arm(context, state, block, result)?;
    let result = if let BlockEmission::TailArm(outputs) = emission {
        aggregate_transfer::commit_mixed_branch_arm(context, state, block, result, outputs)?
    } else {
        result
    };
    if let BlockEmission::Return(outputs) = emission {
        aggregate_transfer::commit_return(context, state, block, &result, outputs)?;
    }
    ownership::release_guarded_scalar(context, state, block.span)?;
    aggregate_transfer::release_scope(context, state, block)?;
    release_scope_runtime_array_owners(context, state, &runtime_array_owners)?;
    if matches!(emission, BlockEmission::Return(_)) {
        aggregate_transfer::release_checked_backing(context, state)?;
    }
    release_automatic_return_result_owners(context, state, &automatic_return_result_owners)?;
    if matches!(emission, BlockEmission::TailArm(_)) {
        aggregate_transfer::release_failure_allocation(context, state, block)?;
        aggregate_transfer::publish_checked_backing_arm(state, block)?;
    }
    state.storage.leave_scope(storage_scope);
    state.values = values;
    state.runtime_array_owners = runtime_array_owners
        .into_iter()
        .map(|(name, owner)| {
            let updated = *state.runtime_array_owners.get(&name).unwrap_or(&owner);
            (name, updated)
        })
        .collect();
    state.runtime_array_allocations = runtime_array_allocations;
    state.runtime_array_lengths = runtime_array_lengths;
    state.runtime_array_live = runtime_array_live;
    state.automatic_return_result_owners = automatic_return_result_owners;
    state.checked_owner_flags = checked_owner_flags;
    Ok(result)
}

fn final_output_expression(block: &ScalarBlock, position: usize) -> &ScalarExpression {
    let first_output = block.items.len() - block.final_output_values.len();
    let ScalarBlockItem::Expression(expression) = &block.items[first_output + position] else {
        unreachable!("canonical final output is an expression item")
    };
    expression
}

fn emit_final_outputs<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
    emission: BlockEmission<'_>,
    retained_ancestor_owners: Option<&BTreeMap<String, bool>>,
) -> Result<EmitValue<'ctx>, String> {
    if matches!(emission, BlockEmission::Return(_)) && aggregate_transfer::forwarded_mixed_return(state) {
        return Ok(EmitValue::Unit);
    }
    let inline = emission.return_outputs().is_some() && return_memory::inline_return_storage(state).is_some();
    if let Some(tail) = canonical_tail_if(block, emission, inline) {
        if matches!(emission, BlockEmission::Return(_)) {
            aggregate_transfer::match_mixed_branch_conditional(state, tail.expression)?;
        }
        let condition =
            take_basic(emit_expression(context, state, tail.condition)?)?.into_int_value();
        return emit_if_value(
            context,
            state,
            condition,
            tail.then_branch,
            tail.else_branch,
            tail.expression,
            |state, branch| {
                emit_block_at(context, state, branch, BlockEmission::TailArm(tail.outputs))
            },
        );
    }
    if inline && block.final_output_values.len() == 1 {
        if let ScalarExpression::Block(nested) = final_output_expression(block, 0) {
            return emit_block_at(context, state, nested, BlockEmission::TailArm(emission.return_outputs().unwrap()));
        }
    }
    if let Some(outputs) = emission.explicit_ordered_outputs(block.final_output_values.len())? {
        return emit_ordered_final_outputs(context, state, block, outputs, None);
    }
    if block.final_output_values.len() == 1 {
        let output = &block.final_output_values[0];
        let expression = final_output_expression(block, 0);
        if matches!(expression, ScalarExpression::Call { .. }) {
            if !inline {
                transfer_runtime_array_return(state, block, expression, &output.ty, retained_ancestor_owners);
            }
            transfer_automatic_return_result(state, expression);
            let value = emit_expression(context, state, expression)?;
            if !state
                .branch_return_outputs
                .contains_key(&(block.span.start, block.span.end))
            {
                record_forwarded_return_owner(context, state, expression, output.position)?;
            }
            return Ok(value);
        }
        let ty = if inline {
            let [declared] = emission.return_outputs().expect("inline result declaration").outputs.as_slice() else {
                return Err("invalid fixed copy return arity".into());
            };
            &declared.ty
        } else {
            emission.expected_numeric().unwrap_or(&output.ty)
        };
        let value = match aggregate_transfer::emit_checked_ordinary_snapshot(context, state, expression, None)? {
            Some(value) => value,
            None => emit_typed_expression(context, state, expression, ty)?,
        };
        if !state
            .branch_return_outputs
            .contains_key(&(block.span.start, block.span.end))
        {
            record_forwarded_return_owner(context, state, expression, output.position)?;
        }
        if let Some(pair) = pack_widened_join_arm(context, state, expression, &value)? {
            return Ok(pair);
        }
        // A sealed inline Copy return captures bytes before scope cleanup.
        // Its source owners cannot escape through those bytes.
        if !inline {
            transfer_runtime_array_return(state, block, expression, ty, retained_ancestor_owners);
        }
        transfer_automatic_return_result(state, expression);
        return Ok(value);
    }
    let outputs = crate::ScalarOutputSequence {
        outputs: block
            .final_output_values
            .iter()
            .map(|value| crate::ScalarOutput {
                ty: value.ty.clone(),
                span: value.span,
            })
            .collect(),
        span: block.span,
    };
    let values = block
        .final_output_values
        .iter()
        .enumerate()
        .map(|(position, output)| {
            let expression = final_output_expression(block, position);
            emit_typed_expression(context, state, expression, &output.ty)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !state
        .branch_return_outputs
        .contains_key(&(block.span.start, block.span.end))
    {
        for position in 0..block.final_output_values.len() {
            let expression = final_output_expression(block, position);
            record_forwarded_return_owner(context, state, expression, position)?;
        }
    }
    for (position, output) in block.final_output_values.iter().enumerate() {
        let expression = final_output_expression(block, position);
        transfer_runtime_array_return(state, block, expression, &output.ty, retained_ancestor_owners);
        transfer_automatic_return_result(state, expression);
    }
    build_aggregate(context, state, &outputs, values)
}

fn emit_project_final_outputs<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
    module: &ScalarModule,
    modules: &[&ScalarModule],
    emission: BlockEmission<'_>,
    retained_ancestor_owners: Option<&BTreeMap<String, bool>>,
) -> Result<EmitValue<'ctx>, String> {
    if matches!(emission, BlockEmission::Return(_)) && aggregate_transfer::forwarded_mixed_return(state) {
        return Ok(EmitValue::Unit);
    }
    let inline = emission.return_outputs().is_some() && return_memory::inline_return_storage(state).is_some();
    if let Some(tail) = canonical_tail_if(block, emission, inline) {
        if matches!(emission, BlockEmission::Return(_)) {
            aggregate_transfer::match_mixed_branch_conditional(state, tail.expression)?;
        }
        let condition = take_basic(emit_project_expression(
            context,
            state,
            tail.condition,
            module,
            modules,
        )?)?
        .into_int_value();
        return emit_if_value(
            context,
            state,
            condition,
            tail.then_branch,
            tail.else_branch,
            tail.expression,
            |state, branch| {
                emit_project_block_at(
                    context,
                    state,
                    branch,
                    module,
                    modules,
                    BlockEmission::TailArm(tail.outputs),
                )
            },
        );
    }
    if inline && block.final_output_values.len() == 1 {
        if let ScalarExpression::Block(nested) = final_output_expression(block, 0) {
            return emit_project_block_at(context, state, nested, module, modules, BlockEmission::TailArm(emission.return_outputs().unwrap()));
        }
    }
    if let Some(outputs) = emission.explicit_ordered_outputs(block.final_output_values.len())? {
        return emit_ordered_final_outputs(context, state, block, outputs, Some((module, modules)));
    }
    if block.final_output_values.len() == 1 {
        let output = &block.final_output_values[0];
        let expression = final_output_expression(block, 0);
        if matches!(expression, ScalarExpression::Call { .. }) {
            if !inline {
                transfer_runtime_array_return(state, block, expression, &output.ty, retained_ancestor_owners);
            }
            transfer_automatic_return_result(state, expression);
            let value = emit_project_expression(context, state, expression, module, modules)?;
            if !state
                .branch_return_outputs
                .contains_key(&(block.span.start, block.span.end))
            {
                record_forwarded_return_owner(context, state, expression, output.position)?;
            }
            return Ok(value);
        }
        let ty = if inline {
            let [declared] = emission.return_outputs().expect("inline result declaration").outputs.as_slice() else {
                return Err("invalid fixed copy return arity".into());
            };
            &declared.ty
        } else {
            emission.expected_numeric().unwrap_or(&output.ty)
        };
        let value = match aggregate_transfer::emit_checked_ordinary_snapshot(
            context, state, expression, Some(ProjectCallScope { module, modules }),
        )? {
            Some(value) => value,
            None => emit_project_typed_expression(context, state, expression, ty, module, modules)?,
        };
        if !state
            .branch_return_outputs
            .contains_key(&(block.span.start, block.span.end))
        {
            record_forwarded_return_owner(context, state, expression, output.position)?;
        }
        if let Some(pair) = pack_widened_join_arm(context, state, expression, &value)? {
            return Ok(pair);
        }
        // A sealed inline Copy return captures bytes before scope cleanup.
        // Its source owners cannot escape through those bytes.
        if !inline {
            transfer_runtime_array_return(state, block, expression, ty, retained_ancestor_owners);
        }
        transfer_automatic_return_result(state, expression);
        return Ok(value);
    }
    let outputs = crate::ScalarOutputSequence {
        outputs: block
            .final_output_values
            .iter()
            .map(|value| crate::ScalarOutput {
                ty: value.ty.clone(),
                span: value.span,
            })
            .collect(),
        span: block.span,
    };
    let values = block
        .final_output_values
        .iter()
        .enumerate()
        .map(|(position, output)| {
            let expression = final_output_expression(block, position);
            emit_project_typed_expression(context, state, expression, &output.ty, module, modules)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !state
        .branch_return_outputs
        .contains_key(&(block.span.start, block.span.end))
    {
        for position in 0..block.final_output_values.len() {
            let expression = final_output_expression(block, position);
            record_forwarded_return_owner(context, state, expression, position)?;
        }
    }
    for (position, output) in block.final_output_values.iter().enumerate() {
        let expression = final_output_expression(block, position);
        transfer_runtime_array_return(state, block, expression, &output.ty, retained_ancestor_owners);
        transfer_automatic_return_result(state, expression);
    }
    build_aggregate(context, state, &outputs, values)
}

fn emit_ordered_final_outputs<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    block: &ScalarBlock,
    outputs: &crate::ScalarOutputSequence,
    project: Option<(&ScalarModule, &[&ScalarModule])>,
) -> Result<EmitValue<'ctx>, String> {
    let values = block
        .final_expressions()
        .zip(&outputs.outputs)
        .map(|((expression, _), output)| match project {
            Some((module, modules)) => emit_project_typed_expression(
                context, state, expression, &output.ty, module, modules,
            ),
            None => emit_typed_expression(context, state, expression, &output.ty),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !state
        .branch_return_outputs
        .contains_key(&(block.span.start, block.span.end))
    {
        for (position, (expression, _)) in block.final_expressions().enumerate() {
            record_forwarded_return_owner(context, state, expression, position)?;
        }
    }
    record_named_branch_return_owners(context, state, block)?;
    for ((expression, _), output) in block.final_expressions().zip(&outputs.outputs) {
        transfer_runtime_array_return(state, block, expression, &output.ty, None);
        transfer_automatic_return_result(state, expression);
    }
    build_aggregate(context, state, outputs, values)
}

enum AssignmentDestination<'ctx> {
    Unit,
    Existing(PointerValue<'ctx>),
    NewStorage {
        name: String,
        slot: PointerValue<'ctx>,
    },
}

struct JoinedAssignmentPrevious<'ctx> {
    owner_slot: PointerValue<'ctx>,
    storage: PointerValue<'ctx>,
    owner: inkwell::values::IntValue<'ctx>,
    address: inkwell::values::IntValue<'ctx>,
}

fn capture_joined_assignment_previous<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    target: &crate::scalar::ScalarAssignmentTarget,
    ty: &ScalarType,
    destination: &AssignmentDestination<'ctx>,
) -> Result<Option<JoinedAssignmentPrevious<'ctx>>, String> {
    if matches!(destination, AssignmentDestination::Unit) {
        return Ok(None);
    }
    let crate::ScalarPlace::Name { name, .. } = &target.place else {
        return Ok(None);
    };
    let Some((owner_slot, _)) = state.joined_return_bindings.get(name).cloned() else {
        return Ok(None);
    };
    if !matches!(ty, ScalarType::CheckedReference { .. }) {
        return Err(format!(
            "joined return binding {name} lacks checked-reference type"
        ));
    }
    let AssignmentDestination::Existing(destination) = destination else {
        unreachable!("joined destination has existing storage")
    };
    let owner = state
        .builder
        .build_load(context.bool_type(), owner_slot, "joined_previous_owner")
        .map_err(builder_error)?
        .into_int_value();
    let address = state
        .builder
        .build_load(
            basic_type(context, ty, state.target_layout)?,
            *destination,
            "joined_previous_address",
        )
        .map_err(builder_error)?
        .into_int_value();
    Ok(Some(JoinedAssignmentPrevious {
        owner_slot,
        storage: *destination,
        owner,
        address,
    }))
}

#[derive(Clone, Copy)]
struct JoinedAssignmentSource<'ctx> {
    owner_slot: PointerValue<'ctx>,
    storage: PointerValue<'ctx>,
    owned: inkwell::values::IntValue<'ctx>,
}

struct AssignmentValue<'ctx> {
    value: EmitValue<'ctx>,
    ty: ScalarType,
    provenance: RuntimeArrayAssignmentProvenance,
    // Output ordinal within this evaluated RHS (not the group's LHS slot).
    output_position: usize,
    call_flag: Option<inkwell::values::IntValue<'ctx>>,
    joined_source: Option<JoinedAssignmentSource<'ctx>>,
    runtime_array_source: Option<PointerValue<'ctx>>,
    checked_rebind_destination: Option<usize>,
    literal_array_replacement: Option<(usize, usize)>,
    fresh_array_recommit: Option<(crate::scalar::ClosedFreshRecommit, inkwell::values::IntValue<'ctx>)>,
}

struct RuntimeArrayAssignment<'ctx> {
    owns: bool,
    previous: Option<ownership::RuntimeArrayRelease<'ctx>>,
    same_array_rebind: bool,
    literal_array_call: Option<usize>,
    fresh_array_slots: Option<(PointerValue<'ctx>, PointerValue<'ctx>, inkwell::values::IntValue<'ctx>)>,
}

fn stage_runtime_array_assignments<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    assignment: &ScalarAssignment,
    values: &[AssignmentValue<'ctx>],
    destinations: &[AssignmentDestination<'ctx>],
) -> Result<Vec<Option<RuntimeArrayAssignment<'ctx>>>, String> {
    let mut owns = values
        .iter()
        .map(|value| value.provenance.result.owns())
        .collect::<Vec<_>>();
    let mut transferred = std::collections::HashSet::new();
    let mut sources = Vec::new();
    let mut self_replacements = Vec::new();
    for (position, ((target, value), destination)) in assignment
        .targets
        .iter()
        .zip(values)
        .zip(destinations)
        .enumerate()
    {
        if !matches!(value.ty, ScalarType::RuntimeArray { .. })
            || !matches!(target.place, crate::ScalarPlace::Name { .. })
        {
            continue;
        }
        let Some(source) = value.runtime_array_source else {
            continue;
        };
        if matches!(destination, AssignmentDestination::Existing(slot) if *slot == source) {
            self_replacements.push((position, source));
        } else {
            owns[position] = transferred.insert(source);
            if owns[position] {
                sources.push((
                    value
                        .provenance
                        .source_name
                        .as_ref()
                        .expect("runtime-array lexical source"),
                    source,
                ));
            }
        }
    }
    let mut retains_storage = vec![false; values.len()];
    for (position, source) in self_replacements {
        owns[position] = !transferred.contains(&source);
        retains_storage[position] = true;
    }
    let mut staged = Vec::with_capacity(values.len());
    for (position, ((target, value), destination)) in assignment
        .targets
        .iter()
        .zip(values)
        .zip(destinations)
        .enumerate()
    {
        if let Some((call, expected)) = value.literal_array_replacement {
            if state.body_call_nodes.and_then(|nodes| nodes.place_ordinal(&target.place)) != Some(expected)
                || !matches!(value.ty, ScalarType::RuntimeArray { .. })
                || !matches!(target.place, crate::ScalarPlace::Name { .. })
                || !matches!(destination, AssignmentDestination::Existing(_))
            {
                return Err("checked literal array replacement does not match its assignment".into());
            }
            staged.push(Some(RuntimeArrayAssignment {
                owns: false, previous: None, same_array_rebind: false,
                literal_array_call: Some(call),
                fresh_array_slots: None,
            }));
            continue;
        }
        if let Some(expected) = value.checked_rebind_destination {
            if state.body_call_nodes.and_then(|nodes| nodes.place_ordinal(&target.place)) != Some(expected)
                || !matches!(value.ty, ScalarType::RuntimeArray { .. })
            {
                return Err("proved runtime-array rebind does not match the assignment destination".into());
            }
            let crate::ScalarPlace::Name { name, .. } = &target.place else {
                return Err("proved runtime-array rebind has no lexical destination".into());
            };
            if !matches!(destination, AssignmentDestination::Existing(_))
                || state.runtime_array_allocations.get(name) != Some(&false)
            {
                return Err("proved runtime-array rebind has no existing pointer storage".into());
            }
            staged.push(Some(RuntimeArrayAssignment {
                owns: false,
                previous: None,
                same_array_rebind: true,
                literal_array_call: None,
                fresh_array_slots: None,
            }));
            continue;
        }
        let crate::ScalarPlace::Name { name, .. } = &target.place else {
            staged.push(None);
            continue;
        };
        if !matches!(value.ty, ScalarType::RuntimeArray { .. }) {
            staged.push(None);
            continue;
        }
        if let Some((proof, length)) = value.fresh_array_recommit {
            if state.body_call_nodes.and_then(|nodes| nodes.place_ordinal(&target.place)) != Some(proof.destination)
                || !matches!(destination, AssignmentDestination::Existing(_))
                || !matches!(&value.value, EmitValue::Basic(BasicValueEnum::PointerValue(_)))
                || retains_storage[position]
            {
                return Err("fresh owner recommit differs from prepared destination".into());
            }
            let previous = ownership::capture_runtime_array_release(context, state, name)?;
            let fresh_array_slots = if proof.forward {
                if previous.is_some() {
                    return Err("forwarding fresh owner cannot discard a local allocation".into());
                }
                None
            } else {
                if previous.is_none() {
                    return Err("fresh owner recommit has no displaced local allocation".into());
                }
                Some((
                    *state.runtime_array_lengths.get(name).ok_or("fresh owner recommit lacks extent storage")?,
                    *state.runtime_array_live.get(name).ok_or("fresh owner recommit lacks live storage")?,
                    length,
                ))
            };
            staged.push(Some(RuntimeArrayAssignment {
                owns: !proof.forward, previous, same_array_rebind: false,
                literal_array_call: None, fresh_array_slots,
            }));
            continue;
        }
        let protected = retains_storage[position]
            || matches!(destination, AssignmentDestination::Existing(slot) if transferred.contains(slot));
        let previous = if protected {
            None
        } else {
            ownership::capture_runtime_array_release(context, state, name)?
        };
        staged.push(Some(RuntimeArrayAssignment {
            owns: owns[position],
            previous,
            same_array_rebind: false,
            literal_array_call: None,
            fresh_array_slots: None,
        }));
    }
    for (name, slot) in sources {
        if state.storage.get(name).map(|(storage, _)| *storage) == Some(slot) {
            state.runtime_array_owners.insert(name.clone(), false);
        }
    }
    Ok(staged)
}

fn stage_joined_assignment_owners<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    assignment: &ScalarAssignment,
    values: &[AssignmentValue<'ctx>],
    previous: &mut [Option<JoinedAssignmentPrevious<'ctx>>],
) -> Result<Vec<Option<inkwell::values::IntValue<'ctx>>>, String> {
    let mut owners = vec![None; previous.len()];
    let mut transferred = HashMap::new();
    let mut sources = Vec::new();
    let mut self_replacements = Vec::new();
    for (position, ((target, value), previous)) in assignment
        .targets
        .iter()
        .zip(values)
        .zip(previous.iter())
        .enumerate()
    {
        let (Some(source), Some(previous)) = (value.joined_source, previous) else {
            continue;
        };
        let classification = state
            .joined_assignment_owners
            .get(&(target.target_span.start, target.target_span.end))
            .ok_or_else(|| {
                format!(
                    "missing validated assignment owner at {:?}",
                    target.target_span
                )
            })?;
        if matches!(classification, CheckedAssignmentOwner::Borrowed) {
            continue;
        }
        if source.owner_slot == previous.owner_slot && source.storage == previous.storage {
            self_replacements.push((position, source));
            continue;
        }
        match transferred.entry((source.owner_slot, source.storage)) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(source.owned);
                owners[position] = Some(source.owned);
                sources.push(source);
            }
            std::collections::hash_map::Entry::Occupied(_) => {
                owners[position] = Some(context.bool_type().const_zero())
            }
        }
    }
    for (position, source) in self_replacements {
        owners[position] = Some(
            if transferred.contains_key(&(source.owner_slot, source.storage)) {
                context.bool_type().const_zero()
            } else {
                source.owned
            },
        );
    }
    for previous in previous.iter_mut().flatten() {
        if let Some(owned) = transferred.get(&(previous.owner_slot, previous.storage)) {
            let retained = state
                .builder
                .build_not(*owned, "joined_owner_not_transferred")
                .map_err(builder_error)?;
            previous.owner = state
                .builder
                .build_and(previous.owner, retained, "joined_previous_eligible")
                .map_err(builder_error)?;
        }
    }
    for source in sources {
        state
            .builder
            .build_store(source.owner_slot, context.bool_type().const_zero())
            .map_err(builder_error)?;
    }
    Ok(owners)
}

fn retain_assignment_destination<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    target: &crate::scalar::ScalarAssignmentTarget,
    ty: &ScalarType,
    project: Option<ProjectCallScope<'_>>,
) -> Result<AssignmentDestination<'ctx>, String> {
    if *ty == ScalarType::Unit {
        return Ok(AssignmentDestination::Unit);
    }
    match &target.place {
        crate::ScalarPlace::Name { name, .. } => {
            match state.storage.get(name).map(|(slot, _)| *slot).or_else(|| {
                state
                    .globals
                    .get(name)
                    .map(|(global, _)| global.as_pointer_value())
            }) {
                Some(destination) => Ok(AssignmentDestination::Existing(destination)),
                None => {
                    let slot = entry_storage_alloca(context, state, ty, name)?;
                    Ok(AssignmentDestination::NewStorage {
                        name: name.clone(),
                        slot,
                    })
                }
            }
        }
        crate::ScalarPlace::Index { base, index, .. }
            if matches!(ty, ScalarType::Struct(_) | ScalarType::Array { .. }) =>
        {
            Ok(AssignmentDestination::Existing(
                memory::checked_fixed_assignment_element_pointer(context, state, base, index, project)?,
            ))
        }
        place => Ok(AssignmentDestination::Existing(place_pointer(
            context, state, place, project,
        )?)),
    }
}

fn install_assignment_destination<'ctx>(
    state: &mut EmitState<'ctx, '_>,
    target: &crate::scalar::ScalarAssignmentTarget,
    ty: &ScalarType,
    destination: AssignmentDestination<'ctx>,
) -> Option<PointerValue<'ctx>> {
    match destination {
        AssignmentDestination::Unit => {
            state.values.insert(target.target.clone(), EmitValue::Unit);
            None
        }
        AssignmentDestination::Existing(pointer) => Some(pointer),
        AssignmentDestination::NewStorage { name, slot } => {
            state.storage.insert(name, (slot, ty.clone()));
            Some(slot)
        }
    }
}

fn emit_widened_join_replace<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    assignment: &ScalarAssignment,
    project: Option<ProjectCallScope<'_>>,
) -> Result<bool, String> {
    let Some(plan) = state.aggregate.as_ref().map(|aggregate| aggregate.plan())
        .filter(|plan| !plan.widened_join_replacements.is_empty()) else { return Ok(false) };
    let ([target], [rhs]) =
        (assignment.targets.as_slice(), assignment.values.as_slice()) else { return Ok(false) };
    let nodes = state.body_call_nodes.ok_or("missing prepared widened join replacement")?;
    let Some(destination) = nodes.place_ordinal(&target.place) else { return Ok(false) };
    let Some((expected, _, owner)) = plan.widened_join_replacements.get(&destination).copied()
    else { return Ok(false) };
    let nullable = plan.ordinary_array_descriptor_replacements.contains(&destination);
    if !nullable && !matches!(rhs, ScalarExpression::If { .. }) {
        return Err("widened replacement changed the prepared conditional".into());
    }
    let crate::ScalarPlace::Name { name, .. } = &target.place
    else { return Err("widened join replacement requires a lexical owner".into()); };
    if nodes.expression_ordinal(rhs) != Some(expected)
        || nodes.lexical_existing_destination_span(&target.place) != Some(owner)
    { return Err("widened join replacement changed its prepared operands".into()); }
    let (_, ty) = state.storage.get(name).cloned().ok_or("missing widened join target declaration")?;
    let signed = memory::SignedRuntimeAddressDomain::for_closed_receiver(state.target_layout, &ty,
        plan.signed_ordinary_array_descriptor_replacements.get(&destination))?;
    let pair = match project {
        Some(project) => emit_project_typed_expression(context, state, rhs, &ty, project.module, project.modules)?,
        None => emit_typed_expression(context, state, rhs, &ty)?,
    };
    install_widened_join(context, state, (name, owner, &ty), true, (nullable, signed), &pair)?;
    Ok(true)
}

fn emit_widened_owning_replace<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    assignment: &ScalarAssignment,
    project: Option<ProjectCallScope<'_>>,
) -> Result<bool, String> {
    let Some(plan) = state.aggregate.as_ref().map(|aggregate| aggregate.plan())
        .filter(|plan| !plan.widened_owning_receives.is_empty()) else { return Ok(false) };
    let ([target], [call @ ScalarExpression::Call { arguments, .. }]) =
        (assignment.targets.as_slice(), assignment.values.as_slice()) else { return Ok(false) };
    let nodes = state.body_call_nodes.ok_or("missing prepared widened owning replacement")?;
    let Some((source, receiver, Some((_, destination)))) = nodes.expression_ordinal(call)
        .and_then(|call| plan.widened_owning_receives.get(&call)).copied()
    else { return Ok(false) };
    let (crate::ScalarPlace::Name { name: target_name, .. }, [source_arg @ ScalarExpression::Name { name: source_name, .. }]) =
        (&target.place, arguments.as_slice()) else { return Err("invalid widened owning replacement".into()); };
    if nodes.place_ordinal(&target.place) != Some(destination)
        || nodes.lexical_existing_destination_span(&target.place) != Some(receiver)
        || nodes.lexical_expression_declaration(source_arg) != Some(source)
    { return Err("widened owning replacement changed its prepared operands".into()); }
    let (_, ty) = state.storage.get(target_name).cloned().ok_or("missing widened target declaration")?;
    let value = match project {
        Some(project) => emit_project_typed_expression(context, state, call, &ty, project.module, project.modules)?,
        None => emit_typed_expression(context, state, call, &ty)?,
    };
    aggregate_transfer::install_widened_owning_local(context, state, (source_name, source),
        (target_name, receiver, &ty), true, &value)?;
    Ok(true)
}

fn emit_widened_rebind<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    assignment: &ScalarAssignment,
    project: Option<ProjectCallScope<'_>>,
) -> Result<bool, String> {
    let Some(plan) = state.aggregate.as_ref().map(|aggregate| aggregate.plan())
        .filter(|plan| !plan.widened_rebinds.is_empty()) else { return Ok(false) };
    let ([target], [value @ ScalarExpression::Name { name: source, .. }]) =
        (assignment.targets.as_slice(), assignment.values.as_slice()) else { return Ok(false) };
    let nodes = state.body_call_nodes.ok_or("missing prepared widened array rebind")?;
    let Some(proof) = nodes.place_ordinal(&target.place).and_then(|ordinal| plan.widened_rebinds.get(&ordinal)).copied()
    else { return Ok(false) };
    let crate::ScalarPlace::Name { name: target, .. } = &target.place
    else { return Err("widened rebind requires an exact lexical destination".into()) };
    if nodes.lexical_expression_declaration(value) != Some(proof.source) || source == target {
        return Err("widened rebind differs from its checked source".into());
    }
    let (backing, ty) = state.storage.get(source).cloned().ok_or("missing widened rebind source")?;
    let (previous, target_ty) = state.storage.get(target).cloned().ok_or("missing widened rebind destination")?;
    let Some(&(declaration, extent @ LocalRuntimeExtent::WidenedLiteral { .. }, checked)) =
        state.local_runtime_extents.get(&backing)
    else { return Err("missing exact widened rebind extent".into()) };
    if declaration != proof.source || backing == previous
        || !state.local_runtime_extents.get(&previous).is_some_and(|(declaration, extent, _)|
            *declaration == proof.target && matches!(extent, LocalRuntimeExtent::WidenedLiteral { .. }))
        || [source, target].into_iter().any(|name| state.runtime_array_allocations.get(name) != Some(&true)
            || state.runtime_array_owners.get(name) != Some(&true))
    {
        return Err("widened rebind lacks two original live backing receipts".into());
    }
    let rhs = match project {
        Some(project) => emit_project_typed_expression(context, state, value, &ty, project.module, project.modules)?,
        None => emit_typed_expression(context, state, value, &ty)?,
    };
    if !matches!(rhs, EmitValue::Basic(BasicValueEnum::PointerValue(actual)) if actual == backing) {
        return Err("widened rebind changed the original source pointer".into());
    }
    ownership::consume_runtime_array_release(context, state,
        ownership::RuntimeArrayRelease { pointer: previous.into(), live: None })?;
    install_runtime_array_binding_owner(state, target, true, true, Some(source));
    state.storage.insert(target.clone(), (backing, target_ty));
    state.local_runtime_extents.remove(&previous);
    state.local_runtime_extents.insert(backing, (proof.target, extent, checked));
    Ok(true)
}

fn emit_assignment<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    assignment: &ScalarAssignment,
) -> Result<EmitValue<'ctx>, String> {
    if ownership::replace_guarded_runtime_owner(context, state, assignment)? {
        return Ok(EmitValue::Unit);
    }
    if emit_widened_join_replace(context, state, assignment, None)?
        || emit_widened_owning_replace(context, state, assignment, None)?
        || emit_widened_rebind(context, state, assignment, None)? {
        return Ok(EmitValue::Unit);
    }
    if let Some(value) = local_callable::replace(context, state, assignment, None)? {
        return Ok(value);
    }
    let expected = assignment
        .targets
        .iter()
        .map(|target| assignment_target_type(state, target))
        .collect::<Vec<_>>();
    let values = materialize_assignment_values(
        context,
        state,
        &assignment.values,
        |position| expected.get(position).and_then(Option::as_ref),
        |state, value, expected| match expected {
            Some(ty) => emit_typed_expression(context, state, value, ty),
            None => emit_expression(context, state, value),
        },
    )?;
    let destinations = assignment
        .targets
        .iter()
        .zip(&values)
        .map(|(target, value)| retain_assignment_destination(context, state, target, &value.ty, None))
        .collect::<Result<Vec<_>, String>>()?;
    let mut previous = assignment
        .targets
        .iter()
        .zip(&values)
        .zip(&destinations)
        .map(|((target, value), destination)| {
            capture_joined_assignment_previous(context, state, target, &value.ty, destination)
        })
        .collect::<Result<Vec<_>, String>>()?;
    let owners =
        stage_joined_assignment_owners(context, state, assignment, &values, &mut previous)?;
    let runtime_arrays =
        stage_runtime_array_assignments(context, state, assignment, &values, &destinations)?;
    for (position, (((((target, staged), destination), previous), joined_owner), runtime_array)) in
        assignment
            .targets
            .iter()
            .zip(values)
            .zip(destinations)
            .zip(previous)
            .zip(owners)
            .zip(runtime_arrays)
            .enumerate()
    {
        let AssignmentValue {
            value,
            ty,
            provenance: _,
            output_position,
            call_flag,
            joined_source: _,
            runtime_array_source: _,
            checked_rebind_destination: _,
            literal_array_replacement: _,
            fresh_array_recommit: _,
        } = staged;
        let Some(destination) = install_assignment_destination(state, target, &ty, destination)
        else {
            continue;
        };
        let expression = if assignment.values.len() == 1 {
            &assignment.values[0]
        } else {
            &assignment.values[position]
        };
        let value = transition_joined_checked_assignment(
            context,
            state,
            target,
            expression,
            output_position,
            previous,
            &ty,
            value,
            call_flag,
            joined_owner,
        )?;
        if transition_runtime_array_assignment(context, state, target, &value, runtime_array)? {
            store_value(context, state, destination, &ty, value)?;
        }
        aggregate_transfer::enroll_scalar_descriptor(state, target, expression, position, &ty, call_flag)?;
    }
    Ok(EmitValue::Unit)
}
fn emit_project_assignment<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    assignment: &ScalarAssignment,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    if ownership::replace_guarded_runtime_owner(context, state, assignment)? {
        return Ok(EmitValue::Unit);
    }
    if emit_widened_join_replace(context, state, assignment, Some(ProjectCallScope { module, modules }))?
        || emit_widened_owning_replace(context, state, assignment, Some(ProjectCallScope { module, modules }))?
        || emit_widened_rebind(context, state, assignment, Some(ProjectCallScope { module, modules }))? {
        return Ok(EmitValue::Unit);
    }
    if let Some(value) = local_callable::replace(context, state, assignment, Some(ProjectCallScope { module, modules }))? {
        return Ok(value);
    }
    let names = state.project_names.expect("project emission names");
    let scope = names.for_module(module);
    let expected = assignment
        .targets
        .iter()
        .map(|target| {
            if let Some(source) = target.receiver.as_ref().and_then(|receiver| {
                scope.namespaces.get(receiver.as_str())
            }) {
                let position = names.positions.get(*source)
                    .ok_or_else(|| format!("missing namespace target {}", source.path))?;
                let (global, ty) = project_member_global(&names.scopes[*position], &target.target)?;
                Ok((Some(ty.clone()), Some(global)))
            } else {
                Ok((assignment_target_type(state, target), None))
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    let values = materialize_assignment_values(
        context,
        state,
        &assignment.values,
        |position| expected.get(position).and_then(|(ty, _)| ty.as_ref()),
        |state, value, expected| match expected {
            Some(ty) => emit_project_typed_expression(context, state, value, ty, module, modules),
            None => emit_project_expression(context, state, value, module, modules),
        },
    )?;
    let destinations = assignment
        .targets
        .iter()
        .zip(&values)
        .zip(expected)
        .map(|((target, value), (_, global))| {
            let ty = &value.ty;
            if *ty == ScalarType::Unit {
                return Ok(AssignmentDestination::Unit);
            }
            if let Some(global) = global {
                let pointer = global
                    .ok_or_else(|| format!("unknown LLVM member storage {}", target.target))?
                    .as_pointer_value();
                Ok(AssignmentDestination::Existing(pointer))
            } else {
                retain_assignment_destination(context, state, target, ty, Some(ProjectCallScope { module, modules }))
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut previous = assignment
        .targets
        .iter()
        .zip(&values)
        .zip(&destinations)
        .map(|((target, value), destination)| {
            capture_joined_assignment_previous(context, state, target, &value.ty, destination)
        })
        .collect::<Result<Vec<_>, String>>()?;
    let owners =
        stage_joined_assignment_owners(context, state, assignment, &values, &mut previous)?;
    let runtime_arrays =
        stage_runtime_array_assignments(context, state, assignment, &values, &destinations)?;
    for (position, (((((target, staged), destination), previous), joined_owner), runtime_array)) in
        assignment
            .targets
            .iter()
            .zip(values)
            .zip(destinations)
            .zip(previous)
            .zip(owners)
            .zip(runtime_arrays)
            .enumerate()
    {
        let AssignmentValue {
            value,
            ty,
            provenance: _,
            output_position,
            call_flag,
            joined_source: _,
            runtime_array_source: _,
            checked_rebind_destination: _,
            literal_array_replacement: _,
            fresh_array_recommit: _,
        } = staged;
        let Some(destination) = install_assignment_destination(state, target, &ty, destination)
        else {
            continue;
        };
        let expression = if assignment.values.len() == 1 {
            &assignment.values[0]
        } else {
            &assignment.values[position]
        };
        let value = transition_joined_checked_assignment(
            context,
            state,
            target,
            expression,
            output_position,
            previous,
            &ty,
            value,
            call_flag,
            joined_owner,
        )?;
        if transition_runtime_array_assignment(context, state, target, &value, runtime_array)? {
            store_value(context, state, destination, &ty, value)?;
        }
        aggregate_transfer::enroll_scalar_descriptor(state, target, expression, position, &ty, call_flag)?;
    }
    Ok(EmitValue::Unit)
}

fn transition_joined_checked_assignment<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    target: &crate::scalar::ScalarAssignmentTarget,
    expression: &ScalarExpression,
    output: usize,
    previous: Option<JoinedAssignmentPrevious<'ctx>>,
    ty: &ScalarType,
    value: EmitValue<'ctx>,
    call_flag: Option<inkwell::values::IntValue<'ctx>>,
    joined_owner: Option<inkwell::values::IntValue<'ctx>>,
) -> Result<EmitValue<'ctx>, String> {
    let Some(JoinedAssignmentPrevious {
        owner_slot,
        storage: _,
        owner: previous_owner,
        address: previous_address,
    }) = previous
    else {
        return Ok(value);
    };
    let crate::ScalarPlace::Name { name, .. } = &target.place else {
        unreachable!("joined destination is a name")
    };
    let classification = *state
        .joined_assignment_owners
        .get(&(target.target_span.start, target.target_span.end))
        .ok_or_else(|| {
            format!(
                "missing validated assignment owner at {:?}",
                target.target_span
            )
        })?;
    let ScalarType::CheckedReference { inner, .. } = ty else {
        return Err(format!(
            "joined return binding {name} lacks checked-reference type"
        ));
    };
    let (value, owned) = match classification {
        CheckedAssignmentOwner::Local => {
            if let ScalarExpression::Name { .. } = expression {
                if let Some(owned) = joined_owner {
                    (value, owned)
                } else {
                    let copied =
                        emit_automatic_return_result(context, state, take_basic(value)?, inner)?;
                    (
                        EmitValue::Basic(copied),
                        context.bool_type().const_int(1, false),
                    )
                }
            } else {
                let copied =
                    emit_automatic_return_result(context, state, take_basic(value)?, inner)?;
                (
                    EmitValue::Basic(copied),
                    context.bool_type().const_int(1, false),
                )
            }
        }
        CheckedAssignmentOwner::External => {
            let owned = match expression {
                ScalarExpression::Call { span, .. } => match state
                    .checked_call_owners
                    .get(&(span.start, span.end, output))
                    .ok_or_else(|| format!("missing validated checked-call owner at {span:?}"))?
                {
                    CheckedCallOwner::Fresh => context.bool_type().const_int(1, false),
                    CheckedCallOwner::Borrowed => context.bool_type().const_zero(),
                    CheckedCallOwner::Mixed => call_flag
                        .ok_or_else(|| format!("missing mixed-call owner flag at {span:?}"))?,
                },
                ScalarExpression::Name { name: source, .. } => {
                    if let Some(owned) = joined_owner {
                        owned
                    } else if let Some(flag) = state.checked_owner_flags.get(source) {
                        *flag
                    } else if state.automatic_return_result_owners.contains_key(source) {
                        context.bool_type().const_int(1, false)
                    } else {
                        return Err(format!("missing validated external owner for {source}"));
                    }
                }
                _ => {
                    return Err(format!(
                        "joined return assignment at {:?} has no typed owner source",
                        target.target_span
                    ))
                }
            };
            (value, owned)
        }
        CheckedAssignmentOwner::Borrowed => (value, context.bool_type().const_zero()),
    };
    let address = take_basic(value.clone())?.into_int_value();
    let different = state
        .builder
        .build_int_compare(
            IntPredicate::NE,
            previous_address,
            address,
            "joined_owner_replaced",
        )
        .map_err(builder_error)?;
    let live = state
        .builder
        .build_int_compare(
            IntPredicate::NE,
            previous_address,
            previous_address.get_type().const_zero(),
            "joined_previous_live",
        )
        .map_err(builder_error)?;
    let release = state
        .builder
        .build_and(previous_owner, different, "joined_previous_owned")
        .map_err(builder_error)?;
    let release = state
        .builder
        .build_and(release, live, "joined_previous_release")
        .map_err(builder_error)?;
    let function = state
        .builder
        .get_insert_block()
        .ok_or_else(|| "missing joined assignment block".to_owned())?
        .get_parent()
        .ok_or_else(|| "missing joined assignment function".to_owned())?;
    let free = context.append_basic_block(function, "joined_previous_release.live");
    let next = context.append_basic_block(function, "joined_previous_release.next");
    state
        .builder
        .build_conditional_branch(release, free, next)
        .map_err(builder_error)?;
    state.builder.position_at_end(free);
    let pointer = state
        .builder
        .build_int_to_ptr(
            previous_address,
            context.ptr_type(AddressSpace::default()),
            "joined_previous_pointer",
        )
        .map_err(builder_error)?;
    declare_core_runtime(context, state.module);
    let release = state
        .module
        .get_function(CORE_FREE_SYMBOL)
        .ok_or_else(|| "core free runtime declaration is missing".to_owned())?;
    state
        .builder
        .build_call(release, &[pointer.into()], "joined_previous_free")
        .map_err(builder_error)?;
    state
        .builder
        .build_unconditional_branch(next)
        .map_err(builder_error)?;
    state.builder.position_at_end(next);
    state
        .builder
        .build_store(owner_slot, owned)
        .map_err(builder_error)?;
    Ok(value)
}

fn transition_runtime_array_assignment<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    target: &crate::scalar::ScalarAssignmentTarget,
    value: &EmitValue<'ctx>,
    staged: Option<RuntimeArrayAssignment<'ctx>>,
) -> Result<bool, String> {
    let Some(staged) = staged else { return Ok(true) };
    let crate::ScalarPlace::Name { name, .. } = &target.place else {
        return Ok(true);
    };
    if let Some(call) = staged.literal_array_call {
        aggregate_transfer::install_literal_array_replacement(context, state, call, name, value)?;
        return Ok(false);
    }
    if staged.same_array_rebind {
        return Ok(true);
    }
    if let Some(previous) = staged.previous {
        ownership::consume_runtime_array_release(context, state, previous)?;
    }
    if let Some((length_slot, live_slot, length)) = staged.fresh_array_slots {
        state.builder.build_store(length_slot, length).map_err(builder_error)?;
        state.builder.build_store(live_slot, context.bool_type().const_int(1, false)).map_err(builder_error)?;
    }
    state.runtime_array_owners.insert(name.clone(), staged.owns);
    state.runtime_array_allocations.insert(name.clone(), false);
    Ok(true)
}

fn assignment_target_type<'ctx, 'module>(
    state: &EmitState<'ctx, 'module>,
    target: &crate::scalar::ScalarAssignmentTarget,
) -> Option<ScalarType> {
    assignment_place_type(state, &target.place)
}

fn assignment_place_type<'ctx, 'module>(
    state: &EmitState<'ctx, 'module>,
    place: &crate::ScalarPlace,
) -> Option<ScalarType> {
    match place {
        crate::ScalarPlace::Name { name, .. } => state
            .storage
            .get(name.as_str())
            .map(|(_, ty)| ty.clone())
            .or_else(|| state.globals.get(name.as_str()).map(|(_, ty)| ty.clone())),
        crate::ScalarPlace::Field { field, .. } => match field {
            ScalarFieldReference::Resolved(field) => {
                structure(state.structs, field.structure.clone())
                    .ok()?
                    .fields
                    .get(field.index)
                    .filter(|candidate| candidate.id == *field)
                    .map(|field| field.ty.clone())
            }
            ScalarFieldReference::Unresolved { .. } => None,
        },
        crate::ScalarPlace::Dereference { pointer, .. } => {
            pointer_target_type(state, pointer, None)
        }
        crate::ScalarPlace::Index { base, .. } => match assignment_place_type(state, base)? {
            ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
                Some(*element)
            }
            _ => None,
        },
    }
}

fn core_pointer_cast_dereference_type(
    expression: &ScalarExpression,
) -> Option<(&ScalarType, bool)> {
    let ScalarExpression::Call {
        receiver: Some(receiver),
        name,
        type_arguments,
        ..
    } = expression
    else {
        return None;
    };
    if receiver != "core" || name != "pointer_cast" {
        return None;
    }
    match &type_arguments.first()?.ty {
        ScalarType::RawPointer(inner) => Some((inner, false)),
        ScalarType::CheckedReference { inner, .. } => Some((inner, true)),
        _ => None,
    }
}

fn immediate_checked_borrow_type<'module>(
    state: &EmitState<'_, 'module>, pointer: &ScalarExpression,
) -> Option<&'module ScalarType> {
    if !matches!(pointer, ScalarExpression::CheckedAddress { .. }) { return None; }
    state.body_call_nodes?.checked_expression_type(pointer)
        .filter(|ty| matches!(ty, ScalarType::CheckedReference { .. }))
}

fn pointer_target_type<'ctx, 'module>(
    state: &EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    project: Option<ProjectCallScope<'_>>,
) -> Option<ScalarType> {
    match expression {
        ScalarExpression::Name { name, .. } => state
            .storage
            .get(name)
            .map(|(_, ty)| ty)
            .or_else(|| state.globals.get(name).map(|(_, ty)| ty))
            .and_then(|ty| match ty {
                ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => {
                    Some(*inner.clone())
                }
                _ => None,
            }),
        ScalarExpression::CheckedAddress { .. } => match immediate_checked_borrow_type(state, expression)? {
            ScalarType::CheckedReference { inner, .. } => Some(*inner.clone()),
            _ => None,
        },
        ScalarExpression::RawAddress { place, .. } => assignment_place_type(state, place),
        ScalarExpression::Call { .. } if state.has_prepared_calls() => {
            match state.prepared_call_output(expression).ok().flatten()? {
                ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => {
                    Some(*inner)
                }
                _ => None,
            }
        }
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            ..
        } => {
            if let Some((inner, _)) = core_pointer_cast_dereference_type(expression) {
                return Some(inner.clone());
            }
            let qualified = project.map_or_else(
                || {
                    receiver
                        .as_ref()
                        .map_or_else(|| name.clone(), |receiver| format!("{receiver}.{name}"))
                },
                |project| {
                    let target = receiver
                        .as_ref()
                        .and_then(|binding| {
                            project
                                .module
                                .namespace_bindings
                                .iter()
                                .find(|namespace| namespace.binding == *binding)
                                .and_then(|namespace| {
                                    project
                                        .modules
                                        .iter()
                                        .find(|candidate| candidate.source == namespace.target)
                                        .copied()
                                })
                        })
                        .unwrap_or(project.module);
                    project_function_name(&target.source, name)
                },
            );
            state
                .raw_call_symbol(expression, &qualified, type_arguments)
                .ok()
                .flatten()
                .and_then(|target| state.signatures.get(target))
                .and_then(|signature| match signature {
                    ScalarType::Callable { outputs, .. } => outputs.outputs.first(),
                    _ => None,
                })
                .and_then(|output| match &output.ty {
                    ScalarType::CheckedReference { inner, .. } => Some(*inner.clone()),
                    ScalarType::RawPointer(inner) => Some(*inner.clone()),
                    _ => None,
                })
        }
        _ => None,
    }
}

fn dereference_requires_null_guard<'ctx, 'module>(
    state: &EmitState<'ctx, 'module>,
    expression: &ScalarExpression,
    project: Option<ProjectCallScope<'_>>,
) -> bool {
    match expression {
        ScalarExpression::Name { name, .. } => state
            .storage
            .get(name)
            .map(|(_, ty)| ty)
            .or_else(|| state.globals.get(name).map(|(_, ty)| ty))
            .is_some_and(|ty| matches!(ty, ScalarType::CheckedReference { .. })),
        ScalarExpression::CheckedAddress { .. } => immediate_checked_borrow_type(state, expression).is_some(),
        ScalarExpression::Call { .. } if state.has_prepared_calls() => {
            matches!(
                state.prepared_call_output(expression),
                Ok(Some(ScalarType::CheckedReference { .. }))
            )
        }
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            ..
        } => {
            if let Some((_, checked)) = core_pointer_cast_dereference_type(expression) {
                return checked;
            }
            let qualified = project.map_or_else(
                || {
                    receiver
                        .as_ref()
                        .map_or_else(|| name.clone(), |receiver| format!("{receiver}.{name}"))
                },
                |project| {
                    let target = receiver
                        .as_ref()
                        .and_then(|binding| {
                            project
                                .module
                                .namespace_bindings
                                .iter()
                                .find(|namespace| namespace.binding == *binding)
                                .and_then(|namespace| {
                                    project
                                        .modules
                                        .iter()
                                        .find(|candidate| candidate.source == namespace.target)
                                        .copied()
                                })
                        })
                        .unwrap_or(project.module);
                    project_function_name(&target.source, name)
                },
            );
            state
                .raw_call_symbol(expression, &qualified, type_arguments)
                .ok()
                .flatten()
                .and_then(|target| state.signatures.get(target))
                .and_then(|signature| match signature {
                    ScalarType::Callable { outputs, .. } => outputs.outputs.first(),
                    _ => None,
                })
                .is_some_and(|output| matches!(&output.ty, ScalarType::CheckedReference { .. }))
        }
        _ => false,
    }
}

fn emit_checked_dereference_guard<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    address: inkwell::values::IntValue<'ctx>,
) -> Result<(), String> {
    if let Some(access) = state.project_names.and_then(|names| names.checked_access.as_ref()) {
        return emit_checked_access_call(state, &access.non_null, &[address.into()]);
    }
    if state.require_release_checked_access {
        return Err("checked dereference lacks its prepared release helper".to_owned());
    }
    let is_null = state
        .builder
        .build_int_compare(
            IntPredicate::EQ,
            address,
            address.get_type().const_int(0, false),
            "deref_is_null",
        )
        .map_err(builder_error)?;
    emit_checked_place_guard(context, state, is_null, "deref_null_panic", "deref_null_continue")
}

fn emit_checked_access_call<'ctx>(
    state: &EmitState<'ctx, '_>,
    symbol: &str,
    arguments: &[BasicMetadataValueEnum<'ctx>],
) -> Result<(), String> {
    let function = state.functions.get(symbol).copied()
        .ok_or_else(|| "missing sealed release checked-access function".to_owned())?;
    let ty = function.get_type();
    let parameters = ty.get_param_types();
    if ty.get_return_type().is_some() || parameters.len() != arguments.len()
        || !parameters.iter().zip(arguments).all(|(expected, value)| {
            matches!((expected, value),
                (BasicMetadataTypeEnum::IntType(expected), BasicMetadataValueEnum::IntValue(value))
                    if *expected == value.get_type())
        })
    {
        return Err("invalid release checked-access function ABI".to_owned());
    }
    state.builder.build_call(function, arguments, "").map_err(builder_error)?;
    Ok(())
}

fn emit_checked_place_guard<'ctx>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, '_>,
    invalid: inkwell::values::IntValue<'ctx>,
    panic_name: &str,
    continue_name: &str,
) -> Result<(), String> {
    declare_core_runtime(context, state.module);
    let function = state
        .builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .ok_or_else(|| "checked place has no containing function".to_owned())?;
    let panic_block = context.append_basic_block(function, panic_name);
    let continue_block = context.append_basic_block(function, continue_name);
    state
        .builder
        .build_conditional_branch(invalid, panic_block, continue_block)
        .map_err(builder_error)?;
    state.builder.position_at_end(panic_block);
    let panic = state
        .module
        .get_function(CORE_SYSTEM_PANIC_SYMBOL)
        .ok_or_else(|| "core system-panic runtime declaration is missing".to_owned())?;
    state
        .builder
        .build_call(panic, &[], "system_panic")
        .map_err(builder_error)?;
    state.builder.build_unreachable().map_err(builder_error)?;
    state.builder.position_at_end(continue_block);
    Ok(())
}

fn materialize_assignment_values<'ctx, 'module, 'expected>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    expressions: &[ScalarExpression],
    expected: impl Fn(usize) -> Option<&'expected ScalarType>,
    emit: impl FnMut(
        &mut EmitState<'ctx, 'module>,
        &ScalarExpression,
        Option<&ScalarType>,
    ) -> Result<EmitValue<'ctx>, String>,
) -> Result<Vec<AssignmentValue<'ctx>>, String> {
    let mut emit = emit;
    let mut position = 0;
    let mut values = Vec::new();
    for expression in expressions {
        let (checked_rebind_destination, literal_array_replacement, fresh_recommit) = state.aggregate.as_ref()
            .and_then(|aggregate| {
                let call = state.body_call_nodes.and_then(|nodes| nodes.expression_ordinal(expression))?;
                Some((
                    aggregate.same_array_rebind_destination(call),
                    aggregate.literal_array_replacement(call)
                        .or_else(|| aggregate.rebound_array_destination(call))
                        .map(|destination| (call, destination)),
                    aggregate.fresh_array_recommit(call).map(|proof| (call, proof)),
                ))
            })
            .unwrap_or_default();
        let provenance = if literal_array_replacement.is_some() || checked_rebind_destination.is_some() || fresh_recommit.is_some() {
            // Checked receipt routes do not acquire a second lexical owner.
            RuntimeArrayAssignmentProvenance {
                result: RuntimeArrayResultProvenance::CheckedReceipt,
                source_name: None,
            }
        } else {
            runtime_array_assignment_provenance(state, expression)?
        };
        let runtime_array_source = if provenance.result.owns() {
            provenance.source_name.as_ref().map(|name| {
                state
                    .storage
                    .get(name)
                    .expect("owning runtime-array lexical storage")
                    .0
            })
        } else {
            None
        };
        let expected_type = expected(position);
        let value = emit(state, expression, expected_type)?;
        let fresh_recommit = if let Some((call, proof)) = fresh_recommit {
            use crate::scalar::RecommittedArrayExtent;
            let extent = match (proof.size, proof.argument) {
                (RecommittedArrayExtent::Literal(elements), None) => context.i64_type().const_int(elements, false),
                (RecommittedArrayExtent::Formal(_), Some(_)) => state.aggregate.as_mut()
                    .and_then(|aggregate| aggregate.fresh_recommit_extent.take())
                    .filter(|(captured, _)| *captured == call)
                    .map(|(_, length)| length)
                    .ok_or("missing captured fresh recommit extent")?,
                _ => return Err("invalid prepared fresh recommit extent".into()),
            };
            Some((proof, extent))
        } else { None };
        let joined_source = if let ScalarExpression::Name { name, .. } = expression {
            if let Some((owner_slot, _)) = state.joined_return_bindings.get(name) {
                let storage = state
                    .storage
                    .get(name)
                    .expect("joined source has lexical storage")
                    .0;
                let owned = state
                    .builder
                    .build_load(context.bool_type(), *owner_slot, "joined_assignment_owned")
                    .map_err(builder_error)?
                    .into_int_value();
                Some(JoinedAssignmentSource {
                    owner_slot: *owner_slot,
                    storage,
                    owned,
                })
            } else {
                None
            }
        } else {
            None
        };
        let output_types = match &value {
            EmitValue::Aggregate { outputs, .. } => outputs.clone(),
            EmitValue::Unit => vec![ScalarType::Unit],
            EmitValue::Basic(value) => vec![expected_type
                .cloned()
                .unwrap_or_else(|| basic_value_type(*value))],
        };
        for (output_position, ty) in output_types.into_iter().enumerate() {
            let checked_rebind_destination = if output_position == 0 {
                checked_rebind_destination
            } else {
                None
            };
            let literal_array_replacement = if output_position == 0 {
                literal_array_replacement
            } else {
                None
            };
            let value = extract_output(state, &value, output_position)?;
            if ty == ScalarType::Unit {
                values.push(AssignmentValue {
                    value: EmitValue::Unit,
                    ty,
                    provenance: provenance.clone(),
                    output_position,
                    call_flag: None,
                    joined_source: None,
                    runtime_array_source: None,
                    checked_rebind_destination,
                    literal_array_replacement,
                    fresh_array_recommit: None,
                });
                position += 1;
                continue;
            }
            let value = if literal_array_replacement.is_some()
                && matches!(&value, EmitValue::Basic(BasicValueEnum::StructValue(_)))
            {
                // The sealed call result is an already evaluated paired SSA
                // value. Do not materialize it through a legacy pointer slot.
                value
            } else {
                let temporary = entry_storage_alloca(context, state, &ty, "assignment_value")?;
                store_value(context, state, temporary, &ty, value)?;
                if matches!(ty, ScalarType::Struct(_) | ScalarType::Array { .. }) {
                    EmitValue::Basic(temporary.into())
                } else {
                    EmitValue::Basic(
                        state
                            .builder
                            .build_load(
                                basic_type(context, &ty, state.target_layout)?,
                                temporary,
                                "assignment_value",
                            )
                            .map_err(builder_error)?,
                    )
                }
            };
            values.push(AssignmentValue {
                value,
                ty,
                provenance: provenance.clone(),
                output_position,
                call_flag: state.call_owner_flags.get(&output_position).copied(),
                joined_source,
                runtime_array_source,
                checked_rebind_destination,
                literal_array_replacement,
                fresh_array_recommit: if output_position == 0 { fresh_recommit } else { None },
            });
            position += 1;
        }
    }
    Ok(values)
}

fn basic_value_type(value: BasicValueEnum<'_>) -> ScalarType {
    match value {
        BasicValueEnum::IntValue(value) => match value.get_type().get_bit_width() {
            1 => ScalarType::Bool,
            8 => ScalarType::I8,
            16 => ScalarType::I16,
            32 => ScalarType::I32,
            64 => ScalarType::I64,
            128 => ScalarType::I128,
            width => panic!("unsupported LLVM integer width {width}"),
        },
        BasicValueEnum::FloatValue(value)
            if value.get_type() == value.get_type().get_context().f32_type() =>
        {
            ScalarType::F32
        }
        BasicValueEnum::FloatValue(_) => ScalarType::F64,
        BasicValueEnum::PointerValue(_) => ScalarType::RawPointer(Box::new(ScalarType::I8)),
        BasicValueEnum::ArrayValue(_)
        | BasicValueEnum::StructValue(_)
        | BasicValueEnum::VectorValue(_)
        | BasicValueEnum::ScalableVectorValue(_) => {
            panic!("unsupported inferred LLVM assignment value")
        }
    }
}

fn emit_block_item<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    item: &ScalarBlockItem,
) -> Result<EmitValue<'ctx>, String> {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            if binding.fallible_assignment.is_some() {
                let value = if matches!(&binding.value, ScalarExpression::StructLiteral { .. } | ScalarExpression::ArrayLiteral { .. }) {
                    None
                } else {
                    Some(take_basic(emit_typed_expression(context, state, &binding.value, &binding.declared_type)?)?)
                };
                ownership::bind_guarded_scalar(context, state, binding, value, ExpressionPath::Single)?;
                return Ok(EmitValue::Unit);
            }
            if ownership::move_guarded_runtime_owner(context, state, binding)? {
                return Ok(EmitValue::Unit);
            }
            if store_widened_local_array(context, state, binding, None)? {
                return Ok(EmitValue::Unit);
            }
            let value = emit_binding_value(context, state, binding)?;
            store_binding_outputs(context, state, binding, value)?;
            Ok(EmitValue::Unit)
        }
        ScalarBlockItem::Expression(expression) => emit_expression(context, state, expression),
        ScalarBlockItem::Assignment(assignment) => emit_assignment(context, state, assignment),
    }
}

fn emit_project_block_item<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    item: &ScalarBlockItem,
    module: &ScalarModule,
    modules: &[&ScalarModule],
) -> Result<EmitValue<'ctx>, String> {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            if binding.fallible_assignment.is_some() {
                let value = if matches!(&binding.value, ScalarExpression::StructLiteral { .. } | ScalarExpression::ArrayLiteral { .. }) {
                    None
                } else {
                    Some(take_basic(emit_project_typed_expression(
                        context, state, &binding.value, &binding.declared_type, module, modules,
                    )?)?)
                };
                ownership::bind_guarded_scalar(context, state, binding, value, ExpressionPath::Project { module, modules })?;
                return Ok(EmitValue::Unit);
            }
            if ownership::move_guarded_runtime_owner(context, state, binding)?
                || store_widened_local_array(context, state, binding, Some(ProjectCallScope { module, modules }))? {
                return Ok(EmitValue::Unit);
            }
            if binding.is_allocation {
                let receiver = binding
                    .receivers
                    .first()
                    .ok_or_else(|| "runtime allocation has no receiver".to_owned())?;
                let ScalarType::RuntimeArray { element, .. } = &receiver.ty else {
                    return Err("allocation binding requires a runtime array type".to_owned());
                };
                let length = receiver
                    .allocation_length
                    .as_ref()
                    .ok_or_else(|| "runtime allocation has no length".to_owned())?;
                let length = take_basic(emit_project_typed_expression(
                    context,
                    state,
                    length,
                    &ScalarType::U64,
                    module,
                    modules,
                )?)?
                .into_int_value();
                let slot = emit_runtime_array_allocation(context, state, element, length)?;
                if !state
                    .mutable_runtime_allocations
                    .contains(&(receiver.name_span.start, receiver.name_span.end))
                {
                    state
                        .storage
                        .insert(receiver.name.clone(), (slot, receiver.ty.clone()));
                    state
                        .runtime_array_owners
                        .insert(receiver.name.clone(), true);
                    state
                        .runtime_array_allocations
                        .insert(receiver.name.clone(), true);
                    return Ok(EmitValue::Unit);
                }
                let backing = entry_alloca(
                    state,
                    context.ptr_type(AddressSpace::default()).into(),
                    &format!("{}_backing", receiver.name),
                )?;
                state
                    .builder
                    .build_store(backing, slot)
                    .map_err(builder_error)?;
                let length_slot = entry_alloca(
                    state,
                    context.i64_type().into(),
                    &format!("{}_length", receiver.name),
                )?;
                state
                    .builder
                    .build_store(length_slot, length)
                    .map_err(builder_error)?;
                state
                    .runtime_array_lengths
                    .insert(receiver.name.clone(), length_slot);
                let live = entry_alloca(
                    state,
                    context.bool_type().into(),
                    &format!("{}_live", receiver.name),
                )?;
                state
                    .builder
                    .build_store(live, context.bool_type().const_int(1, false))
                    .map_err(builder_error)?;
                state.runtime_array_live.insert(receiver.name.clone(), live);
                state
                    .storage
                    .insert(receiver.name.clone(), (backing, receiver.ty.clone()));
                state
                    .runtime_array_owners
                    .insert(receiver.name.clone(), true);
                state
                    .runtime_array_allocations
                    .insert(receiver.name.clone(), false);
                return Ok(EmitValue::Unit);
            }
            let value = emit_project_typed_expression(
                context,
                state,
                &binding.value,
                &binding.declared_type,
                module,
                modules,
            )?;
            store_binding_outputs(context, state, binding, value)?;
            Ok(EmitValue::Unit)
        }
        ScalarBlockItem::Expression(expression) => {
            emit_project_expression(context, state, expression, module, modules)
        }
        ScalarBlockItem::Assignment(assignment) => {
            emit_project_assignment(context, state, assignment, module, modules)
        }
    }
}

mod expression;
mod self_tail;
use expression::{
    ExpressionPath,
    emit_expression, emit_if_value, emit_project_expression, emit_project_typed_expression,
};
mod return_memory;
use return_memory::{
    emit_automatic_return_result, emit_return, finish_block_return, record_forwarded_return_owner,
    record_named_branch_return_owners,
    transfer_automatic_return_result,
};
fn project_function_name(source: &wosy_syntax::SourceIdentity, name: &str) -> String {
    let mut symbol = String::from("wosy_fn");
    for component in [
        source.package.as_str(),
        source.path.as_str(),
        source.revision.as_str(),
        name,
    ] {
        symbol.push_str("__");
        for byte in component.as_bytes() {
            write!(symbol, "{byte:02x}").expect("writing to a String cannot fail");
        }
    }
    symbol
}

fn llvm_extern_identity(
    source: &wosy_syntax::SourceIdentity,
    binding: &str,
    import_module: &str,
    import_name: &str,
    signature: &ScalarType,
) -> LlvmExternIdentity {
    let internal_name = encoded_llvm_name(
        "wosy_extern",
        [
            source.package.as_str(),
            source.path.as_str(),
            source.revision.as_str(),
            binding,
            import_module,
            import_name,
        ],
    );
    LlvmExternIdentity {
        internal_name,
        import_module: import_module.to_owned(),
        import_name: match import_name.strip_prefix('_') {
            Some(private_name) => private_name.to_owned(),
            None => import_name.to_owned(),
        },
        signature: signature.clone(),
    }
}

fn project_extern_lookup_key(
    source: &wosy_syntax::SourceIdentity,
    binding: &str,
    import_module: &str,
    name: &str,
) -> String {
    encoded_llvm_name(
        "wosy_extern_lookup",
        [
            source.package.as_str(),
            source.path.as_str(),
            source.revision.as_str(),
            binding,
            import_module,
            name,
        ],
    )
}

fn encoded_llvm_name<'a>(prefix: &str, components: impl IntoIterator<Item = &'a str>) -> String {
    let mut symbol = prefix.to_owned();
    for component in components {
        symbol.push_str("__");
        for byte in component.as_bytes() {
            write!(symbol, "{byte:02x}").expect("writing to a String cannot fail");
        }
    }
    symbol
}

struct LlvmSpecialization<'a> {
    function: Cow<'a, ScalarFunction>,
    name: String,
    template: &'a ScalarFunction,
    item_ordinal: usize,
    arm_index: Option<usize>,
}

enum EmissionPolicy {
    RawCompatibility,
    Prepared,
}

#[derive(Clone, Copy)]
enum GenericInitializerInput<'a> {
    RawStandalone(&'a crate::scalar::PreparedProgram),
    RawProject(&'a crate::PreparedScalarProject),
    PreparedRows(&'a crate::PreparedScalarProject),
}

impl GenericInitializerInput<'_> {
    fn requires_release_checked_access(&self) -> bool {
        matches!(self, Self::PreparedRows(prepared) if prepared.requires_release_checked_access())
    }

    fn prepared_render(self) -> bool {
        matches!(self, Self::PreparedRows(_))
    }

    fn has_closed_shared_scalar_reader(self, module: usize, declaration: wosy_syntax::ByteSpan) -> bool {
        matches!(self, Self::PreparedRows(prepared) if prepared.has_closed_shared_scalar_reader(module, declaration))
    }

    fn visit_body_overloads(
        self,
        module: usize,
        item: usize,
        mut visitor: impl FnMut(&ResolvedCallTarget) -> Result<(), String>,
    ) -> Result<(), String> {
        let missing = || {
            format!("internal error: missing callable overload reservation rows (module {module}, item {item})")
        };
        match self {
            Self::RawStandalone(prepared) => prepared
                .body_overload_reservations(item)
                .ok_or_else(missing)?
                .try_for_each(&mut visitor),
            Self::RawProject(prepared) | Self::PreparedRows(prepared) => prepared
                .body_overload_reservations(module, item)
                .ok_or_else(missing)?
                .try_for_each(&mut visitor),
        }
    }

    fn bind_body<'a>(
        &'a self,
        module: usize,
        item: usize,
        arm: Option<usize>,
        body: &'a ScalarBlock,
    ) -> Result<Option<CallableBodyCallNodes<'a>>, String> {
        match self {
            Self::RawStandalone(prepared) => prepared.bind_body(item, arm, body).map(Some),
            Self::RawProject(prepared) | Self::PreparedRows(prepared) => prepared
                .bind_callable_body_call_nodes(module, item, arm, body)
                .map(Some),
        }
    }
}

mod initializer_calls;
use initializer_calls::{
    collect_body_overload_reservations, prepared_initializer_overloads, InitializerSymbols,
    PreparedCall,
};

fn generic_specializations<'a>(
    modules: impl Iterator<
            Item = (
                usize,
                &'a wosy_syntax::SourceIdentity,
                &'a [ScalarItem],
                &'a ModuleInitializationRegion,
            ),
        > + Clone,
    symbol: impl Fn(&wosy_syntax::SourceIdentity, &str) -> String,
    prepared_render: bool,
) -> Result<BTreeMap<usize, BTreeMap<String, LlvmSpecialization<'a>>>, String> {
    let mut templates = HashMap::new();
    let mut specializations = BTreeMap::new();
    for (module_ordinal, source, items, _) in modules.clone() {
        specializations.insert(module_ordinal, BTreeMap::new());
        for (item_ordinal, item) in items.iter().enumerate() {
            if let ScalarItem::Function(function) = item {
                if !function.generic_parameters.is_empty() && function.overload_arms.is_empty() {
                    templates.insert(
                        (source, function.name_span),
                        (module_ordinal, item_ordinal, function),
                    );
                }
            }
        }
    }
    for (_, _, _, region) in modules {
        let mut reserve = |target: &ModuleInitializationCallTarget| -> Result<(), String> {
            let ModuleInitializationCallTarget::Generic {
                declaration,
                type_arguments,
            } = target
            else {
                return Ok(());
            };
            let &(module_ordinal, item_ordinal, function) = templates
                .get(&(&declaration.source, declaration.range))
                .ok_or_else(|| {
                    format!(
                        "internal error: missing initializer generic declaration {declaration:?}"
                    )
                })?;
            insert_generic_specialization(
                specializations
                    .get_mut(&module_ordinal)
                    .expect("initializer template belongs to an indexed module"),
                function,
                item_ordinal,
                type_arguments,
                &symbol(&declaration.source, &function.name),
            )
        };
        if prepared_render {
            for target in region.call_targets()? {
                reserve(target?)?;
            }
        } else {
            for (_, unqualified, target) in region.legacy_reservation_targets()? {
                if unqualified {
                    reserve(target)?;
                }
            }
        }
    }
    Ok(specializations)
}
fn insert_generic_specialization<'a>(
    specializations: &mut BTreeMap<String, LlvmSpecialization<'a>>,
    function: &'a ScalarFunction,
    item_ordinal: usize,
    arguments: &[ScalarType],
    base: &str,
) -> Result<(), String> {
    if function.generic_parameters.len() != arguments.len() {
        return Err("validated generic call has invalid type argument arity".to_owned());
    }
    let lookup = generic_specialization_key(base, arguments);
    specializations.entry(lookup).or_insert_with(|| {
        let signature = substitute_generic_signature(function, arguments);
        let name = generic_specialization_name(base, arguments);
        let mut body = function.clone();
        body.signature = signature;
        body.generic_parameters.clear();
        LlvmSpecialization {
            function: Cow::Owned(body),
            name,
            template: function,
            item_ordinal,
            arm_index: None,
        }
    });
    Ok(())
}

fn selected_overload_specializations(
    items: &[ScalarItem],
    symbol: impl Fn(&str) -> String,
    selections: impl IntoIterator<Item = (String, ScalarOverloadSelection)>,
) -> BTreeMap<String, LlvmSpecialization<'_>> {
    let overloads = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| match item {
            ScalarItem::Function(function) if !function.overload_arms.is_empty() => {
                Some((function.name.as_str(), (index, function)))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let mut selections = selections.into_iter().collect::<Vec<_>>();
    let mut specializations = BTreeMap::new();
    for (name, (_, overload)) in &overloads {
        for (arm_index, arm) in overload.overload_arms.iter().enumerate() {
            if arm.generic_parameters.is_empty() {
                selections.push((
                    (*name).to_owned(),
                    ScalarOverloadSelection {
                        arm_index,
                        substitutions: BTreeMap::new(),
                    },
                ));
            }
        }
    }
    for (name, selection) in selections {
        let Some(&(item_ordinal, overload)) = overloads.get(name.as_str()) else {
            continue;
        };
        let Some(arm) = overload.overload_arms.get(selection.arm_index) else {
            continue;
        };
        let base = symbol(&overload.name);
        let arguments = arm
            .generic_parameters
            .iter()
            .map(|parameter| selection.substitutions[&parameter.name].clone())
            .collect::<Vec<_>>();
        let lookup = selected_overload_lookup_key(&base, &selection);
        let name = generic_specialization_name(
            &encoded_llvm_name(
                "wosy_overload",
                [base.as_str(), &selection.arm_index.to_string()],
            ),
            &arguments,
        );
        specializations
            .entry(lookup)
            .or_insert_with(|| LlvmSpecialization {
                function: if arguments.is_empty() {
                    Cow::Borrowed(arm)
                } else {
                    let mut function = arm.clone();
                    function.signature = substitute_generic_signature(arm, &arguments);
                    function.generic_parameters.clear();
                    Cow::Owned(function)
                },
                name,
                template: arm,
                item_ordinal,
                arm_index: Some(selection.arm_index),
            });
    }
    specializations
}

fn selected_overload_lookup_key(name: &str, selection: &ScalarOverloadSelection) -> String {
    let substitutions = selection
        .substitutions
        .values()
        .map(generic_type_name)
        .collect::<Vec<_>>()
        .join(",");
    format!("{name}<arm:{},{}>", selection.arm_index, substitutions)
}

fn substitute_generic_signature(function: &ScalarFunction, arguments: &[ScalarType]) -> ScalarType {
    let substitutions = function
        .generic_parameters
        .iter()
        .zip(arguments)
        .map(|(parameter, argument)| (parameter.name.as_str(), argument))
        .collect::<BTreeMap<_, _>>();
    substitute_generic_type(&function.signature, &substitutions)
}

fn substitute_generic_type(
    ty: &ScalarType,
    substitutions: &BTreeMap<&str, &ScalarType>,
) -> ScalarType {
    match ty {
        ScalarType::Named { name, .. } => substitutions
            .get(name.as_str())
            .map_or_else(|| ty.clone(), |replacement| (*replacement).clone()),
        ScalarType::RawPointer(inner) => {
            ScalarType::RawPointer(Box::new(substitute_generic_type(inner, substitutions)))
        }
        ScalarType::CheckedReference { mutability, inner } => ScalarType::CheckedReference {
            mutability: *mutability,
            inner: Box::new(substitute_generic_type(inner, substitutions)),
        },
        ScalarType::Array {
            element,
            length,
            length_span,
            span,
        } => ScalarType::Array {
            element: Box::new(substitute_generic_type(element, substitutions)),
            length: *length,
            length_span: *length_span,
            span: *span,
        },
        ScalarType::Callable {
            outputs,
            parameters,
        } => ScalarType::Callable {
            outputs: crate::ScalarOutputSequence {
                outputs: outputs
                    .outputs
                    .iter()
                    .map(|output| crate::ScalarOutput {
                        ty: substitute_generic_type(&output.ty, substitutions),
                        span: output.span,
                    })
                    .collect(),
                span: outputs.span,
            },
            parameters: parameters
                .iter()
                .map(|parameter| substitute_generic_type(parameter, substitutions))
                .collect(),
        },
        _ => ty.clone(),
    }
}

fn specialization_lookup_key(
    name: &str,
    arguments: &[crate::scalar::ScalarTypeArgument],
) -> String {
    if arguments.is_empty() {
        name.to_owned()
    } else {
        generic_specialization_key(
            name,
            &arguments
                .iter()
                .map(|argument| argument.ty.clone())
                .collect::<Vec<_>>(),
        )
    }
}

fn generic_specialization_key(name: &str, arguments: &[ScalarType]) -> String {
    let tuple = arguments
        .iter()
        .map(generic_type_name)
        .collect::<Vec<_>>()
        .join(",");
    format!("{name}<{tuple}>")
}

fn generic_specialization_name(name: &str, arguments: &[ScalarType]) -> String {
    let tuple = arguments
        .iter()
        .map(generic_type_name)
        .collect::<Vec<_>>()
        .join(",");
    encoded_llvm_name("wosy_generic", [name, tuple.as_str()])
}

fn generic_type_name(ty: &ScalarType) -> String {
    match ty {
        ScalarType::Unit => "unit".into(),
        ScalarType::Bool => "bool".into(),
        ScalarType::I8 => "i8".into(),
        ScalarType::I16 => "i16".into(),
        ScalarType::I32 => "i32".into(),
        ScalarType::I64 => "i64".into(),
        ScalarType::I128 => "i128".into(),
        ScalarType::U8 => "u8".into(),
        ScalarType::U16 => "u16".into(),
        ScalarType::U32 => "u32".into(),
        ScalarType::U64 => "u64".into(),
        ScalarType::U128 => "u128".into(),
        ScalarType::F32 => "f32".into(),
        ScalarType::F64 => "f64".into(),
        ScalarType::Char => "char".into(),
        ScalarType::ArtifactId => "artifact_id".into(),
        ScalarType::RawPointer(inner) => format!("ptr({})", generic_type_name(inner)),
        ScalarType::CheckedReference { mutability, inner } => format!(
            "checked_{}({})",
            match mutability {
                crate::scalar::ScalarReferenceMutability::Shared => "shared",
                crate::scalar::ScalarReferenceMutability::Mutable => "mutable",
            },
            generic_type_name(inner)
        ),
        ScalarType::Array {
            element, length, ..
        } => format!("array({};{length})", generic_type_name(element)),
        ScalarType::RuntimeArray { element, .. } => {
            format!("array({})", generic_type_name(element))
        }
        ScalarType::Callable {
            outputs,
            parameters,
        } => format!(
            "fn({})->({})",
            parameters
                .iter()
                .map(generic_type_name)
                .collect::<Vec<_>>()
                .join(","),
            outputs
                .outputs
                .iter()
                .map(|output| generic_type_name(&output.ty))
                .collect::<Vec<_>>()
                .join(",")
        ),
        ScalarType::Named { name, .. } => format!("named({name})"),
        ScalarType::Qualified {
            receiver, member, ..
        } => format!("qualified({receiver}.{member})"),
        ScalarType::Struct(id) => format!(
            "struct({}:{}:{}:{})",
            id.source.package, id.source.path, id.source.revision, id.index
        ),
        ScalarType::Enum(id) => format!(
            "enum({}:{}:{}:{})",
            id.source.package, id.source.path, id.source.revision, id.index
        ),
        ScalarType::Error => "error".into(),
    }
}

fn project_global_name(source: &wosy_syntax::SourceIdentity, name: &str) -> String {
    project_function_name(source, &format!("global_{name}"))
}

fn project_runtime_array_name(source: &wosy_syntax::SourceIdentity, name: &str) -> String {
    project_global_name(source, name)
}

fn project_namespace_target<'a, 'ctx, 'module>(
    names: &'a ProjectNames<'ctx, 'module>,
    module: &ScalarModule,
    receiver: &str,
) -> Result<&'a ModuleNames<'ctx, 'module>, String> {
    let source = names.for_module(module).namespaces
        .get(receiver)
        .ok_or_else(|| format!("unknown namespace binding {receiver}"))?;
    names.positions
        .get(*source)
        .map(|&position| &names.scopes[position])
        .ok_or_else(|| format!("missing namespace target {}", source.path))
}

fn project_member_global<'ctx, 'module>(
    target: &ModuleNames<'ctx, 'module>,
    name: &str,
) -> Result<(Option<GlobalValue<'ctx>>, &'module ScalarType), String> {
    let member = target.members.get(name).ok_or_else(|| format!("unknown LLVM member {name}"))?;
    let global = target.globals.get(name).map(|(global, _)| *global);
    if *member.ty != ScalarType::Unit && global.is_none() {
        return Err(format!("unknown LLVM member storage {name}"));
    }
    Ok((global, member.ty))
}

fn unit_values(items: &[ScalarItem]) -> BTreeSet<&str> {
    items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Binding(binding) if binding.declared_type == ScalarType::Unit => {
                Some(binding.name.as_str())
            }
            _ => None,
        })
        .collect()
}

fn project_module_names<'ctx, 'module>(
    module: &'module ScalarModule,
    globals: &'module BTreeMap<String, GlobalStorage<'ctx>>,
) -> ModuleNames<'ctx, 'module> {
    let mut names = ModuleNames {
        globals: GlobalNames(BTreeMap::new()),
        unit_values: BTreeSet::new(),
        members: BTreeMap::new(),
        namespaces: BTreeMap::new(),
        externs: BTreeMap::new(),
    };
    for namespace in &module.namespace_bindings {
        names.namespaces.entry(namespace.binding.as_str()).or_insert(&namespace.target);
    }
    for item in &module.items {
        let binding = match item {
            ScalarItem::Binding(binding) => binding,
            ScalarItem::Extern(declaration) => {
                names.externs.entry(declaration.binding.as_str()).or_insert(declaration);
                continue;
            }
            _ => continue,
        };
        let ty = &binding.declared_type;
        let unsigned = matches!(
            ty,
            ScalarType::U8 | ScalarType::U16 | ScalarType::U32 | ScalarType::U64 | ScalarType::U128
        );
        let member = names.members.entry(binding.name.as_str()).or_insert(MemberName {
            ty,
            unsigned: false,
        });
        member.unsigned |= unsigned;
        if *ty == ScalarType::Unit {
            names.unit_values.insert(binding.name.as_str());
        }
        for name in std::iter::once(binding.name.as_str())
            .chain(binding.receivers.iter().map(|receiver| receiver.name.as_str()))
        {
            if let Some(global) = globals.get(&project_global_name(&module.source, name)) {
                names.globals.0.insert(name, global);
            }
        }
    }
    names
}

#[cfg(test)]
mod tests;
