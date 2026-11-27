//! Inspect one direct authored comptime call and emit only its detached evaluator.
use super::{
    analyze_scalar_build, audited_closed_eager_binary, audited_closed_unary, derive_block,
    emit_closed_evaluator, prepare_scalar_project, span_of,
    ClosedComptimeFragment, ClosedComptimeRejection, ComptimeStdoutError, ScalarBlock,
    ScalarBlockItem, ScalarExpression, ScalarItem, ScalarPreparationInputs, ScalarTargetLayout,
    ScalarType, CLOSED_COMPTIME_MAX_SOURCE_BYTES,
};
use super::super::{
    control::{
        CallableBodyCallNodes, CallablePointFacts, InitializerFacts, ModuleInitializationStepKind,
        PlainNameIdentity, QualifiedInitializerStorage,
    },
    derive_scalar_declarations_with_embedding, ConcreteCallSelection, EmbeddedComptimeExpression,
    PreparedScalarProject, ResolvedCallTarget, ScalarBuildValidation, ScalarFunction,
};
use rowan::{ast::SyntaxNodePtr, GreenNode};
use wosy_syntax::{ByteSpan, CanonicalCstRoot, ComptimeCst, CstNode, FreshParseResult, SourceSpan, SyntaxKind, WosyLanguage};


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectComptimeArgument {
    I32(i32),
    U32(u32),
    Bool(bool),
}

/// A checked argument's authored literal origin. If the occurrence is a
/// read, its producer is the exact earlier bypass slot in the same plan.
/// Following producers through those slots preserves the full read chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartupArgumentOrigin {
    literal: SourceSpan,
    prior_bypass_slot: Option<usize>,
}

impl StartupArgumentOrigin {
    pub fn literal(&self) -> &SourceSpan { &self.literal }
    pub fn prior_bypass_slot(&self) -> Option<usize> { self.prior_bypass_slot }
}

/// The proven correspondence between one startup argument and the formal
/// its authored occurrence reads. Pair inputs use ordinary formal order.
#[derive(Debug, PartialEq, Eq)]
pub struct DirectComptimeInput {
    argument: SourceSpan,
    origin: StartupArgumentOrigin,
    formal: SourceSpan,
    read: SourceSpan,
    operand: FormalOperand,
    value: DirectComptimeArgument,
}

impl DirectComptimeInput {
    pub fn argument(&self) -> &SourceSpan { &self.argument }
    pub fn origin(&self) -> &StartupArgumentOrigin { &self.origin }
    pub fn formal(&self) -> &SourceSpan { &self.formal }
    pub fn read(&self) -> &SourceSpan { &self.read }
    pub fn value(&self) -> DirectComptimeArgument { self.value }
}

/// Authored, typed facts from one inspection, not emitted code, a selected
/// artifact or authority to execute a tool. No prepared AST escapes.
#[derive(Debug, PartialEq, Eq)]
pub struct InspectedDirectComptimeCall<const ARITY: usize = 1> {
    site: SourceSpan,
    function: SourceSpan,
    call: SourceSpan,
    destination: SourceSpan,
    inputs: [DirectComptimeInput; ARITY],
    result: ScalarType,
}

impl<const ARITY: usize> InspectedDirectComptimeCall<ARITY> {
    pub fn site(&self) -> &SourceSpan { &self.site }
    pub fn function(&self) -> &SourceSpan { &self.function }
    pub fn call(&self) -> &SourceSpan { &self.call }
    pub fn destination(&self) -> &SourceSpan { &self.destination }
    pub fn inputs(&self) -> &[DirectComptimeInput; ARITY] { &self.inputs }
    pub fn result(&self) -> &ScalarType { &self.result }

    fn tuple(&self) -> [DirectComptimeArgument; ARITY] {
        self.inputs.each_ref().map(DirectComptimeInput::value)
    }
}

impl InspectedDirectComptimeCall<1> {
    pub fn argument(&self) -> &SourceSpan { self.inputs[0].argument() }
    pub fn formal(&self) -> &SourceSpan { self.inputs[0].formal() }
    pub fn read(&self) -> &SourceSpan { self.inputs[0].read() }
    pub fn value(&self) -> DirectComptimeArgument { self.inputs[0].value() }
}

/// Inspect only one module containing a single ordinary function and one
/// binding that immediately calls it with a scalar literal. Its only
/// comptime tail reads the exact formal, alone or in one eager scalar operation.
/// The caller must establish the
/// module's real target-entry authority separately. No general proof,
/// profile lookup, execution, result substitution or cache identity.
pub fn inspect_direct_function_comptime(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout,
) -> Result<InspectedDirectComptimeCall, ClosedComptimeRejection> {
    with_verified_direct_call(fresh, id, layout, |facts, _, _| Ok(facts))
}

/// Exact typed facts and the detached evaluator for this one concrete input.
/// Not an executed result, selected profile, manifest, or cache key.
pub struct DirectFunctionComptimeFragment {
    facts: InspectedDirectComptimeCall,
    evaluator: ClosedComptimeFragment,
}

impl DirectFunctionComptimeFragment {
    pub fn facts(&self) -> &InspectedDirectComptimeCall { &self.facts }
    pub fn evaluator(&self) -> &ClosedComptimeFragment { &self.evaluator }

    pub fn into_parts(self) -> (InspectedDirectComptimeCall, ClosedComptimeFragment) {
        (self.facts, self.evaluator)
    }
}

/// Derive evaluator LLVM from the compiler-observed concrete argument. No
/// API accepts a value or allows an inspected fact from another source to
/// be applied here. Neither the analytical project nor the caller is emitted.
pub fn derive_direct_function_comptime_fragment(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout,
) -> Result<DirectFunctionComptimeFragment, ClosedComptimeRejection> {
    with_verified_direct_call(fresh, id, layout, |facts, body, name_span| {
        emit_bound_evaluator(id, layout, body, name_span, &facts, None)
            .map(|evaluator| DirectFunctionComptimeFragment { facts, evaluator })
    })
}

/// An exact canonical startup occurrence and its index into this plan's
/// evaluator list. The index has no meaning outside its owning plan.
#[derive(Debug)]
pub struct DirectStartupComptimeUse<const ARITY: usize = 1> {
    facts: InspectedDirectComptimeCall<ARITY>,
    evaluator: usize,
}

impl<const ARITY: usize> DirectStartupComptimeUse<ARITY> {
    pub fn facts(&self) -> &InspectedDirectComptimeCall<ARITY> { &self.facts }
    pub fn evaluator_index(&self) -> usize { self.evaluator }
}

/// Detached evaluator IR for a bounded startup list in one authored module.
/// Call sites retain their order and provenance. Nothing runs or persists.
pub struct DirectStartupComptimePlan<const ARITY: usize = 1> {
    calls: Vec<DirectStartupComptimeUse<ARITY>>,
    evaluators: Vec<ClosedComptimeFragment>,
}

impl<const ARITY: usize> DirectStartupComptimePlan<ARITY> {
    pub fn calls(&self) -> &[DirectStartupComptimeUse<ARITY>] { &self.calls }
    pub fn evaluators(&self) -> &[ClosedComptimeFragment] { &self.evaluators }

    pub fn into_parts(self) -> (Vec<DirectStartupComptimeUse<ARITY>>, Vec<ClosedComptimeFragment>) {
        (self.calls, self.evaluators)
    }
}

/// One eager scalar tail and direct literal startup calls to
/// that same declaration. Authenticate the entire ordinary startup list
/// before emitting anything. Equal tuples share one evaluator only within
/// this exact analysis; this does not define a reusable ComptimeKey.
pub fn derive_direct_startup_comptime_plan(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout,
) -> Result<DirectStartupComptimePlan, ClosedComptimeRejection> {
    derive_direct_startup::<1>(fresh, id, layout)
}

/// The same bounded startup proof for two literal arguments and a single
/// eager binary expression reading each exact formal once. The formals and
/// result share an admitted scalar type. Deduplication is on the ordered
/// typed pair, not equivalent IR or either individual input.
pub fn derive_direct_pair_startup_comptime_plan(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout,
) -> Result<DirectStartupComptimePlan<2>, ClosedComptimeRejection> {
    derive_direct_startup::<2>(fresh, id, layout)
}

/// An exact local startup hop into the comptime function. For each formal of
/// that function, the mapping names the outer startup argument that reaches it.
#[derive(Debug)]
pub struct PairStartupForwarding {
    declaration: SourceSpan,
    tail_call: SourceSpan,
    startup_argument_positions: [usize; 2],
}

impl PairStartupForwarding {
    pub fn declaration(&self) -> &SourceSpan { &self.declaration }
    pub fn tail_call(&self) -> &SourceSpan { &self.tail_call }
    pub fn startup_argument_positions(&self) -> [usize; 2] { self.startup_argument_positions }
}

/// Every startup call directly selects the same ordinary forwarder; the
/// authenticated tail hop leads to the exact comptime declaration. The
/// evaluator slots use comptime formal order, not the forwarder's order.
pub struct ForwardedPairStartupComptimePlan {
    forwarding: PairStartupForwarding,
    plan: DirectStartupComptimePlan<2>,
}

impl ForwardedPairStartupComptimePlan {
    pub fn forwarding(&self) -> &PairStartupForwarding { &self.forwarding }
    pub fn calls(&self) -> &[DirectStartupComptimeUse<2>] { self.plan.calls() }
    pub fn evaluators(&self) -> &[ClosedComptimeFragment] { self.plan.evaluators() }

    pub fn into_parts(self) -> (PairStartupForwarding, DirectStartupComptimePlan<2>) {
        (self.forwarding, self.plan)
    }
}

/// Project only one unconditional, complete tail call that forwards each
/// typed formal exactly once, possibly reordered. No result, runner, startup
/// emission, arbitrary call-graph proof or profile/builder authority. The
/// caller must separately authenticate the module as the selected target entry.
pub fn derive_forwarded_pair_startup_comptime_plan(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout,
) -> Result<ForwardedPairStartupComptimePlan, ClosedComptimeRejection> {
    with_verified_startup_calls::<2, _>(fresh, id, layout, false,
        StartupEntry::OneForwarder, |calls, body, name_span, forwarding| {
            let VerifiedStartupPaths::OneForwarder(forwarding) = forwarding else {
                return Err(ClosedComptimeRejection::Emitter("verified tail hop disappeared".into()));
            };
            let plan = emit_startup_plan(id, layout, body, name_span, calls, None)?;
            Ok(ForwardedPairStartupComptimePlan { forwarding, plan })
        })
}

/// An authored read of one exact checked formal at a tail. On a call edge,
/// the array order is the callee's formal order; this position names the
/// caller's formal.
#[derive(Debug)]
pub struct PairStartupHopInput {
    caller_position: usize,
    caller_formal: SourceSpan,
    read: SourceSpan,
}

impl PairStartupHopInput {
    pub fn caller_position(&self) -> usize { self.caller_position }
    pub fn caller_formal(&self) -> &SourceSpan { &self.caller_formal }
    pub fn read(&self) -> &SourceSpan { &self.read }
}

/// One checked, complete tail edge between exact local declarations.
#[derive(Debug)]
pub struct PairStartupCallHop {
    caller: SourceSpan,
    callee: SourceSpan,
    tail_call: SourceSpan,
    inputs: [PairStartupHopInput; 2],
}

impl PairStartupCallHop {
    pub fn caller(&self) -> &SourceSpan { &self.caller }
    pub fn callee(&self) -> &SourceSpan { &self.callee }
    pub fn tail_call(&self) -> &SourceSpan { &self.tail_call }
    pub fn inputs(&self) -> &[PairStartupHopInput; 2] { &self.inputs }

    fn positions(&self) -> [usize; 2] { self.inputs.each_ref().map(PairStartupHopInput::caller_position) }
}

/// Independently verified branch tail: either a forwarding edge in this
/// plan's hop table or an ordinary direct return of one exact data formal.
#[derive(Debug)]
pub enum PairStartupBranch {
    Forward(usize),
    ReturnFormal(PairStartupHopInput),
}

impl PairStartupBranch {
    pub fn hop_index(&self) -> Option<usize> {
        match self { Self::Forward(index) => Some(*index), Self::ReturnFormal(_) => None }
    }
}

/// Authored selector and both independently checked alternatives.
#[derive(Debug)]
pub struct PairStartupSelector {
    declaration: SourceSpan,
    formal: SourceSpan,
    condition: SourceSpan,
    when_true: PairStartupBranch,
    when_false: PairStartupBranch,
}

impl PairStartupSelector {
    pub fn declaration(&self) -> &SourceSpan { &self.declaration }
    pub fn formal(&self) -> &SourceSpan { &self.formal }
    pub fn condition(&self) -> &SourceSpan { &self.condition }
    pub fn when_true(&self) -> &PairStartupBranch { &self.when_true }
    pub fn when_false(&self) -> &PairStartupBranch { &self.when_false }
}

/// The checked Boolean argument that selected an actual branch. It is an
/// authored literal or a read of an earlier proven ordinary bypass.
#[derive(Clone, Debug)]
pub struct PairStartupBranchSelection {
    selector: usize,
    argument: SourceSpan,
    origin: StartupArgumentOrigin,
    value: bool,
}

impl PairStartupBranchSelection {
    pub fn selector_index(&self) -> usize { self.selector }
    pub fn argument(&self) -> &SourceSpan { &self.argument }
    pub fn literal(&self) -> &SourceSpan { self.origin.literal() }
    pub fn origin(&self) -> &StartupArgumentOrigin { &self.origin }
    pub fn value(&self) -> bool { self.value }
}

/// Path from one ordinary startup call into the selected comptime
/// declaration. Hop indices address the owning plan's authored edge table.
#[derive(Clone, Debug)]
pub struct PairStartupCallPath {
    hops: std::sync::Arc<[usize]>,
    positions: [usize; 2],
    branch: Option<PairStartupBranchSelection>,
}

impl PairStartupCallPath {
    pub fn hop_indices(&self) -> &[usize] { &self.hops }
    pub fn startup_argument_positions(&self) -> [usize; 2] { self.positions }
    pub fn branch_selection(&self) -> Option<&PairStartupBranchSelection> { self.branch.as_ref() }
}

/// An ordinary startup slot proven not to reach the comptime site. The
/// original call still returns its selected data argument to its checked
/// destination. These are authored facts, not emitted or substituted code.
#[derive(Debug)]
pub struct PairStartupBypass {
    call: SourceSpan,
    destination: SourceSpan,
    branch: PairStartupBranchSelection,
    returned: CheckedStartupArgument,
}

impl PairStartupBypass {
    pub fn call(&self) -> &SourceSpan { &self.call }
    pub fn destination(&self) -> &SourceSpan { &self.destination }
    pub fn branch_selection(&self) -> &PairStartupBranchSelection { &self.branch }
    pub fn returned_argument(&self) -> &SourceSpan { &self.returned.occurrence }
    pub fn returned_origin(&self) -> &StartupArgumentOrigin { &self.returned.origin }
    pub fn returned_value(&self) -> DirectComptimeArgument { self.returned.value }
}

/// One startup occurrence in the ordinary initialization order. A bypass
/// has no evaluator or evaluator index.
#[derive(Debug)]
pub enum PairStartupOccurrence<'a> {
    Specialization(&'a DirectStartupComptimeUse<2>, &'a PairStartupCallPath),
    Bypass(&'a PairStartupBypass),
}

#[derive(Debug)]
enum PairStartupOrder {
    Specialization(usize),
    Bypass(PairStartupBypass),
}

/// Typed startup proof without retained evaluator modules or result authority.
#[derive(Debug)]
pub struct PairStartupCallGraphEvidence {
    site_id: String,
    site: SourceSpan,
    layout: ScalarTargetLayout,
    calls: Vec<DirectStartupComptimeUse<2>>,
    hops: Vec<PairStartupCallHop>,
    selectors: Vec<PairStartupSelector>,
    paths: Vec<PairStartupCallPath>,
    order: Vec<PairStartupOrder>,
}

impl PairStartupCallGraphEvidence {
    pub fn site_id(&self) -> &str { &self.site_id }

    /// The authored site still exists when no startup occurrence reaches it.
    pub fn site(&self) -> &SourceSpan { &self.site }
    pub fn layout(&self) -> ScalarTargetLayout { self.layout }
    pub fn hops(&self) -> &[PairStartupCallHop] { &self.hops }
    pub fn selectors(&self) -> &[PairStartupSelector] { &self.selectors }
    /// Only calls that actually reach the comptime site.
    pub fn calls(&self) -> impl ExactSizeIterator<Item = (&DirectStartupComptimeUse<2>, &PairStartupCallPath)> {
        self.calls.iter().zip(&self.paths)
    }
    /// All ordinary startup slots, including proven bypasses, in order.
    pub fn occurrences(&self) -> impl ExactSizeIterator<Item = PairStartupOccurrence<'_>> {
        self.order.iter().map(|slot| match slot {
            PairStartupOrder::Specialization(index) =>
                PairStartupOccurrence::Specialization(&self.calls[*index], &self.paths[*index]),
            PairStartupOrder::Bypass(slot) => PairStartupOccurrence::Bypass(slot),
        })
    }
}

pub struct PairStartupCallGraphComptimePlan {
    evidence: PairStartupCallGraphEvidence,
    evaluators: Vec<ClosedComptimeFragment>,
    wasip1_stdout: Option<(usize, RetainedPairOrdinaryCst)>,
}

/// Same bound graph, exact compiler-produced P1 IR and resolved result type
/// at each evaluator index. Construction consumes the compiler's own plan:
/// neither loose adapter inputs nor previously detached IR can become P1.
#[derive(Debug)]
pub struct BoundedPairStartupWasip1Output {
    evidence: PairStartupCallGraphEvidence,
    evaluators: Vec<BoundedStartupWasip1Evaluator>,
    ordinary: RetainedPairOrdinaryCst,
    ir_bytes: usize,
    max_stdout: usize,
}

type AuthoredNode = SyntaxNodePtr<WosyLanguage>;

#[derive(Debug)]
struct RetainedPairOrdinaryCst {
    green: GreenNode,
    site: AuthoredNode,
    callable_tail: AuthoredNode,
    initializers: Vec<AuthoredNode>,
    source_bytes: usize,
}

/// A normal canonical CST view of this exact compiler-owned continuation.
/// Opening the view reconstructs only Rowan's red root; it does not parse,
/// type, replace occurrences, compile, apply results or publish anything.
pub struct PairOrdinaryContinuation<'plan> {
    canonical: CanonicalCstRoot,
    retained: &'plan RetainedPairOrdinaryCst,
    evidence: &'plan PairStartupCallGraphEvidence,
}

impl PairOrdinaryContinuation<'_> {
    pub fn canonical_cst(&self) -> &CanonicalCstRoot { &self.canonical }
    pub fn comptime_site(&self) -> CstNode { self.retained.site.to_node(&self.canonical.root) }
    pub fn callable_tail(&self) -> CstNode { self.retained.callable_tail.to_node(&self.canonical.root) }

    /// Exact initializer addresses with the existing authenticated cases,
    /// in the same ordinary initialization order. Bypasses have no evaluator index.
    pub fn initializers(&self) -> impl ExactSizeIterator<Item = (CstNode, PairStartupOccurrence<'_>)> {
        self.retained.initializers.iter().zip(self.evidence.occurrences()).map(|(node, case)| {
            (node.to_node(&self.canonical.root), case)
        })
    }
}

#[derive(Debug)]
pub struct BoundedStartupWasip1Evaluator {
    ir: std::sync::Arc<Vec<u8>>,
    result_type: ScalarType,
    output_span: ByteSpan,
}

/// All-or-nothing typed interpretation of observed terminal stdout. This
/// borrows the exact compiler plan; it carries no execution provenance.
pub struct ObservedPairStartupWasip1<'plan> {
    plan: &'plan BoundedPairStartupWasip1Output,
    values: Vec<ScalarExpression>,
}

impl ObservedPairStartupWasip1<'_> {
    pub fn plan(&self) -> &BoundedPairStartupWasip1Output { self.plan }

    pub fn occurrences(&self) -> impl ExactSizeIterator<Item = (PairStartupOccurrence<'_>, Option<&ScalarExpression>)> {
        self.plan.evidence.occurrences().map(|case| { let value = self.value(&case); (case, value) })
    }

    pub fn initializers(&self) -> impl ExactSizeIterator<Item = (CstNode, PairStartupOccurrence<'_>, Option<&ScalarExpression>)> {
        let canonical = self.plan.ordinary_continuation().canonical;
        self.plan.ordinary.initializers.iter().zip(self.occurrences())
            .map(move |(node, (case, value))| (node.to_node(&canonical.root), case, value))
    }

    fn value(&self, case: &PairStartupOccurrence<'_>) -> Option<&ScalarExpression> {
        match case {
            PairStartupOccurrence::Specialization(call, _) => Some(&self.values[call.evaluator_index()]),
            PairStartupOccurrence::Bypass(_) => None,
        }
    }
}

impl BoundedPairStartupWasip1Output {
    pub fn evidence(&self) -> &PairStartupCallGraphEvidence { &self.evidence }
    pub fn evaluators(&self) -> &[BoundedStartupWasip1Evaluator] { &self.evaluators }
    pub fn ir_bytes(&self) -> usize { self.ir_bytes }
    pub fn max_stdout(&self) -> usize { self.max_stdout }
    /// Logical authored bytes, not a Rowan heap/RSS measurement.
    pub fn authored_source_bytes(&self) -> usize { self.ordinary.source_bytes }

    /// Read exactly one bounded value per evaluator in compiler index order.
    /// The caller only supplies terminal bytes; it cannot select their type,
    /// target slot, CST occurrence, or claim that any executable ran.
    pub fn observe_stdout<'plan, 'bytes, T, E>(
        &'plan self, terminals: &'bytes [T],
        terminal_bytes: impl Fn(usize, &'bytes T) -> Result<&'bytes [u8], E>,
        count_error: impl FnOnce(usize, usize) -> E,
        output_error: impl Fn(usize, ComptimeStdoutError) -> E,
    ) -> Result<ObservedPairStartupWasip1<'plan>, E> {
        let expected = self.evaluators.len();
        let supplied = terminals.len();
        if supplied != expected { return Err(count_error(expected, supplied)); }
        let values = self.evaluators.iter().zip(terminals).enumerate().map(|(index, (evaluator, terminal))| {
            evaluator.parse_stdout(terminal_bytes(index, terminal)?, self.max_stdout).map_err(|error| output_error(index, error))
        }).collect::<Result<_, _>>()?;
        Ok(ObservedPairStartupWasip1 { plan: self, values })
    }

    pub fn ordinary_continuation(&self) -> PairOrdinaryContinuation<'_> {
        let canonical = CanonicalCstRoot::new(self.evidence.site().source.clone(),
            CstNode::new_root(self.ordinary.green.clone()));
        PairOrdinaryContinuation { canonical, retained: &self.ordinary, evidence: &self.evidence }
    }
}

impl BoundedStartupWasip1Evaluator {
    pub fn bytes(&self) -> &[u8] { self.ir.as_slice() }
    pub fn result_type(&self) -> &ScalarType { &self.result_type }

    pub(super) fn output_span(&self) -> ByteSpan { self.output_span }

    /// An additional read-only view of the already emitted text. The owning
    /// typed plan retains its allocation; modifying a new view cannot alter it.
    pub fn shared_ir(&self) -> std::sync::Arc<Vec<u8>> { self.ir.clone() }
}

impl PairStartupCallGraphComptimePlan {
    pub fn evidence(&self) -> &PairStartupCallGraphEvidence { &self.evidence }
    pub fn hops(&self) -> &[PairStartupCallHop] { self.evidence.hops() }
    pub fn selectors(&self) -> &[PairStartupSelector] { self.evidence.selectors() }
    pub fn calls(&self) -> impl ExactSizeIterator<Item = (&DirectStartupComptimeUse<2>, &PairStartupCallPath)> {
        self.evidence.calls()
    }
    pub fn occurrences(&self) -> impl ExactSizeIterator<Item = PairStartupOccurrence<'_>> {
        self.evidence.occurrences()
    }
    pub fn evaluators(&self) -> &[ClosedComptimeFragment] { &self.evaluators }

    pub fn into_parts(self) -> (PairStartupCallGraphEvidence, Vec<ClosedComptimeFragment>) {
        (self.evidence, self.evaluators)
    }

    /// Bound aggregate retained IR while serializing the ordinary emitted text.
    /// This is still a non-executable compiler projection, not readback.
    pub fn into_bounded_wasip1_output(self, max_ir_bytes: usize) -> std::io::Result<BoundedPairStartupWasip1Output> {
        let (max_stdout, ordinary) = self.wasip1_stdout.ok_or_else(|| std::io::Error::new(
            std::io::ErrorKind::Unsupported, "the startup graph did not emit WASIp1 stdout"))?;
        let mut remaining = max_ir_bytes;
        let mut evaluators = Vec::with_capacity(self.evaluators.len());
        for ClosedComptimeFragment { llvm, result_type, site, .. } in self.evaluators {
            let bytes = llvm.to_bounded_text_bytes(remaining)?;
            remaining -= bytes.len();
            evaluators.push(BoundedStartupWasip1Evaluator { ir: std::sync::Arc::new(bytes), result_type, output_span: site.range });
        }
        Ok(BoundedPairStartupWasip1Output {
            evidence: self.evidence, evaluators, ordinary, ir_bytes: max_ir_bytes - remaining, max_stdout,
        })
    }
}

/// Inspect the selected comptime function and safe local forwarders,
/// followed by ordinary startup bindings. A
/// binding may call the comptime function directly or enter any checked
/// acyclic forwarding path through complete tail calls. A startup may
/// enter a selector whose first formal is a proven Boolean at that call.
/// Each branch must forward the two data formals or return one directly.
/// An argument can only be a checked literal or an exact read of an
/// earlier ordinary bypass result. Those results add no specialization;
/// a comptime result can never supply another argument. Selectors are
/// not intermediate hops. Equal final tuples share one evaluator across
/// paths, not by equal IR.
/// The caller still authenticates the selected target entry. No execution,
/// durable key/result, arbitrary branch control, or cross-module claim.
pub fn derive_pair_startup_call_graph_comptime_plan(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout,
) -> Result<PairStartupCallGraphComptimePlan, ClosedComptimeRejection> {
    derive_pair_startup_graph_with_delivery(fresh, id, layout, None)
}

/// Route the existing checked bound scalar evaluator and its ordinary
/// output type into the incumbent WASI preview1 stdout emitter. No tools,
/// runtime, result, or permission to launch.
pub fn derive_pair_startup_call_graph_wasip1_stdout(
    fresh: &FreshParseResult, id: &str, max_stdout: usize,
) -> Result<PairStartupCallGraphComptimePlan, ClosedComptimeRejection> {
    derive_pair_startup_graph_with_delivery(fresh, id, ScalarTargetLayout::WASM32, Some(max_stdout))
}

fn derive_pair_startup_graph_with_delivery(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout, max_wasip1_stdout: Option<usize>,
) -> Result<PairStartupCallGraphComptimePlan, ClosedComptimeRejection> {
    with_verified_startup_calls::<2, _>(fresh, id, layout, false,
        StartupEntry::PairCallGraph { retain_cst: max_wasip1_stdout.is_some() }, |calls, body, name_span, proof| {
            let VerifiedStartupPaths::Graph { site, hops, selectors, paths, order, ordinary } = proof else {
                return Err(ClosedComptimeRejection::Emitter("verified call graph disappeared".into()));
            };
            let (calls, evaluators) = emit_startup_plan(id, layout, body, name_span, calls, max_wasip1_stdout)?.into_parts();
            let evidence = PairStartupCallGraphEvidence { site_id: id.to_owned(), site, layout, calls, hops, selectors, paths, order };
            let wasip1_stdout = max_wasip1_stdout.zip(ordinary);
            Ok(PairStartupCallGraphComptimePlan { evidence, evaluators, wasip1_stdout })
        })
}

fn derive_direct_startup<const ARITY: usize>(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout,
) -> Result<DirectStartupComptimePlan<ARITY>, ClosedComptimeRejection> {
    with_verified_startup_calls::<ARITY, _>(fresh, id, layout, false,
        StartupEntry::Direct, |calls, body, name_span, _| {
            emit_startup_plan(id, layout, body, name_span, calls, None)
        })
}

fn emit_startup_plan<const ARITY: usize>(
    id: &str, layout: ScalarTargetLayout, body: &ScalarBlock, name_span: ByteSpan,
    calls: Vec<InspectedDirectComptimeCall<ARITY>>, max_wasip1_stdout: Option<usize>,
) -> Result<DirectStartupComptimePlan<ARITY>, ClosedComptimeRejection> {
    let mut plan = DirectStartupComptimePlan {
        calls: Vec::with_capacity(calls.len()), evaluators: Vec::new(),
    };
    let mut values = Vec::new();
    for facts in calls {
        // Function, exact reads and layout are fixed in this analysis.
        let tuple = facts.tuple();
        let evaluator = match values.iter().position(|value| *value == tuple) {
            Some(index) => index,
            None => {
                let index = plan.evaluators.len();
                plan.evaluators.push(emit_bound_evaluator(id, layout, body, name_span, &facts, max_wasip1_stdout)?);
                values.push(tuple);
                index
            }
        };
        plan.calls.push(DirectStartupComptimeUse { facts, evaluator });
    }
    Ok(plan)
}

fn emit_bound_evaluator<const ARITY: usize>(
    id: &str, layout: ScalarTargetLayout, authored_body: &ScalarBlock, name_span: ByteSpan,
    facts: &InspectedDirectComptimeCall<ARITY>, max_wasip1_stdout: Option<usize>,
) -> Result<ClosedComptimeFragment, ClosedComptimeRejection> {
    let mut body = authored_body.clone();
    let ([ScalarBlockItem::Expression(item)], [expression], [output]) = (
        body.items.as_mut_slice(), body.expressions.as_mut_slice(), body.final_output_values.as_mut_slice(),
    ) else {
        return Err(ClosedComptimeRejection::Emitter("verified evaluator body lost its sole output".into()));
    };
    for input in facts.inputs() {
        let span = input.read.range;
        let value = match input.value {
            DirectComptimeArgument::I32(value) => ScalarExpression::Integer { value: value.into(), span },
            DirectComptimeArgument::U32(value) => ScalarExpression::Integer { value: value.into(), span },
            DirectComptimeArgument::Bool(value) => ScalarExpression::Boolean { value, span },
        };
        let Some(read) = input.operand.get_mut(item) else {
            return Err(ClosedComptimeRejection::Emitter("verified formal operand disappeared".into()));
        };
        if !matches!(read, ScalarExpression::Name { span: current, .. } if *current == span) {
            return Err(ClosedComptimeRejection::Emitter("verified formal operand changed identity".into()));
        }
        *read = value;
    }
    *expression = item.clone();
    output.value = item.clone();
    output.ty = facts.result.clone();
    emit_closed_evaluator(id, facts.site.clone(), name_span, layout, facts.result.clone(), Vec::new(), body, max_wasip1_stdout)
}

fn with_verified_direct_call<T>(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout,
    observe: impl FnOnce(InspectedDirectComptimeCall, &ScalarBlock, ByteSpan) -> Result<T, ClosedComptimeRejection>,
) -> Result<T, ClosedComptimeRejection> {
    with_verified_startup_calls::<1, _>(fresh, id, layout, true, StartupEntry::Direct, |mut calls, body, name_span, _| {
        let call = calls.pop().ok_or_else(|| ClosedComptimeRejection::Emitter("verified single call disappeared".into()))?;
        observe(call, body, name_span)
    })
}

#[derive(Clone, Copy)]
enum StartupEntry {
    Direct,
    OneForwarder,
    PairCallGraph { retain_cst: bool },
}

enum VerifiedStartupPaths {
    Direct,
    OneForwarder(PairStartupForwarding),
    Graph {
        site: SourceSpan,
        hops: Vec<PairStartupCallHop>,
        selectors: Vec<PairStartupSelector>,
        paths: Vec<PairStartupCallPath>,
        order: Vec<PairStartupOrder>,
        ordinary: Option<RetainedPairOrdinaryCst>,
    },
}

// The analytical project stays private. The continuation is called only
// after every edge, startup occurrence and exact formal read were verified.
fn with_verified_startup_calls<const ARITY: usize, T>(
    fresh: &FreshParseResult, id: &str, layout: ScalarTargetLayout, single: bool, entry: StartupEntry,
    observe: impl FnOnce(
        Vec<InspectedDirectComptimeCall<ARITY>>, &ScalarBlock, ByteSpan, VerifiedStartupPaths,
    ) -> Result<T, ClosedComptimeRejection>,
) -> Result<T, ClosedComptimeRejection> {
    use ClosedComptimeRejection as Error;
    let parsed = fresh.result();
    if !matches!(layout, ScalarTargetLayout::NATIVE64 | ScalarTargetLayout::WASM32) {
        return Err(Error::UnsupportedLayout);
    }
    if !parsed.is_valid() { return Err(Error::InvalidParse); }
    if parsed.text.len() > CLOSED_COMPTIME_MAX_SOURCE_BYTES { return Err(Error::SourceLimit); }
    let scope = |span| SourceSpan::new(parsed.source.clone(), span);
    let mut sites = parsed.root.descendants().filter_map(|node| {
        let site = ComptimeCst::from_node(&node)?;
        (site.id.text() == id).then_some((node, site))
    });
    let (node, site) = sites.next().ok_or(Error::SiteNotFound)?;
    if let Some((_, repeated)) = sites.next() { return Err(Error::RepeatedSite(scope(repeated.span))); }
    let unsupported = |feature| Error::Unsupported { site: scope(site.span), feature };
    let (tail, declaration_item, source) = function_tail(&node)
        .ok_or_else(|| unsupported("expected the only tail of one ordinary function"))?;
    let mut authored_items = source.children()
        .filter(|child| !matches!(child.kind(), SyntaxKind::BoundaryStart | SyntaxKind::BoundaryEnd)).peekable();
    if authored_items.next().as_ref() != Some(&declaration_item) {
        return Err(unsupported("expected the only tail of one ordinary function"));
    }
    let safe_forwarder = |declaration: &CstNode| {
        !declaration.children_with_tokens().any(|token| token.kind() == SyntaxKind::Unsafe)
    };
    let mut forwarders = Vec::new();
    match entry {
        StartupEntry::Direct => {}
        StartupEntry::OneForwarder => {
            let declaration = authored_items.next().and_then(|item| only(item.children()))
                .filter(|node| node.kind() == SyntaxKind::FunctionDecl).filter(safe_forwarder)
                .ok_or_else(|| unsupported("expected one safe authored forwarder before startup bindings"))?;
            forwarders.push(wosy_syntax::byte_span(&declaration));
        }
        StartupEntry::PairCallGraph { .. } => {
            while let Some(declaration) = authored_items.peek().and_then(|item| only(item.children()))
                .filter(|node| node.kind() == SyntaxKind::FunctionDecl)
            {
                if !safe_forwarder(&declaration) {
                    return Err(unsupported("comptime startup requires safe authored forwarding declarations"));
                }
                forwarders.push(wosy_syntax::byte_span(&declaration));
                authored_items.next();
            }
        }
    }
    let mut count = 0;
    let mut cst_initializers = matches!(entry, StartupEntry::PairCallGraph { retain_cst: true }).then(Vec::new);
    for item in authored_items {
        let Some(binding) = only(item.children()).filter(|child| child.kind() == SyntaxKind::BindingDecl) else {
            return Err(unsupported("each authored startup item must be one ordinary binding"));
        };
        count += 1;
        if single && count > 1 {
            return Err(unsupported("single-call projection requires exactly one startup initializer"));
        }
        if let Some(cst_initializers) = &mut cst_initializers {
            let initializer = binding.children().find(|node| node.kind() == SyntaxKind::OutputList)
                .and_then(|outputs| only(outputs.children().filter(|node| node.kind() == SyntaxKind::Expression)))
                .ok_or_else(|| unsupported("ordinary continuation requires one authored initializer expression"))?;
            cst_initializers.push(SyntaxNodePtr::new(&initializer));
        }
    }
    if count == 0 { return Err(unsupported("no direct startup call was authored")); }
    sole_final_value(&site.block)
        .filter(primitive_operand_source)
        .ok_or_else(|| unsupported("comptime body must only read its formal"))?;
    let authored_body = derive_block(&site.block);
    let [ScalarBlockItem::Expression(authored)] = authored_body.items.as_slice() else {
        return Err(unsupported("comptime body must only read its formal"));
    };
    let mut authored_reads = Vec::with_capacity(ARITY);
    match ARITY {
        1 => {
            let (operand, read) = FormalOperand::select(authored)
                .ok_or_else(|| unsupported(match authored {
                    ScalarExpression::Unary { .. } | ScalarExpression::Binary { .. } =>
                        "comptime accepts one audited eager scalar operator with one formal read",
                    _ => "comptime body must only read its formal",
                }))?;
            authored_reads.push((operand, span_of(read)));
        }
        2 => {
            let ScalarExpression::Binary { operator, left, right, .. } = authored else {
                return Err(unsupported("paired comptime requires an audited binary read of both formals"));
            };
            if !audited_closed_eager_binary(operator) || !is_name(left) || !is_name(right) {
                return Err(unsupported("paired comptime requires an audited binary read of both formals"));
            }
            authored_reads.extend([(FormalOperand::Left, span_of(left)), (FormalOperand::Right, span_of(right))]);
        }
        _ => return Err(unsupported("unsupported comptime literal tuple arity")),
    }

    // The ordinary contextual typer visits the actual authored scalar
    // block. This is not a substitute literal or an executable lowering:
    // the analytical project stays inside this function, and is never emitted.
    let block = ScalarExpression::Block(authored_body);
    let declarations = derive_scalar_declarations_with_embedding(parsed.canonical_cst(), Some(
        EmbeddedComptimeExpression { site: &node, expression: &tail, value: &block },
    ));
    let prepared = prepare_scalar_project(
        vec![(declarations, Vec::new())], vec![parsed.source.clone()],
        ScalarPreparationInputs { target_layout: layout },
    ).map_err(|error| Error::OrdinaryDiagnostics(error.into_ordered_diagnostics()))?;
    let [module] = prepared.project().modules.as_slice() else { unreachable!("one supplied module"); };
    let [ScalarItem::Function(function), rest @ ..] = module.items.as_slice() else {
        return Err(unsupported("expected one ordinary function before startup"));
    };
    let Some(region) = prepared.initialization_region(0) else {
        return Err(unsupported("ordinary entry initialization was not prepared"));
    };
    let Some((wrappers, bindings)) = rest.split_at_checked(forwarders.len()) else {
        return Err(unsupported("ordinary entry does not contain the authored forwarder"));
    };
    let mut declarations = Vec::with_capacity(1 + wrappers.len());
    declarations.push(scope(function.name_span));
    for (item, span) in wrappers.iter().zip(&forwarders) {
        let ScalarItem::Function(wrapper) = item else {
            return Err(unsupported("ordinary entry does not contain the authored forwarder"));
        };
        if wrapper.span != *span {
            return Err(unsupported("ordinary entry does not contain the authored forwarder"));
        }
        declarations.push(scope(wrapper.name_span));
    }
    let first_binding = declarations.len();
    if bindings.len() != count || region.steps.len() != count
        || region.steps.iter().enumerate().any(|(offset, step)| step.item != offset + first_binding
            || step.kind != ModuleInitializationStepKind::BindingInitialization)
    {
        return Err(unsupported("ordinary entry initialization order does not match the authored list"));
    }

    let build = analyze_scalar_build(&prepared);
    if !build.diagnostics().is_empty() { return Err(Error::OrdinaryDiagnostics(build.diagnostics().to_vec())); }
    let [ScalarBlockItem::Expression(ScalarExpression::Block(body))] = function.body.items.as_slice() else {
        return Err(unsupported("ordinary callable tail does not match the authored block"));
    };
    let [ScalarBlockItem::Expression(typed)] = body.items.as_slice() else {
        return Err(unsupported("ordinary block does not contain one scalar result"));
    };
    let points = build.callable_point_facts(0, 0, None)
        .ok_or_else(|| unsupported("ordinary parameter binding facts are unavailable"))?;
    let nodes = prepared.bind_callable_body_call_nodes(0, 0, None, &function.body)
        .map_err(|_| unsupported("ordinary callable occurrences are unavailable"))?;
    let result_type = nodes.checked_expression_type(typed)
        .ok_or_else(|| unsupported("ordinary comptime scalar result type is unavailable"))?;
    let mut reads = Vec::with_capacity(ARITY);
    for (operand, read_span) in authored_reads {
        let read = operand.get(typed)
            .filter(|value| matches!(value, ScalarExpression::Name { span, .. } if *span == read_span))
            .ok_or_else(|| unsupported("ordinary block does not contain the authored formal read"))?;
        let Some(PlainNameIdentity::Lexical(binding)) = nodes.plain_name(read) else {
            return Err(unsupported("authored read does not select the exact ordinary formal"));
        };
        if nodes.checked_expression_type(read) != Some(result_type) {
            return Err(unsupported("comptime reads and result require one exact scalar type"));
        }
        reads.push((operand, read_span, binding));
    }
    let mut formal_reads = Vec::with_capacity(ARITY);
    for ordinal in 0..ARITY {
        let (formal, formal_type) = nodes.formal(points, ordinal)
            .ok_or_else(|| unsupported("no ordinary formal binding was prepared"))?;
        if formal_type != result_type {
            return Err(unsupported("comptime reads and result require one exact scalar type"));
        }
        let index = reads.iter().position(|(_, _, read)| *read == formal)
            .ok_or_else(|| unsupported("authored read does not select the exact ordinary formal"))?;
        let (operand, read, _) = reads.swap_remove(index);
        formal_reads.push((operand, read, formal.declaration_span));
    }

    let facts = prepared.initializer_facts(0)
        .map_err(|_| unsupported("ordinary initializer occurrence facts are unavailable"))?;
    let mut hops = Vec::with_capacity(wrappers.len());
    let mut selectors = Vec::new();
    let mut graph = vec![PairGraphNode::Terminal];
    for (offset, item) in wrappers.iter().enumerate() {
        let ScalarItem::Function(wrapper) = item else { unreachable!("checked ordinary forwarding prefix"); };
        let index = offset + 1;
        let checked = check_pair_forwarding(
            &prepared, &build, wrapper, index, &declarations, entry, result_type, &scope, &unsupported,
        )?;
        match checked {
            CheckedPairForwarder::Plain(hop) => {
                graph.push(PairGraphNode::Plain(hops.len()));
                hops.push(hop);
            }
            CheckedPairForwarder::Select { formal, condition, when_true, when_false } => {
                graph.push(PairGraphNode::Selector(selectors.len()));
                let mut register = |branch| match branch {
                    CheckedPairAlternative::Forward(hop) => {
                        let index = hops.len();
                        hops.push(hop);
                        PairStartupBranch::Forward(index)
                    }
                    CheckedPairAlternative::ReturnFormal(formal) => PairStartupBranch::ReturnFormal(formal),
                };
                selectors.push(PairStartupSelector {
                    declaration: declarations[index].clone(), formal, condition,
                    when_true: register(when_true), when_false: register(when_false),
                });
            }
        }
    }
    let routes = if matches!(entry, StartupEntry::PairCallGraph { .. }) {
        verified_pair_paths(&graph, &hops, &selectors, &unsupported)?
    } else { Vec::new() };
    let mut materialized = std::collections::HashMap::new();
    let mut selected = Vec::with_capacity(count);
    let mut paths = Vec::new();
    let mut order = Vec::new();
    for item in bindings {
        let ScalarItem::Binding(binding) = item else {
            return Err(unsupported("expected one argument binding at each startup step"));
        };
        if binding.receivers.len() != 1 {
            return Err(unsupported("expected one direct initializer destination"));
        }
        let call = &binding.value;
        let ScalarExpression::Call { receiver: None, type_arguments, arguments, .. } = call else {
            return Err(unsupported("expected a direct ordinary initializer call"));
        };
        if !matches!(entry, StartupEntry::PairCallGraph { .. }) && arguments.len() != ARITY {
            return Err(unsupported("the exact literal tuple arity is required"));
        }
        if !type_arguments.is_empty() { return Err(unsupported("generic arguments are not admitted")); }
        let destination = facts.binding_output(binding, 0, call)
            .map_err(|_| unsupported("call is not the entire ordinary initializer"))?;
        let target = facts.checked_direct_call(call)
            .map_err(|_| unsupported("no exact direct declaration was selected"))?;
        let selected_declaration = declarations.iter().position(|declaration| declaration == target.declaration);
        let chosen = match (entry, selected_declaration) {
            (StartupEntry::Direct, Some(0)) => 0,
            (StartupEntry::OneForwarder, Some(1)) => 1,
            (StartupEntry::PairCallGraph { .. }, Some(index)) => index,
            _ => return Err(unsupported("initializer did not select the authored function")),
        };
        if target.concrete != &ConcreteCallSelection::Declaration {
            return Err(unsupported("initializer did not select the authored function"));
        }
        let route = routes.get(chosen);
        let expected_arguments = if matches!(route, Some(ResolvedPairRoute::Select { .. })) { 3 } else { ARITY };
        let [result] = target.outputs else {
            return Err(unsupported("one contextual scalar result is required"));
        };
        if target.parameters.len() != expected_arguments || destination.ty != &result.ty || &result.ty != result_type {
            return Err(unsupported("the declared result must match the exact read formals' scalar type"));
        }
        let startup = StartupArgumentFacts {
            initializers: &facts,
            prior: matches!(entry, StartupEntry::PairCallGraph { .. }).then_some(order.as_slice()),
            scope: &scope,
            unsupported: &unsupported,
        };
        let data_shape = if startup.prior.is_some() {
            "startup argument must be a checked literal or earlier ordinary bypass read"
        } else {
            "argument must be an exact scalar literal"
        };
        let path = match route {
            Some(ResolvedPairRoute::Select { selector, when_true, when_false }) => {
                let [argument, left, right] = arguments.as_slice() else {
                    return Err(unsupported("conditional startup requires a checked selector and two data arguments"));
                };
                const SELECTOR_SHAPE: &str = "conditional startup requires a checked Boolean literal or earlier ordinary bypass read";
                if target.parameters[0] != ScalarType::Bool { return Err(unsupported(SELECTOR_SHAPE)); }
                let observed = startup.check(argument, &ScalarType::Bool, SELECTOR_SHAPE)?;
                let DirectComptimeArgument::Bool(value) = observed.value else {
                    return Err(unsupported(SELECTOR_SHAPE));
                };
                let choice = PairStartupBranchSelection {
                    selector: *selector, argument: observed.occurrence, origin: observed.origin, value,
                };
                match if value { when_true } else { when_false } {
                    ResolvedPairAlternative::Forward(path) => {
                        Some(path.materialize(&graph, &hops, &mut materialized, Some(choice)))
                    }
                    ResolvedPairAlternative::ReturnFormal(formal) => {
                        // The bypass excludes this comptime site, not the
                        // ordinary argument evaluations. Check both even when
                        // the tail only reads one of them.
                        if &target.parameters[1] != result_type || &target.parameters[2] != result_type {
                            return Err(unsupported("argument does not have the selected formal type"));
                        }
                        let left = startup.check(left, result_type, data_shape)?;
                        let right = startup.check(right, result_type, data_shape)?;
                        let returned = match formal.caller_position() {
                            1 => left,
                            2 => right,
                            _ => return Err(unsupported("conditional branch did not preserve its exact data argument")),
                        };
                        order.push(PairStartupOrder::Bypass(PairStartupBypass {
                            call: scope(span_of(call)), destination: destination.destination,
                            branch: choice, returned,
                        }));
                        continue;
                    }
                }
            }
            Some(ResolvedPairRoute::Plain(path)) => {
                if arguments.len() != ARITY { return Err(unsupported("the exact literal tuple arity is required")); }
                Some(path.materialize(&graph, &hops, &mut materialized, None))
            }
            None => None,
        };
        let positions = if let Some(path) = &path {
            Some(path.positions)
        } else if chosen == 1 {
            Some(hops[0].edge.positions())
        } else { None };
        let argument_positions: [usize; ARITY] = if let Some(positions) = positions {
            positions.as_slice().try_into().map_err(|_| unsupported("forwarding requires the exact pair arity"))?
        } else { std::array::from_fn(|ordinal| ordinal) };
        let mut inputs = Vec::with_capacity(ARITY);
        for (argument_position, &(operand, read, formal)) in argument_positions.into_iter().zip(&formal_reads) {
            let argument = &arguments[argument_position];
            let parameter = &target.parameters[argument_position];
            if parameter != result_type {
                return Err(unsupported("argument does not have the selected formal type"));
            }
            let observed = startup.check(argument, parameter, data_shape)?;
            inputs.push(DirectComptimeInput {
                argument: observed.occurrence, origin: observed.origin,
                formal: scope(formal), read: scope(read), operand, value: observed.value,
            });
        }
        let inputs = inputs.try_into()
            .map_err(|_| unsupported("ordinary literal tuple did not preserve every formal"))?;
        if let Some(path) = path {
            paths.push(path);
            order.push(PairStartupOrder::Specialization(selected.len()));
        }
        selected.push(InspectedDirectComptimeCall {
            site: scope(site.span), function: declarations[0].clone(), call: scope(span_of(call)),
            destination: destination.destination, inputs, result: result.ty.clone(),
        });
    }
    let proof = match entry {
        StartupEntry::Direct => VerifiedStartupPaths::Direct,
        StartupEntry::OneForwarder => {
            let edge = hops.pop().expect("one checked authored forwarding declaration").edge;
            let startup_argument_positions = edge.positions();
            VerifiedStartupPaths::OneForwarder(PairStartupForwarding {
                declaration: edge.caller, tail_call: edge.tail_call, startup_argument_positions,
            })
        }
        StartupEntry::PairCallGraph { .. } => VerifiedStartupPaths::Graph {
            site: scope(site.span), hops: hops.into_iter().map(|hop| hop.edge).collect(), selectors, paths, order,
            ordinary: cst_initializers.map(|initializers| RetainedPairOrdinaryCst {
                green: parsed.canonical_cst().root.green().into_owned(),
                site: SyntaxNodePtr::new(&node), callable_tail: SyntaxNodePtr::new(&tail),
                initializers, source_bytes: parsed.text.len(),
            }),
        },
    };
    observe(selected, body, super::super::token_span(&site.id), proof)
}

#[derive(Debug)]
struct CheckedStartupArgument {
    occurrence: SourceSpan,
    origin: StartupArgumentOrigin,
    value: DirectComptimeArgument,
}

struct StartupArgumentFacts<'a> {
    initializers: &'a InitializerFacts<'a>,
    prior: Option<&'a [PairStartupOrder]>,
    scope: &'a dyn Fn(ByteSpan) -> SourceSpan,
    unsupported: &'a dyn Fn(&'static str) -> ClosedComptimeRejection,
}

impl StartupArgumentFacts<'_> {
    fn check(
        &self, argument: &ScalarExpression, parameter: &ScalarType, shape_error: &'static str,
    ) -> Result<CheckedStartupArgument, ClosedComptimeRejection> {
        let unsupported = self.unsupported;
        let argument_ty = self.initializers.checked_expression_type(argument)
            .map_err(|_| unsupported("ordinary argument type is unavailable"))?;
        if argument_ty != parameter {
            return Err(unsupported("argument does not have the selected formal type"));
        }
        let occurrence = (self.scope)(span_of(argument));
        let literal = match (argument, parameter) {
            (ScalarExpression::Integer { value, .. }, ScalarType::I32) =>
                i32::try_from(value).ok().map(DirectComptimeArgument::I32),
            (ScalarExpression::Integer { value, .. }, ScalarType::U32) =>
                u32::try_from(value).ok().map(DirectComptimeArgument::U32),
            (ScalarExpression::Boolean { value, .. }, ScalarType::Bool) => Some(DirectComptimeArgument::Bool(*value)),
            _ => None,
        };
        if let Some(value) = literal {
            return Ok(CheckedStartupArgument {
                origin: StartupArgumentOrigin { literal: occurrence.clone(), prior_bypass_slot: None },
                occurrence, value,
            });
        }
        let Some(prior) = self.prior.filter(|_| is_name(argument)) else {
            return Err(unsupported(shape_error));
        };
        let no_producer = || unsupported("startup argument must read an exact earlier ordinary bypass destination");
        let Ok(QualifiedInitializerStorage::Module(declaration)) = self.initializers.plain_name(argument) else {
            return Err(no_producer());
        };
        // Prior holds only verified startup slots, in canonical initializer
        // order. A specialization, current/future slot, or another same-
        // spelled declaration cannot authorize a value.
        let (producer, bypass) = prior.iter().enumerate().find_map(|(index, slot)| {
            match slot {
                PairStartupOrder::Bypass(bypass) if bypass.destination() == &declaration => Some((index, bypass)),
                _ => None,
            }
        }).ok_or_else(no_producer)?;
        let value = bypass.returned_value();
        if !matches!((value, parameter),
            (DirectComptimeArgument::I32(_), ScalarType::I32) |
            (DirectComptimeArgument::U32(_), ScalarType::U32) |
            (DirectComptimeArgument::Bool(_), ScalarType::Bool))
        {
            return Err(no_producer());
        }
        Ok(CheckedStartupArgument {
            occurrence,
            origin: StartupArgumentOrigin {
                literal: bypass.returned_origin().literal().clone(), prior_bypass_slot: Some(producer),
            },
            value,
        })
    }
}

struct IndexedPairHop {
    target: usize,
    edge: PairStartupCallHop,
}

#[derive(Clone, Copy)]
enum PairGraphNode {
    Terminal,
    Plain(usize),
    Selector(usize),
}

enum CheckedPairForwarder {
    Plain(IndexedPairHop),
    Select {
        formal: SourceSpan, condition: SourceSpan, when_true: CheckedPairAlternative, when_false: CheckedPairAlternative,
    },
}

enum CheckedPairAlternative {
    Forward(IndexedPairHop),
    ReturnFormal(PairStartupHopInput),
}

enum ResolvedPairRoute<'a> {
    Plain(VerifiedPairSuffix),
    Select { selector: usize, when_true: ResolvedPairAlternative<'a>, when_false: ResolvedPairAlternative<'a> },
}

enum ResolvedPairAlternative<'a> {
    Forward(VerifiedPairSuffix),
    ReturnFormal(&'a PairStartupHopInput),
}

#[derive(Clone, Copy)]
struct VerifiedPairSuffix {
    first_edge: Option<usize>,
    positions: [usize; 2],
}

impl VerifiedPairSuffix {
    fn materialize(
        self,
        graph: &[PairGraphNode],
        edges: &[IndexedPairHop],
        paths: &mut std::collections::HashMap<Option<usize>, std::sync::Arc<[usize]>>,
        branch: Option<PairStartupBranchSelection>,
    ) -> PairStartupCallPath {
        let hops = paths.entry(self.first_edge).or_insert_with(|| {
            let mut hops = Vec::new();
            let mut current = self.first_edge;
            while let Some(edge) = current {
                hops.push(edge);
                current = match graph[edges[edge].target] {
                    PairGraphNode::Plain(next) => Some(next),
                    PairGraphNode::Terminal => None,
                    PairGraphNode::Selector(_) => unreachable!("verified suffix contains only pure hops"),
                };
            }
            hops.into()
        });
        PairStartupCallPath { hops: hops.clone(), positions: self.positions, branch }
    }
}

fn verified_pair_paths<'a>(
    graph: &[PairGraphNode], hops: &[IndexedPairHop], selectors: &'a [PairStartupSelector],
    unsupported: &impl Fn(&'static str) -> ClosedComptimeRejection,
) -> Result<Vec<ResolvedPairRoute<'a>>, ClosedComptimeRejection> {
    // Both branches are independently checked. A forwarding alternative
    // must reach the target through the memoized pure DAG; a direct return
    // stops there without interpreting arbitrary ordinary expressions.
    struct Proof<'a> {
        graph: &'a [PairGraphNode],
        hops: &'a [IndexedPairHop],
        complete: Vec<Option<VerifiedPairSuffix>>,
        active: Vec<bool>,
        unsupported: &'a dyn Fn(&'static str) -> ClosedComptimeRejection,
    }
    impl Proof<'_> {
        fn visit(&mut self, mut index: usize) -> Result<VerifiedPairSuffix, ClosedComptimeRejection> {
            // Walk original declarations iteratively; a route ends at a
            // completed suffix, an unsupported transfer, or the first repeated node.
            let mut pending = Vec::new();
            let mut suffix = loop {
                if let Some(path) = self.complete[index] { break path; }
                let PairGraphNode::Plain(edge) = self.graph[index] else {
                    return Err((self.unsupported)("conditional forwarder may only be selected by an ordinary startup initializer"));
                };
                if self.active[index] {
                    return Err(ClosedComptimeRejection::Unsupported {
                        site: self.hops[edge].edge.tail_call.clone(), feature: "comptime startup forwarding contains a cycle",
                    });
                }
                self.active[index] = true;
                pending.push((index, edge));
                index = self.hops[edge].target;
            };
            for (index, edge) in pending.into_iter().rev() {
                suffix = self.prepend(edge, suffix);
                self.active[index] = false;
                self.complete[index] = Some(suffix);
            }
            Ok(suffix)
        }

        fn alternative<'a>(&mut self, branch: &'a PairStartupBranch) -> Result<ResolvedPairAlternative<'a>, ClosedComptimeRejection> {
            match branch {
                PairStartupBranch::Forward(index) => self.through(*index).map(ResolvedPairAlternative::Forward),
                PairStartupBranch::ReturnFormal(formal) => Ok(ResolvedPairAlternative::ReturnFormal(formal)),
            }
        }

        fn through(&mut self, index: usize) -> Result<VerifiedPairSuffix, ClosedComptimeRejection> {
            let suffix = self.visit(self.hops[index].target)?;
            Ok(self.prepend(index, suffix))
        }

        fn prepend(&self, index: usize, suffix: VerifiedPairSuffix) -> VerifiedPairSuffix {
            let positions = self.hops[index].edge.positions();
            VerifiedPairSuffix {
                first_edge: Some(index),
                positions: suffix.positions.map(|position| positions[position]),
            }
        }
    }
    let mut proof = Proof {
        graph, hops, complete: vec![None; graph.len()], active: vec![false; graph.len()], unsupported,
    };
    proof.complete[0] = Some(VerifiedPairSuffix { first_edge: None, positions: [0, 1] });
    graph.iter().enumerate().map(|(index, node)| match *node {
        PairGraphNode::Selector(selector) => {
            let selection = &selectors[selector];
            Ok(ResolvedPairRoute::Select {
                selector, when_true: proof.alternative(selection.when_true())?, when_false: proof.alternative(selection.when_false())?,
            })
        }
        _ => proof.visit(index).map(ResolvedPairRoute::Plain),
    }).collect()
}

fn check_pair_forwarding(
    prepared: &PreparedScalarProject, build: &ScalarBuildValidation<'_>, wrapper: &ScalarFunction,
    index: usize, declarations: &[SourceSpan], entry: StartupEntry,
    result_type: &ScalarType, scope: &impl Fn(ByteSpan) -> SourceSpan,
    unsupported: &impl Fn(&'static str) -> ClosedComptimeRejection,
) -> Result<CheckedPairForwarder, ClosedComptimeRejection> {
    let [ScalarBlockItem::Expression(tail)] = wrapper.body.items.as_slice() else {
        return Err(unsupported("forwarder must contain only one unconditional direct call"));
    };
    let conditional = matches!(entry, StartupEntry::PairCallGraph { .. }) && matches!(tail, ScalarExpression::If { .. });
    if !conditional && !matches!(tail, ScalarExpression::Call { receiver: None, .. }) {
        return Err(unsupported("forwarder must contain only one unconditional direct call"));
    }
    if wrapper.parameters.len() != if conditional { 3 } else { 2 } {
        return Err(unsupported("forwarding requires the exact pair arity"));
    }
    // The item is the verified authored wrapper, not a name lookup.
    let nodes = prepared.bind_callable_body_call_nodes(0, index, None, &wrapper.body)
        .map_err(|_| unsupported("ordinary forwarding occurrences are unavailable"))?;
    let points = build.callable_point_facts(0, index, None)
        .ok_or_else(|| unsupported("ordinary forwarding formal facts are unavailable"))?;
    if nodes.final_output(tail) != Some(0) || nodes.evaluation_point(points, tail).is_none()
        || nodes.checked_expression_type(tail) != Some(result_type)
    {
        return Err(unsupported("forwarding call does not itself evaluate as the exact output"));
    }
    let facts = PairEdgeFacts { nodes, points, index, declarations, entry, result_type, scope, unsupported };
    if conditional {
        let ScalarExpression::If { condition, then_branch, else_branch, .. } = tail else {
            unreachable!("checked conditional tail");
        };
        let Some((formal, ScalarType::Bool)) = facts.nodes.formal(points, 0) else {
            return Err(unsupported("conditional forwarding requires its first checked Boolean formal"));
        };
        if facts.nodes.checked_expression_type(condition) != Some(&ScalarType::Bool)
            || !matches!(facts.nodes.plain_name(condition), Some(PlainNameIdentity::Lexical(id)) if id == formal)
        {
            return Err(unsupported("conditional forwarding must read its first checked Boolean formal directly"));
        }
        let branch = |block: &ScalarBlock| {
            let [ScalarBlockItem::Expression(call)] = block.items.as_slice() else {
                return Err(unsupported("each conditional branch must be one complete tail call"));
            };
            if !block.final_expressions().next().is_some_and(|(output, _)| std::ptr::eq(output, call))
                || facts.nodes.block_ordinal(block).is_none()
                || facts.nodes.evaluation_point(points, call).is_none()
                || facts.nodes.checked_expression_type(call) != Some(result_type)
            {
                return Err(unsupported("each conditional branch must be one complete tail call"));
            }
            match call {
                ScalarExpression::Call { .. } => facts.edge(call, [1, 2]).map(CheckedPairAlternative::Forward),
                output => facts.formal_read(output, [1, 2])
                    .map(CheckedPairAlternative::ReturnFormal)
                    .ok_or_else(|| unsupported("a nonforwarding branch must return one exact checked data formal")),
            }
        };
        return Ok(CheckedPairForwarder::Select {
            formal: scope(formal.declaration_span), condition: scope(span_of(condition)),
            when_true: branch(then_branch)?, when_false: branch(else_branch)?,
        });
    }
    facts.edge(tail, [0, 1]).map(CheckedPairForwarder::Plain)
}

struct PairEdgeFacts<'a> {
    nodes: CallableBodyCallNodes<'a>,
    points: &'a CallablePointFacts,
    index: usize,
    declarations: &'a [SourceSpan],
    entry: StartupEntry,
    result_type: &'a ScalarType,
    scope: &'a dyn Fn(ByteSpan) -> SourceSpan,
    unsupported: &'a dyn Fn(&'static str) -> ClosedComptimeRejection,
}

impl PairEdgeFacts<'_> {
    fn edge(&self, call: &ScalarExpression, data_formals: [usize; 2]) -> Result<IndexedPairHop, ClosedComptimeRejection> {
        let unsupported = self.unsupported;
        let ScalarExpression::Call { receiver: None, type_arguments, arguments, .. } = call else {
            return Err(unsupported("forwarder must contain only one unconditional direct call"));
        };
        let [left, right] = arguments.as_slice() else {
            return Err(unsupported("forwarding requires the exact pair arity"));
        };
        if !type_arguments.is_empty() { return Err(unsupported("forwarding requires the exact pair arity")); }
        let target = match self.nodes.target(call) {
            Ok(ResolvedCallTarget::ModuleCallable { source, declaration_span, concrete: ConcreteCallSelection::Declaration }) =>
                self.declarations.iter().position(|decl| source == &decl.source && declaration_span == &decl.range),
            _ => None,
        };
        let target = match (self.entry, target) {
            (StartupEntry::OneForwarder, Some(0)) => 0,
            (StartupEntry::PairCallGraph { .. }, Some(target)) => target,
            (StartupEntry::PairCallGraph { .. }, _) =>
                return Err(unsupported("forwarding call did not select a verified local declaration")),
            _ => return Err(unsupported("forwarding call did not select the authored comptime function")),
        };
        let (Some(left), Some(right)) = (self.formal_read(left, data_formals), self.formal_read(right, data_formals)) else {
            return Err(unsupported("forwarding must pass each exact formal without intervening computation"));
        };
        if left.caller_position == right.caller_position {
            return Err(unsupported("forwarding must pass each exact formal without intervening computation"));
        }
        Ok(IndexedPairHop {
            target,
            edge: PairStartupCallHop {
                caller: self.declarations[self.index].clone(), callee: self.declarations[target].clone(),
                tail_call: (self.scope)(span_of(call)), inputs: [left, right],
            },
        })
    }

    fn formal_read(&self, expression: &ScalarExpression, data_formals: [usize; 2]) -> Option<PairStartupHopInput> {
        let Some(PlainNameIdentity::Lexical(binding)) = self.nodes.plain_name(expression) else { return None; };
        if self.nodes.checked_expression_type(expression) != Some(self.result_type) { return None; }
        let caller_position = data_formals.into_iter().find(|ordinal| self.nodes.formal(self.points, *ordinal)
            .is_some_and(|(id, ty)| id == binding && ty == self.result_type))?;
        Some(PairStartupHopInput {
            caller_position, caller_formal: (self.scope)(binding.declaration_span), read: (self.scope)(span_of(expression)),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FormalOperand {
    Result,
    Unary,
    Left,
    Right,
}

impl FormalOperand {
    fn select(expression: &ScalarExpression) -> Option<(Self, &ScalarExpression)> {
        let literal = |value: &ScalarExpression| matches!(value,
            ScalarExpression::Boolean { .. } | ScalarExpression::Integer { .. });
        match expression {
            value if is_name(value) => Some((Self::Result, value)),
            ScalarExpression::Unary { operator, operand, .. }
                if audited_closed_unary(operator) && is_name(operand) => Some((Self::Unary, operand)),
            ScalarExpression::Binary { operator, left, right, .. } if audited_closed_eager_binary(operator) => {
                if is_name(left) && literal(right) { Some((Self::Left, left)) }
                else if literal(left) && is_name(right) { Some((Self::Right, right)) }
                else { None }
            }
            _ => None,
        }
    }

    fn get<'a>(self, expression: &'a ScalarExpression) -> Option<&'a ScalarExpression> {
        match (self, expression) {
            (Self::Result, value) => Some(value),
            (Self::Unary, ScalarExpression::Unary { operand, .. }) => Some(operand),
            (Self::Left, ScalarExpression::Binary { left, .. }) => Some(left),
            (Self::Right, ScalarExpression::Binary { right, .. }) => Some(right),
            _ => None,
        }
    }

    fn get_mut<'a>(self, expression: &'a mut ScalarExpression) -> Option<&'a mut ScalarExpression> {
        match (self, expression) {
            (Self::Result, value) => Some(value),
            (Self::Unary, ScalarExpression::Unary { operand, .. }) => Some(operand),
            (Self::Left, ScalarExpression::Binary { left, .. }) => Some(left),
            (Self::Right, ScalarExpression::Binary { right, .. }) => Some(right),
            _ => None,
        }
    }
}

fn is_name(value: &ScalarExpression) -> bool {
    matches!(value, ScalarExpression::Name { .. })
}

// Bound the syntactic projection before detached AST extraction. The AST,
// not syntax spelling, selects the operator and its authenticated read.
fn primitive_operand_source(expression: &CstNode) -> bool {
    let mut operator_nodes = 0;
    expression.descendants().enumerate().all(|(index, node)| {
        index < 16 && match node.kind() {
            SyntaxKind::Expression | SyntaxKind::Primary | SyntaxKind::Parenthesized | SyntaxKind::Operator => true,
            SyntaxKind::Unary | SyntaxKind::Binary => {
                operator_nodes += 1;
                operator_nodes == 1
            }
            _ => false,
        }
    })
}

fn only<T>(mut values: impl Iterator<Item = T>) -> Option<T> {
    let value = values.next()?;
    values.next().is_none().then_some(value)
}

fn sole_final_value(block: &CstNode) -> Option<CstNode> {
    let list = only(block.children()).filter(|child| child.kind() == SyntaxKind::FinalOutputList)?;
    let output = only(list.children()).filter(|child| child.kind() == SyntaxKind::OutputList)?;
    only(output.children()).filter(|child| child.kind() == SyntaxKind::Expression)
}

fn function_tail(node: &CstNode) -> Option<(CstNode, CstNode, CstNode)> {
    let primary = node.parent().filter(|p| p.kind() == SyntaxKind::Primary)?;
    let expression = primary.parent().filter(|p| p.kind() == SyntaxKind::Expression)?;
    let output = expression.parent().filter(|p| p.kind() == SyntaxKind::OutputList)?;
    let tail = output.parent().filter(|p| p.kind() == SyntaxKind::FinalOutputList)?;
    let block = tail.parent().filter(|p| p.kind() == SyntaxKind::Block)?;
    if sole_final_value(&block).as_ref() != Some(&expression) { return None; }
    let function = block.parent().filter(|p| p.kind() == SyntaxKind::FunctionDecl)?;
    if function.children_with_tokens().any(|child| child.kind() == SyntaxKind::Unsafe) { return None; }
    let item = function.parent().filter(|p| p.kind() == SyntaxKind::Item)?;
    let source = item.parent().filter(|p| p.kind() == SyntaxKind::Root)?;
    Some((expression, item, source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::super::derive_scalar_declarations_from_cst;
    use wosy_syntax::SourceIdentity;

    fn fresh(source: &str) -> FreshParseResult {
        let source = format!("%%start\n# WHY: café na órbita ☕\n{source}\n%%end");
        wosy_syntax::parse_with_snapshot(
            SourceIdentity::new("owned".into(), "entry".into(), "main.w".into(), "authored".into()),
            source, &["WHY".into()],
        )
    }

    fn spelling<'a>(fresh: &'a FreshParseResult, span: &SourceSpan) -> &'a str {
        assert_eq!(span.source, fresh.result().source);
        &fresh.result().text[span.range.start as usize..span.range.end as usize]
    }

    #[test]
    fn direct_comptime_inspection_binds_typed_literal_call_and_authored_formal() {
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            for (ty, literal, value, expected) in [
                ("i32", "7", DirectComptimeArgument::I32(7), ScalarType::I32),
                ("u32", "4294967295", DirectComptimeArgument::U32(u32::MAX), ScalarType::U32),
                ("bool", "true", DirectComptimeArgument::Bool(true), ScalarType::Bool),
            ] {
                let authored = fresh(&format!(
                    "{ty}({ty}) solve = fn(input) {{ comptime value {{ input }} }};\n{ty} answer = solve({literal});"
                ));
                assert!(authored.result().is_valid());
                let fact = inspect_direct_function_comptime(&authored, "value", layout)
                    .unwrap_or_else(|why| panic!("exact typed inspection refused {ty} on {layout:?}: {why:?}"));
                assert_eq!(fact.value(), value);
                assert_eq!(fact.result(), &expected);
                assert_eq!(spelling(&authored, fact.site()), "comptime value { input }");
                assert_eq!(spelling(&authored, fact.function()), "solve");
                assert_eq!(spelling(&authored, fact.call()), format!("solve({literal})"));
                assert_eq!(spelling(&authored, fact.argument()), literal);
                assert_eq!(spelling(&authored, fact.formal()), "input");
                assert_eq!(spelling(&authored, fact.read()), "input");
                assert_ne!(fact.formal(), fact.read());

                let ordinary = derive_scalar_declarations_from_cst(authored.result().canonical_cst());
                let [diagnostic] = ordinary.diagnostics() else { panic!("untouched ordinary refusal"); };
                assert_eq!(diagnostic.code, "B0003");
                assert_eq!(&diagnostic.labels[0].span, fact.site());
                assert!(ordinary.items.is_empty());
            }
        }
    }

    #[test]
    fn direct_comptime_evaluator_emits_only_the_compiler_bound_argument() {
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let mut i32_evaluators = Vec::new();
            for (ty, literal, value) in [
                ("i32", "7", DirectComptimeArgument::I32(7)),
                ("i32", "9", DirectComptimeArgument::I32(9)),
                ("u32", "4294967295", DirectComptimeArgument::U32(u32::MAX)),
                ("bool", "false", DirectComptimeArgument::Bool(false)),
                ("bool", "true", DirectComptimeArgument::Bool(true)),
            ] {
                let direct = fresh(&format!(
                    "{ty}({ty}) solve = fn(input) {{ comptime value {{ input }} }};\n{ty} answer = solve({literal});"
                ));
                let exact = derive_direct_function_comptime_fragment(&direct, "value", layout)
                    .unwrap_or_else(|error| panic!("typed evaluator {ty} on {layout:?}: {error:?}"));
                let ordinary = fresh(&format!("{ty} answer = comptime value {{ {literal} }};"));
                let expected = super::super::derive_closed_comptime_fragment(&ordinary, "value", layout, |_| None)
                    .unwrap_or_else(|error| panic!("literal oracle {ty}: {error:?}"));
                assert_eq!(exact.facts().value(), value);
                assert_eq!(spelling(&direct, exact.facts().argument()), literal);
                assert_eq!(exact.facts().site(), exact.evaluator().site());
                assert_eq!(exact.facts().result(), exact.evaluator().result_type());
                assert_eq!(exact.evaluator().layout(), layout);
                assert!(exact.evaluator().parameter_uses().is_empty());
                let actual = exact.evaluator().llvm().to_text();
                assert_eq!(actual, expected.llvm().to_text());
                assert!(!actual.contains("@solve("), "analytical program leaked into evaluator");
                if ty == "i32" { i32_evaluators.push(actual); }
            }
            assert_eq!(i32_evaluators.len(), 2);
            assert_ne!(i32_evaluators[0], i32_evaluators[1]);

            let unbound = fresh("i32(i32) solve = fn(input) { comptime value { input } };\ni32 answer = solve(1 | 2);");
            assert!(matches!(derive_direct_function_comptime_fragment(&unbound, "value", layout),
                Err(ClosedComptimeRejection::Unsupported { feature: "argument must be an exact scalar literal", .. })));
        }
    }

    const STARTUP_FUNCTION: &str = "i32(i32) solve = fn(input) { comptime value { input } };";

    #[test]
    fn direct_startup_plan_preserves_order_and_shares_only_identical_typed_tuples() {
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let authored = fresh(&format!(
                "{STARTUP_FUNCTION}\ni32 first = solve(7);\ni32 second = solve(9);\ni32 third = solve(7);"
            ));
            let plan = derive_direct_startup_comptime_plan(&authored, "value", layout)
                .unwrap_or_else(|error| panic!("finite startup plan: {error:?}"));
            let [a, b, c] = plan.calls() else { panic!("three canonical startup occurrences"); };
            assert_eq!([a.evaluator_index(), b.evaluator_index(), c.evaluator_index()], [0, 1, 0]);
            for (call, literal, value) in [(a, "7", 7), (b, "9", 9), (c, "7", 7)] {
                let facts = call.facts();
                assert_eq!(facts.value(), DirectComptimeArgument::I32(value));
                assert_eq!(facts.result(), &ScalarType::I32);
                assert_eq!(facts.function(), a.facts().function());
                assert_eq!(facts.formal(), a.facts().formal());
                assert_eq!(facts.read(), a.facts().read());
                assert_eq!(facts.site(), a.facts().site());
                assert_eq!(spelling(&authored, facts.argument()), literal);
                assert_eq!(spelling(&authored, facts.call()), format!("solve({literal})"));
            }
            assert!(a.facts().call().range.end < b.facts().call().range.start);
            assert!(b.facts().call().range.end < c.facts().call().range.start);
            let [seven, nine] = plan.evaluators() else { panic!("two exact evaluator tuples"); };
            for (evaluator, literal) in [(seven, "7"), (nine, "9")] {
                let ordinary = fresh(&format!("i32 answer = comptime value {{ {literal} }};"));
                let expected = super::super::derive_closed_comptime_fragment(&ordinary, "value", layout, |_| None).unwrap();
                assert_eq!(evaluator.llvm().to_text(), expected.llvm().to_text());
                assert_eq!(evaluator.site(), a.facts().site());
                assert_eq!(evaluator.layout(), layout);
            }
            assert_ne!(seven.llvm().to_text(), nine.llvm().to_text());
            assert!(matches!(inspect_direct_function_comptime(&authored, "value", layout),
                Err(ClosedComptimeRejection::Unsupported { feature: "single-call projection requires exactly one startup initializer", .. })));
        }
    }

    #[test]
    fn direct_startup_plan_refuses_a_late_unproven_call_and_preserves_long_lists() {
        let layout = ScalarTargetLayout::WASM32;
        let computed = fresh(&format!(
            "{STARTUP_FUNCTION}\ni32 first = solve(7);\ni32 second = solve(first);"
        ));
        assert!(computed.result().is_valid());
        assert!(matches!(derive_direct_startup_comptime_plan(&computed, "value", layout),
            Err(ClosedComptimeRejection::Unsupported { feature: "argument must be an exact scalar literal", .. })));
        let unrelated = fresh(&format!(
            "{STARTUP_FUNCTION}\ni32 first = solve(7);\ni32 unrelated = 9;"
        ));
        assert!(unrelated.result().is_valid());
        assert!(matches!(derive_direct_startup_comptime_plan(&unrelated, "value", layout),
            Err(ClosedComptimeRejection::Unsupported { feature: "expected a direct ordinary initializer call", .. })));

        let mut full = STARTUP_FUNCTION.to_owned();
        for index in 0..23 {
            full.push_str(&format!("\ni32 result_{index} = solve(7);"));
        }
        let at_limit = derive_direct_startup_comptime_plan(&fresh(&full), "value", layout).unwrap();
        assert_eq!(at_limit.calls().len(), 23);
        assert_eq!(at_limit.evaluators().len(), 1);
        assert!(at_limit.calls().iter().all(|call| call.evaluator_index() == 0));
        full.push_str("\ni32 last = solve(7);");
        let over = fresh(&full);
        assert!(over.result().is_valid());
        let extended = derive_direct_startup_comptime_plan(&over, "value", layout).unwrap();
        assert_eq!(extended.calls().len(), 24);
        assert_eq!(extended.evaluators().len(), 1);
        assert_eq!(extended.evaluators()[0].llvm().to_text(), at_limit.evaluators()[0].llvm().to_text());
    }

    #[test]
    fn direct_comptime_eager_scalar_operation_is_not_an_identity_projection() {
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            for (ty, body, literal, bound_expression, argument) in [
                ("u32", "input ^ 255", "7", "7 ^ 255", DirectComptimeArgument::U32(7)),
                ("u32", "255 ^ input", "9", "255 ^ 9", DirectComptimeArgument::U32(9)),
                ("i32", "~input", "7", "~7", DirectComptimeArgument::I32(7)),
                ("bool", "!input", "true", "!true", DirectComptimeArgument::Bool(true)),
                ("bool", "input != true", "true", "true != true", DirectComptimeArgument::Bool(true)),
            ] {
                let direct = fresh(&format!(
                    "{ty}({ty}) solve = fn(input) {{ comptime value {{ {body} }} }};\n{ty} answer = solve({literal});"
                ));
                assert!(direct.result().is_valid());
                let actual = derive_direct_function_comptime_fragment(&direct, "value", layout)
                    .unwrap_or_else(|error| panic!("direct body {body} on {layout:?}: {error:?}"));
                assert_eq!(spelling(&direct, actual.facts().read()), "input");
                assert_eq!(spelling(&direct, actual.facts().argument()), literal);
                assert_eq!(actual.facts().value(), argument);
                let emit_closed = |expression: &str| {
                    let authored = fresh(&format!("{ty} answer = comptime value {{ {expression} }};"));
                    super::super::derive_closed_comptime_fragment(&authored, "value", layout, |_| None)
                        .unwrap_or_else(|error| panic!("ordinary oracle {expression}: {error:?}"))
                        .llvm().to_text()
                };
                let text = actual.evaluator().llvm().to_text();
                assert_eq!(text, emit_closed(bound_expression));
                assert_ne!(text, emit_closed(literal), "{body} was reduced to an identity projection");
            }

            let batch = fresh("u32(u32) solve = fn(input) { comptime value { input ^ 255 } };\nu32 first = solve(7);\nu32 second = solve(9);\nu32 third = solve(7);");
            let plan = derive_direct_startup_comptime_plan(&batch, "value", layout).unwrap();
            assert_eq!(plan.calls().iter().map(DirectStartupComptimeUse::evaluator_index).collect::<Vec<_>>(), [0, 1, 0]);
            let [first, next] = plan.evaluators() else { panic!("two evaluator tuples"); };
            assert_ne!(first.llvm().to_text(), next.llvm().to_text());
        }
    }

    #[test]
    fn direct_comptime_eager_scope_rejects_unaudited_operations_and_multiple_reads() {
        for (out, input, body, argument) in [
            ("i32", "i32", "input + 1", "7"),
            ("i32", "i32", "-input", "7"),
            ("i32", "i32", "input << 1", "7"),
            ("bool", "i32", "input < 8", "7"),
            ("bool", "bool", "false && input", "true"),
            ("u32", "u32", "input ^ input", "7"),
        ] {
            let authored = fresh(&format!(
                "{out}({input}) solve = fn(input) {{ comptime value {{ {body} }} }};\n{out} answer = solve({argument});"
            ));
            assert!(authored.result().is_valid(), "fixture {body}");
            assert!(matches!(derive_direct_function_comptime_fragment(&authored, "value", ScalarTargetLayout::WASM32),
                Err(ClosedComptimeRejection::Unsupported {
                    feature: "comptime accepts one audited eager scalar operator with one formal read", ..
                })), "unexpectedly admitted {body}");
        }
    }

    const PAIRED_FUNCTION: &str = "u32(u32, u32) solve = fn(left, right) { comptime value { right ^ left } };";

    #[test]
    fn paired_startup_keeps_ordinary_formal_order_even_when_reversed_ir_is_equal() {
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let authored = fresh(&format!(
                "{PAIRED_FUNCTION}\nu32 first = solve(7, 9);\nu32 reversed = solve(9, 7);\nu32 repeated = solve(7, 9);\nu32 changed = solve(7, 11);"
            ));
            assert!(authored.result().is_valid());
            let plan = derive_direct_pair_startup_comptime_plan(&authored, "value", layout)
                .unwrap_or_else(|error| panic!("typed pair on {layout:?}: {error:?}"));
            let [first, reversed, repeated, changed] = plan.calls() else { panic!("four ordinary calls"); };
            assert_eq!([first.evaluator_index(), reversed.evaluator_index(), repeated.evaluator_index(),
                changed.evaluator_index()], [0, 1, 0, 2]);
            for (call, expected) in [(first, [7, 9]), (reversed, [9, 7]), (repeated, [7, 9]), (changed, [7, 11])] {
                let facts = call.facts();
                assert_eq!(facts.tuple(), expected.map(DirectComptimeArgument::U32));
                assert_eq!(facts.site(), first.facts().site());
                assert_eq!(facts.function(), first.facts().function());
                assert_eq!(facts.result(), &ScalarType::U32);
                for (input, (value, name)) in facts.inputs().iter().zip(expected.into_iter().zip(["left", "right"])) {
                    assert_eq!(spelling(&authored, input.formal()), name);
                    assert_eq!(spelling(&authored, input.read()), name);
                    assert_eq!(spelling(&authored, input.argument()), value.to_string());
                }
                assert!(facts.inputs()[0].argument().range.end < facts.inputs()[1].argument().range.start);
                assert!(facts.inputs()[1].read().range.end < facts.inputs()[0].read().range.start);
            }
            let [a, b, different] = plan.evaluators() else { panic!("three fully typed distinct tuples"); };
            let emit_closed = |expression: &str| {
                let root = fresh(&format!("u32 answer = comptime value {{ {expression} }};"));
                super::super::derive_closed_comptime_fragment(&root, "value", layout, |_| None)
                    .unwrap().llvm().to_text()
            };
            assert_eq!(a.llvm().to_text(), emit_closed("9 ^ 7"));
            assert_eq!(b.llvm().to_text(), emit_closed("7 ^ 9"));
            assert_eq!(different.llvm().to_text(), emit_closed("11 ^ 7"));
            assert_eq!(a.llvm().to_text(), b.llvm().to_text());
            assert_ne!(a.llvm().to_text(), different.llvm().to_text());
            assert!(derive_direct_startup_comptime_plan(&authored, "value", layout).is_err());
        }
    }

    fn authored_call_graph() -> String {
        format!(
            "{PAIRED_FUNCTION}\n\
             u32(u32, u32) near = fn(near_left, near_right) {{ solve(near_right, near_left) }};\n\
             u32(u32, u32) center = fn(center_left, center_right) {{ near(center_right, center_left) }};\n\
             u32(u32, u32) side = fn(side_left, side_right) {{ near(side_left, side_right) }};\n\
             u32(u32, u32) far = fn(far_left, far_right) {{ center(far_left, far_right) }};\n\
             u32 root_first = center(7, 9);\n\
             u32 root_second = side(9, 7);\n\
             u32 root_third = side(7, 9);\n\
             u32 root_direct = solve(7, 9);\n\
             u32 root_far = far(7, 11);\n\
             u32 root_repeat = far(7, 9);"
        )
    }

    fn authored_selector() -> String {
        format!(
            "{PAIRED_FUNCTION}\n\
             u32(u32, u32) lower_hop = fn(lower_left, lower_right) {{ solve(lower_right, lower_left) }};\n\
             u32(u32, u32) upper_hop = fn(upper_left, upper_right) {{ lower_hop(upper_left, upper_right) }};\n\
             u32(bool, u32, u32) choose = fn(select_path, data_left, data_right) {{\n\
                 if (select_path) {{ upper_hop(data_left, data_right) }} else {{ solve(data_left, data_right) }}\n\
             }};\n\
             u32 root_yes = choose(true, 7, 9);\n\
             u32 root_no = choose(false, 7, 9);\n\
             u32 root_together = choose(true, 9, 7);\n\
             u32 root_different = choose(false, 7, 11);\n\
             u32 root_direct = solve(7, 9);"
        )
    }

    #[test]
    fn pair_call_graph_uses_actual_checked_literal_to_select_one_verified_tail_branch() {
        let source = fresh(&authored_selector());
        assert!(source.result().is_valid());
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let plan = derive_pair_startup_call_graph_comptime_plan(&source, "value", layout)
                .unwrap_or_else(|error| panic!("typed literal branch graph on {layout:?}: {error:?}"));
            let [selector] = plan.selectors() else { panic!("one authored selector"); };
            assert_eq!(spelling(&source, selector.declaration()), "choose");
            assert_eq!(spelling(&source, selector.formal()), "select_path");
            assert_eq!(spelling(&source, selector.condition()), "select_path");
            assert_ne!(selector.condition(), selector.formal());
            assert_eq!((selector.when_true().hop_index(), selector.when_false().hop_index()), (Some(2), Some(3)));
            for (index, target, tail) in [
                (selector.when_true().hop_index().unwrap(), "upper_hop", "upper_hop(data_left, data_right)"),
                (selector.when_false().hop_index().unwrap(), "solve", "solve(data_left, data_right)"),
            ] {
                let hop = &plan.hops()[index];
                assert_eq!(selector.declaration(), hop.caller());
                assert_eq!(spelling(&source, hop.callee()), target);
                assert_eq!(spelling(&source, hop.tail_call()), tail);
                for ((input, ordinal), name) in hop.inputs().iter().zip([1, 2]).zip(["data_left", "data_right"]) {
                    assert_eq!(input.caller_position(), ordinal);
                    assert_eq!(spelling(&source, input.caller_formal()), name);
                    assert_eq!(spelling(&source, input.read()), name);
                }
            }
            let expected: [(Option<bool>, &str, &[usize], [usize; 2], [u32; 2], usize); 5] = [
                (Some(true), "choose(true, 7, 9)", &[2, 1, 0], [2, 1], [9, 7], 0),
                (Some(false), "choose(false, 7, 9)", &[3], [1, 2], [7, 9], 1),
                (Some(true), "choose(true, 9, 7)", &[2, 1, 0], [2, 1], [7, 9], 1),
                (Some(false), "choose(false, 7, 11)", &[3], [1, 2], [7, 11], 2),
                (None, "solve(7, 9)", &[], [0, 1], [7, 9], 1),
            ];
            assert_eq!(plan.calls().len(), expected.len());
            for ((call, path), (branch, outer, hops, positions, tuple, slot)) in plan.calls().zip(expected) {
                assert_eq!(spelling(&source, call.facts().call()), outer);
                assert_eq!(path.hop_indices(), hops);
                assert_eq!(path.startup_argument_positions(), positions);
                assert_eq!(call.facts().tuple(), tuple.map(DirectComptimeArgument::U32));
                assert_eq!(call.evaluator_index(), slot);
                assert_eq!(path.branch_selection().map(PairStartupBranchSelection::value), branch);
                if let Some(choice) = path.branch_selection() {
                    assert_eq!(choice.selector_index(), 0);
                    assert_eq!(spelling(&source, choice.literal()), if choice.value() { "true" } else { "false" });
                    assert!(call.facts().inputs().iter().all(|input| choice.literal().range.end < input.argument().range.start));
                }
                for (input, value) in call.facts().inputs().iter().zip(tuple) {
                    assert_eq!(spelling(&source, input.argument()), value.to_string());
                }
            }
            let [swapped, unchanged, changed] = plan.evaluators() else { panic!("three distinct final slots"); };
            for (evaluator, expression) in [(swapped, "7 ^ 9"), (unchanged, "9 ^ 7"), (changed, "11 ^ 7")] {
                let ordinary = fresh(&format!("u32 answer = comptime value {{ {expression} }};"));
                let oracle = super::super::derive_closed_comptime_fragment(&ordinary, "value", layout, |_| None).unwrap();
                assert_eq!(evaluator.llvm().to_text(), oracle.llvm().to_text());
            }
            assert_eq!(swapped.llvm().to_text(), unchanged.llvm().to_text());
            assert_ne!(swapped.llvm().to_text(), changed.llvm().to_text());
        }
    }

    fn selectors_with_bypass() -> String {
        format!(
            "{PAIRED_FUNCTION}\n\
             u32(u32, u32) swap_data = fn(swap_left, swap_right) {{ solve(swap_right, swap_left) }};\n\
             u32(bool, u32, u32) choose = fn(select_path, data_left, data_right) {{\n\
                 if (select_path) {{ swap_data(data_left, data_right) }} else {{ data_right }}\n\
             }};\n\
             u32(bool, u32, u32) mirror = fn(select_mirror, other_left, other_right) {{\n\
                 if (select_mirror) {{ other_left }} else {{ solve(other_left, other_right) }}\n\
             }};"
        )
    }

    #[test]
    fn pair_call_graph_keeps_ordinary_bypass_results_and_specializations_in_startup_order() {
        let source = fresh(&format!(
            "{}\n\
             u32 skip_early = choose(false, 7, 91);\n\
             u32 hit_early = choose(true, 7, 9);\n\
             u32 skip_late = mirror(true, 112, 8);\n\
             u32 hit_late = mirror(false, 9, 7);\n\
             u32 hit_direct = solve(9, 7);",
            selectors_with_bypass(),
        ));
        assert!(source.result().is_valid());
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let plan = derive_pair_startup_call_graph_comptime_plan(&source, "value", layout)
                .unwrap_or_else(|error| panic!("checked bypass on {layout:?}: {error:?}"));
            let [selector, mirror] = plan.selectors() else { panic!("two authored selectors"); };
            for (branch, position, name) in [(selector.when_false(), 2, "data_right"), (mirror.when_true(), 1, "other_left")] {
                let PairStartupBranch::ReturnFormal(formal) = branch else { panic!("bypass branch must retain the authored output read"); };
                assert_eq!(formal.caller_position(), position);
                assert_eq!(spelling(&source, formal.caller_formal()), name);
                assert_eq!(spelling(&source, formal.read()), name);
                assert_ne!(formal.caller_formal(), formal.read());
                assert_eq!(branch.hop_index(), None);
            }
            let [early, hit, late, hit_again, direct] = plan.occurrences().collect::<Vec<_>>().try_into().unwrap_or_else(|slots: Vec<_>| {
                panic!("expected all five startup slots, got {}", slots.len());
            });
            for (slot, call, name, selector, truth, result) in [
                (early, "choose(false, 7, 91)", "skip_early", 0, false, 91u32),
                (late, "mirror(true, 112, 8)", "skip_late", 1, true, 112u32),
            ] {
                let PairStartupOccurrence::Bypass(slot) = slot else { panic!("ordinary slot may not be silently dropped or specialized"); };
                assert_eq!(spelling(&source, slot.call()), call);
                assert_eq!(spelling(&source, slot.destination()), name);
                assert_eq!(slot.branch_selection().selector_index(), selector);
                assert_eq!(slot.branch_selection().value(), truth);
                assert_eq!(spelling(&source, slot.branch_selection().literal()), if truth { "true" } else { "false" });
                assert_eq!(spelling(&source, slot.returned_argument()), result.to_string());
                assert_eq!(slot.returned_value(), DirectComptimeArgument::U32(result));
            }
            for (slot, destination, expected_path) in [
                (hit, "hit_early", &[1usize, 0][..]), (hit_again, "hit_late", &[2usize][..]),
                (direct, "hit_direct", &[][..]),
            ] {
                let PairStartupOccurrence::Specialization(call, path) = slot else { panic!("ordinary reached slot lost its specialization"); };
                assert_eq!(spelling(&source, call.facts().destination()), destination);
                assert_eq!(call.facts().tuple(), [DirectComptimeArgument::U32(9), DirectComptimeArgument::U32(7)]);
                assert_eq!(path.hop_indices(), expected_path);
                assert_eq!(call.evaluator_index(), 0);
            }
            assert_eq!(plan.calls().len(), 3);
            let [only] = plan.evaluators() else { panic!("only reached identical tuples evaluate"); };
            let ordinary = fresh("u32 answer = comptime value { 7 ^ 9 };");
            let oracle = super::super::derive_closed_comptime_fragment(&ordinary, "value", layout, |_| None).unwrap();
            assert_eq!(only.llvm().to_text(), oracle.llvm().to_text());
        }
    }

    #[test]
    fn pair_call_graph_proves_zero_evaluators_when_all_checked_slots_bypass_the_site() {
        let mut text = selectors_with_bypass();
        for slot in 0..23 {
            text.push_str(&format!("\nu32 skip_{slot} = choose(false, 1, {slot});"));
        }
        let source = fresh(&text);
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let plan = derive_pair_startup_call_graph_comptime_plan(&source, "value", layout)
                .unwrap_or_else(|error| panic!("empty evaluation plan on {layout:?}: {error:?}"));
            assert_eq!(plan.calls().len(), 0);
            assert!(plan.evaluators().is_empty());
            assert_eq!(plan.occurrences().len(), 23);
            for (index, slot) in plan.occurrences().enumerate() {
                let PairStartupOccurrence::Bypass(slot) = slot else { panic!("only ordinary returns"); };
                assert_eq!(spelling(&source, slot.destination()), format!("skip_{index}"));
                assert_eq!(slot.returned_value(), DirectComptimeArgument::U32(u32::try_from(index).unwrap()));
            }
        }

    }

    #[test]
    fn pair_call_graph_tracks_ordinary_read_chains_to_distinct_authored_literals() {
        let source = fresh(&format!(
            "{}\n\
             u32 seed_value = choose(false, 1, 37);\n\
             u32 carried = mirror(true, seed_value, 999);\n\
             u32 same_number = choose(false, 1, 37);\n\
             u32 reached = choose(true, 5, carried);\n\
             u32 duplicate_tuple = solve(same_number, 5);",
            selectors_with_bypass(),
        ));
        assert!(source.result().is_valid());
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let plan = derive_pair_startup_call_graph_comptime_plan(&source, "value", layout)
                .unwrap_or_else(|error| panic!("checked startup read graph on {layout:?}: {error:?}"));
            let slots = plan.occurrences().collect::<Vec<_>>();
            let [PairStartupOccurrence::Bypass(seed), PairStartupOccurrence::Bypass(carried),
                PairStartupOccurrence::Bypass(other), PairStartupOccurrence::Specialization(first, path),
                PairStartupOccurrence::Specialization(second, _)] = slots.as_slice()
            else { panic!("expected three ordinary producers and two later continuations"); };
            assert_eq!(seed.returned_value(), DirectComptimeArgument::U32(37));
            assert_eq!(carried.returned_value(), seed.returned_value());
            assert_eq!(spelling(&source, carried.returned_argument()), "seed_value");
            assert_eq!(carried.returned_origin().literal(), seed.returned_argument());
            assert_eq!(carried.returned_origin().prior_bypass_slot(), Some(0));
            assert_eq!(seed.returned_origin().prior_bypass_slot(), None);
            assert_ne!(other.returned_origin().literal(), seed.returned_origin().literal());
            assert_eq!(spelling(&source, other.returned_origin().literal()), "37");
            assert_eq!(path.startup_argument_positions(), [2, 1]);
            assert_eq!(path.hop_indices(), &[1, 0]);
            for (call, producer, read, literal) in [
                (first, 1, "carried", seed.returned_argument()),
                (second, 2, "same_number", other.returned_argument()),
            ] {
                assert_eq!(call.facts().tuple(), [DirectComptimeArgument::U32(37), DirectComptimeArgument::U32(5)]);
                assert_eq!(call.evaluator_index(), 0);
                let [first, second] = call.facts().inputs();
                assert_eq!(spelling(&source, first.argument()), read);
                assert_eq!(spelling(&source, first.formal()), "left");
                assert_eq!(first.origin().literal(), literal);
                assert_eq!(first.origin().prior_bypass_slot(), Some(producer));
                assert_eq!(second.origin().literal(), second.argument());
                assert_eq!(second.origin().prior_bypass_slot(), None);
            }
            let [only] = plan.evaluators() else { panic!("one canonical concrete tuple"); };
            let oracle = super::super::derive_closed_comptime_fragment(
                &fresh("u32 answer = comptime value { 5 ^ 37 };"), "value", layout, |_| None,
            ).unwrap();
            assert_eq!(only.llvm().to_text(), oracle.llvm().to_text());
        }
    }

    #[test]
    fn pair_call_graph_can_select_from_a_checked_ordinary_boolean_producer() {
        let source = fresh(
            "bool(bool, bool) solve = fn(data_left, data_right) { comptime value { data_right != data_left } };\n\
             bool(bool, bool, bool) decide = fn(select_path, flag_left, flag_right) {\n\
                 if (select_path) { solve(flag_left, flag_right) } else { flag_right }\n\
             };\n\
             bool saved_true = decide(false, false, true);\n\
             bool early_hit = decide(saved_true, false, false);\n\
             bool saved_false = decide(false, true, false);\n\
             bool skip_again = decide(saved_false, true, saved_true);\n\
             bool later_hit = decide(skip_again, false, true);",
        );
        assert!(source.result().is_valid());
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let plan = derive_pair_startup_call_graph_comptime_plan(&source, "value", layout)
                .unwrap_or_else(|error| panic!("checked Boolean producers on {layout:?}: {error:?}"));
            let slots = plan.occurrences().collect::<Vec<_>>();
            let [PairStartupOccurrence::Bypass(saved_true), PairStartupOccurrence::Specialization(early, early_path),
                PairStartupOccurrence::Bypass(saved_false), PairStartupOccurrence::Bypass(skip),
                PairStartupOccurrence::Specialization(late, late_path)] = slots.as_slice()
            else { panic!("only true selections reach the authored site"); };
            assert_eq!(plan.evaluators().len(), 2);
            assert_eq!(early.facts().tuple(), [DirectComptimeArgument::Bool(false); 2]);
            assert_eq!(late.facts().tuple(), [DirectComptimeArgument::Bool(false), DirectComptimeArgument::Bool(true)]);
            for (choice, name, producer, literal, expected_truth) in [
                (early_path.branch_selection().unwrap(), "saved_true", 0, saved_true.returned_argument(), true),
                (skip.branch_selection(), "saved_false", 2, saved_false.returned_argument(), false),
                (late_path.branch_selection().unwrap(), "skip_again", 3, saved_true.returned_argument(), true),
            ] {
                assert_eq!(spelling(&source, choice.argument()), name);
                assert_eq!(choice.origin().prior_bypass_slot(), Some(producer));
                assert_eq!(choice.literal(), literal);
                assert_eq!(choice.value(), expected_truth);
            }
            assert_eq!(spelling(&source, skip.returned_argument()), "saved_true");
            assert_eq!(skip.returned_origin().prior_bypass_slot(), Some(0));
            assert_eq!(skip.returned_value(), DirectComptimeArgument::Bool(true));
        }
    }

    #[test]
    fn pair_call_graph_never_supplies_a_value_for_unfinished_comptime() {
        let layout = ScalarTargetLayout::WASM32;
        let prefix = selectors_with_bypass();
        for appended in [
            "u32 pending_site = solve(7, 9);\nu32 next_site = solve(pending_site, 5);",
            "u32 pending_site = solve(7, 9);\nu32 ordinary_skip = choose(false, pending_site, 5);",
        ] {
            let source = fresh(&format!("{prefix}\n{appended}"));
            assert!(source.result().is_valid());
            let rejected = derive_pair_startup_call_graph_comptime_plan(&source, "value", layout);
            assert!(matches!(&rejected,
                Err(ClosedComptimeRejection::Unsupported {
                    feature: "startup argument must read an exact earlier ordinary bypass destination", ..
                })),
                "no producer may come from a specialization or an ignored argument: {:?}", rejected.err());
        }
        let forward = fresh(&format!(
            "{prefix}\nu32 early = solve(future_slot, 5);\nu32 future_slot = choose(false, 7, 9);",
        ));
        assert!(forward.result().is_valid());
        let rejected = derive_pair_startup_call_graph_comptime_plan(&forward, "value", layout);
        assert!(matches!(&rejected, Err(ClosedComptimeRejection::Unsupported {
            feature: "startup argument must read an exact earlier ordinary bypass destination", ..
        })), "no producer can come from a later slot: {:?}", rejected.err());
    }

    #[test]
    fn pair_call_graph_refuses_unproved_bypass_alternatives_and_discarded_computed_arguments() {
        let prefix = selectors_with_bypass();
        for (text, feature) in [
            (format!("{}\nu32 hit = choose(true, 7, 9);", prefix.replace("else { data_right }", "else { data_left ^ data_right }")),
                "a nonforwarding branch must return one exact checked data formal"),
            (format!("{}\nu32 skip = choose(false, 7, 9);", prefix.replace("swap_data(data_left, data_right)", "swap_data(data_left ^ data_right, data_right)")),
                "forwarding must pass each exact formal without intervening computation"),
            (format!("{prefix}\nu32 skip = choose(false, 3 ^ 1, 9);"),
                "startup argument must be a checked literal or earlier ordinary bypass read"),
        ] {
            let source = fresh(&text);
            assert!(source.result().is_valid());
            let outcome = derive_pair_startup_call_graph_comptime_plan(&source, "value", ScalarTargetLayout::WASM32);
            assert!(matches!(&outcome, Err(ClosedComptimeRejection::Unsupported { feature: actual, .. }) if *actual == feature),
                "expected {feature}, got {:?}", outcome.err());
        }
    }

    #[test]
    fn pair_call_graph_refuses_computed_conditions_and_nonliteral_startup_truth() {
        let source = authored_selector();
        for (text, feature) in [
            (source.replace("if (select_path)", "if (!select_path)"),
                "conditional forwarding must read its first checked Boolean formal directly"),
            (source.replace("choose(true, 7, 9)", "choose(!false, 7, 9)"),
                "conditional startup requires a checked Boolean literal or earlier ordinary bypass read"),
        ] {
            let fresh = fresh(&text);
            assert!(fresh.result().is_valid());
            let outcome = derive_pair_startup_call_graph_comptime_plan(&fresh, "value", ScalarTargetLayout::WASM32);
            assert!(matches!(&outcome, Err(ClosedComptimeRejection::Unsupported { feature: actual, .. }) if *actual == feature),
                "expected {feature}, got {:?}", outcome.err());
        }
    }

    #[test]
    fn pair_call_graph_requires_complete_pure_checked_tails_in_unselected_branches() {
        let only_true = authored_selector().replace("choose(false,", "choose(true,");
        for (text, feature) in [
            (only_true.replace("else { solve(data_left, data_right) }",
                "else { solve(data_left ^ data_right, data_right) }"),
                "forwarding must pass each exact formal without intervening computation"),
            (only_true.replace("else { solve(data_left, data_right) }",
                "else { u32 shuffled = data_left ^ data_right; solve(shuffled, data_right) }"),
                "each conditional branch must be one complete tail call"),
        ] {
            let fresh = fresh(&text);
            assert!(fresh.result().is_valid());
            let outcome = derive_pair_startup_call_graph_comptime_plan(&fresh, "value", ScalarTargetLayout::WASM32);
            assert!(matches!(&outcome, Err(ClosedComptimeRejection::Unsupported { feature: actual, .. }) if *actual == feature),
                "expected {feature}, got {:?}", outcome.err());
        }
    }

    #[test]
    fn pair_call_graph_proves_different_paths_and_reuses_only_the_final_tuple() {
        let source = fresh(&authored_call_graph());
        assert!(source.result().is_valid());
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let plan = derive_pair_startup_call_graph_comptime_plan(&source, "value", layout)
                .unwrap_or_else(|error| panic!("typed call graph on {layout:?}: {error:?}"));
            let expected_hops = [
                ("near", "solve", "solve(near_right, near_left)", [1, 0], ["near_right", "near_left"]),
                ("center", "near", "near(center_right, center_left)", [1, 0], ["center_right", "center_left"]),
                ("side", "near", "near(side_left, side_right)", [0, 1], ["side_left", "side_right"]),
                ("far", "center", "center(far_left, far_right)", [0, 1], ["far_left", "far_right"]),
            ];
            assert_eq!(plan.hops().len(), expected_hops.len());
            for (hop, (caller, callee, tail, positions, names)) in plan.hops().iter().zip(expected_hops) {
                assert_eq!(spelling(&source, hop.caller()), caller);
                assert_eq!(spelling(&source, hop.callee()), callee);
                assert_eq!(spelling(&source, hop.tail_call()), tail);
                for ((input, position), name) in hop.inputs().iter().zip(positions).zip(names) {
                    assert_eq!(input.caller_position(), position);
                    assert_eq!(spelling(&source, input.caller_formal()), name);
                    assert_eq!(spelling(&source, input.read()), name);
                    assert_ne!(input.caller_formal(), input.read());
                }
            }
            let expected_calls: [(&str, &[usize], [usize; 2], [u32; 2], usize); 6] = [
                ("center(7, 9)", &[1, 0], [0, 1], [7, 9], 0),
                ("side(9, 7)", &[2, 0], [1, 0], [7, 9], 0),
                ("side(7, 9)", &[2, 0], [1, 0], [9, 7], 1),
                ("solve(7, 9)", &[], [0, 1], [7, 9], 0),
                ("far(7, 11)", &[3, 1, 0], [0, 1], [7, 11], 2),
                ("far(7, 9)", &[3, 1, 0], [0, 1], [7, 9], 0),
            ];
            assert_eq!(plan.calls().len(), expected_calls.len());
            for ((call, path), (spelling_in_entry, hops, positions, tuple, slot)) in plan.calls().zip(expected_calls) {
                let facts = call.facts();
                assert_eq!(spelling(&source, facts.call()), spelling_in_entry);
                assert_eq!(spelling(&source, facts.function()), "solve");
                assert_eq!(path.hop_indices(), hops);
                assert_eq!(path.startup_argument_positions(), positions);
                assert_eq!(facts.tuple(), tuple.map(DirectComptimeArgument::U32));
                assert_eq!(call.evaluator_index(), slot);
                for (input, value) in facts.inputs().iter().zip(tuple) {
                    assert_eq!(spelling(&source, input.argument()), value.to_string());
                }
                assert_eq!(facts.inputs()[0].argument().range.start < facts.inputs()[1].argument().range.start,
                    positions == [0, 1]);
            }
            let [ordered, swapped, different] = plan.evaluators() else { panic!("three typed tuple slots"); };
            for (evaluator, expression) in [(ordered, "9 ^ 7"), (swapped, "7 ^ 9"), (different, "11 ^ 7")] {
                let ordinary = fresh(&format!("u32 answer = comptime value {{ {expression} }};"));
                let oracle = super::super::derive_closed_comptime_fragment(&ordinary, "value", layout, |_| None).unwrap();
                assert_eq!(evaluator.llvm().to_text(), oracle.llvm().to_text());
            }
            assert_eq!(ordered.llvm().to_text(), swapped.llvm().to_text());
            assert_ne!(ordered.llvm().to_text(), different.llvm().to_text());
        }
    }

    #[test]
    fn pair_call_graph_refuses_a_real_cycle_and_computation_in_a_shared_edge() {
        let layout = ScalarTargetLayout::WASM32;
        let cyclic = fresh(&format!(
            "{PAIRED_FUNCTION}\n\
             u32(u32, u32) loop_node = fn(loop_left, loop_right) {{ loop_node(loop_right, loop_left) }};\n\
             u32 root_safe = solve(7, 9);\n\
             u32 root_cyclic = loop_node(7, 9);"
        ));
        assert!(cyclic.result().is_valid());
        let Err(ClosedComptimeRejection::Unsupported { site, feature: "comptime startup forwarding contains a cycle" }) =
            derive_pair_startup_call_graph_comptime_plan(&cyclic, "value", layout)
        else { panic!("the checked self edge must be refused as cyclic"); };
        assert_eq!(spelling(&cyclic, &site), "loop_node(loop_right, loop_left)");

        let remapped = fresh(&authored_call_graph().replace("solve(near_right, near_left)", "solve(near_right ^ near_left, near_left)"));
        assert!(remapped.result().is_valid());
        assert!(matches!(derive_pair_startup_call_graph_comptime_plan(&remapped, "value", layout),
            Err(ClosedComptimeRejection::Unsupported {
                feature: "forwarding must pass each exact formal without intervening computation", ..
            })));
    }

    #[test]
    fn pair_call_graph_preserves_long_finite_paths_and_startup() {
        let authored = |hops: usize, calls: usize| {
            use std::fmt::Write;
            let mut authored = PAIRED_FUNCTION.to_owned();
            let mut previous = "solve".to_owned();
            for index in 0..hops {
                let name = format!("relay_{index}");
                writeln!(authored, "\nu32(u32, u32) {name} = fn(chain_left, chain_right) {{ {previous}(chain_right, chain_left) }};").unwrap();
                previous = name;
            }
            for index in 0..calls {
                writeln!(authored, "\nu32 startup_{index} = {previous}(7, 9);").unwrap();
            }
            fresh(&authored)
        };
        let (hops, calls) = (96, 128);
        let source = authored(hops, calls);
        assert!(source.result().is_valid());
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let plan = derive_pair_startup_call_graph_comptime_plan(&source, "value", layout).unwrap();
            assert_eq!(plan.hops().len(), hops);
            assert_eq!(plan.calls().len(), calls);
            assert_eq!(plan.evaluators().len(), 1);
            let expected: Vec<_> = (0..hops).rev().collect();
            let shared = plan.calls().next().unwrap().1.hop_indices();
            for (call, path) in plan.calls() {
                assert_eq!(path.hop_indices(), expected);
                assert!(std::ptr::eq(path.hop_indices(), shared));
                assert_eq!(path.startup_argument_positions(), [0, 1]);
                assert_eq!(call.facts().tuple(), [DirectComptimeArgument::U32(7), DirectComptimeArgument::U32(9)]);
            }
            let baseline = derive_pair_startup_call_graph_comptime_plan(&authored(0, 1), "value", layout).unwrap();
            assert_eq!(plan.evaluators()[0].llvm().to_text(), baseline.evaluators()[0].llvm().to_text());
        }

        let wasm = |source| derive_pair_startup_call_graph_wasip1_stdout(source, "value", 128)
            .unwrap().into_bounded_wasip1_output(1 << 20).unwrap();
        let long = wasm(&source);
        let short = wasm(&authored(0, 1));
        assert_eq!(long.evaluators().len(), 1);
        assert_eq!(long.evaluators()[0].bytes(), short.evaluators()[0].bytes());
        let retained = long.ordinary_continuation();
        for (index, (node, occurrence)) in retained.initializers().enumerate() {
            assert_eq!(node.text().to_string(), "relay_95(7, 9)");
            assert_eq!(node.ancestors().last().unwrap(), retained.canonical_cst().root);
            let PairStartupOccurrence::Specialization(call, _) = occurrence else { panic!("exact selected route"); };
            assert_eq!(call.evaluator_index(), 0);
            assert!(index < calls);
        }
        assert_eq!(retained.initializers().len(), calls);
    }


    #[test]
    fn forwarded_pair_reaches_the_exact_typed_target_with_the_formal_permutation() {
        let outer_tuples = [[7, 9], [9, 7], [7, 9], [7, 11]];
        for (tail, permutation) in [("solve(upstream_left, upstream_right)", [0, 1]), ("solve(upstream_right, upstream_left)", [1, 0])] {
            let source = fresh(&format!(
                "{PAIRED_FUNCTION}\nu32(u32, u32) relay = fn(upstream_left, upstream_right) {{ {tail} }};\n\
                 u32 first = relay(7, 9);\nu32 reversed = relay(9, 7);\nu32 repeated = relay(7, 9);\n\
                 u32 changed = relay(7, 11);"
            ));
            assert!(source.result().is_valid());
            for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
                let forwarded = derive_forwarded_pair_startup_comptime_plan(&source, "value", layout)
                    .unwrap_or_else(|error| panic!("typed forwarding on {layout:?}: {error:?}"));
                assert_eq!(spelling(&source, forwarded.forwarding().declaration()), "relay");
                assert_eq!(spelling(&source, forwarded.forwarding().tail_call()), tail);
                assert_eq!(forwarded.forwarding().startup_argument_positions(), permutation);
                let expected = outer_tuples.map(|tuple| permutation.map(|ordinal| tuple[ordinal]));
                assert_eq!(forwarded.calls().len(), expected.len());
                assert_eq!(forwarded.calls().iter().map(DirectStartupComptimeUse::evaluator_index)
                    .collect::<Vec<_>>(), [0, 1, 0, 2]);
                for (call, values) in forwarded.calls().iter().zip(expected) {
                    let facts = call.facts();
                    assert!(spelling(&source, facts.call()).starts_with("relay("));
                    assert_eq!(spelling(&source, facts.function()), "solve");
                    assert_eq!(facts.tuple(), values.map(DirectComptimeArgument::U32));
                    for (input, (value, formal)) in facts.inputs().iter().zip(values.into_iter().zip(["left", "right"])) {
                        assert_eq!(spelling(&source, input.formal()), formal);
                        assert_eq!(spelling(&source, input.read()), formal);
                        assert_eq!(spelling(&source, input.argument()), value.to_string());
                    }
                    let args = facts.inputs();
                    assert_eq!(args[0].argument().range.start < args[1].argument().range.start, permutation == [0, 1]);
                }
                assert_eq!(forwarded.evaluators().len(), 3);
                for (eval, [left, right]) in forwarded.evaluators().iter().zip([expected[0], expected[1], expected[3]]) {
                    let root = fresh(&format!("u32 answer = comptime value {{ {right} ^ {left} }};"));
                    let ordinary = super::super::derive_closed_comptime_fragment(&root, "value", layout, |_| None).unwrap();
                    assert_eq!(eval.llvm().to_text(), ordinary.llvm().to_text());
                }
                assert!(matches!(derive_direct_pair_startup_comptime_plan(&source, "value", layout),
                    Err(ClosedComptimeRejection::Unsupported {
                        feature: "each authored startup item must be one ordinary binding", ..
                    })));
            }
        }
    }

    #[test]
    fn forwarded_pair_refuses_preprocessing_and_a_call_that_is_not_the_whole_tail() {
        for (body, expected) in [
            ("solve(left ^ right, right)", "forwarding must pass each exact formal without intervening computation"),
            ("solve(right, left) ^ left", "forwarder must contain only one unconditional direct call"),
        ] {
            let source = fresh(&format!(
                "{PAIRED_FUNCTION}\nu32(u32, u32) relay = fn(left, right) {{ {body} }};\n\
                 u32 answer = relay(7, 9);"
            ));
            assert!(source.result().is_valid(), "ordinary authored wrapper: {body}");
            assert!(matches!(derive_forwarded_pair_startup_comptime_plan(&source, "value", ScalarTargetLayout::WASM32),
                Err(ClosedComptimeRejection::Unsupported { feature, .. }) if feature == expected),
                "did not require exact unconditional forwarding: {body}");
        }
    }

    #[test]
    fn forwarded_pair_refuses_a_late_initializer_with_a_different_resolved_target() {
        let source = fresh(&format!(
            "{PAIRED_FUNCTION}\nu32(u32, u32) relay = fn(left, right) {{ solve(right, left) }};\n\
             u32 first = relay(7, 9);\nu32 second = solve(7, 9);"
        ));
        assert!(source.result().is_valid());
        assert!(matches!(derive_forwarded_pair_startup_comptime_plan(&source, "value", ScalarTargetLayout::WASM32),
            Err(ClosedComptimeRejection::Unsupported { feature: "initializer did not select the authored function", .. })));
    }

    #[test]
    fn paired_startup_refuses_a_missing_formal_and_a_late_nonliteral_operand() {
        let layout = ScalarTargetLayout::WASM32;
        let repeated_read = fresh("u32(u32, u32) solve = fn(left, right) { comptime value { left ^ left } };\nu32 first = solve(7, 9);");
        assert!(repeated_read.result().is_valid());
        let Err(ClosedComptimeRejection::OrdinaryDiagnostics(diagnostics)) =
            derive_direct_pair_startup_comptime_plan(&repeated_read, "value", layout)
        else { panic!("the real ordinary typer must refuse the unread formal"); };
        assert!(diagnostics.iter().any(|diagnostic| diagnostic.code == "B0009"
            && diagnostic.labels.iter().any(|label| spelling(&repeated_read, &label.span) == "right")));

        let late = fresh(&format!("{PAIRED_FUNCTION}\nu32 first = solve(7, 9);\nu32 later = solve(9, first);"));
        assert!(late.result().is_valid());
        assert!(matches!(derive_direct_pair_startup_comptime_plan(&late, "value", layout),
            Err(ClosedComptimeRejection::Unsupported { feature: "argument must be an exact scalar literal", .. })));

        let short_circuit = fresh("bool(bool, bool) solve = fn(left, right) { comptime value { left && right } };\nbool first = solve(false, true);");
        assert!(short_circuit.result().is_valid());
        assert!(matches!(derive_direct_pair_startup_comptime_plan(&short_circuit, "value", layout),
            Err(ClosedComptimeRejection::Unsupported { feature: "paired comptime requires an audited binary read of both formals", .. })));
    }

    #[test]
    fn direct_comptime_inspection_does_not_guess_values_targets_or_lexical_reads() {
        let layout = ScalarTargetLayout::WASM32;
        for (source, reason) in [
            (
                "i32(i32) solve = fn(input) { comptime value { input } };\ni32 answer = solve(1 | 2);",
                "argument must be an exact scalar literal",
            ),
            (
                "i32 preceding = 1;\ni32(i32) solve = fn(input) { comptime value { input } };\ni32 answer = solve(7);",
                "expected the only tail of one ordinary function",
            ),
            (
                "i32(i32) solve = fn(input) { comptime value { 7 } };\ni32 answer = solve(7);",
                "comptime body must only read its formal",
            ),
        ] {
            let authored = fresh(source);
            assert!(authored.result().is_valid());
            match inspect_direct_function_comptime(&authored, "value", layout) {
                Err(ClosedComptimeRejection::Unsupported { feature, .. }) => assert_eq!(feature, reason),
                wrong => panic!("expected {reason}; got {wrong:?}"),
            }
        }
        let wrong_type = fresh("i32(i32) solve = fn(input) { comptime value { input } };\ni32 answer = solve(true);");
        assert!(wrong_type.result().is_valid());
        assert!(matches!(inspect_direct_function_comptime(&wrong_type, "value", layout),
            Err(ClosedComptimeRejection::OrdinaryDiagnostics(_))));
    }
}
