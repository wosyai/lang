//! Exact, non-emitting inventory of the one nested U8 caller. Neither the
//! linked boundary nor the compilation partition is admitted by this proof.
use super::print_finite::{PrintFiniteOpen, Trace};
use super::*;
use crate::scalar::ScalarExternModule;

pub(super) struct U8NestedCallerOpen<'a> {
    pub(super) caller: EntryId,
    pub(super) producer: Receipt<'a>,
    pub(super) forwarded: Receipt<'a>,
    pub(super) formatting: DirectCall,
    pub(super) consuming: DirectCall,
    pub(super) print: PrintFiniteOpen,
    pub(super) markers: [MarkerOpen; 3],
    partition: PartitionAdmissionOpen,
}

struct PartitionAdmissionOpen;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MarkerDeclarationOpen {
    pub(super) source: SourceSpan,
    pub(super) binding: String,
    pub(super) module: String,
    pub(super) member: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MarkerOpen {
    pub(super) site: SourceSpan,
    pub(super) point: CfgPointId,
    pub(super) declaration: MarkerDeclarationOpen,
    pub(super) number: u64,
}

type E = ScalarExpression;
type T = ScalarType;
type I = ScalarBlockItem;

fn need(condition: bool, why: &'static str) -> Result<(), Rejected> {
    condition.then_some(()).ok_or(Rejected::Contradictory(why))
}

impl<'a> Ledger<'a> {
    pub(super) fn u8_nested_caller_open(
        &self,
        caller: &CallableBody<'a>,
    ) -> Result<U8NestedCallerOpen<'a>, Rejected> {
        need(
            std::ptr::eq(self.build, caller.build),
            "nested caller artifact provenance",
        )?;
        let (params, results) = caller.entry.signature()?;
        need(
            caller.entry.id.arm.is_none() && params.is_empty() && results == [&T::Unit],
            "nested caller signature",
        )?;
        let root = &caller.entry.function.body;
        let [I::Expression(before), initialize, I::Expression(outer @ E::Call { arguments, .. }), I::Expression(after), I::Expression(finish)] =
            root.items.as_slice()
        else {
            return Err(Rejected::Contradictory("nested caller exact root"));
        };
        let [inner @ E::Call {
            arguments: formatter_arguments,
            ..
        }] = arguments.as_slice()
        else {
            return Err(Rejected::Contradictory("nested print actual"));
        };
        let [scalar] = formatter_arguments.as_slice() else {
            return Err(Rejected::Contradictory("nested format actual"));
        };
        let formatting = self.direct_call(caller, inner)?;
        let consuming = self.direct_call(caller, outer)?;
        let [T::Struct(descriptor)] = formatting.results.as_slice() else {
            return Err(Rejected::Contradictory("nested format descriptor output"));
        };
        let [actual] = consuming.actuals.as_slice() else {
            return Err(Rejected::Contradictory("nested print actual"));
        };
        need(
            actual.point == formatting.point
                && actual.ty == T::Struct(descriptor.clone())
                && consuming.results == [T::U64, T::Bool]
                && consuming.target.arm.is_none()
                && consuming.target.source == formatting.target.source
                && formatting.target.arm.is_some(),
            "nested descriptor point and slot",
        )?;

        let project = self.build.prepared().project();
        let app = &project.modules[caller.entry.id.body.0];
        let mut std = app
            .namespace_bindings
            .iter()
            .filter(|namespace| namespace.binding == "std");
        need(
            std.next()
                .is_some_and(|row| row.target == formatting.target.source)
                && std.next().is_none(),
            "nested canonical std namespace",
        )?;
        let std_module = &project.modules[formatting.target.body.0];
        need(
            matches!(std_module.items.get(formatting.target.body.1),
            Some(ScalarItem::Function(group)) if group.name == "format"),
            "nested selected format group",
        )?;
        let arm = self.body(formatting.target.body, formatting.target.arm)?;
        let (arm_params, arm_results) = arm.entry.signature()?;
        need(
            arm.entry.id == formatting.target
                && arm_params == [T::U8]
                && arm_results == [&T::Struct(descriptor.clone())],
            "nested selected U8 arm",
        )?;

        // Reuse the full actual forwarded-arm proof, including its canonical
        // formal, evaluation event, distinct producer, and return output.
        let [I::Expression(helper_call @ E::Call { .. })] =
            arm.entry.function.body.items.as_slice()
        else {
            return Err(Rejected::Contradictory("nested exact format arm body"));
        };
        let helper = self.direct_call(&arm, helper_call)?;
        need(
            helper.target.arm.is_none()
                && helper.target.source == formatting.target.source
                && matches!(std_module.items.get(helper.target.body.1),
                Some(ScalarItem::Function(fun)) if fun.name == "_format_u8"),
            "nested selected u8 producer",
        )?;
        let producer = self.producer(helper.target.body)?;
        let forwarded = self.forward(&arm, &producer)?;
        need(
            forwarded.entry == formatting.target && producer.descriptor == *descriptor,
            "nested producer forwarding proof",
        )?;
        let print_body = self.body(consuming.target.body, None)?;
        let print = self.print_finite_open(&print_body)?;
        need(
            print.print == consuming.target && print.target_layout == app.target_layout,
            "nested selected print proof",
        )?;

        // All Wosy caller events, branches, scope and terminal edges are covered.
        // Marker calls have no relevant pointer value; their linked effects
        // remain entirely open and do not imply an admission theorem.
        let mut trace = Trace::new(caller, root)?;
        let before = self.nested_marker(caller, &mut trace, before, 0)?;
        let source = trace.bind(initialize, &T::U8, |t, value| t.integer(value, &T::U8, 203))?;
        let scalar_point = trace.read(scalar, &T::U8, &source)?;
        let nested_point = trace.call(
            inner,
            &T::Struct(descriptor.clone()),
            caller
                .nodes
                .target(inner)
                .map_err(|_| Rejected::Missing("nested format target"))?,
            &[scalar_point],
        )?;
        let print_point = trace.call(
            outer,
            &T::U64,
            caller
                .nodes
                .target(outer)
                .map_err(|_| Rejected::Missing("nested print target"))?,
            &[nested_point],
        )?;
        need(
            nested_point == formatting.point && print_point == consuming.point,
            "nested evaluated caller order",
        )?;
        let after = self.nested_marker(caller, &mut trace, after, 1)?;
        let finish = self.nested_marker(caller, &mut trace, finish, 2)?;
        need(
            before.declaration == after.declaration && before.declaration == finish.declaration,
            "nested shared marker declaration",
        )?;
        trace.finish_unit(root)?;

        Ok(U8NestedCallerOpen {
            caller: caller.entry.id.clone(),
            producer,
            forwarded,
            formatting,
            consuming,
            print,
            markers: [before, after, finish],
            partition: PartitionAdmissionOpen,
        })
    }

    fn nested_marker(
        &self,
        caller: &CallableBody<'a>,
        trace: &mut Trace<'a, '_>,
        expr: &E,
        number: u64,
    ) -> Result<MarkerOpen, Rejected> {
        let E::Call {
            arguments,
            type_arguments,
            span,
            ..
        } = expr
        else {
            return Err(Rejected::Contradictory("nested marker expression"));
        };
        let [marker, sentinel, left, right] = arguments.as_slice() else {
            return Err(Rejected::Contradictory("nested marker arity"));
        };
        need(type_arguments.is_empty(), "nested marker specialization")?;
        let target = caller
            .nodes
            .target(expr)
            .map_err(|_| Rejected::Missing("nested marker target"))?;
        let ResolvedCallTarget::ExternCallable {
            source,
            declaration_span,
        } = target
        else {
            return Err(Rejected::UnsupportedTarget);
        };
        need(
            source == &caller.entry.id.source,
            "nested marker declaration source",
        )?;
        let module = &self.build.prepared().project().modules[caller.entry.id.body.0];
        let mut rows = module
            .items
            .iter()
            .filter_map(|item| match item {
                ScalarItem::Extern(external) => Some(external),
                _ => None,
            })
            .flat_map(|external| external.functions.iter().map(move |fun| (external, fun)))
            .filter(|(_, fun)| fun.name_span == *declaration_span);
        let (external, function) = rows
            .next()
            .ok_or(Rejected::Missing("nested marker declaration"))?;
        let raw = T::RawPointer(Box::new(T::U8));
        let (
            ScalarExternModule::Valid(import),
            T::Callable {
                parameters,
                outputs,
            },
        ) = (&external.actual_module, &function.signature)
        else {
            return Err(Rejected::UnsupportedTarget);
        };
        need(
            rows.next().is_none()
                && external.kind == "wasm"
                && external.binding == "observer"
                && import == "fixture_discard"
                && function.name == "checkpoint"
                && parameters == &[T::I32, T::I32, raw.clone(), raw.clone()]
                && matches!(outputs.outputs.as_slice(), [output] if output.ty == T::Unit),
            "nested marker declaration",
        )?;
        let marker = trace.integer(marker, &T::I32, number)?;
        let sentinel = trace.integer(sentinel, &T::I32, 0)?;
        let null = |trace: &mut Trace<'a, '_>, value: &E| {
            need(
                matches!(value, E::Name { .. })
                    && matches!(
                        caller.nodes.plain_name(value),
                        Some(PlainNameIdentity::Null)
                    ),
                "nested marker null only",
            )?;
            trace.evaluate(value, Some(&raw))
        };
        let left = null(trace, left)?;
        let right = null(trace, right)?;
        let point = trace.call(expr, &T::Unit, target, &[marker, sentinel, left, right])?;
        Ok(MarkerOpen {
            site: SourceSpan::new(source.clone(), *span),
            point,
            number,
            declaration: MarkerDeclarationOpen {
                source: SourceSpan::new(source.clone(), *declaration_span),
                binding: external.binding.clone(),
                module: import.clone(),
                member: function.name.clone(),
            },
        })
    }
}
