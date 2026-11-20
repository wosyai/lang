#![allow(dead_code)]

use std::collections::BTreeMap;

use wosy_syntax::{ByteSpan, SourceIdentity, SourceSpan};

use super::formatter_proof::{Proof, ProofBatch, Unproved};
use crate::scalar::control::{
    CallableBodyCallNodes, CallablePointFacts, InitializerCallNodes,
    ModuleInitializationCallTarget, PlainNameIdentity,
};
use crate::scalar::{
    CfgPointId, CfgPointKind, ConcreteCallSelection, ReferenceScopeId, ResolvedCallTarget,
    ScalarOverloadSelection, ScalarReferenceCfg, ScalarReferenceMutability, ScalarStructId,
};
use crate::{
    ScalarBlockItem, ScalarBuildValidation, ScalarExpression, ScalarFunction, ScalarItem,
    ScalarType,
};

type Body = (usize, usize);
type SourceKey<'a> = (&'a str, &'a str, &'a str, &'a str);
type SpanKey = (u32, u32);

fn source_key(source: &SourceIdentity) -> SourceKey<'_> {
    (
        &source.project,
        &source.package,
        &source.path,
        &source.revision,
    )
}

fn span_key(span: ByteSpan) -> SpanKey {
    (span.start, span.end)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IndexEntry {
    Unique(Body),
    Ambiguous,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EntryId {
    source: SourceIdentity,
    declaration: ByteSpan,
    body: Body,
    arm: Option<usize>,
}

struct Entry<'a> {
    id: EntryId,
    function: &'a ScalarFunction,
}

impl Entry<'_> {
    fn signature(&self) -> Result<(&[ScalarType], Vec<&ScalarType>), Rejected> {
        let ScalarType::Callable {
            parameters,
            outputs,
        } = &self.function.signature
        else {
            return Err(Rejected::Contradictory("callable signature"));
        };
        Ok((
            parameters,
            outputs.outputs.iter().map(|output| &output.ty).collect(),
        ))
    }
}

#[derive(Debug)]
enum Rejected {
    Diagnostics,
    Missing(&'static str),
    AmbiguousDeclaration,
    UnsupportedSpecialization,
    UnsupportedTarget,
    Contradictory(&'static str),
    Producer(Unproved),
}

struct Ledger<'a> {
    build: &'a ScalarBuildValidation<'a>,
    declarations: BTreeMap<SourceKey<'a>, BTreeMap<SpanKey, IndexEntry>>,
    producers: ProofBatch<'a>,
}

#[derive(Debug, PartialEq, Eq)]
struct CallSlot {
    point: CfgPointId,
    ty: ScalarType,
}

#[derive(Debug, PartialEq, Eq)]
struct DirectCall {
    caller: EntryId,
    target: EntryId,
    source: SourceSpan,
    point: CfgPointId,
    actuals: Vec<CallSlot>,
    // Ordered signature types only; no provenance claim about any result.
    results: Vec<ScalarType>,
}

#[derive(Debug, PartialEq, Eq)]
struct CheckedConsumerOpen {
    call: DirectCall,
    descriptor_parameter: usize,
    checked_output: usize,
    mode: ScalarReferenceMutability,
    referent: ScalarType,
}

#[derive(Debug, PartialEq, Eq)]
struct InitializerTargetOnly {
    source: SourceSpan,
    target: EntryId,
}

struct CallableBody<'a> {
    build: &'a ScalarBuildValidation<'a>,
    entry: Entry<'a>,
    nodes: CallableBodyCallNodes<'a>,
    facts: &'a CallablePointFacts,
    cfg: &'a ScalarReferenceCfg,
}

struct InitializerBody<'a> {
    build: &'a ScalarBuildValidation<'a>,
    source: &'a SourceIdentity,
    nodes: InitializerCallNodes<'a>,
}

// Cannot be constructed from an inventory receipt. A forwarder records the
// exact input certificate's target while retaining its own concrete identity.
enum Certificate {
    Producer(Proof),
    Forwarded { target: EntryId, site: CfgPointId },
}

struct Receipt<'a> {
    build: &'a ScalarBuildValidation<'a>,
    entry: EntryId,
    descriptor: ScalarStructId,
    evidence: Certificate,
}

impl<'a> Ledger<'a> {
    fn new(build: &'a ScalarBuildValidation<'a>) -> Result<Self, Rejected> {
        if !build.diagnostics().is_empty() {
            return Err(Rejected::Diagnostics);
        }
        let mut ledger = Self {
            build,
            declarations: BTreeMap::new(),
            producers: ProofBatch::new(build),
        };
        for (module, prepared) in build.prepared().project().modules.iter().enumerate() {
            for (item, declaration) in prepared.items.iter().enumerate() {
                if let ScalarItem::Function(function) = declaration {
                    ledger.insert(&prepared.source, function.name_span, (module, item));
                }
            }
        }
        Ok(ledger)
    }

    fn insert(&mut self, source: &'a SourceIdentity, declaration: ByteSpan, body: Body) {
        self.declarations
            .entry(source_key(source))
            .or_default()
            .entry(span_key(declaration))
            .and_modify(|entry| *entry = IndexEntry::Ambiguous)
            .or_insert(IndexEntry::Unique(body));
    }

    fn lookup(
        &self,
        source: &SourceIdentity,
        declaration: ByteSpan,
        selection: &ConcreteCallSelection,
    ) -> Result<Entry<'a>, Rejected> {
        let body = match self
            .declarations
            .get(&source_key(source))
            .and_then(|entries| entries.get(&span_key(declaration)))
        {
            None => return Err(Rejected::Missing("declaration")),
            Some(IndexEntry::Ambiguous) => return Err(Rejected::AmbiguousDeclaration),
            Some(IndexEntry::Unique(body)) => *body,
        };
        let arm = match selection {
            ConcreteCallSelection::Declaration => None,
            ConcreteCallSelection::Overload(ScalarOverloadSelection {
                arm_index,
                substitutions,
            }) if substitutions.is_empty() => Some(*arm_index),
            ConcreteCallSelection::Overload(_) => return Err(Rejected::UnsupportedSpecialization),
        };
        let entry = self.entry_at(body, arm)?;
        if entry.id.source != *source || entry.id.declaration != declaration {
            return Err(Rejected::Contradictory("indexed declaration"));
        }
        Ok(entry)
    }

    fn entry_at(&self, body: Body, arm: Option<usize>) -> Result<Entry<'a>, Rejected> {
        let module = self
            .build
            .prepared()
            .project()
            .modules
            .get(body.0)
            .ok_or(Rejected::Missing("module"))?;
        let Some(ScalarItem::Function(fun)) = module.items.get(body.1) else {
            return Err(Rejected::UnsupportedTarget);
        };
        if !fun.generic_parameters.is_empty() {
            return Err(Rejected::UnsupportedSpecialization);
        }
        let function = match arm {
            None if fun.overload_arms.is_empty() => fun,
            None => return Err(Rejected::UnsupportedSpecialization),
            Some(index) => fun
                .overload_arms
                .get(index)
                .ok_or(Rejected::UnsupportedSpecialization)?,
        };
        if !function.generic_parameters.is_empty() || !function.overload_arms.is_empty() {
            return Err(Rejected::UnsupportedSpecialization);
        }
        Ok(Entry {
            id: EntryId {
                source: module.source.clone(),
                declaration: fun.name_span,
                body,
                arm,
            },
            function,
        })
    }

    fn body(&self, body: Body, arm: Option<usize>) -> Result<CallableBody<'a>, Rejected> {
        let entry = self.entry_at(body, arm)?;
        let nodes = self
            .build
            .prepared()
            .bind_callable_body_call_nodes(body.0, body.1, arm, &entry.function.body)
            .map_err(|_| Rejected::Missing("bound body"))?;
        let cfg = self
            .build
            .reference_cfg(body.0, body.1, arm)
            .ok_or(Rejected::Missing("body cfg"))?;
        let facts = self
            .build
            .callable_point_facts(body.0, body.1, arm)
            .ok_or(Rejected::Missing("body facts"))?;
        Ok(CallableBody {
            build: self.build,
            entry,
            nodes,
            facts,
            cfg,
        })
    }

    fn initializer(&self, module: usize) -> Result<InitializerBody<'a>, Rejected> {
        let prepared = self
            .build
            .prepared()
            .project()
            .modules
            .get(module)
            .ok_or(Rejected::Missing("initializer module"))?;
        let region = self
            .build
            .prepared()
            .initialization_region(module)
            .ok_or(Rejected::Missing("initializer region"))?;
        let nodes = region
            .bind_call_nodes(&prepared.items)
            .map_err(|_| Rejected::Missing("bound initializer"))?;
        Ok(InitializerBody {
            build: self.build,
            source: &prepared.source,
            nodes,
        })
    }

    fn initializer_target(
        &self,
        initializer: &InitializerBody<'a>,
        call: &ScalarExpression,
    ) -> Result<InitializerTargetOnly, Rejected> {
        if !std::ptr::eq(self.build, initializer.build) {
            return Err(Rejected::Contradictory("initializer artifact provenance"));
        }
        let ScalarExpression::Call { span, .. } = call else {
            return Err(Rejected::UnsupportedTarget);
        };
        let target = initializer
            .nodes
            .target(call)
            .map_err(|_| Rejected::Missing("initializer target"))?;
        let ModuleInitializationCallTarget::Direct {
            declaration,
            concrete,
        } = target
        else {
            return Err(
                if matches!(target, ModuleInitializationCallTarget::Generic { .. }) {
                    Rejected::UnsupportedSpecialization
                } else {
                    Rejected::UnsupportedTarget
                },
            );
        };
        let entry = self.lookup(&declaration.source, declaration.range, concrete)?;
        Ok(InitializerTargetOnly {
            source: SourceSpan::new(initializer.source.clone(), *span),
            target: entry.id,
        })
    }

    fn direct_call(
        &self,
        body: &CallableBody<'a>,
        call: &ScalarExpression,
    ) -> Result<DirectCall, Rejected> {
        if !std::ptr::eq(self.build, body.build) {
            return Err(Rejected::Contradictory("body artifact provenance"));
        }
        let ScalarExpression::Call {
            arguments,
            span,
            type_arguments,
            ..
        } = call
        else {
            return Err(Rejected::UnsupportedTarget);
        };
        let target = body
            .nodes
            .target(call)
            .map_err(|_| Rejected::Missing("body target"))?;
        let ResolvedCallTarget::ModuleCallable {
            source,
            declaration_span,
            concrete,
        } = target
        else {
            return Err(Rejected::UnsupportedTarget);
        };
        if !type_arguments.is_empty() {
            return Err(Rejected::UnsupportedSpecialization);
        }
        let entry = self.lookup(source, *declaration_span, concrete)?;
        let (parameters, results) = entry.signature()?;
        let point = body
            .eval(call)
            .ok_or(Rejected::Missing("body call evaluation"))?;
        let event = body
            .cfg
            .points
            .get(point.0)
            .ok_or(Rejected::Missing("body call event"))?;
        let CfgPointKind::Call {
            output: 0,
            target: actual_target,
            argument_points,
            expression,
        } = &event.kind
        else {
            return Err(Rejected::Contradictory("body call event"));
        };
        let source = SourceSpan::new(body.entry.id.source.clone(), *span);
        if event.id != point
            || event.source_span != source
            || target != actual_target
            || expression != call
            || parameters.len() != arguments.len()
            || argument_points.len() != arguments.len()
        {
            return Err(Rejected::Contradictory("body call identity"));
        }
        let actuals = arguments
            .iter()
            .zip(parameters)
            .zip(argument_points)
            .map(|((argument, parameter), point)| {
                let ty = body
                    .nodes
                    .checked_expression_type(argument)
                    .ok_or(Rejected::Missing("actual type"))?;
                let arg_event = body
                    .cfg
                    .points
                    .get(point.0)
                    .ok_or(Rejected::Missing("actual event"))?;
                if body.eval(argument) != Some(*point)
                    || *point == event.id
                    || arg_event.id != *point
                    || arg_event.source_span.source != source.source
                    || ty != parameter
                {
                    return Err(Rejected::Contradictory("ordered actual slot"));
                }
                Ok(CallSlot {
                    point: *point,
                    ty: ty.clone(),
                })
            })
            .collect::<Result<_, _>>()?;
        let result = body
            .nodes
            .checked_expression_type(call)
            .ok_or(Rejected::Missing("result type"))?;
        if result != results.first().copied().unwrap_or(&ScalarType::Unit) {
            return Err(Rejected::Contradictory("primary result slot"));
        }
        Ok(DirectCall {
            caller: body.entry.id.clone(),
            target: entry.id.clone(),
            source,
            point,
            actuals,
            results: results.into_iter().cloned().collect(),
        })
    }

    fn producer(&self, body: Body) -> Result<Receipt<'a>, Rejected> {
        let entry = self.entry_at(body, None)?;
        let (_, results) = entry.signature()?;
        let [ScalarType::Struct(descriptor)] = results.as_slice() else {
            return Err(Rejected::Contradictory("producer result"));
        };
        let evidence = self
            .producers
            .prove(body.0, body.1)
            .map_err(Rejected::Producer)?;
        if evidence.output != 0
            || evidence.owner.source != entry.id.source
            || evidence.field.structure != *descriptor
        {
            return Err(Rejected::Contradictory("producer evidence"));
        }
        Ok(Receipt {
            build: self.build,
            entry: entry.id.clone(),
            descriptor: descriptor.clone(),
            evidence: Certificate::Producer(evidence),
        })
    }

    fn forward(
        &self,
        body: &CallableBody<'a>,
        proven_target: &Receipt<'a>,
    ) -> Result<Receipt<'a>, Rejected> {
        if !std::ptr::eq(self.build, proven_target.build) || body.entry.id == proven_target.entry {
            return Err(Rejected::Contradictory("forward certificate provenance"));
        }
        let block = &body.entry.function.body;
        let [ScalarBlockItem::Expression(value @ ScalarExpression::Call { arguments, .. })] =
            block.items.as_slice()
        else {
            return Err(Rejected::Contradictory("pure forwarding body"));
        };
        if block.unsafe_context || body.nodes.final_output(value) != Some(0) {
            return Err(Rejected::Contradictory("pure forwarding body"));
        }
        let call = self.direct_call(body, value)?;
        let (parameters, outputs) = body.entry.signature()?;
        let [ScalarType::Struct(descriptor)] = outputs.as_slice() else {
            return Err(Rejected::Contradictory("forward result type"));
        };
        if call.target != proven_target.entry
            || *descriptor != proven_target.descriptor
            || !matches!(call.results.as_slice(), [ScalarType::Struct(id)] if id == descriptor)
            || parameters.len() != arguments.len()
        {
            return Err(Rejected::Contradictory("forward target/output"));
        }
        for (ordinal, (argument, slot)) in arguments.iter().zip(&call.actuals).enumerate() {
            let (binding, ty) = body
                .nodes
                .formal(body.facts, ordinal)
                .ok_or(Rejected::Missing("forward formal"))?;
            if arguments.len() != call.actuals.len()
                || ty != &slot.ty
                || &parameters[ordinal] != ty
                || !matches!(body.nodes.plain_name(argument), Some(PlainNameIdentity::Lexical(id)) if id == binding)
                || !matches!(&body.cfg.points[slot.point.0].kind, CfgPointKind::Read { binding: id } if id == binding)
            {
                return Err(Rejected::Contradictory("forward formal route"));
            }
        }
        let output = body.unchecked_return(&call)?;
        body.forward_executable_path(&call, output)?;
        Ok(Receipt {
            build: self.build,
            entry: body.entry.id.clone(),
            descriptor: descriptor.clone(),
            evidence: Certificate::Forwarded {
                target: call.target,
                site: call.point,
            },
        })
    }

    fn checked_open(
        &self,
        body: &CallableBody<'a>,
        value: &ScalarExpression,
        descriptor: &Receipt<'a>,
    ) -> Result<CheckedConsumerOpen, Rejected> {
        if !std::ptr::eq(self.build, descriptor.build) {
            return Err(Rejected::Contradictory("checked appointment provenance"));
        }
        let call = self.direct_call(body, value)?;
        if !matches!(call.actuals.as_slice(), [CallSlot { ty: ScalarType::Struct(id), .. }] if id == &descriptor.descriptor)
        {
            return Err(Rejected::Contradictory("checked appointment descriptor"));
        }
        let [ScalarType::CheckedReference { mutability, inner }] = call.results.as_slice() else {
            return Err(Rejected::Contradictory("checked appointment result"));
        };
        if !matches!(inner.as_ref(), ScalarType::F32 | ScalarType::F64) {
            return Err(Rejected::Contradictory("checked appointment referent"));
        }
        let (mode, referent) = (*mutability, inner.as_ref().clone());
        Ok(CheckedConsumerOpen {
            call,
            descriptor_parameter: 0,
            checked_output: 0,
            mode,
            referent,
        })
    }
}

impl CallableBody<'_> {
    fn eval(&self, value: &ScalarExpression) -> Option<CfgPointId> {
        self.nodes.evaluation_point(self.facts, value)
    }

    fn unchecked_return(&self, call: &DirectCall) -> Result<CfgPointId, Rejected> {
        let mut outputs = self
            .cfg
            .points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::ReturnOutput { .. }));
        let output = outputs.next().ok_or(Rejected::Missing("forward return"))?;
        let site = self
            .cfg
            .points
            .get(call.point.0)
            .ok_or(Rejected::Missing("forward call"))?;
        if outputs.next().is_some()
            || !matches!(output.kind, CfgPointKind::ReturnOutput { output: 0, value_point, checked: false } if value_point == call.point)
            || output.source_span != call.source
            || output.scope != site.scope
            || output.id == call.point
            || site.successors != [output.id]
        {
            return Err(Rejected::Contradictory("forward return identity"));
        }
        Ok(output.id)
    }

    fn forward_executable_path(
        &self,
        call: &DirectCall,
        output: CfgPointId,
    ) -> Result<(), Rejected> {
        let invalid = || Rejected::Contradictory("forward executable path");
        // A plain root body contributes Entry, ScopeEnter, ReturnOutput, ScopeExit,
        // and terminal Return in addition to the reads and the call. Reject extra
        // events even if disconnected rather than certifying an incomplete graph.
        if self.cfg.points.len() != call.actuals.len() + 6 {
            return Err(invalid());
        }
        let mut next = Some(CfgPointId(0));
        let mut visited = std::collections::BTreeSet::new();
        let mut step =
            |expected: Option<CfgPointId>, check: &dyn Fn(&CfgPointKind) -> bool| -> Option<()> {
                let id = next?;
                let point = self.cfg.points.get(id.0)?;
                if expected.is_some_and(|expected| expected != id)
                    || point.id != id
                    || !visited.insert(id.0)
                    || point.scope != ReferenceScopeId(0)
                    || point.source_span.source != self.entry.id.source
                    || !check(&point.kind)
                {
                    return None;
                }
                next = match point.successors.as_slice() {
                    [] => None,
                    [successor] => Some(*successor),
                    _ => return None,
                };
                Some(())
            };
        step(None, &|kind| matches!(kind, CfgPointKind::Entry)).ok_or_else(invalid)?;
        step(None, &|kind| matches!(kind, CfgPointKind::ScopeEnter)).ok_or_else(invalid)?;
        for slot in &call.actuals {
            step(Some(slot.point), &|kind| {
                matches!(kind, CfgPointKind::Read { .. })
            })
            .ok_or_else(invalid)?;
        }
        step(Some(call.point), &|kind| {
            matches!(kind, CfgPointKind::Call { .. })
        })
        .ok_or_else(invalid)?;
        step(Some(output), &|kind| {
            matches!(kind, CfgPointKind::ReturnOutput { .. })
        })
        .ok_or_else(invalid)?;
        step(None, &|kind| matches!(kind, CfgPointKind::ScopeExit)).ok_or_else(invalid)?;
        step(None, &|kind| matches!(kind, CfgPointKind::Return)).ok_or_else(invalid)?;
        if next.is_some() {
            return Err(invalid());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
