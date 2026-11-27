//! Closed evaluator IR candidates, not specialization or result authority.
use super::{
    build_session::BuildRequestRejection, read_index_using, read_verified_node_with,
    retention_plan::SourceLease, rooted_llvm::bounded_partition_bytes,
    source_proposal::{take, verified_authored_parse},
    source_reconciliation::{bounded_source_records, checked_source_header_using},
    validate_file_using, BaseCstAdmissionLimits, FileTree, Result, SourceStore,
};
use surrealkv::{Mode, Transaction};
use wosy_compiler::{
    derive_closed_comptime_fragment, derive_direct_function_comptime_fragment,
    derive_first_source_initializer_comptime, derive_forwarded_pair_startup_comptime_plan, ClosedComptimeFragment,
    ClosedComptimeRejection, ComptimeParameter, ComptimeParameterRejection, DirectStartupComptimePlan,
    derive_pair_startup_call_graph_comptime_plan, derive_pair_startup_call_graph_wasip1_stdout,
    DirectStartupComptimeUse, InspectedDirectComptimeCall,
    BoundedPairStartupWasip1Output, PairStartupCallGraphEvidence, PairStartupForwarding, ScalarTargetLayout, ScalarType,
    CLOSED_COMPTIME_MAX_SOURCE_BYTES,
};
use wosy_protocol::{source_preparation_parts, CstKindEncoding, FileKey, NodeId};
use wosy_syntax::{FreshParseResult, SourceSpan};

const MAX_SELECTOR_BYTES: usize = 4096;
const MAX_EVALUATOR_IR_BYTES: usize = 1 << 20;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SiteScope {
    AuthoredFile,
    FirstTargetInitializer,
    DirectTargetCall,
}

pub struct RootedClosedComptimeRequest {
    pub profile: String,
    pub file: FileKey,
    pub id: String,
    /// Explicit projection layout, not the ordinary artifact layout and
    /// not a granted or resolved evaluator backend.
    pub evaluator_layout: ScalarTargetLayout,
    pub source_limits: BaseCstAdmissionLimits,
    pub output_bytes: usize,
}

impl RootedClosedComptimeRequest {
    pub(super) fn check_selector_bytes(&self) -> Result<()> {
        let bytes = self.profile.len().saturating_add(self.file.package.len())
            .saturating_add(self.file.path.as_path().as_os_str().as_encoded_bytes().len())
            .saturating_add(self.id.len());
        if bytes > MAX_SELECTOR_BYTES { return Err("closed comptime selector exceeds byte budget".into()); }
        Ok(())
    }
}

#[derive(Debug)]
pub enum RootedClosedComptimeProjection {
    Refused(RootedClosedComptimeRefusal),
    Emitted(RootedClosedComptimeFragment),
}

#[derive(Debug)]
pub enum RootedClosedComptimeRefusal {
    ProfileNotSelected,
    ProfileNotWasip1,
    FileNotRooted,
    NotTargetEntry,
    DirectCallRequiresSingleModule,
    UnsupportedSourceEpoch,
    CompilerImageUnavailable,
    AuthoredGrammarNotVerified,
    ParameterAuthorityUnavailable { site: SourceSpan, name: String },
    Compiler(ClosedComptimeRejection),
}

/// Exact evaluator IR from the leased authored file and local compiler image.
/// The calling route defines placement. This is not a whole-program
/// reachability verdict, an execution command, a result, or a cache entry.
#[derive(Debug)]
pub struct RootedClosedComptimeFragment {
    pub source_root: NodeId,
    pub authored_cst: NodeId,
    pub compiler_image: NodeId,
    pub target: String,
    pub profile: String,
    pub id: String,
    pub site: SourceSpan,
    pub evaluator_layout: ScalarTargetLayout,
    pub result_type: ScalarType,
    pub bytes: Vec<u8>,
    pub digest: NodeId,
}

/// Read-only association of the direct typed call and LLVM with the exact
/// source lease, selected profile and compiler image. No result or cache key.
#[derive(Debug)]
pub struct RootedDirectFunctionComptimeFragment {
    rooted: RootedClosedComptimeFragment,
    call: InspectedDirectComptimeCall,
}

impl RootedDirectFunctionComptimeFragment {
    pub fn rooted(&self) -> &RootedClosedComptimeFragment { &self.rooted }
    pub fn call(&self) -> &InspectedDirectComptimeCall { &self.call }
}

#[derive(Debug)]
pub enum RootedDirectFunctionComptimeProjection {
    Refused(RootedClosedComptimeRefusal),
    Emitted(RootedDirectFunctionComptimeFragment),
}

/// One source-authorized in-memory plan. Its ordered calls and evaluator
/// indices are only meaningful together, for this exact root and profile.
#[derive(Debug)]
pub struct RootedDirectStartupComptimePlan<const ARITY: usize = 1> {
    source: RootedComptimeIdentity,
    id: String,
    evaluator_layout: ScalarTargetLayout,
    calls: Vec<DirectStartupComptimeUse<ARITY>>,
    evaluators: Vec<RootedStartupComptimeEvaluator>,
}

impl<const ARITY: usize> RootedDirectStartupComptimePlan<ARITY> {
    pub fn source_root(&self) -> NodeId { self.source.root }
    pub fn authored_cst(&self) -> NodeId { self.source.authored_cst }
    pub fn compiler_image(&self) -> NodeId { self.source.compiler_image }
    pub fn target(&self) -> &str { &self.source.target }
    pub fn profile(&self) -> &str { &self.source.profile }
    pub fn id(&self) -> &str { &self.id }
    pub fn evaluator_layout(&self) -> ScalarTargetLayout { self.evaluator_layout }
    pub fn calls(&self) -> &[DirectStartupComptimeUse<ARITY>] { &self.calls }
    pub fn evaluators(&self) -> &[RootedStartupComptimeEvaluator] { &self.evaluators }
}

#[derive(Debug)]
pub struct RootedStartupComptimeEvaluator {
    bytes: StartupEvaluatorBytes,
    digest: NodeId,
}

#[derive(Debug)]
enum StartupEvaluatorBytes {
    Detached(Vec<u8>),
    Wasip1(std::sync::Arc<Vec<u8>>),
}

impl RootedStartupComptimeEvaluator {
    pub fn bytes(&self) -> &[u8] {
        match &self.bytes {
            StartupEvaluatorBytes::Detached(bytes) => bytes,
            StartupEvaluatorBytes::Wasip1(bytes) => bytes.as_slice(),
        }
    }
    pub fn digest(&self) -> NodeId { self.digest }
}

#[derive(Debug)]
pub enum RootedDirectStartupComptimeProjection<const ARITY: usize = 1> {
    Refused(RootedClosedComptimeRefusal),
    Emitted(RootedDirectStartupComptimePlan<ARITY>),
}

/// Exact retained entry/ordinary route evidence. The wrapped plan owns all
/// admitted partitions, so no forwarding receipt exists on partial egress.
#[derive(Debug)]
pub struct RootedForwardedPairStartupComptimePlan {
    plan: RootedDirectStartupComptimePlan<2>,
    forwarding: PairStartupForwarding,
}

impl RootedForwardedPairStartupComptimePlan {
    pub fn plan(&self) -> &RootedDirectStartupComptimePlan<2> { &self.plan }
    pub fn forwarding(&self) -> &PairStartupForwarding { &self.forwarding }
}

#[derive(Debug)]
pub enum RootedForwardedPairStartupComptimeProjection {
    Refused(RootedClosedComptimeRefusal),
    Emitted(RootedForwardedPairStartupComptimePlan),
}

/// The compiler's ordered typed startup slots and candidate evaluator
/// text for one pending source scope, not a whole-program or result receipt.
#[derive(Debug)]
pub struct RootedPairCallGraphComptimePlan {
    source: RootedComptimeIdentity,
    typed: PairCallGraphDelivery,
    evaluators: Vec<RootedStartupComptimeEvaluator>,
}

#[derive(Debug)]
enum PairCallGraphDelivery {
    Detached(PairStartupCallGraphEvidence),
    Wasip1(BoundedPairStartupWasip1Output),
}

impl RootedPairCallGraphComptimePlan {
    pub fn source_root(&self) -> NodeId { self.source.root }
    pub fn authored_cst(&self) -> NodeId { self.source.authored_cst }
    pub fn compiler_image(&self) -> NodeId { self.source.compiler_image }
    pub fn target(&self) -> &str { &self.source.target }
    pub fn profile(&self) -> &str { &self.source.profile }
    pub fn id(&self) -> &str { self.evidence().site_id() }
    pub fn evidence(&self) -> &PairStartupCallGraphEvidence {
        match &self.typed {
            PairCallGraphDelivery::Detached(evidence) => evidence,
            PairCallGraphDelivery::Wasip1(plan) => plan.evidence(),
        }
    }
    /// Present only for the explicitly requested preview1 stdout delivery.
    pub fn max_wasip1_stdout(&self) -> Option<usize> { self.wasip1_output().map(|plan| plan.max_stdout()) }
    pub fn wasip1_output(&self) -> Option<&BoundedPairStartupWasip1Output> {
        match &self.typed {
            PairCallGraphDelivery::Wasip1(plan) => Some(plan),
            PairCallGraphDelivery::Detached(_) => None,
        }
    }
    pub fn evaluators(&self) -> &[RootedStartupComptimeEvaluator] { &self.evaluators }
}

#[derive(Debug)]
pub enum RootedPairCallGraphComptimeProjection {
    Refused(RootedClosedComptimeRefusal),
    Emitted(RootedPairCallGraphComptimePlan),
}

type StartupComptimeDerivation<const ARITY: usize> =
    fn(&FreshParseResult, &str, ScalarTargetLayout)
        -> std::result::Result<DirectStartupComptimePlan<ARITY>, ClosedComptimeRejection>;

type SelectedComptime<T> = std::result::Result<T, RootedClosedComptimeRefusal>;

#[derive(Debug)]
struct RootedComptimeIdentity {
    root: NodeId,
    authored_cst: NodeId,
    compiler_image: NodeId,
    target: String,
    profile: String,
}

impl RootedComptimeIdentity {
    fn fragment(self, fragment: ClosedComptimeFragment, output_bytes: usize) -> Result<RootedClosedComptimeFragment> {
        let (bytes, digest) = bounded_partition_bytes(fragment.llvm(), output_bytes.min(MAX_EVALUATOR_IR_BYTES))?;
        Ok(RootedClosedComptimeFragment {
            source_root: self.root, authored_cst: self.authored_cst, compiler_image: self.compiler_image,
            target: self.target, profile: self.profile, id: fragment.id().into(), site: fragment.site().clone(),
            evaluator_layout: fragment.layout(), result_type: fragment.result_type().clone(), bytes, digest,
        })
    }
}

struct VerifiedComptimeSource {
    source: RootedComptimeIdentity,
    fresh: FreshParseResult,
}

fn compiler_refusal(reason: ClosedComptimeRejection) -> RootedClosedComptimeRefusal {
    match reason {
        ClosedComptimeRejection::Parameter { site, name, reason: ComptimeParameterRejection::UnsupportedProfileValue } =>
            RootedClosedComptimeRefusal::ParameterAuthorityUnavailable { site, name },
        other => RootedClosedComptimeRefusal::Compiler(other),
    }
}

fn first_entry_candidate(
    identity: RootedComptimeIdentity, fresh: &FreshParseResult, request: &RootedClosedComptimeRequest,
) -> Result<SelectedComptime<RootedClosedComptimeFragment>> {
    Ok(match derive_first_source_initializer_comptime(fresh, &request.id, request.evaluator_layout) {
        Ok(fragment) => Ok(identity.fragment(fragment, request.output_bytes)?),
        Err(reason) => Err(compiler_refusal(reason)),
    })
}

fn closed_projection(answer: SelectedComptime<RootedClosedComptimeFragment>) -> RootedClosedComptimeProjection {
    match answer {
        Ok(fragment) => RootedClosedComptimeProjection::Emitted(fragment),
        Err(reason) => RootedClosedComptimeProjection::Refused(reason),
    }
}

fn bounded_evaluators(
    fragments: Vec<ClosedComptimeFragment>, output_bytes: usize,
) -> Result<Vec<RootedStartupComptimeEvaluator>> {
    let mut evaluators = Vec::with_capacity(fragments.len());
    let mut remaining = output_bytes.min(MAX_EVALUATOR_IR_BYTES);
    for fragment in fragments {
        let (bytes, digest) = bounded_partition_bytes(fragment.llvm(), remaining)?;
        remaining -= bytes.len();
        evaluators.push(RootedStartupComptimeEvaluator { bytes: StartupEvaluatorBytes::Detached(bytes), digest });
    }
    Ok(evaluators)
}

impl SourceStore {
    fn pending_comptime_identity(
        &self, root: NodeId, authored_cst: NodeId, target: String, request: &RootedClosedComptimeRequest,
    ) -> Option<RootedComptimeIdentity> {
        let compiler_image = self.derivation_image()?;
        Some(RootedComptimeIdentity { root, authored_cst, compiler_image, target, profile: request.profile.clone() })
    }

    /// Textual evaluator projection from the selected file's already-verified
    /// parse. The caller owns the exact proposal transaction and selector checks.
    pub(super) fn project_first_comptime_from_pending_parse(
        &self, root: NodeId, authored_cst: NodeId, target: String,
        request: &RootedClosedComptimeRequest, fresh: &FreshParseResult,
    ) -> Result<RootedClosedComptimeProjection> {
        let Some(identity) = self.pending_comptime_identity(root, authored_cst, target, request) else {
            return Ok(RootedClosedComptimeProjection::Refused(RootedClosedComptimeRefusal::CompilerImageUnavailable));
        };
        first_entry_candidate(identity, fresh, request).map(closed_projection)
    }

    /// Project the ordinary finite graph from the same verified pending CST.
    pub(super) fn project_pair_call_graph_from_pending_parse(
        &self, root: NodeId, authored_cst: NodeId, target: String,
        request: &RootedClosedComptimeRequest, fresh: &FreshParseResult, max_wasip1_stdout: Option<usize>,
    ) -> Result<RootedPairCallGraphComptimeProjection> {
        let Some(source) = self.pending_comptime_identity(root, authored_cst, target, request) else {
            return Ok(RootedPairCallGraphComptimeProjection::Refused(RootedClosedComptimeRefusal::CompilerImageUnavailable));
        };
        let derived = match max_wasip1_stdout {
            Some(max_stdout) if request.evaluator_layout == ScalarTargetLayout::WASM32 =>
                derive_pair_startup_call_graph_wasip1_stdout(fresh, &request.id, max_stdout),
            Some(_) => Err(ClosedComptimeRejection::UnsupportedLayout),
            None => derive_pair_startup_call_graph_comptime_plan(fresh, &request.id, request.evaluator_layout),
        };
        let plan = match derived {
            Ok(plan) => plan,
            Err(reason) => return Ok(RootedPairCallGraphComptimeProjection::Refused(compiler_refusal(reason))),
        };
        let (typed, evaluators) = if max_wasip1_stdout.is_some() {
            let plan = plan.into_bounded_wasip1_output(request.output_bytes.min(MAX_EVALUATOR_IR_BYTES))?;
            let evaluators = plan.evaluators().iter().map(|evaluator| RootedStartupComptimeEvaluator {
                bytes: StartupEvaluatorBytes::Wasip1(evaluator.shared_ir()),
                digest: NodeId(*blake3::hash(evaluator.bytes()).as_bytes()),
            }).collect();
            (PairCallGraphDelivery::Wasip1(plan), evaluators)
        } else {
            let (evidence, fragments) = plan.into_parts();
            (PairCallGraphDelivery::Detached(evidence), bounded_evaluators(fragments, request.output_bytes)?)
        };
        Ok(RootedPairCallGraphComptimeProjection::Emitted(RootedPairCallGraphComptimePlan {
            source, typed, evaluators,
        }))
    }

    pub(super) fn project_closed_comptime_for_lease(
        &self, lease: &SourceLease, request: RootedClosedComptimeRequest, scope: SiteScope,
    ) -> Result<RootedClosedComptimeProjection> {
        let answer = self.project_comptime_for_lease(lease, request, scope, |input, request| {
            let derived = match scope {
                // A profile named in SourceConfig is not an authoritative
                // parameter table. Even source defaults cannot be selected.
                SiteScope::AuthoredFile => derive_closed_comptime_fragment(&input.fresh, &request.id,
                    request.evaluator_layout, |_| Some(ComptimeParameter::UnsupportedValue)),
                SiteScope::FirstTargetInitializer => return first_entry_candidate(input.source, &input.fresh, request),
                SiteScope::DirectTargetCall => return Err("direct comptime requires its typed route".into()),
            };
            Ok(match derived {
                Ok(fragment) => Ok(input.source.fragment(fragment, request.output_bytes)?),
                Err(reason) => Err(compiler_refusal(reason)),
            })
        })?;
        Ok(closed_projection(answer))
    }

    pub(super) fn project_direct_function_comptime_for_lease(
        &self, lease: &SourceLease, request: RootedClosedComptimeRequest,
    ) -> Result<RootedDirectFunctionComptimeProjection> {
        let answer = self.project_comptime_for_lease(lease, request, SiteScope::DirectTargetCall, |input, request| {
            let plan = match derive_direct_function_comptime_fragment(&input.fresh, &request.id, request.evaluator_layout) {
                Ok(plan) => plan,
                Err(reason) => return Ok(Err(compiler_refusal(reason))),
            };
            let (call, fragment) = plan.into_parts();
            let rooted = input.source.fragment(fragment, request.output_bytes)?;
            Ok(Ok(RootedDirectFunctionComptimeFragment { rooted, call }))
        })?;
        Ok(match answer {
            Ok(fragment) => RootedDirectFunctionComptimeProjection::Emitted(fragment),
            Err(reason) => RootedDirectFunctionComptimeProjection::Refused(reason),
        })
    }

    pub(super) fn project_direct_startup_comptime_for_lease<const ARITY: usize>(
        &self, lease: &SourceLease, request: RootedClosedComptimeRequest, derive: StartupComptimeDerivation<ARITY>,
    ) -> Result<RootedDirectStartupComptimeProjection<ARITY>> {
        let answer = self.project_startup_comptime_for_lease(lease, request, |fresh, id, layout| {
            derive(fresh, id, layout).map(|plan| ((), plan))
        })?;
        Ok(match answer {
            Ok(((), plan)) => RootedDirectStartupComptimeProjection::Emitted(plan),
            Err(reason) => RootedDirectStartupComptimeProjection::Refused(reason),
        })
    }

    pub(super) fn project_forwarded_pair_startup_comptime_for_lease(
        &self, lease: &SourceLease, request: RootedClosedComptimeRequest,
    ) -> Result<RootedForwardedPairStartupComptimeProjection> {
        let answer = self.project_startup_comptime_for_lease(lease, request, |fresh, id, layout| {
            derive_forwarded_pair_startup_comptime_plan(fresh, id, layout).map(|plan| plan.into_parts())
        })?;
        Ok(match answer {
            Ok((forwarding, plan)) => RootedForwardedPairStartupComptimeProjection::Emitted(
                RootedForwardedPairStartupComptimePlan { plan, forwarding }),
            Err(reason) => RootedForwardedPairStartupComptimeProjection::Refused(reason),
        })
    }

    fn project_startup_comptime_for_lease<const ARITY: usize, Evidence>(
        &self, lease: &SourceLease, request: RootedClosedComptimeRequest,
        derive: impl FnOnce(&FreshParseResult, &str, ScalarTargetLayout)
            -> std::result::Result<(Evidence, DirectStartupComptimePlan<ARITY>), ClosedComptimeRejection>,
    ) -> Result<SelectedComptime<(Evidence, RootedDirectStartupComptimePlan<ARITY>)>> {
        self.project_comptime_for_lease(lease, request, SiteScope::DirectTargetCall, |input, request| {
            let (evidence, plan) = match derive(&input.fresh, &request.id, request.evaluator_layout) {
                Ok(derived) => derived,
                Err(reason) => return Ok(Err(compiler_refusal(reason))),
            };
            let (calls, fragments) = plan.into_parts();
            let evaluators = bounded_evaluators(fragments, request.output_bytes)?;
            Ok(Ok((evidence, RootedDirectStartupComptimePlan {
                source: input.source, id: request.id.clone(), evaluator_layout: request.evaluator_layout,
                calls, evaluators,
            })))
        })
    }

    fn project_comptime_for_lease<T>(
        &self, lease: &SourceLease, request: RootedClosedComptimeRequest, scope: SiteScope,
        project: impl FnOnce(VerifiedComptimeSource, &RootedClosedComptimeRequest) -> Result<SelectedComptime<T>>,
    ) -> Result<SelectedComptime<T>> {
        let grant = lease.grant();
        let (tx, _read) = self.begin_leased_source(grant, Mode::ReadOnly)?
            .ok_or(BuildRequestRejection::Expired)?;
        let answer = self.verified_comptime_source(&tx, grant.root, &request, scope)
            .and_then(|verified| match verified {
                Ok(input) => project(input, &request),
                Err(reason) => Ok(Err(reason)),
            });
        lease.finish_current(|| ()).ok_or(BuildRequestRejection::Expired)?;
        answer
    }

    fn verified_comptime_source(
        &self, tx: &Transaction, root: NodeId, request: &RootedClosedComptimeRequest, scope: SiteScope,
    ) -> Result<SelectedComptime<VerifiedComptimeSource>> {
        use RootedClosedComptimeRefusal as Refusal;

        let limits = request.source_limits;
        let mut records = bounded_source_records(tx, limits);
        let (top, config) = checked_source_header_using(root, &mut records)?;
        let parts = source_preparation_parts(&top).expect("checked source preparation");
        if parts.kind_encoding() != CstKindEncoding::Current {
            return Ok(Err(Refusal::UnsupportedSourceEpoch));
        }
        if !config.profiles.contains_key(&request.profile) {
            return Ok(Err(Refusal::ProfileNotSelected));
        }
        let Some(image) = self.derivation_image() else {
            return Ok(Err(Refusal::CompilerImageUnavailable));
        };
        let mut imports = limits.graph_edges;
        let index = read_index_using(top, records, |count| {
            take(&mut imports, count, "closed comptime source index exceeds the import budget")
        })?;
        let Some(file) = index.files.get(&request.file) else { return Ok(Err(Refusal::FileNotRooted)) };
        if scope != SiteScope::AuthoredFile && request.file != config.entry {
            return Ok(Err(Refusal::NotTargetEntry));
        }
        if scope == SiteScope::DirectTargetCall && index.files.len() != 1 {
            return Ok(Err(Refusal::DirectCallRequiresSingleModule));
        }
        if file.metadata.bytes as usize > CLOSED_COMPTIME_MAX_SOURCE_BYTES {
            return Ok(Err(Refusal::Compiler(ClosedComptimeRejection::SourceLimit)));
        }
        let mut work = limits.cst_work();
        let checked = validate_file_using(file, &config.parser.categories, FileTree::Authored,
            CstKindEncoding::Current, Some(&mut work), |id, work| {
                read_verified_node_with(tx, id, |bytes| work.expect("bounded authored expansion").read(bytes))
            })?;
        let Some((fresh, _)) = verified_authored_parse(file, &config.parser.categories, &checked)? else {
            return Ok(Err(Refusal::AuthoredGrammarNotVerified));
        };
        Ok(Ok(VerifiedComptimeSource {
            source: RootedComptimeIdentity {
                root, authored_cst: file.authored, compiler_image: image,
                target: config.target, profile: request.profile.clone(),
            },
            fresh,
        }))
    }
}
