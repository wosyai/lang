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
    derive_closed_comptime_fragment, derive_direct_function_comptime_fragment, derive_first_source_initializer_comptime,
    ClosedComptimeRejection, ComptimeParameter, InspectedDirectComptimeCall,
    ComptimeParameterRejection, ScalarTargetLayout, ScalarType, CLOSED_COMPTIME_MAX_SOURCE_BYTES,
};
use wosy_protocol::{source_preparation_parts, CstKindEncoding, FileKey, NodeId};
use wosy_syntax::SourceSpan;

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

type SelectedComptime = std::result::Result<
    (RootedClosedComptimeFragment, Option<InspectedDirectComptimeCall>), RootedClosedComptimeRefusal,
>;

impl SourceStore {
    pub(super) fn project_closed_comptime_for_lease(
        &self, lease: &SourceLease, request: RootedClosedComptimeRequest, scope: SiteScope,
    ) -> Result<RootedClosedComptimeProjection> {
        Ok(match self.project_comptime_for_lease(lease, request, scope)? {
            Ok((fragment, None)) => RootedClosedComptimeProjection::Emitted(fragment),
            Ok((_, Some(_))) => return Err("direct argument facts require the direct comptime route".into()),
            Err(reason) => RootedClosedComptimeProjection::Refused(reason),
        })
    }

    pub(super) fn project_direct_function_comptime_for_lease(
        &self, lease: &SourceLease, request: RootedClosedComptimeRequest,
    ) -> Result<RootedDirectFunctionComptimeProjection> {
        Ok(match self.project_comptime_for_lease(lease, request, SiteScope::DirectTargetCall)? {
            Ok((rooted, Some(call))) => RootedDirectFunctionComptimeProjection::Emitted(
                RootedDirectFunctionComptimeFragment { rooted, call }),
            Ok((_, None)) => return Err("verified direct comptime argument was not retained".into()),
            Err(reason) => RootedDirectFunctionComptimeProjection::Refused(reason),
        })
    }

    fn project_comptime_for_lease(
        &self, lease: &SourceLease, request: RootedClosedComptimeRequest, scope: SiteScope,
    ) -> Result<SelectedComptime> {
        let grant = lease.grant();
        let (tx, _read) = self.begin_leased_source(grant, Mode::ReadOnly)?
            .ok_or(BuildRequestRejection::Expired)?;
        let answer = self.project_closed_comptime(&tx, grant.root, request, scope);
        lease.finish_current(|| ()).ok_or(BuildRequestRejection::Expired)?;
        answer
    }

    fn project_closed_comptime(
        &self, tx: &Transaction, root: NodeId, request: RootedClosedComptimeRequest, scope: SiteScope,
    ) -> Result<SelectedComptime> {
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
        let Some(fresh) = verified_authored_parse(file, &config.parser.categories, &checked)? else {
            return Ok(Err(Refusal::AuthoredGrammarNotVerified));
        };
        // SourceConfig has no authoritative consuming profile parameter
        // table. An authored default is not evidence of absence.
        let derived = match scope {
            SiteScope::AuthoredFile => derive_closed_comptime_fragment(&fresh, &request.id, request.evaluator_layout,
                |_| Some(ComptimeParameter::UnsupportedValue)).map(|fragment| (fragment, None)),
            SiteScope::FirstTargetInitializer => derive_first_source_initializer_comptime(&fresh, &request.id,
                request.evaluator_layout).map(|fragment| (fragment, None)),
            SiteScope::DirectTargetCall => derive_direct_function_comptime_fragment(&fresh, &request.id,
                request.evaluator_layout).map(|direct| {
                    let (call, fragment) = direct.into_parts();
                    (fragment, Some(call))
                }),
        };
        let (fragment, direct_call) = match derived {
            Ok(output) => output,
            Err(ClosedComptimeRejection::Parameter { site, name, reason: ComptimeParameterRejection::UnsupportedProfileValue }) =>
                return Ok(Err(Refusal::ParameterAuthorityUnavailable { site, name })),
            Err(reason) => return Ok(Err(Refusal::Compiler(reason))),
        };
        let (bytes, digest) = bounded_partition_bytes(fragment.llvm(), request.output_bytes.min(MAX_EVALUATOR_IR_BYTES))?;
        Ok(Ok((RootedClosedComptimeFragment {
            source_root: root, authored_cst: file.authored, compiler_image: image,
            target: config.target, profile: request.profile, id: fragment.id().into(), site: fragment.site().clone(),
            evaluator_layout: fragment.layout(), result_type: fragment.result_type().clone(), bytes, digest,
        }, direct_call)))
    }
}
