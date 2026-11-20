//! Finite typed Wosy-side route only. The externally linked WASI effect stays open.
use super::*;
use crate::scalar::control::{
    BindingDefinition, DirectDestinationIdentity, NamedPlaceRootIdentity,
};
use crate::scalar::{
    CoreOperationId, ReferenceBindingId, ReferenceBindingKind, ReferencePlaceProjection,
    ScalarExternModule, ScalarFieldReference, ScalarStructFieldId, ScalarTargetLayout,
};
use crate::{BinaryOperator as Op, ScalarBlock, ScalarPlace};

#[derive(Debug)]
pub(super) struct PrintFiniteOpen {
    pub(super) print: EntryId,
    pub(super) wrapper: EntryId,
    pub(super) target_layout: ScalarTargetLayout,
    pub(super) boundary: LinkedBoundaryOpen,
}

impl<'a> Ledger<'a> {
    pub(super) fn print_finite_open(&self, body: &CallableBody<'a>) -> PResult<PrintFiniteOpen> {
        need(
            std::ptr::eq(body.build, self.build) && body.entry.id.arm.is_none(),
            "print body provenance",
        )?;
        need(body.entry.function.name == "print", "print selected entry")?;
        let (parameters, outputs) = body.entry.signature()?;
        let [T::Struct(text_id)] = parameters else {
            return Err(Rejected::Contradictory("print signature"));
        };
        need(
            outputs == [&T::U64, &T::Bool] && text_id.source == body.entry.id.source,
            "print signature",
        )?;
        let text_ty = T::Struct(text_id.clone());
        let text = formal(body, 0, &text_ty)?;
        let data_ty = pointer(&T::U8);
        let [text_data, text_length] = self.fields(text_id, [&data_ty, &T::U64])?;
        let root = &body.entry.function.body;
        let [reported, zero, complete, max32, maximum, success, iovec, nwritten, status, count, guarded, I::Expression(out_count), I::Expression(out_complete)] =
            root.items.as_slice()
        else {
            return Err(Rejected::Contradictory("print full body shape"));
        };
        let (Some(T::Struct(iovec_id)), Some(T::Struct(nwritten_id))) =
            (binding_type(iovec), binding_type(nwritten))
        else {
            return Err(Rejected::Contradictory("print local ABI aggregates"));
        };
        need(
            iovec_id != nwritten_id
                && iovec_id.source == nwritten_id.source
                && iovec_id.source != text_id.source,
            "print local ABI aggregates",
        )?;
        let source_module = &self.build.prepared().project().modules[body.entry.id.body.0];
        let mut namespace_rows = source_module
            .namespace_bindings
            .iter()
            .filter(|namespace| namespace.binding == "preview1");
        need(
            namespace_rows
                .next()
                .is_some_and(|row| row.target == iovec_id.source)
                && namespace_rows.next().is_none(),
            "print canonical wrapper namespace",
        )?;
        let iovec_ty = T::Struct(iovec_id.clone());
        let nwritten_ty = T::Struct(nwritten_id.clone());
        let [iovec_data, iovec_length] = self.fields(iovec_id, [&data_ty, &T::U32])?;
        let [nwritten_value] = self.fields(nwritten_id, [&T::U32])?;
        let text_length = |trace: &mut Trace<'_, '_>, value: &E| {
            trace.member(value, &T::U64, &text, &text_length)
        };
        let mut t = Trace::new(body, root)?;
        let reported = t.bind(reported, &T::U64, |t, e| t.integer(e, &T::U64, 0))?;
        let zero = t.bind(zero, &T::U64, |t, e| {
            t.convert(e, CoreOperationId::IntExtend, &T::U64, |t, x| {
                t.integer(x, &T::I32, 0)
            })
        })?;
        let length_is_zero = |t: &mut Trace<'_, '_>, e: &E| {
            t.binary(e, Op::Equal, text_length, |t, x| t.read(x, &T::U64, &zero))
        };
        let complete = t.bind(complete, &T::Bool, length_is_zero)?;
        let max32 = t.bind(max32, &T::U32, |t, e| {
            t.integer(e, &T::U32, u32::MAX.into())
        })?;
        let maximum = t.bind(maximum, &T::U64, |t, e| {
            t.convert(e, CoreOperationId::IntExtend, &T::U64, |t, x| {
                t.read(x, &T::U32, &max32)
            })
        })?;
        let success = t.bind(success, &T::I32, |t, e| t.integer(e, &T::I32, 0))?;
        let iovec = t.bind(iovec, &iovec_ty, |t, e| {
            let E::StructLiteral { fields, .. } = e else {
                return Err(Rejected::Contradictory("print Iovec literal"));
            };
            let [data, length] = fields.as_slice() else {
                return Err(Rejected::Contradictory("print Iovec literal"));
            };
            need(
                t.body.nodes.literal_field(&data.value) == Some(&iovec_data)
                    && t.body.nodes.literal_field(&length.value) == Some(&iovec_length),
                "print Iovec fields",
            )?;
            t.member(&data.value, &data_ty, &text, &text_data)?;
            t.convert(
                &length.value,
                CoreOperationId::IntTrunc,
                &T::U32,
                text_length,
            )?;
            t.evaluate(e, Some(&iovec_ty))
        })?;
        let nwritten = t.bind(nwritten, &nwritten_ty, |t, e| {
            let E::StructLiteral { fields, .. } = e else {
                return Err(Rejected::Contradictory("print Nwritten literal"));
            };
            let [value] = fields.as_slice() else {
                return Err(Rejected::Contradictory("print Nwritten literal"));
            };
            need(
                t.body.nodes.literal_field(&value.value) == Some(&nwritten_value),
                "print Nwritten field",
            )?;
            t.integer(&value.value, &T::U32, 0)?;
            t.evaluate(e, Some(&nwritten_ty))
        })?;
        need(
            iovec != nwritten && iovec != text && nwritten != text,
            "print exact distinct local roots",
        )?;
        let status = t.bind(status, &T::I32, |t, e| t.integer(e, &T::I32, 0))?;
        let count = t.bind(count, &T::U32, |t, e| t.integer(e, &T::U32, 0))?;
        let mut selected = None;
        t.branch(guarded, |t, e| t.binary(e, Op::And,
            |t, left| t.binary(left, Op::NotEqual, text_length, |t, x| t.read(x, &T::U64, &zero)),
            |t, right| t.binary(right, Op::LessEqual, text_length, |t, x| t.read(x, &T::U64, &maximum))),
            |t, items| {
                let [I::Expression(unsafe_expr @ E::Block(unsafe_block)), count_store, status_branch] = items else {
                    return Err(Rejected::Contradictory("print guarded I/O shape"));
                };
                t.with_scope(unsafe_block, true, |t, items| {
                    let [call_store] = items else {
                        return Err(Rejected::Contradictory("print unsafe write shape"));
                    };
                    t.assign(call_store, &status, &T::I32, |t, e| {
                        let E::Call { arguments, type_arguments, .. } = e else {
                            return Err(Rejected::Contradictory("print wrapper call shape"));
                        };
                        let [fd, vec, written] = arguments.as_slice() else {
                            return Err(Rejected::Contradictory("print wrapper actuals"));
                        };
                        need(type_arguments.is_empty(), "print wrapper specialization")?;
                        let target = t.body.nodes.target(e).map_err(|_| Rejected::Missing("print wrapper target"))?;
                        let ResolvedCallTarget::ModuleCallable {
                            source, declaration_span, concrete: ConcreteCallSelection::Declaration,
                        } = target else {
                            return Err(Rejected::UnsupportedTarget);
                        };
                        need(source == &iovec_id.source, "print wrapper source")?;
                        let entry = self.lookup(source, *declaration_span, &ConcreteCallSelection::Declaration)?;
                        need(entry.function.name == "fd_write_once", "print selected wrapper")?;
                        let args = [t.integer(fd, &T::I32, 1)?,
                            t.address(vec, &iovec_ty, &iovec)?, t.address(written, &nwritten_ty, &nwritten)?];
                        let point = t.call(e, &T::I32, target, &args)?;
                        selected = Some(entry.id);
                        Ok(point)
                    })
                })?;
                t.evaluate(unsafe_expr, None)?;
                t.assign(count_store, &count, &T::U32,
                    |t, e| t.member(e, &T::U32, &nwritten, &nwritten_value))?;
                t.branch(status_branch,
                    |t, e| t.binary(e, Op::Equal,
                        |t, v| t.read(v, &T::I32, &status), |t, v| t.read(v, &T::I32, &success)),
                    |t, path| {
                        let [set_count, set_complete] = path else {
                            return Err(Rejected::Contradictory("print success path"));
                        };
                        t.assign(set_count, &reported, &T::U64, |t, e|
                            t.convert(e, CoreOperationId::IntExtend, &T::U64, |t, x| t.read(x, &T::U32, &count)))?;
                        t.assign(set_complete, &complete, &T::Bool, |t, e| t.binary(e, Op::Equal,
                            |t, v| t.read(v, &T::U64, &reported), text_length))
                    },
                    |t, path| {
                        let [clear_count, clear_complete] = path else {
                            return Err(Rejected::Contradictory("print failed path"));
                        };
                        t.assign(clear_count, &reported, &T::U64, |t, e| t.integer(e, &T::U64, 0))?;
                        t.assign(clear_complete, &complete, &T::Bool, |t, e| t.boolean(e, false))
                    })
            }, |t, items| {
                let [clear_count, check_empty] = items else {
                    return Err(Rejected::Contradictory("print guarded fallback"));
                };
                t.assign(clear_count, &reported, &T::U64, |t, e| t.integer(e, &T::U64, 0))?;
                t.assign(check_empty, &complete, &T::Bool, length_is_zero)
            })?;
        need(reported != complete, "print independent scalar outputs")?;
        let results = [
            (out_count, t.read(out_count, &T::U64, &reported)?),
            (out_complete, t.read(out_complete, &T::Bool, &complete)?),
        ];
        t.finish(root, &results)?;
        let selected = selected.ok_or(Rejected::Missing("print wrapper selection"))?;
        let wrapper = self.body(selected.body, None)?;
        need(wrapper.entry.id == selected, "print wrapper selection")?;
        let boundary = self.print_wrapper_open(&wrapper, &iovec_ty, &nwritten_ty)?;
        need(
            self.build.prepared().project().modules[wrapper.entry.id.body.0].target_layout
                == source_module.target_layout,
            "print target layout",
        )?;
        Ok(PrintFiniteOpen {
            print: body.entry.id.clone(),
            wrapper: selected,
            target_layout: source_module.target_layout,
            boundary,
        })
    }

    fn fields<const N: usize>(
        &self,
        id: &ScalarStructId,
        types: [&T; N],
    ) -> PResult<[ScalarStructFieldId; N]> {
        let mut rows = self
            .build
            .prepared()
            .project()
            .modules
            .iter()
            .filter(|module| module.source == id.source)
            .flat_map(|module| module.structs.iter())
            .filter(|structure| structure.id == *id);
        let s = rows
            .next()
            .ok_or(Rejected::Missing("print canonical struct"))?;
        need(
            rows.next().is_none() && s.fields.len() == N,
            "print canonical struct",
        )?;
        let mut fields = Vec::with_capacity(N);
        for (index, (field, ty)) in s.fields.iter().zip(types).enumerate() {
            need(
                field.ty == *ty
                    && field.id.structure == *id
                    && field.id.index == index
                    && field.declaration_index == index,
                "print canonical struct field",
            )?;
            fields.push(field.id.clone());
        }
        fields
            .try_into()
            .map_err(|_| Rejected::Contradictory("print canonical struct"))
    }

    pub(super) fn print_wrapper_open(
        &self,
        body: &CallableBody<'a>,
        iovec: &T,
        nwritten: &T,
    ) -> PResult<LinkedBoundaryOpen> {
        let slot_types = [T::I32, pointer(iovec), pointer(nwritten)];
        let (parameters, outputs) = body.entry.signature()?;
        need(
            parameters == slot_types && outputs == [&T::I32],
            "print wrapper signature",
        )?;
        let fd = formal(body, 0, &slot_types[0])?;
        let iovec = formal(body, 1, &slot_types[1])?;
        let nwritten = formal(body, 2, &slot_types[2])?;
        need(
            fd != iovec && fd != nwritten && iovec != nwritten,
            "print wrapper distinct formals",
        )?;
        let root = &body.entry.function.body;
        let [bind, I::Expression(output)] = root.items.as_slice() else {
            return Err(Rejected::Contradictory("print full wrapper shape"));
        };
        let mut t = Trace::new(body, root)?;
        let mut boundary = None;
        let result = t.bind(bind, &T::I32, |t, e| {
            let E::Block(block) = e else {
                return Err(Rejected::Contradictory("print wrapper unsafe block"));
            };
            t.with_scope(block, true, |t, items| {
                let [I::Expression(
                    call @ E::Call {
                        arguments,
                        type_arguments,
                        ..
                    },
                )] = items
                else {
                    return Err(Rejected::Contradictory("print wrapper unsafe body"));
                };
                let [fd_arg, iovec_arg, vectors, nwritten_arg] = arguments.as_slice() else {
                    return Err(Rejected::Contradictory("print extern actuals"));
                };
                need(
                    type_arguments.is_empty()
                        && block
                            .final_expressions()
                            .next()
                            .is_some_and(|(value, _)| std::ptr::eq(value, call)),
                    "print extern result route",
                )?;
                let target = t
                    .body
                    .nodes
                    .target(call)
                    .map_err(|_| Rejected::Missing("print extern target"))?;
                let ResolvedCallTarget::ExternCallable {
                    source,
                    declaration_span,
                } = target
                else {
                    return Err(Rejected::UnsupportedTarget);
                };
                need(
                    source == &body.entry.id.source,
                    "print extern declaration source",
                )?;
                let args = [
                    t.read(fd_arg, &slot_types[0], &fd)?,
                    t.read(iovec_arg, &slot_types[1], &iovec)?,
                    t.integer(vectors, &T::I32, 1)?,
                    t.read(nwritten_arg, &slot_types[2], &nwritten)?,
                ];
                t.call(call, &T::I32, target, &args)?;
                boundary = Some(self.open_extern(
                    source,
                    *declaration_span,
                    [
                        slot_types[0].clone(),
                        slot_types[1].clone(),
                        T::I32,
                        slot_types[2].clone(),
                    ],
                )?);
                Ok(())
            })?;
            t.evaluate(e, Some(&T::I32))
        })?;
        let value = t.read(output, &T::I32, &result)?;
        t.finish(root, &[(output, value)])?;
        boundary.ok_or(Rejected::Missing("print extern boundary"))
    }

    fn open_extern(
        &self,
        source: &SourceIdentity,
        span: ByteSpan,
        expected: [T; 4],
    ) -> PResult<LinkedBoundaryOpen> {
        let mut externs = self
            .build
            .prepared()
            .project()
            .modules
            .iter()
            .filter(|module| &module.source == source)
            .flat_map(|module| module.items.iter())
            .filter_map(|item| match item {
                ScalarItem::Extern(external) => Some(external),
                _ => None,
            })
            .flat_map(|external| external.functions.iter().map(move |fun| (external, fun)))
            .filter(|(_, fun)| fun.name_span == span);
        let (external, fun) = externs
            .next()
            .ok_or(Rejected::Missing("print extern declaration"))?;
        need(
            externs.next().is_none(),
            "print ambiguous extern declaration",
        )?;
        let (
            ScalarExternModule::Valid(module),
            T::Callable {
                parameters,
                outputs,
            },
        ) = (&external.actual_module, &fun.signature)
        else {
            return Err(Rejected::UnsupportedTarget);
        };
        need(
            external.kind == "wasm"
                && external.binding == "_wasi"
                && module == "wasi_snapshot_preview1"
                && fun.name == "fd_write"
                && fun.unsafe_marker
                && parameters == &expected
                && outputs.outputs.len() == 1
                && outputs.outputs[0].ty == T::I32,
            "print extern signature",
        )?;
        Ok(LinkedBoundaryOpen {
            source: SourceSpan::new(source.clone(), span),
            binding: external.binding.clone(),
            module: module.clone(),
            member: fun.name.clone(),
            parameters: expected.into(),
            output: T::I32,
        })
    }
}

fn binding_type(item: &I) -> Option<&T> {
    match item {
        I::LocalBinding(binding) => Some(&binding.declared_type),
        _ => None,
    }
}

fn formal(body: &CallableBody<'_>, slot: usize, expected: &T) -> PResult<ReferenceBindingId> {
    let (identity, ty) = body
        .nodes
        .formal(body.facts, slot)
        .ok_or(Rejected::Missing("print formal slot"))?;
    need(
        ty == expected
            && identity.source == body.entry.id.source
            && identity.kind == ReferenceBindingKind::Declared,
        "print formal slot",
    )?;
    Ok(identity.clone())
}

#[derive(Debug)]
pub(super) struct LinkedBoundaryOpen {
    pub(super) source: SourceSpan,
    pub(super) binding: String,
    pub(super) module: String,
    pub(super) member: String,
    pub(super) parameters: Vec<ScalarType>,
    pub(super) output: ScalarType,
}

type PResult<T> = Result<T, Rejected>;
type E = ScalarExpression;
type K = CfgPointKind;
type T = ScalarType;
type I = ScalarBlockItem;

fn need(condition: bool, why: &'static str) -> PResult<()> {
    condition.then_some(()).ok_or(Rejected::Contradictory(why))
}

fn pointer(ty: &T) -> T {
    T::RawPointer(Box::new(ty.clone()))
}

// Each protocol walk consumes every point once in the canonical construction
// order. The frontier records independently required successors, including
// both ordered branch successors and the shared merge.
pub(super) struct Trace<'a, 'b> {
    body: &'b CallableBody<'a>,
    cursor: usize,
    tail: Option<CfgPointId>,
    scope: ReferenceScopeId,
    next_scope: usize,
}

impl<'a, 'b> Trace<'a, 'b> {
    pub(super) fn new(body: &'b CallableBody<'a>, block: &ScalarBlock) -> PResult<Self> {
        let mut trace = Self {
            body,
            cursor: 0,
            tail: None,
            scope: ReferenceScopeId(0),
            next_scope: 1,
        };
        trace.point(Some(body.entry.function.span), |kind| {
            matches!(kind, K::Entry)
        })?;
        trace.point(Some(block.span), |kind| matches!(kind, K::ScopeEnter))?;
        Ok(trace)
    }

    fn point(
        &mut self,
        span: Option<ByteSpan>,
        matches: impl FnOnce(&K) -> bool,
    ) -> PResult<CfgPointId> {
        let point = self
            .body
            .cfg
            .points
            .get(self.cursor)
            .ok_or(Rejected::Missing("print CFG point"))?;
        let id = CfgPointId(self.cursor);
        need(
            point.id == id
                && point.scope == self.scope
                && point.source_span.source == self.body.entry.id.source
                && span.is_none_or(|span| span == point.source_span.range)
                && matches(&point.kind),
            "print CFG point",
        )?;
        if let Some(previous) = self.tail {
            self.edges(previous, &[id])?;
        }
        self.tail = Some(id);
        self.cursor += 1;
        Ok(id)
    }

    fn edges(&self, from: CfgPointId, expected: &[CfgPointId]) -> PResult<()> {
        need(
            self.body.cfg.points[from.0].successors == expected,
            "print CFG order",
        )
    }

    fn value(
        &mut self,
        expression: &E,
        ty: Option<&T>,
        matches: impl FnOnce(&K) -> bool,
    ) -> PResult<CfgPointId> {
        if let Some(ty) = ty {
            need(
                self.body
                    .nodes
                    .checked_expression_type(expression)
                    .ok_or(Rejected::Missing("print checked expression"))?
                    == ty,
                "print checked expression",
            )?;
        }
        let span = match expression {
            E::Name { span, .. }
            | E::Member { span, .. }
            | E::Integer { span, .. }
            | E::Boolean { span, .. }
            | E::RawAddress { span, .. }
            | E::StructLiteral { span, .. }
            | E::Binary { span, .. }
            | E::Call { span, .. } => *span,
            E::Block(block) => block.span,
            _ => return Err(Rejected::Contradictory("print unclassified expression")),
        };
        let id = self.point(Some(span), matches)?;
        need(
            self.body.eval(expression) == Some(id),
            "print evaluated occurrence",
        )?;
        Ok(id)
    }

    pub(super) fn evaluate(&mut self, expr: &E, ty: Option<&T>) -> PResult<CfgPointId> {
        self.value(
            expr,
            ty,
            |kind| matches!(kind, K::Evaluate(value) if value == expr),
        )
    }

    pub(super) fn integer(&mut self, expr: &E, ty: &T, expected: u64) -> PResult<CfgPointId> {
        need(
            matches!(expr, E::Integer { value, .. } if value == &expected.into()),
            "print integer",
        )?;
        self.evaluate(expr, Some(ty))
    }

    fn boolean(&mut self, expr: &E, expected: bool) -> PResult<CfgPointId> {
        need(
            matches!(expr, E::Boolean { value, .. } if *value == expected),
            "print boolean",
        )?;
        self.evaluate(expr, Some(&T::Bool))
    }

    pub(super) fn read(
        &mut self,
        expr: &E,
        ty: &T,
        identity: &ReferenceBindingId,
    ) -> PResult<CfgPointId> {
        need(
            matches!(expr, E::Name { .. })
                && matches!(self.body.nodes.plain_name(expr), Some(PlainNameIdentity::Lexical(id)) if id == identity),
            "print lexical read",
        )?;
        self.value(
            expr,
            Some(ty),
            |kind| matches!(kind, K::Read { binding } if binding == identity),
        )
    }

    fn member(
        &mut self,
        expr: &E,
        ty: &T,
        identity: &ReferenceBindingId,
        field: &ScalarStructFieldId,
    ) -> PResult<CfgPointId> {
        let (owner, selected) = self
            .body
            .nodes
            .lexical_member(expr)
            .ok_or(Rejected::Missing("print lexical member"))?;
        need(
            owner == identity && selected == field,
            "print lexical member",
        )?;
        self.value(expr, Some(ty), |kind| {
            let K::ReadPlace { place, pointer_point: None, candidates, reads_origin: false } = kind else {
                return false;
            };
            matches!((place, candidates.as_slice()),
                (ScalarPlace::Field { base, field: ScalarFieldReference::Resolved(actual), .. }, [candidate])
                if matches!(base.as_ref(), ScalarPlace::Name { .. })
                && actual == field && candidate.place == *place && candidate.binding == *identity
                && matches!(candidate.projections.as_slice(),
                    [ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(id))] if id == field))
        })
    }

    fn address(&mut self, expr: &E, ty: &T, identity: &ReferenceBindingId) -> PResult<CfgPointId> {
        let E::RawAddress {
            place: place @ ScalarPlace::Name { .. },
            ..
        } = expr
        else {
            return Err(Rejected::Contradictory("print direct raw address"));
        };
        need(
            matches!(self.body.nodes.named_place_root(place), Some(NamedPlaceRootIdentity::Lexical(id)) if id == identity),
            "print raw address root",
        )?;
        self.evaluate(expr, Some(&pointer(ty)))
    }

    pub(super) fn call(
        &mut self,
        expr: &E,
        ty: &T,
        target: &ResolvedCallTarget,
        args: &[CfgPointId],
    ) -> PResult<CfgPointId> {
        let E::Call { span, .. } = expr else {
            return Err(Rejected::Contradictory("print call"));
        };
        need(
            self.body.nodes.target(expr).ok() == Some(target),
            "print call target",
        )?;
        let id = self.value(expr, Some(ty), |kind| {
            matches!(kind, K::Call { output: 0, target: actual, expression, argument_points }
                if actual == target && expression == expr && argument_points == args)
        })?;
        need(
            self.body.cfg.points[id.0].source_span.range == *span,
            "print call span",
        )?;
        Ok(id)
    }

    fn convert(
        &mut self,
        expr: &E,
        operation: CoreOperationId,
        ty: &T,
        actual: impl FnOnce(&mut Self, &E) -> PResult<CfgPointId>,
    ) -> PResult<CfgPointId> {
        let E::Call {
            arguments,
            type_arguments,
            ..
        } = expr
        else {
            return Err(Rejected::Contradictory("print integer conversion"));
        };
        let ([arg], [type_arg]) = (arguments.as_slice(), type_arguments.as_slice()) else {
            return Err(Rejected::Contradictory("print integer conversion"));
        };
        need(type_arg.ty == *ty, "print integer conversion")?;
        let input = actual(self, arg)?;
        self.call(expr, ty, &ResolvedCallTarget::Core(operation), &[input])
    }

    fn binary(
        &mut self,
        expr: &E,
        expected: Op,
        left_fn: impl FnOnce(&mut Self, &E) -> PResult<CfgPointId>,
        right_fn: impl FnOnce(&mut Self, &E) -> PResult<CfgPointId>,
    ) -> PResult<CfgPointId> {
        let E::Binary {
            operator,
            left,
            right,
            ..
        } = expr
        else {
            return Err(Rejected::Contradictory("print scalar condition"));
        };
        need(*operator == expected, "print scalar condition")?;
        left_fn(self, left)?;
        right_fn(self, right)?;
        self.evaluate(expr, Some(&T::Bool))
    }

    pub(super) fn bind(
        &mut self,
        item: &I,
        ty: &T,
        initialize: impl FnOnce(&mut Self, &E) -> PResult<CfgPointId>,
    ) -> PResult<ReferenceBindingId> {
        let I::LocalBinding(binding) = item else {
            return Err(Rejected::Contradictory("print local declaration"));
        };
        need(
            !binding.is_allocation && binding.receivers.len() <= 1 && binding.declared_type == *ty,
            "print local declaration",
        )?;
        let rhs = initialize(self, &binding.value)?;
        let Some(BindingDefinition::Initialized { bind, value }) = self
            .body
            .nodes
            .binding_definition(self.body.facts, binding, 0)
        else {
            return Err(Rejected::Missing("print binding definition"));
        };
        let point = self.point(Some(binding.name_span),
            |kind| matches!(kind, K::Bind { binding: id }
                if id.declaration_span == binding.name_span && id.kind == ReferenceBindingKind::Declared
                && id.source == self.body.entry.id.source
                && id.function_span == self.body.entry.function.span
                && id.block_span == self.body.entry.function.body.span))?;
        need(bind == point && value == rhs, "print binding route")?;
        let K::Bind { binding } = &self.body.cfg.points[point.0].kind else {
            unreachable!()
        };
        Ok(binding.clone())
    }

    fn assign(
        &mut self,
        item: &I,
        identity: &ReferenceBindingId,
        ty: &T,
        rhs: impl FnOnce(&mut Self, &E) -> PResult<CfgPointId>,
    ) -> PResult<()> {
        let I::Assignment(assignment) = item else {
            return Err(Rejected::Contradictory("print assignment"));
        };
        let ([target], [expression]) =
            (assignment.targets.as_slice(), assignment.values.as_slice())
        else {
            return Err(Rejected::Contradictory("print assignment arity"));
        };
        need(
            matches!(&target.place, ScalarPlace::Name { .. })
                && matches!(self.body.nodes.direct_destination(&target.place),
                Some(DirectDestinationIdentity::Existing(binding)) if binding == identity),
            "print assignment destination",
        )?;
        let destination_type = self
            .body
            .nodes
            .checked_place_type(&target.place)
            .ok_or(Rejected::Missing("print assignment destination type"))?;
        need(destination_type == ty, "print assignment destination type")?;
        let expression_type = self
            .body
            .nodes
            .checked_expression_type(expression)
            .ok_or(Rejected::Missing("print assignment RHS type"))?;
        need(
            expression_type == destination_type,
            "print assignment RHS type",
        )?;
        let rhs = rhs(self, expression)?;
        let install = self
            .body
            .nodes
            .frozen_install(self.body.facts, &target.place)
            .ok_or(Rejected::Missing("print assignment install"))?;
        let point = self.point(Some(target.target_span), |kind| {
            matches!(kind, K::Assign { target: write, rhs_point, .. }
                if write.binding == *identity && write.place == target.place
                && write.projections.is_empty() && *rhs_point == rhs)
        })?;
        need(
            install.rhs == rhs && install.install == point,
            "print assignment order",
        )
    }

    fn with_scope(
        &mut self,
        block: &ScalarBlock,
        unsafe_context: bool,
        f: impl FnOnce(&mut Self, &[I]) -> PResult<()>,
    ) -> PResult<(CfgPointId, CfgPointId)> {
        need(block.unsafe_context == unsafe_context, "print scope safety")?;
        let parent = self.scope;
        self.scope = ReferenceScopeId(self.next_scope);
        self.next_scope += 1;
        let start = self.point(Some(block.span), |kind| matches!(kind, K::ScopeEnter))?;
        f(self, &block.items)?;
        let end = self.point(Some(block.span), |kind| matches!(kind, K::ScopeExit))?;
        self.scope = parent;
        Ok((start, end))
    }

    fn branch(
        &mut self,
        item: &I,
        check: impl FnOnce(&mut Self, &E) -> PResult<CfgPointId>,
        then: impl FnOnce(&mut Self, &[I]) -> PResult<()>,
        otherwise: impl FnOnce(&mut Self, &[I]) -> PResult<()>,
    ) -> PResult<()> {
        let I::Expression(E::If {
            condition,
            then_branch,
            else_branch,
            span,
        }) = item
        else {
            return Err(Rejected::Contradictory("print guarded branch"));
        };
        let condition_point = check(self, condition)?;
        let condition_span = self.body.cfg.points[condition_point.0].source_span.range;
        let branch = self.point(
            Some(condition_span),
            |kind| matches!(kind, K::Branch { condition: actual } if actual == condition.as_ref()),
        )?;
        self.tail = None;
        let (then_start, then_end) = self.with_scope(then_branch, false, then)?;
        self.tail = None;
        let (else_start, _else_end) = self.with_scope(else_branch, false, otherwise)?;
        self.edges(branch, &[then_start, else_start])?;
        let join = self.point(Some(*span), |kind| matches!(kind, K::Join))?;
        self.edges(then_end, &[join])
    }

    pub(super) fn finish_unit(mut self, root: &ScalarBlock) -> PResult<()> {
        need(
            !root.unsafe_context && root.final_output_values.is_empty(),
            "nested unit final outputs",
        )?;
        let exit = self.point(Some(root.span), |kind| matches!(kind, K::ScopeExit))?;
        self.edges(exit, &[])?;
        need(
            self.cursor == self.body.cfg.points.len() && self.body.cfg.allocations.is_empty(),
            "nested unclassified CFG effect",
        )
    }

    fn finish(mut self, root: &ScalarBlock, outputs: &[(&E, CfgPointId)]) -> PResult<()> {
        need(
            !root.unsafe_context && root.final_output_values.len() == outputs.len(),
            "print return arity",
        )?;
        for (ordinal, (value, point)) in outputs.iter().enumerate() {
            need(
                self.body.nodes.final_output(value) == Some(ordinal),
                "print canonical return slot",
            )?;
            let span = self.body.cfg.points[point.0].source_span.range;
            self.point(Some(span), |kind| {
                matches!(kind,
                K::ReturnOutput { output, value_point, checked: false }
                if *output == ordinal && *value_point == *point)
            })?;
        }
        self.point(Some(root.span), |kind| matches!(kind, K::ScopeExit))?;
        let ret = self.point(None, |kind| matches!(kind, K::Return))?;
        self.edges(ret, &[])?;
        need(
            self.cursor == self.body.cfg.points.len() && self.body.cfg.allocations.is_empty(),
            "print unclassified CFG effect",
        )
    }
}
