use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use num_bigint::BigInt;
use rowan::NodeOrToken;
use serde::{Deserialize, Serialize};
use wosy_syntax::{
    ByteSpan, CanonicalCstRoot, CstNode, CstToken, ParseResult, SourceIdentity, SourceSpan,
    SyntaxKind,
};

use super::{Diagnostic, DiagnosticLabel, DiagnosticLabelKind, DiagnosticSeverity};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScalarType {
    Unit,
    Bool,
    I8,
    I16,
    I32,
    I64,
    I128,
    U8,
    U16,
    U32,
    U64,
    U128,
    F32,
    F64,
    Char,
    ArtifactId,
    RawPointer(Box<ScalarType>),
    CheckedReference {
        mutability: ScalarReferenceMutability,
        inner: Box<ScalarType>,
    },
    Callable {
        outputs: ScalarOutputSequence,
        parameters: Vec<ScalarType>,
    },
    Named {
        name: String,
        span: ByteSpan,
    },
    Qualified {
        receiver: String,
        receiver_span: ByteSpan,
        member: String,
        member_span: ByteSpan,
        span: ByteSpan,
    },
    Struct(ScalarStructId),
    Enum(ScalarEnumId),
    Array {
        element: Box<ScalarType>,
        length: u64,
        length_span: ByteSpan,
        span: ByteSpan,
    },
    RuntimeArray {
        element: Box<ScalarType>,
        span: ByteSpan,
    },
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScalarReferenceMutability {
    Shared,
    Mutable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CopyPolicy {
    Copy,
    Move,
    Deferred,
}

pub fn copy_policy(
    ty: &ScalarType,
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
    generic_parameters: &BTreeSet<String>,
) -> Result<CopyPolicy, ScalarType> {
    match ty {
        ScalarType::Unit
        | ScalarType::Bool
        | ScalarType::I8
        | ScalarType::I16
        | ScalarType::I32
        | ScalarType::I64
        | ScalarType::I128
        | ScalarType::U8
        | ScalarType::U16
        | ScalarType::U32
        | ScalarType::U64
        | ScalarType::U128
        | ScalarType::F32
        | ScalarType::F64
        | ScalarType::Char
        | ScalarType::ArtifactId
        | ScalarType::RawPointer(_)
        | ScalarType::Callable { .. }
        | ScalarType::CheckedReference {
            mutability: ScalarReferenceMutability::Shared,
            ..
        } => Ok(CopyPolicy::Copy),
        ScalarType::Struct(id) => structs
            .iter()
            .find(|structure| structure.id == *id)
            .map(|structure| structure.copy_policy)
            .ok_or_else(|| ty.clone()),
        ScalarType::Enum(id) => enums
            .iter()
            .find(|enumeration| enumeration.id == *id)
            .map(|enumeration| enumeration.copy_policy)
            .ok_or_else(|| ty.clone()),
        ScalarType::Array { element, .. } => {
            copy_policy(element, structs, enums, generic_parameters)
        }
        ScalarType::CheckedReference {
            mutability: ScalarReferenceMutability::Mutable,
            ..
        }
        | ScalarType::RuntimeArray { .. } => Ok(CopyPolicy::Move),
        ScalarType::Named { name, .. } if generic_parameters.contains(name) => {
            Ok(CopyPolicy::Deferred)
        }
        ScalarType::Named { .. } | ScalarType::Qualified { .. } | ScalarType::Error => {
            Err(ty.clone())
        }
    }
}

fn resolve_copy_policies(
    structs: &mut [ScalarStruct],
    enums: &mut [ScalarEnum],
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    for enumeration in enums.iter_mut() {
        if enumeration.copy_modifier_span.is_some() {
            enumeration.copy_policy = CopyPolicy::Copy;
        }
    }
    loop {
        let promotable = structs
            .iter()
            .enumerate()
            .filter(|(_, structure)| {
                structure.copy_modifier_span.is_some()
                    && structure.copy_policy == CopyPolicy::Move
                    && structure.fields.iter().all(|field| {
                        copy_policy(&field.ty, structs, enums, &BTreeSet::new())
                            == Ok(CopyPolicy::Copy)
                    })
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if promotable.is_empty() {
            break;
        }
        for index in promotable {
            structs[index].copy_policy = CopyPolicy::Copy;
        }
    }
    for structure in structs.iter() {
        if let Some(span) = structure.copy_modifier_span {
            if structure.copy_policy == CopyPolicy::Copy {
                continue;
            }
            if structure
                .fields
                .iter()
                .any(|field| copy_policy(&field.ty, structs, enums, &BTreeSet::new()).is_err())
            {
                continue;
            }
            diagnostics.push(super::Diagnostic {
                code: "B0003".to_owned(),
                severity: super::DiagnosticSeverity::Error,
                message: "copy declaration contains a move-only member".to_owned(),
                labels: vec![super::DiagnosticLabel {
                    kind: super::DiagnosticLabelKind::Primary,
                    span: SourceSpan::new(structure.id.source.clone(), span),
                    message: "copy declaration contains a move-only member".to_owned(),
                }],
                notes: Vec::new(),
            });
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReferencePlaceId {
    pub source: SourceIdentity,
    pub function_span: ByteSpan,
    pub declaration_span: ByteSpan,
    pub block_span: ByteSpan,
    pub binding: ReferenceBindingId,
    pub projections: Vec<ReferencePlaceProjection>,
    pub place: ScalarPlace,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ReferenceBindingId {
    pub source: SourceIdentity,
    pub function_span: ByteSpan,
    pub declaration_span: ByteSpan,
    pub block_span: ByteSpan,
    pub kind: ReferenceBindingKind,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum ReferenceBindingKind {
    Declared,
    CallOutput { point: CfgPointId, output: usize },
}

impl Ord for ReferenceBindingId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (
            &self.source.project,
            &self.source.package,
            &self.source.path,
            &self.source.revision,
            self.function_span.start,
            self.function_span.end,
            self.declaration_span.start,
            self.declaration_span.end,
            self.block_span.start,
            self.block_span.end,
            &self.kind,
        )
            .cmp(&(
                &other.source.project,
                &other.source.package,
                &other.source.path,
                &other.source.revision,
                other.function_span.start,
                other.function_span.end,
                other.declaration_span.start,
                other.declaration_span.end,
                other.block_span.start,
                other.block_span.end,
                &other.kind,
            ))
    }
}

impl PartialOrd for ReferenceBindingId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReferencePlaceProjection {
    Field(ScalarFieldReference),
    Index(ScalarExpression),
    Dereference(Box<ReferenceLoanId>),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReferenceLoanId {
    pub mode: ScalarReferenceMutability,
    pub origin_place: ReferencePlaceId,
    pub creation_span: ByteSpan,
    pub parent: Option<Box<ReferenceLoanId>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReturnedAllocationId {
    pub source: SourceIdentity,
    pub function_span: ByteSpan,
    pub declaration_span: ByteSpan,
    pub creation_span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReferenceOrigin {
    Fresh {
        allocation: ReturnedAllocationId,
        loan: ReferenceLoanId,
    },
    BorrowedFrom {
        parameter: usize,
        loan: ReferenceLoanId,
    },
    Null {
        span: ByteSpan,
    },
    Invalid {
        origin_span: ByteSpan,
        conflict_span: ByteSpan,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReferenceFlowPoint {
    pub id: CfgPointId,
    pub kind: CfgPointKind,
    pub source_span: SourceSpan,
    pub scope: ReferenceScopeId,
    pub possible_origins: Option<BTreeSet<ReferenceOriginId>>,
    pub successors: Vec<CfgPointId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct ReferenceOriginId(pub usize);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct CfgPointId(pub usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReferenceScopeId(pub usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CoreOperationId {
    Alloc,
    Free,
    Invalidate,
    Rebind,
    SystemPanic,
    ThisArtifactId,
    DeclareArtifact,
    IntTrunc,
    IntExtend,
    UintToFloat,
    SintToFloat,
    FloatToSintTrunc,
    FloatToUintTrunc,
    FloatTrunc,
    FloatExtend,
    Bitcast,
    Offset,
    Load,
    PointerCast,
}

impl CoreOperationId {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "alloc" => Self::Alloc,
            "free" => Self::Free,
            "invalidate" => Self::Invalidate,
            "rebind" => Self::Rebind,
            "system_panic" => Self::SystemPanic,
            "this_artifact_id" => Self::ThisArtifactId,
            "declare_artifact" => Self::DeclareArtifact,
            "int_trunc" => Self::IntTrunc,
            "int_extend" => Self::IntExtend,
            "uint_to_float" => Self::UintToFloat,
            "sint_to_float" => Self::SintToFloat,
            "float_to_sint_trunc" => Self::FloatToSintTrunc,
            "float_to_uint_trunc" => Self::FloatToUintTrunc,
            "float_trunc" => Self::FloatTrunc,
            "float_extend" => Self::FloatExtend,
            "bitcast" => Self::Bitcast,
            "offset" => Self::Offset,
            "load" => Self::Load,
            "pointer_cast" => Self::PointerCast,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ConcreteCallSelection {
    Declaration,
    Overload(ScalarOverloadSelection),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ResolvedCallTarget {
    Core(CoreOperationId),
    LocalCallable(ReferenceBindingId),
    ModuleCallable {
        source: SourceIdentity,
        declaration_span: ByteSpan,
        concrete: ConcreteCallSelection,
    },
    ExternCallable {
        source: SourceIdentity,
        declaration_span: ByteSpan,
    },
    CallableField {
        receiver_place: ReferencePlaceId,
        field: ScalarStructFieldId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResolvedCallSite {
    source_span: SourceSpan,
    target: ResolvedCallTarget,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum InvalidationInputs {
    Invalidate,
    Free,
    Rebind {
        raw_address: ScalarExpression,
        raw_span: SourceSpan,
        length: ScalarExpression,
        length_span: SourceSpan,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InvalidationCandidate {
    pub origin: ReferenceOriginId,
    pub place: ReferencePlaceId,
    pub loan: ReferenceLoanId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReleaseTarget {
    Raw {
        binding: ReferenceBindingId,
    },
    Checked {
        address_point: CfgPointId,
        candidates: Vec<InvalidationCandidate>,
        address_span: SourceSpan,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CfgPointKind {
    Entry,
    ScopeEnter,
    ScopeExit,
    Evaluate(ScalarExpression),
    Borrow {
        place: ReferencePlaceId,
        loan: ReferenceLoanId,
    },
    Bind {
        binding: ReferenceBindingId,
    },
    Assign {
        target: ReferencePlaceId,
        rhs_point: CfgPointId,
        previous_origins: BTreeSet<ReferenceOriginId>,
        previous_loans: Vec<ReferenceLoanId>,
    },
    AssignThrough {
        place: ScalarPlace,
        pointer_point: CfgPointId,
        rhs_point: CfgPointId,
        candidates: Vec<InvalidationCandidate>,
        previous_origins: BTreeSet<ReferenceOriginId>,
        previous_loans: Vec<ReferenceLoanId>,
    },
    Read {
        binding: ReferenceBindingId,
    },
    ReadPlace {
        place: ScalarPlace,
        pointer_point: Option<CfgPointId>,
        candidates: Vec<ReferencePlaceId>,
        reads_origin: bool,
    },
    Call {
        expression: ScalarExpression,
        output: usize,
        target: ResolvedCallTarget,
        argument_points: Vec<CfgPointId>,
    },
    Move {
        place: ReferencePlaceId,
        ty: ScalarType,
        owner: ReferenceBindingId,
    },
    DeferredMove {
        place: ReferencePlaceId,
        ty: ScalarType,
        owner: ReferenceBindingId,
    },
    DeferredArrayElement {
        place: ScalarPlace,
        ty: ScalarType,
    },
    MoveThrough {
        place: ScalarPlace,
        ty: ScalarType,
        pointer_point: CfgPointId,
        candidates: Vec<InvalidationCandidate>,
    },
    DeferredMoveThrough {
        place: ScalarPlace,
        ty: ScalarType,
        pointer_point: CfgPointId,
        candidates: Vec<InvalidationCandidate>,
    },
    Release {
        target: ReleaseTarget,
    },
    Invalidate {
        address_point: CfgPointId,
        candidates: Vec<InvalidationCandidate>,
        operation: CoreOperationId,
        address_span: SourceSpan,
        inputs: InvalidationInputs,
    },
    Branch {
        condition: ScalarExpression,
    },
    Join,
    LoopTest {
        condition: ScalarExpression,
    },
    ReturnOutput {
        output: usize,
        value_point: CfgPointId,
        checked: bool,
    },
    Return,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarReferenceCfg {
    pub facts: Vec<ReferenceOrigin>,
    pub points: Vec<ReferenceFlowPoint>,
    pub initial_points: Vec<ReferenceFlowPoint>,
    pub allocations: BTreeSet<ReferenceBindingId>,
    #[serde(skip)]
    pub specialized_policies: Vec<SpecializedCopyPolicy>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpecializedCopyPolicy {
    pub call_point: CfgPointId,
    pub template_source: SourceIdentity,
    pub template_function_span: ByteSpan,
    pub template_point: CfgPointId,
    pub ty: ScalarType,
    pub policy: CopyPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReferenceAnalysisErrorReason {
    MissingResolvedBinding,
    MissingResolvedCallTarget,
    MissingCheckedReferenceOrigin,
    MissingRequiredDereferenceOrigin,
    MoveFromArrayElement,
    MovedPlace,
}

impl ReferenceAnalysisErrorReason {
    fn message(self) -> &'static str {
        match self {
            Self::MissingResolvedBinding => "missing resolved binding",
            Self::MissingResolvedCallTarget => "missing resolved call target",
            Self::MissingCheckedReferenceOrigin => "missing checked-reference origin",
            Self::MissingRequiredDereferenceOrigin => "missing required dereference origin",
            Self::MoveFromArrayElement => "array elements cannot be moved individually",
            Self::MovedPlace => "read or move of unavailable place",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReferenceAnalysisError {
    MissingBinding {
        source_span: SourceSpan,
        name: String,
    },
    MissingOrigin {
        source_span: SourceSpan,
        binding_id: ReferenceBindingId,
        reason: ReferenceAnalysisErrorReason,
    },
    MissingContext {
        source_span: SourceSpan,
        reason: ReferenceAnalysisErrorReason,
    },
    ConflictingPlace {
        source_span: SourceSpan,
        origins: Vec<SourceSpan>,
    },
}

impl ReferenceAnalysisError {
    fn source_span(&self) -> &SourceSpan {
        match self {
            Self::MissingBinding { source_span, .. }
            | Self::MissingOrigin { source_span, .. }
            | Self::MissingContext { source_span, .. }
            | Self::ConflictingPlace { source_span, .. } => source_span,
        }
    }

    fn reason(&self) -> ReferenceAnalysisErrorReason {
        match self {
            Self::MissingBinding { .. } => ReferenceAnalysisErrorReason::MissingResolvedBinding,
            Self::MissingOrigin { reason, .. } | Self::MissingContext { reason, .. } => *reason,
            Self::ConflictingPlace { .. } => ReferenceAnalysisErrorReason::MovedPlace,
        }
    }

    fn diagnostic(&self) -> super::Diagnostic {
        let message = self.reason().message().to_owned();
        let mut labels = vec![super::DiagnosticLabel {
            kind: super::DiagnosticLabelKind::Primary,
            span: self.source_span().clone(),
            message: message.clone(),
        }];
        if let Self::ConflictingPlace { origins, .. } = self {
            for origin in origins {
                if !labels.iter().any(|label| label.span == *origin) {
                    labels.push(super::DiagnosticLabel {
                        kind: super::DiagnosticLabelKind::Secondary,
                        span: origin.clone(),
                        message: "origin of conflicting place or loan".to_owned(),
                    });
                }
            }
        }
        super::Diagnostic {
            code: "B0003".to_owned(),
            severity: super::DiagnosticSeverity::Error,
            message,
            labels,
            notes: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarStructId {
    pub source: SourceIdentity,
    pub index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct ScalarAllocationIdentity {
    pub binding: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScalarAutomaticReturnResultState {
    Live,
    Transferred,
    Released,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarAutomaticReturnResult {
    pub aggregate_type: ScalarType,
    pub allocation_identity: ScalarAllocationIdentity,
    pub state: ScalarAutomaticReturnResultState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarEnumId {
    pub source: SourceIdentity,
    pub index: usize,
}

impl Ord for ScalarEnumId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (
            &self.source.project,
            &self.source.package,
            &self.source.path,
            &self.source.revision,
            self.index,
        )
            .cmp(&(
                &other.source.project,
                &other.source.package,
                &other.source.path,
                &other.source.revision,
                other.index,
            ))
    }
}

impl PartialOrd for ScalarEnumId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScalarStructId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (
            &self.source.project,
            &self.source.package,
            &self.source.path,
            &self.source.revision,
            self.index,
        )
            .cmp(&(
                &other.source.project,
                &other.source.package,
                &other.source.path,
                &other.source.revision,
                other.index,
            ))
    }
}

impl PartialOrd for ScalarStructId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct ScalarStructFieldId {
    pub structure: ScalarStructId,
    pub index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScalarFieldReference {
    Unresolved { name: String, span: ByteSpan },
    Resolved(ScalarStructFieldId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarTargetLayout {
    pub pointer_size: u64,
    pub pointer_alignment: u64,
}

impl ScalarTargetLayout {
    pub const WASM32: Self = Self {
        pointer_size: 4,
        pointer_alignment: 4,
    };

    pub const NATIVE64: Self = Self {
        pointer_size: 8,
        pointer_alignment: 8,
    };
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarOutput {
    pub ty: ScalarType,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarOutputSequence {
    pub outputs: Vec<ScalarOutput>,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScalarPlace {
    Name {
        name: String,
        span: ByteSpan,
    },
    Field {
        base: Box<ScalarPlace>,
        field: ScalarFieldReference,
        span: ByteSpan,
    },
    Dereference {
        pointer: Box<ScalarExpression>,
        span: ByteSpan,
    },
    Index {
        base: Box<ScalarPlace>,
        index: Box<ScalarExpression>,
        span: ByteSpan,
        index_span: ByteSpan,
    },
}

#[derive(Clone, Debug, PartialEq)]
enum ScalarPlaceIdentity {
    Name(String),
    Field(Box<ScalarPlaceIdentity>, ScalarFieldIdentity),
    Dereference(Box<ScalarExpressionIdentity>),
    Index(Box<ScalarPlaceIdentity>, Box<ScalarExpressionIdentity>),
}

#[derive(Clone, Debug, PartialEq)]
enum ScalarFieldIdentity {
    Unresolved(String),
    Resolved(ScalarStructFieldId),
}

#[derive(Clone, Debug, PartialEq)]
enum ScalarExpressionIdentity {
    Name(String),
    Member(String, String),
    Integer(BigInt),
    InvalidInteger,
    Float(f64),
    InvalidFloat,
    Boolean(bool),
    Char(char),
    Utf8(Vec<u8>),
    RawAddress(ScalarPlaceIdentity),
    CheckedAddress(ScalarReferenceMutability, ScalarPlaceIdentity),
    Dereference(ScalarPlaceIdentity),
    IndexedRead(ScalarPlaceIdentity),
    StructLiteral(Vec<(String, ScalarExpressionIdentity)>),
    ArrayLiteral(Vec<ScalarExpressionIdentity>),
    Binary(
        BinaryOperator,
        Box<ScalarExpressionIdentity>,
        Box<ScalarExpressionIdentity>,
    ),
    Unary(UnaryOperator, Box<ScalarExpressionIdentity>),
    Call(
        Option<String>,
        String,
        Vec<ScalarTypeIdentity>,
        Vec<ScalarExpressionIdentity>,
    ),
    If(
        Box<ScalarExpressionIdentity>,
        ScalarBlockIdentity,
        ScalarBlockIdentity,
    ),
    UnitIf(Box<ScalarExpressionIdentity>, ScalarBlockIdentity),
    Block(ScalarBlockIdentity),
}

#[derive(Clone, Debug, PartialEq)]
enum ScalarTypeIdentity {
    Unit,
    Bool,
    I8,
    I16,
    I32,
    I64,
    I128,
    U8,
    U16,
    U32,
    U64,
    U128,
    F32,
    F64,
    Char,
    ArtifactId,
    RawPointer(Box<ScalarTypeIdentity>),
    CheckedReference(ScalarReferenceMutability, Box<ScalarTypeIdentity>),
    Callable(Vec<ScalarTypeIdentity>, Vec<ScalarTypeIdentity>),
    Named(String),
    Qualified(String, String),
    Struct(ScalarStructId),
    Enum(ScalarEnumId),
    Array(Box<ScalarTypeIdentity>, u64),
    RuntimeArray(Box<ScalarTypeIdentity>),
    Error,
}

#[derive(Clone, Debug, PartialEq)]
struct ScalarBlockIdentity {
    items: Vec<ScalarBlockItemIdentity>,
    expressions: Vec<ScalarExpressionIdentity>,
    final_output_values: Vec<(usize, ScalarTypeIdentity, ScalarExpressionIdentity)>,
    unsafe_context: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum ScalarBlockItemIdentity {
    LocalBinding(
        String,
        ScalarTypeIdentity,
        ScalarExpressionIdentity,
        Vec<(String, ScalarTypeIdentity)>,
    ),
    Expression(ScalarExpressionIdentity),
    Assignment(Vec<ScalarPlaceIdentity>, Vec<ScalarExpressionIdentity>),
    While(ScalarExpressionIdentity, ScalarBlockIdentity),
}

fn scalar_place_identity(place: &ScalarPlace) -> ScalarPlaceIdentity {
    match place {
        ScalarPlace::Name { name, .. } => ScalarPlaceIdentity::Name(name.clone()),
        ScalarPlace::Field { base, field, .. } => ScalarPlaceIdentity::Field(
            Box::new(scalar_place_identity(base)),
            match field {
                ScalarFieldReference::Unresolved { name, .. } => {
                    ScalarFieldIdentity::Unresolved(name.clone())
                }
                ScalarFieldReference::Resolved(id) => ScalarFieldIdentity::Resolved(id.clone()),
            },
        ),
        ScalarPlace::Dereference { pointer, .. } => {
            ScalarPlaceIdentity::Dereference(Box::new(scalar_expression_identity(pointer)))
        }
        ScalarPlace::Index { base, index, .. } => ScalarPlaceIdentity::Index(
            Box::new(scalar_place_identity(base)),
            Box::new(scalar_expression_identity(index)),
        ),
    }
}

fn scalar_place_target_span(place: &ScalarPlace) -> ByteSpan {
    match place {
        ScalarPlace::Name { span, .. } | ScalarPlace::Dereference { span, .. } => *span,
        ScalarPlace::Field { base, span, .. } | ScalarPlace::Index { base, span, .. } => {
            ByteSpan::new(scalar_place_target_span(base).start, span.end)
        }
    }
}

fn scalar_expression_identity(expression: &ScalarExpression) -> ScalarExpressionIdentity {
    match expression {
        ScalarExpression::Name { name, .. } => ScalarExpressionIdentity::Name(name.clone()),
        ScalarExpression::Member { receiver, name, .. } => {
            ScalarExpressionIdentity::Member(receiver.clone(), name.clone())
        }
        ScalarExpression::Integer { value, .. } => ScalarExpressionIdentity::Integer(value.clone()),
        ScalarExpression::InvalidInteger { .. } => ScalarExpressionIdentity::InvalidInteger,
        ScalarExpression::Float { value, .. } => ScalarExpressionIdentity::Float(*value),
        ScalarExpression::InvalidFloat { .. } => ScalarExpressionIdentity::InvalidFloat,
        ScalarExpression::Boolean { value, .. } => ScalarExpressionIdentity::Boolean(*value),
        ScalarExpression::Char { value, .. } => ScalarExpressionIdentity::Char(*value),
        ScalarExpression::Utf8 { value, .. } => ScalarExpressionIdentity::Utf8(value.clone()),
        ScalarExpression::RawAddress { place, .. } => {
            ScalarExpressionIdentity::RawAddress(scalar_place_identity(place))
        }
        ScalarExpression::CheckedAddress {
            mutability, place, ..
        } => ScalarExpressionIdentity::CheckedAddress(*mutability, scalar_place_identity(place)),
        ScalarExpression::Dereference { place, .. } => {
            ScalarExpressionIdentity::Dereference(scalar_place_identity(place))
        }
        ScalarExpression::IndexedRead { place, .. } => {
            ScalarExpressionIdentity::IndexedRead(scalar_place_identity(place))
        }
        ScalarExpression::StructLiteral { fields, .. } => ScalarExpressionIdentity::StructLiteral(
            fields
                .iter()
                .map(|field| (field.name.clone(), scalar_expression_identity(&field.value)))
                .collect(),
        ),
        ScalarExpression::ArrayLiteral { elements, .. } => ScalarExpressionIdentity::ArrayLiteral(
            elements.iter().map(scalar_expression_identity).collect(),
        ),
        ScalarExpression::Binary {
            operator,
            left,
            right,
            ..
        } => ScalarExpressionIdentity::Binary(
            operator.clone(),
            Box::new(scalar_expression_identity(left)),
            Box::new(scalar_expression_identity(right)),
        ),
        ScalarExpression::Unary {
            operator, operand, ..
        } => ScalarExpressionIdentity::Unary(
            operator.clone(),
            Box::new(scalar_expression_identity(operand)),
        ),
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            arguments,
            ..
        } => ScalarExpressionIdentity::Call(
            receiver.clone(),
            name.clone(),
            type_arguments
                .iter()
                .map(|argument| scalar_type_identity(&argument.ty))
                .collect(),
            arguments.iter().map(scalar_expression_identity).collect(),
        ),
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => ScalarExpressionIdentity::If(
            Box::new(scalar_expression_identity(condition)),
            scalar_block_identity(then_branch),
            scalar_block_identity(else_branch),
        ),
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => ScalarExpressionIdentity::UnitIf(
            Box::new(scalar_expression_identity(condition)),
            scalar_block_identity(then_branch),
        ),
        ScalarExpression::Block(block) => {
            ScalarExpressionIdentity::Block(scalar_block_identity(block))
        }
    }
}

fn scalar_type_identity(ty: &ScalarType) -> ScalarTypeIdentity {
    match ty {
        ScalarType::Unit => ScalarTypeIdentity::Unit,
        ScalarType::Bool => ScalarTypeIdentity::Bool,
        ScalarType::I8 => ScalarTypeIdentity::I8,
        ScalarType::I16 => ScalarTypeIdentity::I16,
        ScalarType::I32 => ScalarTypeIdentity::I32,
        ScalarType::I64 => ScalarTypeIdentity::I64,
        ScalarType::I128 => ScalarTypeIdentity::I128,
        ScalarType::U8 => ScalarTypeIdentity::U8,
        ScalarType::U16 => ScalarTypeIdentity::U16,
        ScalarType::U32 => ScalarTypeIdentity::U32,
        ScalarType::U64 => ScalarTypeIdentity::U64,
        ScalarType::U128 => ScalarTypeIdentity::U128,
        ScalarType::F32 => ScalarTypeIdentity::F32,
        ScalarType::F64 => ScalarTypeIdentity::F64,
        ScalarType::Char => ScalarTypeIdentity::Char,
        ScalarType::ArtifactId => ScalarTypeIdentity::ArtifactId,
        ScalarType::RawPointer(inner) => {
            ScalarTypeIdentity::RawPointer(Box::new(scalar_type_identity(inner)))
        }
        ScalarType::CheckedReference { mutability, inner } => {
            ScalarTypeIdentity::CheckedReference(*mutability, Box::new(scalar_type_identity(inner)))
        }
        ScalarType::Callable {
            outputs,
            parameters,
        } => ScalarTypeIdentity::Callable(
            outputs
                .outputs
                .iter()
                .map(|output| scalar_type_identity(&output.ty))
                .collect(),
            parameters.iter().map(scalar_type_identity).collect(),
        ),
        ScalarType::Named { name, .. } => ScalarTypeIdentity::Named(name.clone()),
        ScalarType::Qualified {
            receiver, member, ..
        } => ScalarTypeIdentity::Qualified(receiver.clone(), member.clone()),
        ScalarType::Struct(id) => ScalarTypeIdentity::Struct(id.clone()),
        ScalarType::Enum(id) => ScalarTypeIdentity::Enum(id.clone()),
        ScalarType::Array {
            element, length, ..
        } => ScalarTypeIdentity::Array(Box::new(scalar_type_identity(element)), *length),
        ScalarType::RuntimeArray { element, .. } => {
            ScalarTypeIdentity::RuntimeArray(Box::new(scalar_type_identity(element)))
        }
        ScalarType::Error => ScalarTypeIdentity::Error,
    }
}

fn scalar_block_identity(block: &ScalarBlock) -> ScalarBlockIdentity {
    ScalarBlockIdentity {
        items: block
            .items
            .iter()
            .map(|item| match item {
                ScalarBlockItem::LocalBinding(binding) => ScalarBlockItemIdentity::LocalBinding(
                    binding.name.clone(),
                    scalar_type_identity(&binding.declared_type),
                    scalar_expression_identity(&binding.value),
                    binding
                        .receivers
                        .iter()
                        .map(|receiver| (receiver.name.clone(), scalar_type_identity(&receiver.ty)))
                        .collect(),
                ),
                ScalarBlockItem::Expression(expression) => {
                    ScalarBlockItemIdentity::Expression(scalar_expression_identity(expression))
                }
                ScalarBlockItem::Assignment(assignment) => ScalarBlockItemIdentity::Assignment(
                    assignment
                        .targets
                        .iter()
                        .map(|target| scalar_place_identity(&target.place))
                        .collect(),
                    assignment
                        .values
                        .iter()
                        .map(scalar_expression_identity)
                        .collect(),
                ),
                ScalarBlockItem::While(while_expression) => ScalarBlockItemIdentity::While(
                    scalar_expression_identity(&while_expression.condition),
                    scalar_block_identity(&while_expression.body),
                ),
            })
            .collect(),
        expressions: block
            .expressions
            .iter()
            .map(scalar_expression_identity)
            .collect(),
        final_output_values: block
            .final_output_values
            .iter()
            .map(|value| {
                (
                    value.position,
                    scalar_type_identity(&value.ty),
                    scalar_expression_identity(&value.value),
                )
            })
            .collect(),
        unsafe_context: block.unsafe_context,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarLayout {
    pub size: u64,
    pub alignment: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarStructField {
    pub id: ScalarStructFieldId,
    pub name: String,
    pub name_span: ByteSpan,
    pub ty: ScalarType,
    pub declaration_index: usize,
    pub offset: Option<u64>,
    pub layout: Option<ScalarLayout>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarStruct {
    pub id: ScalarStructId,
    pub name: String,
    pub name_span: ByteSpan,
    pub fields: Vec<ScalarStructField>,
    pub copy_policy: CopyPolicy,
    pub copy_modifier_span: Option<ByteSpan>,
    pub span: ByteSpan,
    pub layout: Option<ScalarLayout>,
}

fn resolved_struct_field<'a>(
    receiver_type: &ScalarType,
    field_name: &str,
    structs: &'a [ScalarStruct],
) -> Option<&'a ScalarStructField> {
    let ScalarType::Struct(id) = receiver_type else {
        return None;
    };
    structs
        .iter()
        .find(|structure| structure.id == *id)?
        .fields
        .iter()
        .find(|field| field.name == field_name)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarEnumVariant {
    pub name: String,
    pub name_span: ByteSpan,
    pub tag: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarEnum {
    pub id: ScalarEnumId,
    pub name: String,
    pub name_span: ByteSpan,
    pub variants: Vec<ScalarEnumVariant>,
    pub copy_policy: CopyPolicy,
    pub copy_modifier_span: Option<ByteSpan>,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarOutputReceiver {
    pub name: String,
    pub name_span: ByteSpan,
    pub ty: ScalarType,
    pub allocation_length: Option<ScalarExpression>,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarOutputValue {
    pub position: usize,
    pub ty: ScalarType,
    pub span: ByteSpan,
    pub value: ScalarExpression,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScalarBindingOutputOrigin {
    SingleExpression,
    IndependentExpressions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarStructLiteral {
    pub fields: Vec<ScalarStructLiteralField>,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarStructLiteralField {
    pub name: String,
    pub name_span: ByteSpan,
    pub value: ScalarExpression,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarTypeArgument {
    pub ty: ScalarType,
    pub span: ByteSpan,
}

fn is_error_type(ty: &ScalarType) -> bool {
    matches!(ty, ScalarType::Error)
}

fn scalar_type_equal(left: &ScalarType, right: &ScalarType) -> bool {
    if is_error_type(left) || is_error_type(right) {
        return false;
    }
    match (left, right) {
        (ScalarType::RawPointer(left), ScalarType::RawPointer(right)) => {
            scalar_type_equal(left, right)
        }
        (
            ScalarType::CheckedReference {
                mutability: left_mutability,
                inner: left_inner,
            },
            ScalarType::CheckedReference {
                mutability: right_mutability,
                inner: right_inner,
            },
        ) => left_mutability == right_mutability && scalar_type_equal(left_inner, right_inner),
        (
            ScalarType::Array {
                element: left_element,
                length: left_length,
                ..
            },
            ScalarType::Array {
                element: right_element,
                length: right_length,
                ..
            },
        ) => left_length == right_length && scalar_type_equal(left_element, right_element),
        (
            ScalarType::RuntimeArray { element: left, .. },
            ScalarType::RuntimeArray { element: right, .. },
        ) => scalar_type_equal(left, right),
        (
            ScalarType::RuntimeArray { element: left, .. },
            ScalarType::Array { element: right, .. },
        )
        | (
            ScalarType::Array { element: left, .. },
            ScalarType::RuntimeArray { element: right, .. },
        ) => scalar_type_equal(left, right),
        (ScalarType::Named { name: left, .. }, ScalarType::Named { name: right, .. }) => {
            left == right
        }
        (
            ScalarType::Callable {
                outputs: left_outputs,
                parameters: left_parameters,
            },
            ScalarType::Callable {
                outputs: right_outputs,
                parameters: right_parameters,
            },
        ) => {
            left_outputs.outputs.len() == right_outputs.outputs.len()
                && left_outputs
                    .outputs
                    .iter()
                    .zip(&right_outputs.outputs)
                    .all(|(left, right)| scalar_type_equal(&left.ty, &right.ty))
                && left_parameters.len() == right_parameters.len()
                && left_parameters
                    .iter()
                    .zip(right_parameters)
                    .all(|(left, right)| scalar_type_equal(left, right))
        }
        (ScalarType::RawPointer(_), _) | (_, ScalarType::RawPointer(_)) => false,
        (ScalarType::CheckedReference { .. }, _) | (_, ScalarType::CheckedReference { .. }) => {
            false
        }
        (ScalarType::Callable { .. }, _) | (_, ScalarType::Callable { .. }) => false,
        _ => left == right,
    }
}

fn substitute_type(ty: &ScalarType, substitutions: &BTreeMap<String, ScalarType>) -> ScalarType {
    match ty {
        ScalarType::Named { name, .. } => substitutions
            .get(name)
            .cloned()
            .unwrap_or_else(|| ty.clone()),
        ScalarType::RawPointer(inner) => {
            ScalarType::RawPointer(Box::new(substitute_type(inner, substitutions)))
        }
        ScalarType::CheckedReference { mutability, inner } => ScalarType::CheckedReference {
            mutability: *mutability,
            inner: Box::new(substitute_type(inner, substitutions)),
        },
        ScalarType::Array {
            element,
            length,
            length_span,
            span,
        } => ScalarType::Array {
            element: Box::new(substitute_type(element, substitutions)),
            length: *length,
            length_span: *length_span,
            span: *span,
        },
        ScalarType::Callable {
            outputs,
            parameters,
        } => ScalarType::Callable {
            outputs: ScalarOutputSequence {
                outputs: outputs
                    .outputs
                    .iter()
                    .map(|output| ScalarOutput {
                        ty: substitute_type(&output.ty, substitutions),
                        span: output.span,
                    })
                    .collect(),
                span: outputs.span,
            },
            parameters: parameters
                .iter()
                .map(|parameter| substitute_type(parameter, substitutions))
                .collect(),
        },
        _ => ty.clone(),
    }
}

fn substitute_generic_callable(
    signature: &ScalarType,
    parameters: &[ScalarGenericParameter],
    type_arguments: &[ScalarTypeArgument],
) -> Option<ScalarType> {
    if parameters.is_empty() {
        return Some(signature.clone());
    }
    if parameters.len() != type_arguments.len() {
        return None;
    }
    let substitutions = parameters
        .iter()
        .zip(type_arguments)
        .map(|(parameter, argument)| (parameter.name.clone(), argument.ty.clone()))
        .collect();
    Some(substitute_type(signature, &substitutions))
}

fn infer_overload_substitutions(
    pattern: &ScalarType,
    actual: &ScalarType,
    parameters: &[ScalarGenericParameter],
    substitutions: &mut BTreeMap<String, ScalarType>,
) -> bool {
    if let ScalarType::Named { name, .. } = pattern {
        if parameters.iter().any(|parameter| parameter.name == *name) {
            if let Some(previous) = substitutions.get(name) {
                return scalar_type_equal(previous, actual);
            }
            substitutions.insert(name.clone(), actual.clone());
            return true;
        }
    }
    match (pattern, actual) {
        (ScalarType::RawPointer(pattern), ScalarType::RawPointer(actual)) => {
            infer_overload_substitutions(pattern, actual, parameters, substitutions)
        }
        (
            ScalarType::CheckedReference {
                mutability: pattern_mutability,
                inner: pattern_inner,
            },
            ScalarType::CheckedReference {
                mutability: actual_mutability,
                inner: actual_inner,
            },
        ) if pattern_mutability == actual_mutability => {
            infer_overload_substitutions(pattern_inner, actual_inner, parameters, substitutions)
        }
        (
            ScalarType::Array {
                element: pattern_element,
                length: pattern_length,
                ..
            },
            ScalarType::Array {
                element: actual_element,
                length: actual_length,
                ..
            },
        ) if pattern_length == actual_length => {
            infer_overload_substitutions(pattern_element, actual_element, parameters, substitutions)
        }
        (
            ScalarType::Callable {
                outputs: pattern_outputs,
                parameters: pattern_parameters,
            },
            ScalarType::Callable {
                outputs: actual_outputs,
                parameters: actual_parameters,
            },
        ) => {
            pattern_outputs.outputs.len() == actual_outputs.outputs.len()
                && pattern_parameters.len() == actual_parameters.len()
                && pattern_outputs
                    .outputs
                    .iter()
                    .zip(&actual_outputs.outputs)
                    .all(|(pattern, actual)| {
                        infer_overload_substitutions(
                            &pattern.ty,
                            &actual.ty,
                            parameters,
                            substitutions,
                        )
                    })
                && pattern_parameters
                    .iter()
                    .zip(actual_parameters)
                    .all(|(pattern, actual)| {
                        infer_overload_substitutions(pattern, actual, parameters, substitutions)
                    })
        }
        _ => scalar_type_equal(pattern, actual),
    }
}

#[derive(Clone, Debug)]
enum ScalarOverloadArgument {
    IntegerLiteral,
    Typed(ScalarType),
}

fn infer_overload_argument(
    pattern: &ScalarType,
    argument: &ScalarOverloadArgument,
    parameters: &[ScalarGenericParameter],
    substitutions: &mut BTreeMap<String, ScalarType>,
) -> bool {
    match argument {
        ScalarOverloadArgument::Typed(actual) => {
            infer_overload_substitutions(pattern, actual, parameters, substitutions)
        }
        ScalarOverloadArgument::IntegerLiteral => {
            matches!(pattern, ScalarType::Named { name, .. } if parameters.iter().any(|parameter| parameter.name == *name))
                || is_integer_type(pattern)
        }
    }
}

fn resolve_overload_candidate(
    overload: &ScalarFunction,
    arguments: &[ScalarOverloadArgument],
    expected: Option<&ScalarType>,
) -> Result<(ScalarType, ScalarOverloadSelection), &'static str> {
    let mut matches = Vec::new();
    let mut inconsistent = false;
    let mut output_mismatch = false;
    let mut unresolved_integer_literal = false;
    for (arm_index, arm) in overload.overload_arms.iter().enumerate() {
        let ScalarType::Callable {
            outputs,
            parameters,
        } = &arm.signature
        else {
            continue;
        };
        if parameters.len() != arguments.len() {
            continue;
        }
        let mut substitutions = BTreeMap::new();
        let inputs_match = parameters.iter().zip(arguments).all(|(pattern, argument)| {
            infer_overload_argument(
                pattern,
                argument,
                &arm.generic_parameters,
                &mut substitutions,
            )
        });
        if !inputs_match {
            if !arm.generic_parameters.is_empty() {
                inconsistent = true;
            }
            continue;
        }
        if let Some(expected) = expected {
            let Some(output) = outputs.outputs.first() else {
                continue;
            };
            if !infer_overload_substitutions(
                &output.ty,
                expected,
                &arm.generic_parameters,
                &mut substitutions,
            ) {
                output_mismatch = true;
                continue;
            }
        }
        if arm
            .generic_parameters
            .iter()
            .any(|parameter| !substitutions.contains_key(&parameter.name))
        {
            unresolved_integer_literal |= arguments
                .iter()
                .any(|argument| matches!(argument, ScalarOverloadArgument::IntegerLiteral));
            continue;
        }
        let callable = substitute_type(&arm.signature, &substitutions);
        if let ScalarType::Callable { outputs, .. } = &callable {
            if expected.is_some_and(|expected| {
                outputs
                    .outputs
                    .first()
                    .is_some_and(|output| !scalar_type_equal(&output.ty, expected))
            }) {
                output_mismatch = true;
                continue;
            }
        }
        let ScalarType::Callable { parameters, .. } = &callable else {
            unreachable!("overload arms are callable")
        };
        if !parameters
            .iter()
            .zip(arguments)
            .all(|(parameter, argument)| {
                !matches!(argument, ScalarOverloadArgument::IntegerLiteral)
                    || is_integer_type(parameter)
            })
        {
            continue;
        }
        matches.push((
            callable,
            ScalarOverloadSelection {
                arm_index,
                substitutions,
            },
        ));
    }
    match matches.len() {
        1 => Ok(matches.pop().expect("one overload candidate")),
        0 if output_mismatch => Err("overload output does not match the expected receiver"),
        0 if inconsistent => Err("overload generic substitution is inconsistent"),
        0 if unresolved_integer_literal => {
            Err("integer literal requires a unique overload parameter or output context")
        }
        0 => Err("no overload candidate matches this call"),
        _ => Err("overload call is ambiguous"),
    }
}

fn overload_argument_type(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
) -> Option<ScalarOverloadArgument> {
    match expression {
        ScalarExpression::Name { name, .. } => {
            scope.get(name).cloned().map(ScalarOverloadArgument::Typed)
        }
        ScalarExpression::Integer { .. } | ScalarExpression::InvalidInteger { .. } => {
            Some(ScalarOverloadArgument::IntegerLiteral)
        }
        ScalarExpression::Boolean { .. } => Some(ScalarOverloadArgument::Typed(ScalarType::Bool)),
        ScalarExpression::Char { .. } => Some(ScalarOverloadArgument::Typed(ScalarType::Char)),
        _ => None,
    }
}

fn overload_argument_from_actual(
    expression: &ScalarExpression,
    actual: &ScalarType,
) -> ScalarOverloadArgument {
    if matches!(
        expression,
        ScalarExpression::Integer { .. } | ScalarExpression::InvalidInteger { .. }
    ) {
        ScalarOverloadArgument::IntegerLiteral
    } else {
        ScalarOverloadArgument::Typed(actual.clone())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum UnaryOperator {
    LogicalNot,
    BitwiseNot,
    Negate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ScalarExpression {
    Name {
        name: String,
        span: ByteSpan,
    },
    Member {
        receiver: String,
        name: String,
        enum_tag: Option<u32>,
        receiver_span: ByteSpan,
        name_span: ByteSpan,
        span: ByteSpan,
    },
    Integer {
        value: BigInt,
        span: ByteSpan,
    },
    InvalidInteger {
        span: ByteSpan,
        error_span: Option<ByteSpan>,
    },
    Float {
        value: f64,
        spelling: String,
        span: ByteSpan,
    },
    InvalidFloat {
        span: ByteSpan,
    },
    Boolean {
        value: bool,
        span: ByteSpan,
    },
    Char {
        value: char,
        spelling: String,
        span: ByteSpan,
    },
    Utf8 {
        value: Vec<u8>,
        span: ByteSpan,
    },
    RawAddress {
        place: ScalarPlace,
        span: ByteSpan,
    },
    CheckedAddress {
        mutability: ScalarReferenceMutability,
        place: ScalarPlace,
        span: ByteSpan,
    },
    Dereference {
        place: ScalarPlace,
        span: ByteSpan,
    },
    IndexedRead {
        place: ScalarPlace,
        span: ByteSpan,
    },
    StructLiteral {
        fields: Vec<ScalarStructLiteralField>,
        span: ByteSpan,
    },
    ArrayLiteral {
        elements: Vec<ScalarExpression>,
        span: ByteSpan,
    },
    Binary {
        operator: BinaryOperator,
        left: Box<ScalarExpression>,
        right: Box<ScalarExpression>,
        span: ByteSpan,
    },
    Unary {
        operator: UnaryOperator,
        operand: Box<ScalarExpression>,
        span: ByteSpan,
    },
    Call {
        receiver: Option<String>,
        name: String,
        receiver_span: Option<ByteSpan>,
        name_span: ByteSpan,
        type_arguments: Vec<ScalarTypeArgument>,
        arguments: Vec<ScalarExpression>,
        overload_selection: Option<ScalarOverloadSelection>,
        span: ByteSpan,
    },
    If {
        condition: Box<ScalarExpression>,
        then_branch: ScalarBlock,
        else_branch: ScalarBlock,
        span: ByteSpan,
    },
    UnitIf {
        condition: Box<ScalarExpression>,
        then_branch: ScalarBlock,
        span: ByteSpan,
    },
    Block(ScalarBlock),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarOverloadSelection {
    pub arm_index: usize,
    pub substitutions: BTreeMap<String, ScalarType>,
}

impl Eq for ScalarExpression {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScalarExternModule {
    Valid(String),
    Invalid {
        span: ByteSpan,
        error_span: ByteSpan,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarBlock {
    pub items: Vec<ScalarBlockItem>,
    pub terminated_items: Vec<bool>,
    pub expressions: Vec<ScalarExpression>,
    pub final_output_values: Vec<ScalarOutputValue>,
    pub span: ByteSpan,
    pub unsafe_context: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScalarBlockItem {
    LocalBinding(ScalarBinding),
    Expression(ScalarExpression),
    Assignment(ScalarAssignment),
    While(ScalarWhile),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarAssignment {
    pub receiver: Option<String>,
    pub target: String,
    pub receiver_span: Option<ByteSpan>,
    pub target_span: ByteSpan,
    pub targets: Vec<ScalarAssignmentTarget>,
    pub value: ScalarExpression,
    pub values: Vec<ScalarExpression>,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarAssignmentTarget {
    pub receiver: Option<String>,
    pub target: String,
    pub receiver_span: Option<ByteSpan>,
    pub target_span: ByteSpan,
    pub span: ByteSpan,
    pub place: ScalarPlace,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ResolvedAssignmentOutputId {
    pub source: SourceIdentity,
    pub function_span: ByteSpan,
    pub target_span: ByteSpan,
    pub output_index: usize,
}

fn assignment_function_span(items: &[ScalarItem], assignment_span: ByteSpan) -> Option<ByteSpan> {
    items.iter().find_map(|item| {
        let ScalarItem::Function(function) = item else {
            return None;
        };
        std::iter::once(function)
            .chain(&function.overload_arms)
            .find(|candidate| {
                candidate.body.span.start <= assignment_span.start
                    && assignment_span.end <= candidate.body.span.end
            })
            .map(|candidate| candidate.span)
    })
}

fn record_assignment_outputs(
    facts: &RefCell<HashMap<ResolvedAssignmentOutputId, ScalarType>>,
    source: &SourceIdentity,
    items: &[ScalarItem],
    assignment: &ScalarAssignment,
    outputs: &ScalarOutputSequence,
) {
    let Some(function_span) = assignment_function_span(items, assignment.span) else {
        return;
    };
    for (output_index, (target, output)) in
        assignment.targets.iter().zip(&outputs.outputs).enumerate()
    {
        facts.borrow_mut().insert(
            ResolvedAssignmentOutputId {
                source: source.clone(),
                function_span,
                target_span: target.target_span,
                output_index,
            },
            output.ty.clone(),
        );
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarWhile {
    pub condition: ScalarExpression,
    pub body: ScalarBlock,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarBinding {
    pub name: String,
    pub name_span: ByteSpan,
    pub declared_type: ScalarType,
    pub value: ScalarExpression,
    pub receivers: Vec<ScalarOutputReceiver>,
    pub output_sequence: ScalarOutputSequence,
    pub output_values: Vec<ScalarOutputValue>,
    pub output_origin: ScalarBindingOutputOrigin,
    pub is_allocation: bool,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarNamespace {
    pub binding: String,
    pub package: String,
    pub path: String,
    pub binding_span: ByteSpan,
    pub package_span: ByteSpan,
    pub path_span: ByteSpan,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarFunction {
    pub name: String,
    pub name_span: ByteSpan,
    pub signature: ScalarType,
    pub parameters: Vec<String>,
    pub parameter_spans: Vec<ByteSpan>,
    pub body: ScalarBlock,
    pub span: ByteSpan,
    pub generic_parameters: Vec<ScalarGenericParameter>,
    pub overload_arms: Vec<ScalarFunction>,
    pub reference_cfg: ScalarReferenceCfg,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarGenericParameter {
    pub name: String,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarExternFunction {
    pub name: String,
    pub signature: ScalarType,
    pub unsafe_marker: bool,
    pub name_span: ByteSpan,
    pub signature_span: ByteSpan,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarExtern {
    pub binding: String,
    pub actual_module: ScalarExternModule,
    pub kind: String,
    pub functions: Vec<ScalarExternFunction>,
    pub binding_span: ByteSpan,
    pub module_span: ByteSpan,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScalarItem {
    Namespace(ScalarNamespace),
    Extern(ScalarExtern),
    Binding(ScalarBinding),
    Function(ScalarFunction),
    Executable(ScalarBlockItem),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarProgram {
    pub source: SourceIdentity,
    pub items: Vec<ScalarItem>,
    pub structs: Vec<ScalarStruct>,
    pub enums: Vec<ScalarEnum>,
    pub target_layout: ScalarTargetLayout,
    #[serde(skip)]
    pub resolved_assignment_outputs: RefCell<HashMap<ResolvedAssignmentOutputId, ScalarType>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarNamespaceBinding {
    pub binding: String,
    pub target: SourceIdentity,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarInitializationNode {
    pub item_index: usize,
    pub span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarModule {
    pub source: SourceIdentity,
    pub items: Vec<ScalarItem>,
    pub namespace_bindings: Vec<ScalarNamespaceBinding>,
    pub members: BTreeMap<String, ScalarType>,
    pub initialization_nodes: Vec<ScalarInitializationNode>,
    pub structs: Vec<ScalarStruct>,
    pub enums: Vec<ScalarEnum>,
    pub target_layout: ScalarTargetLayout,
    #[serde(skip)]
    pub resolved_assignment_outputs: RefCell<HashMap<ResolvedAssignmentOutputId, ScalarType>>,
}

impl ScalarModule {
    pub fn new(
        source: SourceIdentity,
        items: Vec<ScalarItem>,
        namespace_bindings: Vec<ScalarNamespaceBinding>,
    ) -> Self {
        let mut members = BTreeMap::new();
        let mut initialization_nodes = Vec::new();
        for (item_index, item) in items.iter().enumerate() {
            let (name, ty, span) = match item {
                ScalarItem::Namespace(_) => continue,
                ScalarItem::Extern(extern_decl) => {
                    for function in &extern_decl.functions {
                        if !is_module_private_name(&function.name) {
                            members.insert(function.name.clone(), function.signature.clone());
                        }
                    }
                    continue;
                }
                ScalarItem::Binding(binding) => {
                    (&binding.name, &binding.declared_type, binding.span)
                }
                ScalarItem::Function(function) => {
                    (&function.name, &function.signature, function.span)
                }
                ScalarItem::Executable(_) => continue,
            };
            if !is_module_private_name(name) {
                members.entry(name.clone()).or_insert_with(|| ty.clone());
            }
            initialization_nodes.push(ScalarInitializationNode { item_index, span });
        }
        Self {
            source,
            items,
            namespace_bindings,
            members,
            initialization_nodes,
            structs: Vec::new(),
            enums: Vec::new(),
            target_layout: ScalarTargetLayout::WASM32,
            resolved_assignment_outputs: RefCell::new(HashMap::new()),
        }
    }

    pub fn from_program(
        program: ScalarProgram,
        namespace_bindings: Vec<ScalarNamespaceBinding>,
    ) -> Self {
        let mut module = Self::new(program.source, program.items, namespace_bindings);
        module.structs = program.structs;
        module.enums = program.enums;
        module.target_layout = program.target_layout;
        module.resolved_assignment_outputs = program.resolved_assignment_outputs;
        module
    }
}

impl ScalarProject {
    pub fn new(modules: Vec<ScalarModule>, initialization_order: Vec<SourceIdentity>) -> Self {
        let mut unique_initialization_order = Vec::new();
        for source in initialization_order {
            if !unique_initialization_order.contains(&source) {
                unique_initialization_order.push(source);
            }
        }
        Self {
            modules,
            initialization_order: unique_initialization_order,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarProject {
    pub modules: Vec<ScalarModule>,
    pub initialization_order: Vec<SourceIdentity>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarProjectValidation {
    pub project: ScalarProject,
    pub diagnostics: Vec<super::Diagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarValidation {
    pub program: ScalarProgram,
    pub diagnostics: Vec<super::Diagnostic>,
}

pub fn derive_scalar_program(parse: &ParseResult) -> ScalarValidation {
    derive_scalar_program_with_layout(parse, ScalarTargetLayout::WASM32)
}

pub fn derive_scalar_program_with_layout(
    parse: &ParseResult,
    target_layout: ScalarTargetLayout,
) -> ScalarValidation {
    derive_scalar_program_from_cst_with_layout(parse.canonical_cst(), target_layout)
}

pub fn derive_scalar_program_from_cst(canonical: &CanonicalCstRoot) -> ScalarValidation {
    derive_scalar_program_from_cst_with_layout(canonical, ScalarTargetLayout::WASM32)
}

pub fn derive_scalar_program_from_cst_with_layout(
    canonical: &CanonicalCstRoot,
    target_layout: ScalarTargetLayout,
) -> ScalarValidation {
    let mut items = Vec::new();
    let mut diagnostics = string_diagnostics(canonical);
    diagnostics.extend(char_diagnostics(canonical));
    diagnostics.extend(integer_diagnostics(canonical));
    diagnostics.extend(float_diagnostics(canonical));
    diagnostics.extend(fixed_array_length_diagnostics(canonical));
    let source_root = canonical
        .root
        .children()
        .find(|node| node.kind() == SyntaxKind::Root)
        .expect("source root");
    let structs = source_root
        .children()
        .chain(source_root.descendants())
        .filter(|node| node.kind() == SyntaxKind::StructDecl)
        .enumerate()
        .map(|(index, node)| {
            derive_struct(
                node,
                ScalarStructId {
                    source: canonical.source.clone(),
                    index,
                },
                target_layout,
            )
        })
        .collect();
    let enums = source_root
        .children()
        .chain(source_root.descendants())
        .filter(|node| node.kind() == SyntaxKind::EnumDecl)
        .enumerate()
        .map(|(index, node)| {
            derive_enum(
                node,
                ScalarEnumId {
                    source: canonical.source.clone(),
                    index,
                },
            )
        })
        .collect();
    let mut generic_function = false;
    for node in source_root.children() {
        match node.kind() {
            SyntaxKind::NamespaceDecl => items.push(ScalarItem::Namespace(derive_namespace(&node))),
            SyntaxKind::ExternDecl => items.push(ScalarItem::Extern(derive_extern(&node))),
            SyntaxKind::BindingDecl => items.push(ScalarItem::Binding(derive_binding(&node))),
            SyntaxKind::FunctionDecl if !generic_function => {
                items.push(ScalarItem::Function(derive_function(&node)))
            }
            SyntaxKind::OverloadDecl => items.push(ScalarItem::Function(derive_overload(&node))),
            SyntaxKind::GenericFunctionDecl => {
                items.push(ScalarItem::Function(derive_generic_function(&node)));
                generic_function = true;
            }
            SyntaxKind::Item => {
                let item_children = node.children().collect::<Vec<_>>();
                let has_generic_function = item_children
                    .iter()
                    .any(|item| item.kind() == SyntaxKind::GenericFunctionDecl);
                for item in item_children {
                    match item.kind() {
                        SyntaxKind::NamespaceDecl => {
                            items.push(ScalarItem::Namespace(derive_namespace(&item)))
                        }
                        SyntaxKind::ExternDecl => {
                            items.push(ScalarItem::Extern(derive_extern(&item)))
                        }
                        SyntaxKind::BindingDecl => {
                            items.push(ScalarItem::Binding(derive_binding(&item)))
                        }
                        SyntaxKind::FunctionDecl if !has_generic_function => {
                            items.push(ScalarItem::Function(derive_function(&item)))
                        }
                        SyntaxKind::OverloadDecl => {
                            items.push(ScalarItem::Function(derive_overload(&item)))
                        }
                        SyntaxKind::GenericFunctionDecl => {
                            items.push(ScalarItem::Function(derive_generic_function(&item)))
                        }
                        SyntaxKind::TopLevelItem => {
                            items.push(ScalarItem::Executable(derive_executable_item(&item)))
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        if node.kind() != SyntaxKind::GenericFunctionDecl {
            generic_function = false;
        }
    }
    let mut program = ScalarProgram {
        source: canonical.source.clone(),
        items,
        structs,
        enums,
        target_layout,
        resolved_assignment_outputs: RefCell::new(HashMap::new()),
    };
    resolve_program_types(&mut program, &mut diagnostics);
    resolve_program_places(&mut program);
    let mut validation_diagnostics = validate(&program);
    let mut diagnostics = diagnostics;
    diagnostics.append(&mut validation_diagnostics);
    if diagnostics.is_empty() {
        record_overload_selections(&mut program);
    }
    if diagnostics.is_empty() {
        let module = ScalarModule::from_program(program.clone(), Vec::new());
        let call_sites = resolve_module_call_sites(&program.source, &program.items, &[module]);
        diagnostics.extend(
            call_sites
                .map(|sites| record_program_reference_origins(&mut program, &sites))
                .unwrap_or_else(|error| vec![error])
                .into_iter()
                .map(|error| error.diagnostic()),
        );
    }
    if diagnostics.is_empty() {
        let module = ScalarModule::from_program(program.clone(), Vec::new());
        diagnostics.extend(
            record_specialized_copy_policies(
                &mut program.items,
                &[module],
                &program.structs,
                &program.enums,
            )
            .into_iter()
            .map(|error| error.diagnostic()),
        );
    }
    ScalarValidation {
        program,
        diagnostics,
    }
}

fn record_overload_selections(program: &mut ScalarProgram) {
    let scope = program
        .items
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
    let overloads = program
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Function(function) if !function.overload_arms.is_empty() => {
                Some((function.name.clone(), function.clone()))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    for item in &mut program.items {
        match item {
            ScalarItem::Binding(binding) => {
                let expected = binding
                    .receivers
                    .first()
                    .map(|receiver| &receiver.ty)
                    .unwrap_or(&binding.declared_type);
                record_expression_overload_selection(
                    &mut binding.value,
                    Some(expected),
                    &scope,
                    &overloads,
                );
            }
            ScalarItem::Function(function) => {
                let mut body_scope = scope.clone();
                if let ScalarType::Callable { parameters, .. } = &function.signature {
                    for (name, ty) in function.parameters.iter().zip(parameters) {
                        body_scope.insert(name.clone(), ty.clone());
                    }
                }
                record_block_overload_selections(&mut function.body, &body_scope, &overloads);
            }
            ScalarItem::Executable(item) => {
                let mut item_scope = scope.clone();
                record_block_item_overload_selections(item, &mut item_scope, &overloads);
            }
            ScalarItem::Namespace(_) | ScalarItem::Extern(_) => {}
        }
    }
}

fn record_expression_overload_selection(
    expression: &mut ScalarExpression,
    expected: Option<&ScalarType>,
    scope: &BTreeMap<String, ScalarType>,
    overloads: &BTreeMap<String, ScalarFunction>,
) {
    match expression {
        ScalarExpression::Call {
            receiver,
            name,
            arguments,
            overload_selection,
            ..
        } => {
            for argument in arguments.iter_mut() {
                record_expression_overload_selection(argument, None, scope, overloads);
            }
            if let (Some(overload), Some(argument_types)) = (
                overloads.get(
                    &receiver
                        .as_ref()
                        .map_or_else(|| name.clone(), |receiver| format!("{receiver}.{name}")),
                ),
                arguments
                    .iter()
                    .map(|argument| overload_argument_type(argument, scope))
                    .collect::<Option<Vec<_>>>(),
            ) {
                if let Ok((_, selection)) =
                    resolve_overload_candidate(overload, &argument_types, expected)
                {
                    *overload_selection = Some(selection);
                }
            }
        }
        ScalarExpression::Binary { left, right, .. } => {
            record_expression_overload_selection(left, None, scope, overloads);
            record_expression_overload_selection(right, None, scope, overloads);
        }
        ScalarExpression::Unary { operand, .. } => {
            record_expression_overload_selection(operand, expected, scope, overloads)
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            record_expression_overload_selection(
                condition,
                Some(&ScalarType::Bool),
                scope,
                overloads,
            );
            record_block_overload_selections(then_branch, scope, overloads);
            record_block_overload_selections(else_branch, scope, overloads);
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            record_expression_overload_selection(
                condition,
                Some(&ScalarType::Bool),
                scope,
                overloads,
            );
            record_block_overload_selections(then_branch, scope, overloads);
        }
        ScalarExpression::Block(block) => record_block_overload_selections(block, scope, overloads),
        ScalarExpression::RawAddress { place, .. }
        | ScalarExpression::CheckedAddress { place, .. }
        | ScalarExpression::Dereference { place, .. } => match place {
            ScalarPlace::Dereference { pointer, .. } => {
                record_expression_overload_selection(pointer, None, scope, overloads)
            }
            ScalarPlace::Field { base, .. } => {
                if let ScalarPlace::Dereference { pointer, .. } = base.as_mut() {
                    record_expression_overload_selection(pointer, None, scope, overloads);
                }
            }
            ScalarPlace::Index { base, index, .. } => {
                record_place_overload_selections(base, scope, overloads);
                record_expression_overload_selection(
                    index,
                    Some(&ScalarType::U64),
                    scope,
                    overloads,
                );
            }
            ScalarPlace::Name { .. } => {}
        },
        ScalarExpression::IndexedRead { place, .. } => {
            record_place_overload_selections(place, scope, overloads)
        }
        ScalarExpression::StructLiteral { fields, .. } => {
            for field in fields {
                record_expression_overload_selection(&mut field.value, None, scope, overloads);
            }
        }
        ScalarExpression::ArrayLiteral { elements, .. } => {
            for element in elements {
                record_expression_overload_selection(element, None, scope, overloads);
            }
        }
        ScalarExpression::Name { .. }
        | ScalarExpression::Member { .. }
        | ScalarExpression::Integer { .. }
        | ScalarExpression::InvalidInteger { .. }
        | ScalarExpression::Float { .. }
        | ScalarExpression::InvalidFloat { .. }
        | ScalarExpression::Boolean { .. }
        | ScalarExpression::Char { .. }
        | ScalarExpression::Utf8 { .. } => {}
    }
}

fn record_place_overload_selections(
    place: &mut ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    overloads: &BTreeMap<String, ScalarFunction>,
) {
    match place {
        ScalarPlace::Name { .. } => {}
        ScalarPlace::Dereference { pointer, .. } => {
            record_expression_overload_selection(pointer, None, scope, overloads)
        }
        ScalarPlace::Field { base, .. } => record_place_overload_selections(base, scope, overloads),
        ScalarPlace::Index { base, index, .. } => {
            record_place_overload_selections(base, scope, overloads);
            record_expression_overload_selection(index, Some(&ScalarType::U64), scope, overloads);
        }
    }
}

fn record_block_overload_selections(
    block: &mut ScalarBlock,
    scope: &BTreeMap<String, ScalarType>,
    overloads: &BTreeMap<String, ScalarFunction>,
) {
    let mut scope = scope.clone();
    for item in &mut block.items {
        record_block_item_overload_selections(item, &mut scope, overloads);
    }
    for output in &mut block.final_output_values {
        record_expression_overload_selection(&mut output.value, None, &scope, overloads);
    }
}

fn record_block_item_overload_selections(
    item: &mut ScalarBlockItem,
    scope: &mut BTreeMap<String, ScalarType>,
    overloads: &BTreeMap<String, ScalarFunction>,
) {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            let expected = binding
                .receivers
                .first()
                .map(|receiver| &receiver.ty)
                .unwrap_or(&binding.declared_type);
            record_expression_overload_selection(
                &mut binding.value,
                Some(expected),
                &scope,
                overloads,
            );
            scope.insert(binding.name.clone(), binding.declared_type.clone());
            for receiver in &binding.receivers {
                scope.insert(receiver.name.clone(), receiver.ty.clone());
            }
        }
        ScalarBlockItem::Expression(expression) => {
            record_expression_overload_selection(expression, None, &scope, overloads)
        }
        ScalarBlockItem::Assignment(assignment) => {
            record_expression_overload_selection(
                &mut assignment.value,
                scope.get(&assignment.target),
                &scope,
                overloads,
            );
            for (value, target) in assignment.values.iter_mut().zip(&assignment.targets) {
                record_expression_overload_selection(
                    value,
                    scope.get(&target.target),
                    &scope,
                    overloads,
                );
            }
        }
        ScalarBlockItem::While(while_expression) => {
            record_block_overload_selections(&mut while_expression.body, &scope, overloads)
        }
    }
}

mod resolution;
use resolution::{
    enum_member_in_module, resolve_module_places, resolve_module_types_in_project,
    resolve_program_places, resolve_program_types, resolve_project_struct_layouts,
};
mod typecheck;
use typecheck::{
    block_item_type_in_module, block_type_in_module, block_type_in_module_expected,
    call_output_sequence_in_module, declare_module_name, expect_module_type, expression_span,
    expression_type_in_module_expected, is_integer_type, is_module_private_name, module_diagnostic,
    operator, unused_binding_spans, validate, validate_extern, validate_identifier_style,
    validate_module_generic_type, validate_module_type, validate_output_receivers_in_module,
    validate_unit_if_positions_in_module,
};
pub fn derive_scalar_diagnostics_from_cst(canonical: &CanonicalCstRoot) -> Vec<super::Diagnostic> {
    let mut diagnostics = string_diagnostics(canonical);
    diagnostics.extend(char_diagnostics(canonical));
    diagnostics.extend(integer_diagnostics(canonical));
    diagnostics.extend(float_diagnostics(canonical));
    diagnostics.extend(fixed_array_length_diagnostics(canonical));
    diagnostics
}

pub fn validate_scalar_project(project: ScalarProject) -> ScalarProjectValidation {
    let mut project = project;
    let mut diagnostics = Vec::new();
    for module in &project.modules {
        module.resolved_assignment_outputs.borrow_mut().clear();
    }
    let struct_lookup = project.modules.clone();
    for module in &mut project.modules {
        resolve_module_types_in_project(module, &struct_lookup);
    }
    resolve_project_struct_layouts(&mut project, &mut diagnostics);
    let mut structs = project
        .modules
        .iter()
        .flat_map(|module| module.structs.clone())
        .collect::<Vec<_>>();
    let mut enums = project
        .modules
        .iter()
        .flat_map(|module| module.enums.clone())
        .collect::<Vec<_>>();
    resolve_copy_policies(&mut structs, &mut enums, &mut diagnostics);
    for module in &mut project.modules {
        for structure in &mut module.structs {
            structure.copy_policy = structs
                .iter()
                .find(|item| item.id == structure.id)
                .expect("project struct policy")
                .copy_policy;
        }
        for enumeration in &mut module.enums {
            enumeration.copy_policy = enums
                .iter()
                .find(|item| item.id == enumeration.id)
                .expect("project enum policy")
                .copy_policy;
        }
    }
    let struct_lookup = project.modules.clone();
    for module in &mut project.modules {
        resolve_module_places(module, &struct_lookup);
    }
    for module in &project.modules {
        validate_unit_if_positions_in_module(module, &mut diagnostics);
        for structure in &module.structs {
            for field in &structure.fields {
                validate_module_type(module, &field.ty, field.name_span, &mut diagnostics);
            }
        }
        let mut declarations = BTreeMap::new();
        let mut declaration_names = BTreeSet::new();
        let mut folded_declarations = BTreeMap::new();
        for item in &module.items {
            let (name, span, ty) = match item {
                ScalarItem::Namespace(namespace) => {
                    validate_identifier_style(
                        module,
                        &namespace.binding,
                        namespace.binding_span,
                        &mut diagnostics,
                    );
                    (&namespace.binding, namespace.span, None)
                }
                ScalarItem::Extern(extern_decl) => {
                    validate_identifier_style(
                        module,
                        &extern_decl.binding,
                        extern_decl.binding_span,
                        &mut diagnostics,
                    );
                    (&extern_decl.binding, extern_decl.span, None)
                }
                ScalarItem::Binding(binding) => {
                    for receiver in &binding.receivers {
                        validate_identifier_style(
                            module,
                            &receiver.name,
                            receiver.name_span,
                            &mut diagnostics,
                        );
                    }
                    (&binding.name, binding.span, Some(&binding.declared_type))
                }
                ScalarItem::Function(function) => {
                    validate_identifier_style(
                        module,
                        &function.name,
                        function.name_span,
                        &mut diagnostics,
                    );
                    (&function.name, function.span, Some(&function.signature))
                }
                ScalarItem::Executable(_) => continue,
            };
            if declare_module_name(
                module,
                name,
                span,
                &mut declaration_names,
                &mut folded_declarations,
                &mut diagnostics,
            ) {
                if let Some(ty) = ty {
                    declarations.insert(name.clone(), ty.clone());
                    if let ScalarItem::Binding(binding) = item {
                        for receiver in &binding.receivers {
                            declarations.insert(receiver.name.clone(), receiver.ty.clone());
                        }
                    }
                    if let ScalarItem::Function(function) = item {
                        if !is_module_private_name(&function.name)
                            && !function.overload_arms.is_empty()
                        {
                            for arm in &function.overload_arms {
                                if arm.generic_parameters.is_empty() {
                                    validate_module_type(
                                        module,
                                        &arm.signature,
                                        arm.span,
                                        &mut diagnostics,
                                    );
                                } else {
                                    validate_module_generic_type(
                                        module,
                                        &arm.signature,
                                        arm.span,
                                        &arm.generic_parameters,
                                        &mut diagnostics,
                                    );
                                }
                            }
                        } else if function.generic_parameters.is_empty() {
                            validate_module_type(module, ty, span, &mut diagnostics);
                        } else {
                            validate_module_generic_type(
                                module,
                                ty,
                                span,
                                &function.generic_parameters,
                                &mut diagnostics,
                            );
                        }
                    } else {
                        validate_module_type(module, ty, span, &mut diagnostics);
                    }
                }
            }
        }
        for item in &module.items {
            if let ScalarItem::Extern(extern_decl) = item {
                validate_extern(module, extern_decl, &mut diagnostics);
            }
        }
        for item in &module.items {
            match item {
                ScalarItem::Namespace(_) | ScalarItem::Extern(_) => {}
                ScalarItem::Binding(binding) => {
                    if binding.is_allocation {
                        for receiver in &binding.receivers {
                            let Some(length) = &receiver.allocation_length else {
                                continue;
                            };
                            let actual = expression_type_in_module_expected(
                                length,
                                Some(&ScalarType::U64),
                                &declarations,
                                &BTreeSet::new(),
                                &BTreeMap::new(),
                                module,
                                &project.modules,
                                &mut diagnostics,
                                false,
                            );
                            expect_module_type(
                                module,
                                &ScalarType::U64,
                                &actual,
                                receiver.span,
                                &mut diagnostics,
                            );
                        }
                        continue;
                    }
                    let actual = expression_type_in_module_expected(
                        &binding.value,
                        Some(&binding.declared_type),
                        &declarations,
                        &BTreeSet::new(),
                        &BTreeMap::new(),
                        module,
                        &project.modules,
                        &mut diagnostics,
                        false,
                    );
                    validate_output_receivers_in_module(
                        &binding.value,
                        &binding.receivers,
                        binding.span,
                        &declarations,
                        module,
                        &project.modules,
                        &mut diagnostics,
                    );
                    expect_module_type(
                        module,
                        &binding.declared_type,
                        &actual,
                        binding.span,
                        &mut diagnostics,
                    );
                }
                ScalarItem::Function(function) => {
                    if !function.overload_arms.is_empty() {
                        continue;
                    }
                    let ScalarType::Callable {
                        outputs,
                        parameters,
                    } = &function.signature
                    else {
                        continue;
                    };
                    if function.parameters.len() != parameters.len() {
                        diagnostics.push(module_diagnostic(
                            module,
                            "B0004",
                            "function parameter arity does not match its type",
                            function.span,
                        ));
                    }
                    let mut scope = declarations.clone();
                    let mut visible_names = declaration_names.clone();
                    let mut folded_names = folded_declarations.clone();
                    for (index, name) in function.parameters.iter().enumerate() {
                        validate_identifier_style(
                            module,
                            name,
                            function.parameter_spans[index],
                            &mut diagnostics,
                        );
                        if index < parameters.len()
                            && declare_module_name(
                                module,
                                name,
                                function.span,
                                &mut visible_names,
                                &mut folded_names,
                                &mut diagnostics,
                            )
                        {
                            scope.insert(name.clone(), parameters[index].clone());
                        }
                    }
                    if outputs.outputs.len() > 1 && !function.body.final_output_values.is_empty() {
                        let final_start =
                            function.body.items.len() - function.body.final_output_values.len();
                        for item in &function.body.items[..final_start] {
                            let _ = match item {
                                ScalarBlockItem::Expression(
                                    expression @ (ScalarExpression::If { .. }
                                    | ScalarExpression::UnitIf { .. }),
                                ) => expression_type_in_module_expected(
                                    expression,
                                    Some(&ScalarType::Unit),
                                    &scope,
                                    &visible_names,
                                    &folded_names,
                                    module,
                                    &project.modules,
                                    &mut diagnostics,
                                    false,
                                ),
                                _ => block_item_type_in_module(
                                    item,
                                    &mut scope,
                                    &mut visible_names,
                                    &mut folded_names,
                                    module,
                                    &project.modules,
                                    &mut diagnostics,
                                    false,
                                ),
                            };
                        }
                        if function.body.final_output_values.len() == 1
                            && matches!(
                                &function.body.final_output_values[0].value,
                                ScalarExpression::Call { .. }
                            )
                        {
                            let value = &function.body.final_output_values[0];
                            if let Some(actual_outputs) = call_output_sequence_in_module(
                                &value.value,
                                &scope,
                                module,
                                &project.modules,
                            ) {
                                expression_type_in_module_expected(
                                    &value.value,
                                    actual_outputs.outputs.first().map(|output| &output.ty),
                                    &scope,
                                    &visible_names,
                                    &folded_names,
                                    module,
                                    &project.modules,
                                    &mut diagnostics,
                                    false,
                                );
                                if actual_outputs.outputs.len() != outputs.outputs.len() {
                                    diagnostics.push(module_diagnostic(
                                        module,
                                        "B0004",
                                        "function output arity does not match its final output list",
                                        value.span,
                                    ));
                                }
                                for (actual, expected) in
                                    actual_outputs.outputs.iter().zip(&outputs.outputs)
                                {
                                    expect_module_type(
                                        module,
                                        &expected.ty,
                                        &actual.ty,
                                        value.span,
                                        &mut diagnostics,
                                    );
                                }
                            }
                        } else {
                            if function.body.final_output_values.len() > 1
                                && function.body.final_output_values.len() != outputs.outputs.len()
                            {
                                diagnostics.push(module_diagnostic(
                                    module,
                                    "B0004",
                                    "function output arity does not match its final output list",
                                    function.body.span,
                                ));
                            }
                            for (value, output) in function
                                .body
                                .final_output_values
                                .iter()
                                .zip(&outputs.outputs)
                            {
                                let actual = expression_type_in_module_expected(
                                    &value.value,
                                    Some(&output.ty),
                                    &scope,
                                    &visible_names,
                                    &folded_names,
                                    module,
                                    &project.modules,
                                    &mut diagnostics,
                                    false,
                                );
                                expect_module_type(
                                    module,
                                    &output.ty,
                                    &actual,
                                    value.span,
                                    &mut diagnostics,
                                );
                            }
                        }
                    } else if let Some(output) = outputs.outputs.first() {
                        let actual = block_type_in_module_expected(
                            &function.body,
                            Some(&output.ty),
                            &scope,
                            &visible_names,
                            &folded_names,
                            module,
                            &project.modules,
                            &mut diagnostics,
                            false,
                        );
                        expect_module_type(
                            module,
                            &output.ty,
                            &actual,
                            function.body.span,
                            &mut diagnostics,
                        );
                    } else {
                        let actual = block_type_in_module(
                            &function.body,
                            &scope,
                            &visible_names,
                            &folded_names,
                            module,
                            &project.modules,
                            &mut diagnostics,
                            false,
                        );
                        expect_module_type(
                            module,
                            &ScalarType::Unit,
                            &actual,
                            function.body.span,
                            &mut diagnostics,
                        );
                    }
                }
                ScalarItem::Executable(executable) => {
                    let mut scope = declarations.clone();
                    let mut visible_names = declaration_names.clone();
                    let mut folded_names = folded_declarations.clone();
                    let _ = block_item_type_in_module(
                        executable,
                        &mut scope,
                        &mut visible_names,
                        &mut folded_names,
                        module,
                        &project.modules,
                        &mut diagnostics,
                        false,
                    );
                }
            }
        }
        for span in unused_binding_spans(&module.items) {
            diagnostics.push(module_diagnostic(
                module,
                "B0009",
                "local binding or parameter has no required static use",
                span,
            ));
        }
    }
    if diagnostics.is_empty() {
        record_project_overload_selections(&mut project);
    }
    if diagnostics.is_empty() {
        let modules = project.modules.clone();
        for module in &mut project.modules {
            let call_sites = resolve_module_call_sites(&module.source, &module.items, &modules);
            diagnostics.extend(
                call_sites
                    .map(|sites| {
                        let structs = modules
                            .iter()
                            .flat_map(|module| module.structs.clone())
                            .collect::<Vec<_>>();
                        let enums = modules
                            .iter()
                            .flat_map(|module| module.enums.clone())
                            .collect::<Vec<_>>();
                        record_module_reference_origins(
                            &module.source,
                            &mut module.items,
                            &sites,
                            &structs,
                            &enums,
                            &module.resolved_assignment_outputs.borrow(),
                        )
                    })
                    .unwrap_or_else(|error| vec![error])
                    .into_iter()
                    .map(|error| error.diagnostic()),
            );
        }
    }
    if diagnostics.is_empty() {
        diagnostics.extend(
            resolve_reference_output_contracts(&mut project.modules)
                .into_iter()
                .map(|error| error.diagnostic()),
        );
    }
    if diagnostics.is_empty() {
        let modules = project.modules.clone();
        let structs = modules
            .iter()
            .flat_map(|module| module.structs.clone())
            .collect::<Vec<_>>();
        let enums = modules
            .iter()
            .flat_map(|module| module.enums.clone())
            .collect::<Vec<_>>();
        for module in &mut project.modules {
            diagnostics.extend(
                record_specialized_copy_policies(&mut module.items, &modules, &structs, &enums)
                    .into_iter()
                    .map(|error| error.diagnostic()),
            );
        }
    }
    ScalarProjectValidation {
        project,
        diagnostics,
    }
}

fn record_project_overload_selections(project: &mut ScalarProject) {
    let modules = project.modules.clone();
    for module in &mut project.modules {
        let scope = module.members.clone();
        let mut overloads = module
            .items
            .iter()
            .filter_map(|item| match item {
                ScalarItem::Function(function) if !function.overload_arms.is_empty() => {
                    Some((function.name.clone(), function.clone()))
                }
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        for namespace in &module.namespace_bindings {
            if let Some(target) = modules
                .iter()
                .find(|candidate| candidate.source == namespace.target)
            {
                for item in &target.items {
                    if let ScalarItem::Function(function) = item {
                        if target.members.contains_key(&function.name)
                            && !function.overload_arms.is_empty()
                        {
                            overloads.insert(
                                format!("{}.{}", namespace.binding, function.name),
                                function.clone(),
                            );
                        }
                    }
                }
            }
        }
        for item in &mut module.items {
            match item {
                ScalarItem::Binding(binding) => {
                    let expected = binding
                        .receivers
                        .first()
                        .map(|receiver| &receiver.ty)
                        .unwrap_or(&binding.declared_type);
                    record_expression_overload_selection(
                        &mut binding.value,
                        Some(expected),
                        &scope,
                        &overloads,
                    );
                }
                ScalarItem::Function(function) => {
                    let mut body_scope = scope.clone();
                    if let ScalarType::Callable { parameters, .. } = &function.signature {
                        for (name, ty) in function.parameters.iter().zip(parameters) {
                            body_scope.insert(name.clone(), ty.clone());
                        }
                    }
                    record_block_overload_selections(&mut function.body, &body_scope, &overloads);
                }
                ScalarItem::Executable(item) => {
                    let mut item_scope = scope.clone();
                    record_block_item_overload_selections(item, &mut item_scope, &overloads);
                }
                ScalarItem::Namespace(_) | ScalarItem::Extern(_) => {}
            }
        }
    }
}

fn derive_namespace(node: &CstNode) -> ScalarNamespace {
    let identifiers: Vec<CstToken> = node
        .children_with_tokens()
        .filter_map(|element| match element {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => Some(token),
            _ => None,
        })
        .collect();
    let path = direct_token(node, SyntaxKind::String).expect("namespace path");
    ScalarNamespace {
        binding: identifiers[0].text().to_owned(),
        package: identifiers[1].text().to_owned(),
        path: path.text().to_owned(),
        binding_span: token_span(&identifiers[0]),
        package_span: token_span(&identifiers[1]),
        path_span: token_span(&path),
        span: wosy_syntax::byte_span(node),
    }
}

fn derive_extern(node: &CstNode) -> ScalarExtern {
    let identifiers: Vec<CstToken> = node
        .children_with_tokens()
        .filter_map(|element| match element {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => Some(token),
            _ => None,
        })
        .collect();
    let module = direct_token(node, SyntaxKind::String).expect("extern module");
    let functions = node
        .children()
        .filter(|child| child.kind() == SyntaxKind::ExternFunction)
        .map(|callable| {
            let signature = callable
                .children()
                .find(|child| child.kind() == SyntaxKind::CallableType)
                .expect("extern signature");
            let name = callable
                .children_with_tokens()
                .filter_map(|element| match element {
                    NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => {
                        Some(token)
                    }
                    _ => None,
                })
                .next()
                .expect("extern symbol");
            ScalarExternFunction {
                name: name.text().to_owned(),
                signature: derive_type(&signature),
                unsafe_marker: callable
                    .children_with_tokens()
                    .any(|element| element.kind() == SyntaxKind::Unsafe),
                name_span: token_span(&name),
                signature_span: wosy_syntax::byte_span(&signature),
                span: wosy_syntax::byte_span(&callable),
            }
        })
        .collect();
    ScalarExtern {
        binding: identifiers[0].text().to_owned(),
        kind: identifiers[1].text().to_owned(),
        actual_module: match decode_string(module.text()) {
            Ok(value) => ScalarExternModule::Valid(value),
            Err((start, length)) => ScalarExternModule::Invalid {
                span: token_span(&module),
                error_span: string_error_span(&module, start, length),
            },
        },
        functions,
        binding_span: token_span(&identifiers[0]),
        module_span: token_span(&module),
        span: wosy_syntax::byte_span(node),
    }
}

fn derive_binding(node: &CstNode) -> ScalarBinding {
    let receiver_list = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::ReceiverList)
        .expect("receiver list");
    let receivers = receiver_list
        .children()
        .filter(|child| child.kind() == SyntaxKind::Receiver)
        .map(|receiver| {
            let name = direct_token(&receiver, SyntaxKind::Identifier).expect("receiver name");
            let allocation_length = receiver
                .children()
                .find(|child| child.kind() == SyntaxKind::Expression)
                .map(|child| derive_expression(&child));
            let ty = allocation_length.as_ref().map_or_else(
                || {
                    let ty_node = direct_nodes(&receiver)
                        .into_iter()
                        .next()
                        .expect("receiver type");
                    derive_type(&ty_node)
                },
                |_| {
                    let element = direct_token(&receiver, SyntaxKind::TypeName)
                        .expect("runtime array element type");
                    ScalarType::RuntimeArray {
                        element: Box::new(type_from_name(element.text(), token_span(&element))),
                        span: wosy_syntax::byte_span(&receiver),
                    }
                },
            );
            ScalarOutputReceiver {
                name: name.text().to_owned(),
                name_span: token_span(&name),
                ty,
                allocation_length,
                span: wosy_syntax::byte_span(&receiver),
            }
        })
        .collect::<Vec<_>>();
    let declared_type = receivers[0].ty.clone();
    let output_list = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::OutputList);
    let outputs = output_list.as_ref().map_or_else(Vec::new, |output_list| {
        output_list
            .children()
            .filter(|child| child.kind() == SyntaxKind::Expression)
            .map(|child| derive_expression(&child))
            .collect::<Vec<_>>()
    });
    let value = outputs
        .first()
        .cloned()
        .unwrap_or(ScalarExpression::Integer {
            value: BigInt::from(0),
            span: wosy_syntax::byte_span(node),
        });
    let name = receivers[0].name.clone();
    let output_values = receivers
        .iter()
        .enumerate()
        .map(|(position, receiver)| ScalarOutputValue {
            position,
            ty: receiver.ty.clone(),
            span: if outputs.is_empty() {
                receiver.span
            } else {
                span_of(if outputs.len() == 1 {
                    &value
                } else {
                    &outputs[position]
                })
            },
            value: if outputs.is_empty() || outputs.len() == 1 {
                value.clone()
            } else {
                outputs[position].clone()
            },
        })
        .collect::<Vec<_>>();
    let output_origin = match outputs.len() {
        1 => ScalarBindingOutputOrigin::SingleExpression,
        _ => ScalarBindingOutputOrigin::IndependentExpressions,
    };
    let output_sequence = ScalarOutputSequence {
        outputs: receivers
            .iter()
            .enumerate()
            .map(|(position, receiver)| ScalarOutput {
                ty: receiver.ty.clone(),
                span: output_values[position].span,
            })
            .collect(),
        span: output_list
            .as_ref()
            .map_or_else(|| wosy_syntax::byte_span(node), wosy_syntax::byte_span),
    };
    ScalarBinding {
        name,
        name_span: receivers[0].name_span,
        declared_type,
        value,
        receivers,
        output_sequence,
        output_values,
        output_origin,
        is_allocation: outputs.is_empty(),
        span: wosy_syntax::byte_span(node),
    }
}

fn derive_struct(
    node: CstNode,
    id: ScalarStructId,
    target_layout: ScalarTargetLayout,
) -> ScalarStruct {
    let name = direct_token(&node, SyntaxKind::Identifier).expect("struct name");
    let copy_modifier_span =
        direct_token(&node, SyntaxKind::CopyModifier).map(|token| token_span(&token));
    let mut fields = node
        .children()
        .filter(|child| child.kind() == SyntaxKind::StructField)
        .enumerate()
        .map(|(index, field)| {
            let ty = derive_type(&direct_nodes(&field).into_iter().next().expect("field type"));
            let field_name = direct_token(&field, SyntaxKind::Identifier).expect("field name");
            ScalarStructField {
                id: ScalarStructFieldId {
                    structure: id.clone(),
                    index,
                },
                name: field_name.text().to_owned(),
                name_span: token_span(&field_name),
                layout: None,
                ty,
                declaration_index: index,
                offset: None,
            }
        })
        .collect::<Vec<_>>();
    compute_initial_struct_fields_layout(&mut fields, target_layout);
    let layout = initial_struct_layout(&fields);
    ScalarStruct {
        id,
        name: name.text().to_owned(),
        name_span: token_span(&name),
        fields,
        copy_policy: CopyPolicy::Move,
        copy_modifier_span,
        span: wosy_syntax::byte_span(&node),
        layout,
    }
}

fn derive_enum(node: CstNode, id: ScalarEnumId) -> ScalarEnum {
    let name = direct_token(&node, SyntaxKind::Identifier).expect("enum name");
    let copy_modifier_span =
        direct_token(&node, SyntaxKind::CopyModifier).map(|token| token_span(&token));
    let variants = node
        .children()
        .filter(|child| child.kind() == SyntaxKind::EnumVariant)
        .enumerate()
        .map(|(tag, variant)| {
            let name = direct_token(&variant, SyntaxKind::Identifier).expect("enum variant");
            ScalarEnumVariant {
                name: name.text().to_owned(),
                name_span: token_span(&name),
                tag: u32::try_from(tag).expect("enum variant tag fits u32"),
            }
        })
        .collect();
    ScalarEnum {
        id,
        name: name.text().to_owned(),
        name_span: token_span(&name),
        variants,
        copy_policy: CopyPolicy::Move,
        copy_modifier_span,
        span: wosy_syntax::byte_span(&node),
    }
}

fn checked_align_offset(offset: u64, alignment: u64) -> Option<u64> {
    offset
        .checked_add(alignment - 1)
        .and_then(|value| value.checked_div(alignment))
        .and_then(|value| value.checked_mul(alignment))
}

fn checked_layout_offset(offset: u64, layout: &ScalarLayout) -> Option<(u64, u64)> {
    let field_offset = checked_align_offset(offset, layout.alignment)?;
    let next_offset = field_offset.checked_add(layout.size)?;
    Some((field_offset, next_offset))
}

fn clear_struct_layout(structure: &mut ScalarStruct) {
    structure.layout = None;
    for field in &mut structure.fields {
        field.layout = None;
        field.offset = None;
    }
}

fn compute_initial_struct_layout(structure: &mut ScalarStruct, target_layout: ScalarTargetLayout) {
    compute_initial_struct_fields_layout(&mut structure.fields, target_layout);
    structure.layout = initial_struct_layout(&structure.fields);
}

fn compute_initial_struct_fields_layout(
    fields: &mut [ScalarStructField],
    target_layout: ScalarTargetLayout,
) {
    for field in fields.iter_mut() {
        field.layout = None;
        field.offset = None;
    }
    let mut offset = 0;
    for index in 0..fields.len() {
        let Ok(layout) = layout_for_type(&fields[index].ty, target_layout) else {
            return;
        };
        let Some((field_offset, next_offset)) = checked_layout_offset(offset, &layout) else {
            for field in fields.iter_mut() {
                field.layout = None;
                field.offset = None;
            }
            return;
        };
        fields[index].offset = Some(field_offset);
        fields[index].layout = Some(layout.clone());
        offset = next_offset;
    }
}

fn initial_struct_layout(fields: &[ScalarStructField]) -> Option<ScalarLayout> {
    let mut offset = 0;
    let mut alignment = 1;
    for field in fields {
        let layout = field.layout.as_ref()?;
        alignment = alignment.max(layout.alignment);
        let (_, next_offset) = checked_layout_offset(offset, layout)?;
        offset = next_offset;
    }
    Some(ScalarLayout {
        size: checked_align_offset(offset, alignment)?,
        alignment,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LayoutError {
    ArraySizeOverflow(ByteSpan),
    AggregateSizeOverflow(ByteSpan),
    InvalidType,
}

fn layout_for_type(
    ty: &ScalarType,
    target_layout: ScalarTargetLayout,
) -> Result<ScalarLayout, LayoutError> {
    Ok(match ty {
        ScalarType::Bool | ScalarType::I8 | ScalarType::U8 => ScalarLayout {
            size: 1,
            alignment: 1,
        },
        ScalarType::I16 | ScalarType::U16 => ScalarLayout {
            size: 2,
            alignment: 2,
        },
        ScalarType::I32 | ScalarType::U32 | ScalarType::Char => ScalarLayout {
            size: 4,
            alignment: 4,
        },
        ScalarType::I64 | ScalarType::U64 => ScalarLayout {
            size: 8,
            alignment: 8,
        },
        ScalarType::I128 | ScalarType::U128 => ScalarLayout {
            size: 16,
            alignment: 16,
        },
        ScalarType::F32 => ScalarLayout {
            size: 4,
            alignment: 4,
        },
        ScalarType::F64 => ScalarLayout {
            size: 8,
            alignment: 8,
        },
        ScalarType::RawPointer(_)
        | ScalarType::CheckedReference { .. }
        | ScalarType::ArtifactId => ScalarLayout {
            size: target_layout.pointer_size,
            alignment: target_layout.pointer_alignment,
        },
        ScalarType::Array {
            element,
            length,
            length_span,
            ..
        } => {
            let element = layout_for_type(element, target_layout)?;
            ScalarLayout {
                size: element
                    .size
                    .checked_mul(*length)
                    .ok_or(LayoutError::ArraySizeOverflow(*length_span))?,
                alignment: element.alignment,
            }
        }
        ScalarType::Error => return Err(LayoutError::InvalidType),
        _ => ScalarLayout {
            size: 0,
            alignment: 1,
        },
    })
}

fn layout_error_source(
    source: &SourceIdentity,
    error: LayoutError,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let (message, span, label) = match error {
        LayoutError::ArraySizeOverflow(span) => (
            "fixed array layout exceeds u64",
            span,
            "fixed array length overflows its layout",
        ),
        LayoutError::AggregateSizeOverflow(span) => (
            "struct layout exceeds u64",
            span,
            "field overflows its enclosing struct layout",
        ),
        LayoutError::InvalidType => return,
    };
    diagnostics.push(super::Diagnostic {
        code: "B0003".to_owned(),
        severity: super::DiagnosticSeverity::Error,
        message: message.to_owned(),
        labels: vec![super::DiagnosticLabel {
            kind: super::DiagnosticLabelKind::Primary,
            span: SourceSpan::new(source.clone(), span),
            message: label.to_owned(),
        }],
        notes: Vec::new(),
    });
}

fn output_sequence(node: &CstNode) -> ScalarOutputSequence {
    ScalarOutputSequence {
        outputs: node
            .children()
            .filter(|child| child.kind() == SyntaxKind::CallableOutput)
            .flat_map(|output| {
                let children = direct_nodes(&output);
                if children.is_empty() {
                    output
                        .children_with_tokens()
                        .filter_map(|element| match element {
                            NodeOrToken::Node(node)
                                if matches!(
                                    node.kind(),
                                    SyntaxKind::RawPointerType | SyntaxKind::CheckedPointerType
                                ) =>
                            {
                                Some(ScalarOutput {
                                    ty: derive_type(&node),
                                    span: wosy_syntax::byte_span(&node),
                                })
                            }
                            NodeOrToken::Token(token) if token.kind() == SyntaxKind::TypeName => {
                                Some(ScalarOutput {
                                    ty: type_from_name(token.text(), token_span(&token)),
                                    span: token_span(&token),
                                })
                            }
                            NodeOrToken::Node(node) if node.kind() == SyntaxKind::QualifiedType => {
                                Some(ScalarOutput {
                                    ty: derive_type(&node),
                                    span: wosy_syntax::byte_span(&node),
                                })
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                } else {
                    children
                        .into_iter()
                        .map(|child| ScalarOutput {
                            ty: derive_type(&child),
                            span: wosy_syntax::byte_span(&child),
                        })
                        .collect()
                }
            })
            .collect(),
        span: wosy_syntax::byte_span(node),
    }
}

fn derive_function(node: &CstNode) -> ScalarFunction {
    let children = direct_nodes(node);
    let signature = derive_type(
        children
            .iter()
            .find(|child| child.kind() == SyntaxKind::CallableType)
            .expect("function signature"),
    );
    let name = direct_token(node, SyntaxKind::Identifier).expect("function name");
    let parameters = children
        .iter()
        .find(|child| child.kind() == SyntaxKind::Parameters)
        .map_or_else(Vec::new, |parameters| {
            parameters
                .children_with_tokens()
                .filter_map(|element| match element {
                    NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => {
                        Some(token.text().to_owned())
                    }
                    _ => None,
                })
                .collect()
        });
    let parameter_spans = children
        .iter()
        .find(|child| child.kind() == SyntaxKind::Parameters)
        .map_or_else(Vec::new, |parameters| {
            parameters
                .children_with_tokens()
                .filter_map(|element| match element {
                    NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => {
                        Some(token_span(&token))
                    }
                    _ => None,
                })
                .collect()
        });
    let mut body = children
        .iter()
        .find(|child| child.kind() == SyntaxKind::Block)
        .map(|node| derive_block(node))
        .expect("function block");
    if let ScalarType::Callable { outputs, .. } = &signature {
        expand_conditional_final_outputs(&mut body, &outputs.outputs);
        for (value, output) in body.final_output_values.iter_mut().zip(&outputs.outputs) {
            value.ty = output.ty.clone();
        }
    }
    ScalarFunction {
        name: name.text().to_owned(),
        name_span: token_span(&name),
        signature,
        parameters,
        parameter_spans,
        body,
        span: wosy_syntax::byte_span(node),
        generic_parameters: Vec::new(),
        overload_arms: Vec::new(),
        reference_cfg: ScalarReferenceCfg {
            facts: Vec::new(),
            points: Vec::new(),
            initial_points: Vec::new(),
            allocations: BTreeSet::new(),
            specialized_policies: Vec::new(),
        },
    }
}

fn derive_generic_parameters(node: &CstNode) -> Vec<ScalarGenericParameter> {
    direct_token(node, SyntaxKind::Identifier)
        .map(|token| {
            vec![ScalarGenericParameter {
                name: token.text().to_owned(),
                span: token_span(&token),
            }]
        })
        .unwrap_or_default()
}

fn derive_generic_function(node: &CstNode) -> ScalarFunction {
    let generic = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::GenericDecl)
        .expect("generic declaration");
    let function = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::FunctionDecl)
        .expect("generic function declaration");
    let mut function = derive_function(&function);
    function.generic_parameters = derive_generic_parameters(&generic);
    function.span = wosy_syntax::byte_span(node);
    function
}

fn derive_overload(node: &CstNode) -> ScalarFunction {
    let name = direct_token(node, SyntaxKind::Identifier).expect("overload name");
    let arms: Vec<ScalarFunction> = node
        .children()
        .filter(|child| child.kind() == SyntaxKind::OverloadArm)
        .map(|arm| {
            let children = direct_nodes(&arm);
            let signature = derive_type(
                children
                    .iter()
                    .find(|child| child.kind() == SyntaxKind::CallableType)
                    .expect("overload arm signature"),
            );
            let parameters = children
                .iter()
                .find(|child| child.kind() == SyntaxKind::Parameters)
                .map_or_else(Vec::new, |parameters| {
                    parameters
                        .children_with_tokens()
                        .filter_map(|element| match element {
                            NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => {
                                Some(token.text().to_owned())
                            }
                            _ => None,
                        })
                        .collect()
                });
            let parameter_spans = children
                .iter()
                .find(|child| child.kind() == SyntaxKind::Parameters)
                .map_or_else(Vec::new, |parameters| {
                    parameters
                        .children_with_tokens()
                        .filter_map(|element| match element {
                            NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => {
                                Some(token_span(&token))
                            }
                            _ => None,
                        })
                        .collect()
                });
            let mut body = derive_block(
                children
                    .iter()
                    .find(|child| child.kind() == SyntaxKind::Block)
                    .expect("overload arm block"),
            );
            if let ScalarType::Callable { outputs, .. } = &signature {
                expand_conditional_final_outputs(&mut body, &outputs.outputs);
                for (value, output) in body.final_output_values.iter_mut().zip(&outputs.outputs) {
                    value.ty = output.ty.clone();
                }
            }
            ScalarFunction {
                name: name.text().to_owned(),
                name_span: token_span(&name),
                signature,
                parameters,
                parameter_spans,
                body,
                span: wosy_syntax::byte_span(&arm),
                generic_parameters: children
                    .iter()
                    .find(|child| child.kind() == SyntaxKind::GenericDecl)
                    .map_or_else(Vec::new, derive_generic_parameters),
                overload_arms: Vec::new(),
                reference_cfg: ScalarReferenceCfg {
                    facts: Vec::new(),
                    points: Vec::new(),
                    initial_points: Vec::new(),
                    allocations: BTreeSet::new(),
                    specialized_policies: Vec::new(),
                },
            }
        })
        .collect();
    let signature = arms.first().expect("overload arm").signature.clone();
    ScalarFunction {
        name: name.text().to_owned(),
        name_span: token_span(&name),
        signature,
        parameters: Vec::new(),
        parameter_spans: Vec::new(),
        body: ScalarBlock {
            items: Vec::new(),
            terminated_items: Vec::new(),
            expressions: Vec::new(),
            final_output_values: Vec::new(),
            span: wosy_syntax::byte_span(node),
            unsafe_context: false,
        },
        span: wosy_syntax::byte_span(node),
        generic_parameters: Vec::new(),
        overload_arms: arms,
        reference_cfg: ScalarReferenceCfg {
            facts: Vec::new(),
            points: Vec::new(),
            initial_points: Vec::new(),
            allocations: BTreeSet::new(),
            specialized_policies: Vec::new(),
        },
    }
}

fn record_program_reference_origins(
    program: &mut ScalarProgram,
    call_sites: &[ResolvedCallSite],
) -> Vec<ReferenceAnalysisError> {
    let mut errors = record_module_reference_origins(
        &program.source,
        &mut program.items,
        call_sites,
        &program.structs,
        &program.enums,
        &program.resolved_assignment_outputs.borrow(),
    );
    if errors.is_empty() {
        let mut modules = vec![ScalarModule::from_program(program.clone(), Vec::new())];
        errors.extend(resolve_reference_output_contracts(&mut modules));
        program.items = modules.remove(0).items;
    }
    errors
}

struct ReferenceFlowBuilder {
    source: SourceIdentity,
    function_span: ByteSpan,
    names: BTreeMap<String, ReferenceBindingId>,
    module_names: HashSet<String>,
    call_sites: Vec<ResolvedCallSite>,
    origins: HashMap<ReferenceBindingId, BTreeSet<ReferenceOriginId>>,
    checked: HashSet<ReferenceBindingId>,
    binding_types: HashMap<ReferenceBindingId, ScalarType>,
    structs: Vec<ScalarStruct>,
    enums: Vec<ScalarEnum>,
    generic_parameters: BTreeSet<String>,
    assignment_outputs: HashMap<ResolvedAssignmentOutputId, ScalarType>,
    dereference_points: HashMap<ByteSpan, CfgPointId>,
    cfg: ScalarReferenceCfg,
    next_scope: usize,
    cursor: Option<CfgPointId>,
}

#[derive(Debug)]
enum ReferenceValue {
    Other,
    Checked(BTreeSet<ReferenceOriginId>),
    Uncomputed,
}

impl ReferenceFlowBuilder {
    fn assignment_output_types(
        &self,
        assignment: &ScalarAssignment,
    ) -> Result<Vec<ScalarType>, ReferenceAnalysisError> {
        assignment
            .targets
            .iter()
            .enumerate()
            .map(|(output_index, target)| {
                let key = ResolvedAssignmentOutputId {
                    source: self.source.clone(),
                    function_span: self.function_span,
                    target_span: target.target_span,
                    output_index,
                };
                self.assignment_outputs.get(&key).cloned().ok_or_else(|| {
                    self.missing_context(
                        target.target_span,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    )
                })
            })
            .collect()
    }

    fn call_target(
        &self,
        expression: &ScalarExpression,
    ) -> Result<ResolvedCallTarget, ReferenceAnalysisError> {
        let span = expression_span(expression);
        self.call_sites
            .iter()
            .find(|site| site.source_span == SourceSpan::new(self.source.clone(), span))
            .map(|site| site.target.clone())
            .ok_or_else(|| {
                self.missing_context(
                    span,
                    ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
                )
            })
    }

    fn intern(&mut self, origin: ReferenceOrigin) -> ReferenceOriginId {
        if let Some(index) = self.cfg.facts.iter().position(|fact| fact == &origin) {
            ReferenceOriginId(index)
        } else {
            let id = ReferenceOriginId(self.cfg.facts.len());
            self.cfg.facts.push(origin);
            id
        }
    }

    fn point(
        &mut self,
        kind: CfgPointKind,
        span: ByteSpan,
        scope: ReferenceScopeId,
        possible_origins: BTreeSet<ReferenceOriginId>,
    ) -> CfgPointId {
        self.point_with_origins(kind, span, scope, Some(possible_origins))
    }

    fn point_with_origins(
        &mut self,
        kind: CfgPointKind,
        span: ByteSpan,
        scope: ReferenceScopeId,
        possible_origins: Option<BTreeSet<ReferenceOriginId>>,
    ) -> CfgPointId {
        let id = CfgPointId(self.cfg.points.len());
        if let Some(previous) = self.cursor {
            self.cfg.points[previous.0].successors.push(id);
        }
        self.cfg.points.push(ReferenceFlowPoint {
            id,
            kind,
            source_span: SourceSpan::new(self.source.clone(), span),
            scope,
            possible_origins,
            successors: Vec::new(),
        });
        self.cursor = Some(id);
        id
    }

    fn uncomputed_point(&mut self, kind: CfgPointKind, span: ByteSpan, scope: ReferenceScopeId) {
        self.point_with_origins(kind, span, scope, None);
    }

    fn missing_origin(
        &self,
        span: ByteSpan,
        binding_id: ReferenceBindingId,
        reason: ReferenceAnalysisErrorReason,
    ) -> ReferenceAnalysisError {
        ReferenceAnalysisError::MissingOrigin {
            source_span: SourceSpan::new(self.source.clone(), span),
            binding_id,
            reason,
        }
    }

    fn missing_context(
        &self,
        span: ByteSpan,
        reason: ReferenceAnalysisErrorReason,
    ) -> ReferenceAnalysisError {
        ReferenceAnalysisError::MissingContext {
            source_span: SourceSpan::new(self.source.clone(), span),
            reason,
        }
    }

    fn binding(
        &self,
        name: &str,
        span: ByteSpan,
    ) -> Result<ReferenceBindingId, ReferenceAnalysisError> {
        self.names
            .get(name)
            .cloned()
            .ok_or_else(|| ReferenceAnalysisError::MissingBinding {
                source_span: SourceSpan::new(self.source.clone(), span),
                name: name.to_owned(),
            })
    }

    fn assignment_place(
        &self,
        place: &ScalarPlace,
        target_span: ByteSpan,
    ) -> Result<ReferencePlaceId, ReferenceAnalysisError> {
        let root = reference_root(place);
        let (binding, parent) = match root {
            ScalarPlace::Name { name, span } => (self.binding(name, *span)?, None),
            ScalarPlace::Dereference { pointer, .. } => {
                let ScalarExpression::Name { name, span } = pointer.as_ref() else {
                    return Err(self.missing_context(
                        target_span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    ));
                };
                let pointer_binding = self.binding(name, *span)?;
                let origins = self.required(&pointer_binding, *span)?;
                let loans = origins
                    .iter()
                    .map(|origin| match &self.cfg.facts[origin.0] {
                        ReferenceOrigin::Fresh { loan, .. }
                        | ReferenceOrigin::BorrowedFrom { loan, .. } => Ok(loan),
                        ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => Err(self
                            .missing_context(
                                *span,
                                ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                            )),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let [loan] = loans.as_slice() else {
                    return Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    ));
                };
                (loan.origin_place.binding.clone(), Some(*loan))
            }
            ScalarPlace::Field { .. } | ScalarPlace::Index { .. } => unreachable!(),
        };
        let mut projections = Vec::new();
        collect_reference_projections(place, parent, &mut projections);
        Ok(ReferencePlaceId {
            source: self.source.clone(),
            function_span: self.function_span,
            declaration_span: binding.declaration_span,
            block_span: binding.block_span,
            binding,
            projections,
            place: place.clone(),
        })
    }

    fn required(
        &self,
        id: &ReferenceBindingId,
        span: ByteSpan,
    ) -> Result<BTreeSet<ReferenceOriginId>, ReferenceAnalysisError> {
        self.origins
            .get(id)
            .filter(|set| !set.is_empty())
            .cloned()
            .ok_or_else(|| {
                self.missing_origin(
                    span,
                    id.clone(),
                    ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                )
            })
    }

    fn pointer_cast_origins(
        &mut self,
        raw: &ScalarExpression,
        mutability: ScalarReferenceMutability,
        span: ByteSpan,
    ) -> Result<BTreeSet<ReferenceOriginId>, ReferenceAnalysisError> {
        let fact = if matches!(raw, ScalarExpression::Name { name, .. } if name == "null") {
            ReferenceOrigin::Null { span }
        } else {
            let binding = match raw {
                ScalarExpression::Name {
                    name,
                    span: raw_span,
                } => self.binding(name, *raw_span)?,
                ScalarExpression::RawAddress { place, .. }
                    if matches!(reference_root(place), ScalarPlace::Name { .. }) =>
                {
                    let ScalarPlace::Name {
                        name,
                        span: raw_span,
                    } = reference_root(place)
                    else {
                        unreachable!()
                    };
                    self.binding(name, *raw_span)?
                }
                _ => ReferenceBindingId {
                    source: self.source.clone(),
                    function_span: self.function_span,
                    declaration_span: span,
                    block_span: self.function_span,
                    kind: ReferenceBindingKind::CallOutput {
                        point: CfgPointId(self.cfg.points.len()),
                        output: 0,
                    },
                },
            };
            let place = match raw {
                ScalarExpression::Name {
                    name,
                    span: raw_span,
                } => ScalarPlace::Name {
                    name: name.clone(),
                    span: *raw_span,
                },
                ScalarExpression::RawAddress { place, .. } => place.clone(),
                _ => ScalarPlace::Name {
                    name: "core.pointer_cast".to_owned(),
                    span,
                },
            };
            ReferenceOrigin::Fresh {
                allocation: ReturnedAllocationId {
                    source: binding.source.clone(),
                    function_span: binding.function_span,
                    declaration_span: binding.declaration_span,
                    creation_span: span,
                },
                loan: ReferenceLoanId {
                    mode: mutability,
                    origin_place: ReferencePlaceId {
                        source: binding.source.clone(),
                        function_span: binding.function_span,
                        declaration_span: binding.declaration_span,
                        block_span: binding.block_span,
                        binding,
                        projections: Vec::new(),
                        place,
                    },
                    creation_span: span,
                    parent: None,
                },
            }
        };
        Ok(BTreeSet::from([self.intern(fact)]))
    }

    fn conditional(
        &mut self,
        condition: &ScalarExpression,
        then_branch: &ScalarBlock,
        else_branch: Option<&ScalarBlock>,
        span: ByteSpan,
        scope: ReferenceScopeId,
    ) -> Result<ReferenceValue, ReferenceAnalysisError> {
        self.expression(condition, scope)?;
        let branch = self.point(
            CfgPointKind::Branch {
                condition: condition.clone(),
            },
            expression_span(condition),
            scope,
            BTreeSet::new(),
        );
        let incoming_names = self.names.clone();
        let incoming_origins = self.origins.clone();
        let incoming_checked = self.checked.clone();
        let then_value = self.block(then_branch)?;
        let then_end = self.cursor.expect("then branch terminal");
        let then_checked = self.checked.clone();

        self.names = incoming_names;
        self.origins = incoming_origins.clone();
        self.checked = incoming_checked.clone();
        self.cursor = Some(branch);
        let (else_value, else_end) = if let Some(else_branch) = else_branch {
            let result = self.block(else_branch)?;
            (result, self.cursor.expect("else branch terminal"))
        } else {
            (None, branch)
        };
        self.checked.extend(then_checked);
        self.origins.clear();
        self.cursor = None;
        let joined_value = then_value.is_some() && else_value.is_some();
        let join = if joined_value {
            self.point_with_origins(CfgPointKind::Join, span, scope, None)
        } else {
            self.point(CfgPointKind::Join, span, scope, BTreeSet::new())
        };
        self.cfg.points[then_end.0].successors.push(join);
        if else_end != branch {
            self.cfg.points[else_end.0].successors.push(join);
        } else {
            self.cfg.points[branch.0].successors.push(join);
        }
        Ok(if joined_value {
            ReferenceValue::Uncomputed
        } else {
            ReferenceValue::Other
        })
    }

    fn expression(
        &mut self,
        expression: &ScalarExpression,
        scope: ReferenceScopeId,
    ) -> Result<ReferenceValue, ReferenceAnalysisError> {
        let span = expression_span(expression);
        match expression {
            ScalarExpression::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => self.conditional(condition, then_branch, Some(else_branch), span, scope),
            ScalarExpression::UnitIf {
                condition,
                then_branch,
                ..
            } => self.conditional(condition, then_branch, None, span, scope),
            ScalarExpression::Block(block) => match self.block(block)? {
                Some(ReferenceValue::Checked(origins)) => {
                    self.point(
                        CfgPointKind::Evaluate(expression.clone()),
                        span,
                        scope,
                        origins.clone(),
                    );
                    Ok(ReferenceValue::Checked(origins))
                }
                Some(ReferenceValue::Uncomputed) => {
                    self.uncomputed_point(CfgPointKind::Evaluate(expression.clone()), span, scope);
                    Ok(ReferenceValue::Uncomputed)
                }
                _ => {
                    self.point(
                        CfgPointKind::Evaluate(expression.clone()),
                        span,
                        scope,
                        BTreeSet::new(),
                    );
                    Ok(ReferenceValue::Other)
                }
            },
            ScalarExpression::Call { arguments, .. } => {
                let target = self.call_target(expression)?;
                let mut address = None;
                let mut checked_release_address = None;
                let mut argument_points = Vec::new();
                for (position, argument) in arguments.iter().enumerate() {
                    self.expression(argument, scope)?;
                    argument_points.push(self.cursor.expect("evaluated call argument"));
                    if position == 0
                        && target == ResolvedCallTarget::Core(CoreOperationId::Free)
                        && (matches!(argument, ScalarExpression::CheckedAddress { .. })
                            || matches!(argument, ScalarExpression::Name { name, .. } if self.names.get(name).is_some_and(|binding| self.checked.contains(binding))))
                    {
                        checked_release_address = self.cursor;
                    }
                    if position == 0
                        && matches!(
                            target,
                            ResolvedCallTarget::Core(
                                CoreOperationId::Invalidate | CoreOperationId::Rebind
                            )
                        )
                    {
                        address = self.cursor;
                    }
                    self.move_value(argument, scope)?;
                }
                if let ResolvedCallTarget::Core(
                    operation @ (CoreOperationId::Invalidate | CoreOperationId::Rebind),
                ) = &target
                {
                    let address_point = address.expect("validated core address argument point");
                    let inputs = match operation {
                        CoreOperationId::Invalidate => InvalidationInputs::Invalidate,
                        CoreOperationId::Rebind => InvalidationInputs::Rebind {
                            raw_address: arguments[1].clone(),
                            raw_span: SourceSpan::new(
                                self.source.clone(),
                                expression_span(&arguments[1]),
                            ),
                            length: arguments[2].clone(),
                            length_span: SourceSpan::new(
                                self.source.clone(),
                                expression_span(&arguments[2]),
                            ),
                        },
                        _ => unreachable!("invalidation operation"),
                    };
                    self.point(
                        CfgPointKind::Invalidate {
                            address_point,
                            candidates: Vec::new(),
                            operation: *operation,
                            address_span: SourceSpan::new(
                                self.source.clone(),
                                expression_span(&arguments[0]),
                            ),
                            inputs,
                        },
                        span,
                        scope,
                        BTreeSet::new(),
                    );
                }
                if target == ResolvedCallTarget::Core(CoreOperationId::Free) {
                    if let Some(address_point) = checked_release_address {
                        self.point(
                            CfgPointKind::Release {
                                target: ReleaseTarget::Checked {
                                    address_point,
                                    candidates: Vec::new(),
                                    address_span: SourceSpan::new(
                                        self.source.clone(),
                                        expression_span(&arguments[0]),
                                    ),
                                },
                            },
                            span,
                            scope,
                            BTreeSet::new(),
                        );
                    } else if let [ScalarExpression::Name {
                        name: place_name,
                        span: argument_span,
                    }] = arguments.as_slice()
                    {
                        let binding = self.binding(place_name, *argument_span)?;
                        self.point(
                            CfgPointKind::Release {
                                target: ReleaseTarget::Raw { binding },
                            },
                            span,
                            scope,
                            BTreeSet::new(),
                        );
                    }
                }
                let cast_origins = match (&target, expression) {
                    (
                        ResolvedCallTarget::Core(CoreOperationId::PointerCast),
                        ScalarExpression::Call { type_arguments, .. },
                    ) => match type_arguments.as_slice() {
                        [argument] => match &argument.ty {
                            ScalarType::CheckedReference { mutability, .. } => {
                                Some(self.pointer_cast_origins(&arguments[0], *mutability, span)?)
                            }
                            _ => None,
                        },
                        _ => None,
                    },
                    _ => None,
                };
                let call = CfgPointKind::Call {
                    expression: expression.clone(),
                    output: 0,
                    target,
                    argument_points,
                };
                if let Some(origins) = cast_origins {
                    self.point(call, span, scope, origins.clone());
                    Ok(ReferenceValue::Checked(origins))
                } else {
                    self.uncomputed_point(call, span, scope);
                    Ok(ReferenceValue::Uncomputed)
                }
            }
            ScalarExpression::Binary { left, right, .. } => {
                self.expression(left, scope)?;
                self.expression(right, scope)?;
                self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                Ok(ReferenceValue::Other)
            }
            ScalarExpression::Unary { operand, .. } => {
                self.expression(operand, scope)?;
                self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                Ok(ReferenceValue::Other)
            }
            ScalarExpression::ArrayLiteral { elements, .. } => {
                for element in elements {
                    self.expression(element, scope)?;
                    self.move_value(element, scope)?;
                }
                self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                Ok(ReferenceValue::Other)
            }
            ScalarExpression::StructLiteral { fields, .. } => {
                for field in fields {
                    self.expression(&field.value, scope)?;
                    self.move_value(&field.value, scope)?;
                }
                self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                Ok(ReferenceValue::Other)
            }
            ScalarExpression::Dereference { place, .. }
            | ScalarExpression::IndexedRead { place, .. } => {
                self.checked_place_read(place, expression, span, scope)
            }
            ScalarExpression::RawAddress { place, .. } => {
                self.place_expressions(place, scope)?;
                self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                Ok(ReferenceValue::Other)
            }
            ScalarExpression::Member {
                receiver,
                name,
                receiver_span,
                ..
            } if self
                .names
                .get(receiver)
                .and_then(|binding| self.binding_types.get(binding))
                .is_some_and(|ty| matches!(ty, ScalarType::Struct(_))) =>
            {
                let binding = self.binding(receiver, *receiver_span)?;
                let ScalarType::Struct(id) = self.binding_types.get(&binding).ok_or_else(|| {
                    self.missing_context(span, ReferenceAnalysisErrorReason::MissingResolvedBinding)
                })?
                else {
                    unreachable!()
                };
                let structure = self
                    .structs
                    .iter()
                    .find(|structure| structure.id == *id)
                    .ok_or_else(|| {
                        self.missing_context(
                            span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?;
                let field = structure
                    .fields
                    .iter()
                    .find(|field| field.name == *name)
                    .ok_or_else(|| {
                        self.missing_context(
                            span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?;
                let place = ScalarPlace::Field {
                    base: Box::new(ScalarPlace::Name {
                        name: receiver.clone(),
                        span: *receiver_span,
                    }),
                    field: ScalarFieldReference::Resolved(field.id.clone()),
                    span,
                };
                self.checked_place_read(&place, expression, span, scope)
            }
            ScalarExpression::Name { name, .. } if name == "null" => {
                let id = self.intern(ReferenceOrigin::Null { span });
                let origins = BTreeSet::from([id]);
                self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    origins.clone(),
                );
                Ok(ReferenceValue::Checked(origins))
            }
            ScalarExpression::Name { name, .. } => {
                if self.module_names.contains(name) && !self.names.contains_key(name) {
                    self.point(
                        CfgPointKind::Evaluate(expression.clone()),
                        span,
                        scope,
                        BTreeSet::new(),
                    );
                    return Ok(ReferenceValue::Other);
                }
                let binding = self.binding(name, span)?;
                if self.checked.contains(&binding) {
                    if self.origins.contains_key(&binding) {
                        let origins = self.required(&binding, span)?;
                        self.point(CfgPointKind::Read { binding }, span, scope, origins.clone());
                        Ok(ReferenceValue::Checked(origins))
                    } else {
                        self.uncomputed_point(CfgPointKind::Read { binding }, span, scope);
                        Ok(ReferenceValue::Uncomputed)
                    }
                } else {
                    self.point(CfgPointKind::Read { binding }, span, scope, BTreeSet::new());
                    Ok(ReferenceValue::Other)
                }
            }
            ScalarExpression::CheckedAddress {
                mutability, place, ..
            } => {
                let root = reference_root(place);
                let parents = match root {
                    ScalarPlace::Dereference { pointer, .. } => {
                        let ReferenceValue::Checked(origins) = self.expression(pointer, scope)?
                        else {
                            let binding = match pointer.as_ref() {
                                ScalarExpression::Name { name, span } => {
                                    self.binding(name, *span)?
                                }
                                _ => return Err(self.missing_context(
                                    span,
                                    ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                                )),
                            };
                            return Err(self.missing_origin(
                                span,
                                binding,
                                ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                            ));
                        };
                        Some(origins)
                    }
                    _ => None,
                };
                self.place_indices(place, scope)?;
                let parent_ids = match &parents {
                    Some(ids) => ids.iter().copied().map(Some).collect::<Vec<_>>(),
                    None => vec![None],
                };
                let mut origins = BTreeSet::new();
                for parent_id in parent_ids {
                    let parent = parent_id.map(|id| self.cfg.facts[id.0].clone());
                    if let Some(
                        ReferenceOrigin::Null { span: origin_span }
                        | ReferenceOrigin::Invalid { origin_span, .. },
                    ) = &parent
                    {
                        let id = self.intern(ReferenceOrigin::Invalid {
                            origin_span: *origin_span,
                            conflict_span: span,
                        });
                        origins.insert(id);
                        self.point(
                            CfgPointKind::Evaluate(expression.clone()),
                            span,
                            scope,
                            BTreeSet::from([id]),
                        );
                        continue;
                    }
                    let (binding, declaration_span, declaration_block, parent_loan) =
                        match (root, &parent) {
                            (
                                ScalarPlace::Name {
                                    name,
                                    span: name_span,
                                },
                                _,
                            ) => {
                                let binding = self.binding(name, *name_span)?;
                                (
                                    binding.clone(),
                                    binding.declaration_span,
                                    binding.block_span,
                                    None,
                                )
                            }
                            (
                                ScalarPlace::Dereference { .. },
                                Some(
                                    ReferenceOrigin::Fresh { loan, .. }
                                    | ReferenceOrigin::BorrowedFrom { loan, .. },
                                ),
                            ) => (
                                loan.origin_place.binding.clone(),
                                loan.origin_place.declaration_span,
                                loan.origin_place.block_span,
                                Some(loan.clone()),
                            ),
                            _ => {
                                return Err(self.missing_context(
                                    span,
                                    ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                                ));
                            }
                        };
                    let mut projections = Vec::new();
                    collect_reference_projections(place, parent_loan.as_ref(), &mut projections);
                    let loan = ReferenceLoanId {
                        mode: *mutability,
                        origin_place: ReferencePlaceId {
                            source: self.source.clone(),
                            function_span: self.function_span,
                            declaration_span,
                            block_span: declaration_block,
                            binding,
                            projections,
                            place: place.clone(),
                        },
                        creation_span: span,
                        parent: parent_loan.map(Box::new),
                    };
                    let fact = match parent {
                        Some(ReferenceOrigin::BorrowedFrom { parameter, .. }) => {
                            ReferenceOrigin::BorrowedFrom {
                                parameter,
                                loan: loan.clone(),
                            }
                        }
                        Some(ReferenceOrigin::Fresh { allocation, .. }) => ReferenceOrigin::Fresh {
                            allocation,
                            loan: loan.clone(),
                        },
                        None => ReferenceOrigin::Fresh {
                            allocation: ReturnedAllocationId {
                                source: self.source.clone(),
                                function_span: self.function_span,
                                declaration_span,
                                creation_span: span,
                            },
                            loan: loan.clone(),
                        },
                        Some(ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. }) => {
                            unreachable!()
                        }
                    };
                    let id = self.intern(fact);
                    origins.insert(id);
                    self.point(
                        CfgPointKind::Borrow {
                            place: loan.origin_place.clone(),
                            loan,
                        },
                        span,
                        scope,
                        BTreeSet::from([id]),
                    );
                }
                Ok(ReferenceValue::Checked(origins))
            }
            _ => {
                self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                Ok(ReferenceValue::Other)
            }
        }
    }

    fn place_expressions(
        &mut self,
        place: &ScalarPlace,
        scope: ReferenceScopeId,
    ) -> Result<(), ReferenceAnalysisError> {
        match place {
            ScalarPlace::Name { name, span } => {
                if self.names.contains_key(name)
                    && self.checked.contains(&self.binding(name, *span)?)
                {
                    let binding = self.binding(name, *span)?;
                    self.point(
                        CfgPointKind::Read { binding },
                        *span,
                        scope,
                        BTreeSet::new(),
                    );
                } else if !self.names.contains_key(name) && !self.module_names.contains(name) {
                    return Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    ));
                }
            }
            ScalarPlace::Dereference { pointer, .. } => {
                self.expression(pointer, scope)?;
                let ScalarPlace::Dereference { span, .. } = place else {
                    unreachable!()
                };
                let point = self.cursor.ok_or_else(|| {
                    self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    )
                })?;
                self.dereference_points.insert(*span, point);
            }
            ScalarPlace::Field { base, .. } => self.place_expressions(base, scope)?,
            ScalarPlace::Index { base, index, .. } => {
                self.place_expressions(base, scope)?;
                self.expression(index, scope)?;
            }
        }
        Ok(())
    }

    fn checked_place_read(
        &mut self,
        place: &ScalarPlace,
        expression: &ScalarExpression,
        span: ByteSpan,
        scope: ReferenceScopeId,
    ) -> Result<ReferenceValue, ReferenceAnalysisError> {
        if let ScalarPlace::Name { name, .. } = reference_root(place) {
            if self.module_names.contains(name) && !self.names.contains_key(name) {
                self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                return Ok(ReferenceValue::Other);
            }
        }
        let reads_checked_value = matches!(
            self.indexed_move_type(place)?,
            ScalarType::CheckedReference { .. }
        );
        let reads_checked_aggregate = if let ScalarPlace::Dereference { pointer, .. } =
            reference_root(place)
        {
            matches!(
                pointer.as_ref(),
                ScalarExpression::Name { name, .. }
                    if matches!(self.binding_types.get(&self.binding(name, span)?),
                        Some(ScalarType::CheckedReference { inner, .. })
                            if matches!(inner.as_ref(), ScalarType::Struct(_) | ScalarType::Array { .. }))
            )
        } else {
            false
        };
        if !reads_checked_value
            && !reads_checked_aggregate
            && matches!(reference_root(place), ScalarPlace::Dereference { pointer, .. } if !matches!(pointer.as_ref(), ScalarExpression::Name { .. }))
        {
            self.place_expressions(place, scope)?;
            self.point(
                CfgPointKind::Evaluate(expression.clone()),
                span,
                scope,
                BTreeSet::new(),
            );
            return Ok(ReferenceValue::Other);
        }
        self.place_expressions(place, scope)?;
        let pointer_point = match reference_root(place) {
            ScalarPlace::Dereference { span, .. } => {
                Some(*self.dereference_points.get(span).ok_or_else(|| {
                    self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    )
                })?)
            }
            ScalarPlace::Name { .. } => None,
            ScalarPlace::Field { .. } | ScalarPlace::Index { .. } => unreachable!(),
        };
        let candidates = if pointer_point.is_none() {
            vec![self.assignment_place(place, span)?]
        } else {
            Vec::new()
        };
        let event = CfgPointKind::ReadPlace {
            place: place.clone(),
            pointer_point,
            candidates,
            reads_origin: reads_checked_value,
        };
        if reads_checked_value || reads_checked_aggregate {
            self.uncomputed_point(event, span, scope);
            Ok(ReferenceValue::Uncomputed)
        } else {
            self.point(event, span, scope, BTreeSet::new());
            Ok(ReferenceValue::Other)
        }
    }

    fn place_indices(
        &mut self,
        place: &ScalarPlace,
        scope: ReferenceScopeId,
    ) -> Result<(), ReferenceAnalysisError> {
        match place {
            ScalarPlace::Name { .. } | ScalarPlace::Dereference { .. } => {}
            ScalarPlace::Field { base, .. } => self.place_indices(base, scope)?,
            ScalarPlace::Index { base, index, .. } => {
                self.place_indices(base, scope)?;
                self.expression(index, scope)?;
            }
        }
        Ok(())
    }

    fn indexed_move_type(&self, place: &ScalarPlace) -> Result<ScalarType, ReferenceAnalysisError> {
        match place {
            ScalarPlace::Name { name, span } => {
                let binding = self.binding(name, *span)?;
                self.binding_types.get(&binding).cloned().ok_or_else(|| {
                    self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    )
                })
            }
            ScalarPlace::Field { base, field, span } => {
                let ScalarType::Struct(id) = self.indexed_move_type(base)? else {
                    return Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    ));
                };
                let ScalarFieldReference::Resolved(field) = field else {
                    return Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    ));
                };
                self.structs
                    .iter()
                    .find(|structure| structure.id == id)
                    .and_then(|structure| {
                        structure
                            .fields
                            .iter()
                            .find(|candidate| candidate.id == *field)
                    })
                    .map(|field| field.ty.clone())
                    .ok_or_else(|| {
                        self.missing_context(
                            *span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })
            }
            ScalarPlace::Index { base, span, .. } => match self.indexed_move_type(base)? {
                ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
                    Ok(*element)
                }
                _ => Err(self
                    .missing_context(*span, ReferenceAnalysisErrorReason::MissingResolvedBinding)),
            },
            ScalarPlace::Dereference { pointer, span } => {
                let pointer_type = match pointer.as_ref() {
                    ScalarExpression::Name { name, .. } => {
                        self.indexed_move_type(&ScalarPlace::Name {
                            name: name.clone(),
                            span: *span,
                        })?
                    }
                    ScalarExpression::Call {
                        receiver: Some(receiver),
                        name,
                        type_arguments,
                        ..
                    } if receiver == "core"
                        && name == "pointer_cast"
                        && type_arguments.len() == 1 =>
                    {
                        type_arguments[0].ty.clone()
                    }
                    _ => {
                        return Err(self.missing_context(
                            *span,
                            ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                        ));
                    }
                };
                match pointer_type {
                    ScalarType::CheckedReference { inner, .. } => Ok(*inner),
                    ScalarType::RawPointer(inner) => Ok(*inner),
                    _ => Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    )),
                }
            }
        }
    }

    fn move_value(
        &mut self,
        value: &ScalarExpression,
        scope: ReferenceScopeId,
    ) -> Result<(), ReferenceAnalysisError> {
        let (name, place, ty, span) = match value {
            ScalarExpression::Name { name, span } => {
                let Some(binding) = self.names.get(name) else {
                    return Ok(());
                };
                (
                    name,
                    ScalarPlace::Name {
                        name: name.clone(),
                        span: *span,
                    },
                    self.binding_types.get(binding).cloned().ok_or_else(|| {
                        self.missing_context(
                            *span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?,
                    *span,
                )
            }
            ScalarExpression::Member {
                receiver,
                name,
                receiver_span,
                span,
                ..
            } => {
                let Some(binding) = self.names.get(receiver) else {
                    return Ok(());
                };
                let Some(receiver_type) = self.binding_types.get(binding) else {
                    return Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    ));
                };
                let ScalarType::Struct(id) = receiver_type else {
                    return Ok(());
                };
                let field = self
                    .structs
                    .iter()
                    .find(|item| item.id == *id)
                    .ok_or_else(|| {
                        self.missing_context(
                            *span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?
                    .fields
                    .iter()
                    .find(|field| field.name == *name)
                    .ok_or_else(|| {
                        self.missing_context(
                            *span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?;
                (
                    receiver,
                    ScalarPlace::Field {
                        base: Box::new(ScalarPlace::Name {
                            name: receiver.clone(),
                            span: *receiver_span,
                        }),
                        field: ScalarFieldReference::Resolved(field.id.clone()),
                        span: *span,
                    },
                    field.ty.clone(),
                    *span,
                )
            }
            ScalarExpression::IndexedRead { place, span } => {
                if let ScalarPlace::Name { name, .. } = reference_root(place) {
                    if self.module_names.contains(name) && !self.names.contains_key(name) {
                        return Ok(());
                    }
                }
                let ty = self.indexed_move_type(place)?;
                let policy = copy_policy(&ty, &self.structs, &self.enums, &self.generic_parameters)
                    .map_err(|_| {
                        self.missing_context(
                            *span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?;
                match policy {
                    CopyPolicy::Copy => return Ok(()),
                    CopyPolicy::Move => {
                        return Err(self.missing_context(
                            *span,
                            ReferenceAnalysisErrorReason::MoveFromArrayElement,
                        ));
                    }
                    CopyPolicy::Deferred => {
                        self.point(
                            CfgPointKind::DeferredArrayElement {
                                place: place.clone(),
                                ty,
                            },
                            *span,
                            scope,
                            BTreeSet::new(),
                        );
                        return Ok(());
                    }
                }
            }
            ScalarExpression::Dereference { place, span } => {
                let ScalarPlace::Dereference {
                    pointer,
                    span: root_span,
                } = reference_root(place)
                else {
                    return Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    ));
                };
                let ty = self.indexed_move_type(place)?;
                let policy = copy_policy(&ty, &self.structs, &self.enums, &self.generic_parameters)
                    .map_err(|_| {
                        self.missing_context(
                            *span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?;
                if policy == CopyPolicy::Copy {
                    return Ok(());
                }
                let ScalarExpression::Name { name, .. } = pointer.as_ref() else {
                    return Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    ));
                };
                let binding = self.binding(name, *span)?;
                let Some(ScalarType::CheckedReference {
                    mutability: ScalarReferenceMutability::Mutable,
                    ..
                }) = self.binding_types.get(&binding)
                else {
                    return Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    ));
                };
                let pointer_point = *self.dereference_points.get(root_span).ok_or_else(|| {
                    self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    )
                })?;
                self.point(
                    if policy == CopyPolicy::Deferred {
                        CfgPointKind::DeferredMoveThrough {
                            place: place.clone(),
                            ty,
                            pointer_point,
                            candidates: Vec::new(),
                        }
                    } else {
                        CfgPointKind::MoveThrough {
                            place: place.clone(),
                            ty,
                            pointer_point,
                            candidates: Vec::new(),
                        }
                    },
                    *span,
                    scope,
                    BTreeSet::new(),
                );
                return Ok(());
            }
            _ => return Ok(()),
        };
        let policy = copy_policy(&ty, &self.structs, &self.enums, &self.generic_parameters)
            .map_err(|_| {
                self.missing_context(span, ReferenceAnalysisErrorReason::MissingResolvedBinding)
            })?;
        if policy == CopyPolicy::Copy {
            return Ok(());
        }
        let binding = self
            .names
            .get(name)
            .expect("validated move binding")
            .clone();
        let mut projections = Vec::new();
        collect_reference_projections(&place, None, &mut projections);
        let place = ReferencePlaceId {
            source: self.source.clone(),
            function_span: self.function_span,
            declaration_span: binding.declaration_span,
            block_span: binding.block_span,
            binding: binding.clone(),
            projections,
            place,
        };
        self.point(
            if policy == CopyPolicy::Deferred {
                CfgPointKind::DeferredMove {
                    place,
                    ty,
                    owner: binding,
                }
            } else {
                CfgPointKind::Move {
                    place,
                    ty,
                    owner: binding,
                }
            },
            span,
            scope,
            BTreeSet::new(),
        );
        Ok(())
    }

    fn additional_receivers(
        &mut self,
        binding: &ScalarBinding,
        block: &ScalarBlock,
        scope: ReferenceScopeId,
        declared: &mut Vec<(String, ReferenceBindingId, Option<ReferenceBindingId>)>,
    ) -> Result<(), ReferenceAnalysisError> {
        for (position, receiver) in binding.receivers.iter().enumerate().skip(1) {
            let value = &binding.output_values[position].value;
            if binding.output_origin == ScalarBindingOutputOrigin::IndependentExpressions {
                self.expression(value, scope)?;
            } else if matches!(value, ScalarExpression::Call { .. }) {
                let argument_points = self
                    .cfg
                    .points
                    .iter()
                    .rev()
                    .find_map(|point| match &point.kind {
                        CfgPointKind::Call {
                            expression,
                            output: 0,
                            argument_points,
                            ..
                        } if expression == value => Some(argument_points.clone()),
                        _ => None,
                    })
                    .expect("evaluated multi-output call");
                self.uncomputed_point(
                    CfgPointKind::Call {
                        expression: value.clone(),
                        output: position,
                        target: self.call_target(value)?,
                        argument_points,
                    },
                    expression_span(value),
                    scope,
                );
            }
            let id = ReferenceBindingId {
                source: self.source.clone(),
                function_span: self.function_span,
                declaration_span: receiver.name_span,
                block_span: block.span,
                kind: ReferenceBindingKind::Declared,
            };
            let previous = self.names.insert(receiver.name.clone(), id.clone());
            self.binding_types.insert(id.clone(), receiver.ty.clone());
            declared.push((receiver.name.clone(), id.clone(), previous));
            if matches!(receiver.ty, ScalarType::CheckedReference { .. }) {
                self.checked.insert(id.clone());
                self.uncomputed_point(
                    CfgPointKind::Bind { binding: id },
                    receiver.name_span,
                    scope,
                );
            } else {
                self.point(
                    CfgPointKind::Bind { binding: id },
                    receiver.name_span,
                    scope,
                    BTreeSet::new(),
                );
            }
        }
        Ok(())
    }

    fn block(
        &mut self,
        block: &ScalarBlock,
    ) -> Result<Option<ReferenceValue>, ReferenceAnalysisError> {
        let scope = ReferenceScopeId(self.next_scope);
        self.next_scope += 1;
        self.point(CfgPointKind::ScopeEnter, block.span, scope, BTreeSet::new());
        let mut declared = Vec::new();
        let mut result = None;
        let mut return_values = Vec::new();
        for (index, item) in block.items.iter().enumerate() {
            match item {
                ScalarBlockItem::LocalBinding(binding) => {
                    let value = if matches!(binding.declared_type, ScalarType::RawPointer(_))
                        && matches!(&binding.value, ScalarExpression::Name { name, .. } if name == "null")
                    {
                        self.point(
                            CfgPointKind::Evaluate(binding.value.clone()),
                            expression_span(&binding.value),
                            scope,
                            BTreeSet::new(),
                        );
                        ReferenceValue::Other
                    } else {
                        self.expression(&binding.value, scope)?
                    };
                    self.move_value(&binding.value, scope)?;
                    let id = ReferenceBindingId {
                        source: self.source.clone(),
                        function_span: self.function_span,
                        declaration_span: binding.name_span,
                        block_span: block.span,
                        kind: ReferenceBindingKind::Declared,
                    };
                    self.binding_types
                        .insert(id.clone(), binding.declared_type.clone());
                    if binding.is_allocation {
                        self.cfg.allocations.insert(id.clone());
                    }
                    let origins = match value {
                        ReferenceValue::Checked(origins) => Some(origins),
                        ReferenceValue::Other
                            if matches!(
                                binding.declared_type,
                                ScalarType::CheckedReference { .. }
                            ) =>
                        {
                            return Err(self.missing_origin(
                                binding.name_span,
                                id,
                                ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                            ));
                        }
                        ReferenceValue::Other => None,
                        ReferenceValue::Uncomputed
                            if matches!(
                                binding.declared_type,
                                ScalarType::CheckedReference { .. }
                            ) =>
                        {
                            self.checked.insert(id.clone());
                            self.origins.remove(&id);
                            self.uncomputed_point(
                                CfgPointKind::Bind {
                                    binding: id.clone(),
                                },
                                binding.name_span,
                                scope,
                            );
                            let old = self.names.insert(binding.name.clone(), id.clone());
                            declared.push((binding.name.clone(), id, old));
                            self.additional_receivers(binding, block, scope, &mut declared)?;
                            continue;
                        }
                        ReferenceValue::Uncomputed => None,
                    };
                    let old = self.names.insert(binding.name.clone(), id.clone());
                    if let Some(origins) = origins {
                        self.checked.insert(id.clone());
                        self.origins.insert(id.clone(), origins.clone());
                        self.point(
                            CfgPointKind::Bind {
                                binding: id.clone(),
                            },
                            binding.name_span,
                            scope,
                            origins,
                        );
                    } else {
                        self.point(
                            CfgPointKind::Bind {
                                binding: id.clone(),
                            },
                            binding.name_span,
                            scope,
                            BTreeSet::new(),
                        );
                    }
                    declared.push((binding.name.clone(), id, old));
                    self.additional_receivers(binding, block, scope, &mut declared)?;
                }
                ScalarBlockItem::Expression(expression) => {
                    let value = self.expression(expression, scope)?;
                    let value_point = self.cursor.expect("evaluated output expression");
                    self.move_value(expression, scope)?;
                    if index >= block.items.len() - block.final_output_values.len() {
                        return_values.push(value_point);
                        if matches!(value, ReferenceValue::Checked(_)) {
                            result = Some(value);
                        } else if matches!(value, ReferenceValue::Uncomputed)
                            && matches!(
                                block.final_output_values
                                    [index - (block.items.len() - block.final_output_values.len())]
                                .ty,
                                ScalarType::CheckedReference { .. }
                            )
                        {
                            result = Some(value);
                        }
                    }
                }
                ScalarBlockItem::Assignment(assignment) => {
                    let output_types = self.assignment_output_types(assignment)?;
                    let mut values = Vec::new();
                    for value in &assignment.values {
                        let result = self.expression(value, scope)?;
                        let rhs_point = self.cursor.expect("evaluated assignment output");
                        self.move_value(value, scope)?;
                        values.push((result, rhs_point));
                    }
                    if assignment.values.len() == 1 {
                        if let ScalarExpression::Call { .. } = &assignment.values[0] {
                            let argument_points = self
                                .cfg
                                .points
                                .iter()
                                .rev()
                                .find_map(|point| match &point.kind {
                                    CfgPointKind::Call {
                                        expression,
                                        output: 0,
                                        argument_points,
                                        ..
                                    } if expression == &assignment.values[0] => {
                                        Some(argument_points.clone())
                                    }
                                    _ => None,
                                })
                                .expect("evaluated assignment call");
                            for position in 1..assignment.targets.len() {
                                let rhs_point = self.point_with_origins(
                                    CfgPointKind::Call {
                                        expression: assignment.values[0].clone(),
                                        output: position,
                                        target: self.call_target(&assignment.values[0])?,
                                        argument_points: argument_points.clone(),
                                    },
                                    expression_span(&assignment.values[0]),
                                    scope,
                                    None,
                                );
                                values.push((ReferenceValue::Uncomputed, rhs_point));
                            }
                        }
                    }
                    for (position, (target, (value, rhs_point))) in
                        assignment.targets.iter().zip(values).enumerate()
                    {
                        if let ScalarPlace::Name { name, span } = &target.place {
                            let id = if let Some(id) = self.names.get(name) {
                                id.clone()
                            } else if self.module_names.contains(name) {
                                if matches!(value, ReferenceValue::Checked(_)) {
                                    return Err(self.missing_context(
                                        *span,
                                        ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                                    ));
                                }
                                self.point(
                                    CfgPointKind::Evaluate(ScalarExpression::Name {
                                        name: name.clone(),
                                        span: *span,
                                    }),
                                    *span,
                                    scope,
                                    BTreeSet::new(),
                                );
                                continue;
                            } else {
                                let id = ReferenceBindingId {
                                    source: self.source.clone(),
                                    function_span: self.function_span,
                                    declaration_span: target.target_span,
                                    block_span: block.span,
                                    kind: ReferenceBindingKind::Declared,
                                };
                                self.names.insert(name.clone(), id.clone());
                                let ty = output_types.get(position).ok_or_else(|| {
                                    self.missing_context(
                                        target.target_span,
                                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                    )
                                })?;
                                self.binding_types.insert(id.clone(), ty.clone());
                                declared.push((name.clone(), id.clone(), None));
                                id
                            };
                            let target_place =
                                self.assignment_place(&target.place, target.target_span)?;
                            let previous_origins =
                                self.origins.get(&id).cloned().unwrap_or_default();
                            let previous_loans = previous_origins
                                .iter()
                                .filter_map(|origin| match &self.cfg.facts[origin.0] {
                                    ReferenceOrigin::Fresh { loan, .. }
                                    | ReferenceOrigin::BorrowedFrom { loan, .. } => {
                                        Some(loan.clone())
                                    }
                                    ReferenceOrigin::Null { .. }
                                    | ReferenceOrigin::Invalid { .. } => None,
                                })
                                .collect();
                            let assign = CfgPointKind::Assign {
                                target: target_place,
                                rhs_point,
                                previous_origins,
                                previous_loans,
                            };
                            if let ReferenceValue::Checked(origins) = value {
                                self.checked.insert(id.clone());
                                self.origins.insert(id.clone(), origins.clone());
                                self.point(assign, *span, scope, origins);
                            } else if matches!(value, ReferenceValue::Uncomputed)
                                && self.checked.contains(&id)
                            {
                                self.checked.insert(id.clone());
                                self.origins.remove(&id);
                                self.uncomputed_point(assign, *span, scope);
                            } else {
                                if self.checked.contains(&id) {
                                    return Err(self.missing_origin(
                                        *span,
                                        id,
                                        ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                                    ));
                                }
                                self.origins.remove(&id);
                                self.point(assign, *span, scope, BTreeSet::new());
                            }
                        } else {
                            if let ScalarPlace::Name { name, span } = reference_root(&target.place)
                            {
                                if self.module_names.contains(name)
                                    && !self.names.contains_key(name)
                                {
                                    self.point(
                                        CfgPointKind::Evaluate(ScalarExpression::Name {
                                            name: name.clone(),
                                            span: *span,
                                        }),
                                        target.target_span,
                                        scope,
                                        BTreeSet::new(),
                                    );
                                    continue;
                                }
                            }
                            self.place_expressions(&target.place, scope)?;
                            if let ScalarPlace::Dereference { span, .. } =
                                reference_root(&target.place)
                            {
                                let pointer_point = *self.dereference_points.get(span).ok_or_else(|| {
                                    self.missing_context(
                                        target.target_span,
                                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                                    )
                                })?;
                                let assign = CfgPointKind::AssignThrough {
                                    place: target.place.clone(),
                                    pointer_point,
                                    rhs_point,
                                    candidates: Vec::new(),
                                    previous_origins: BTreeSet::new(),
                                    previous_loans: Vec::new(),
                                };
                                match value {
                                    ReferenceValue::Checked(origins) => {
                                        self.point(assign, target.target_span, scope, origins);
                                    }
                                    ReferenceValue::Uncomputed
                                        if matches!(
                                            output_types[position],
                                            ScalarType::CheckedReference { .. }
                                        ) =>
                                    {
                                        self.uncomputed_point(assign, target.target_span, scope);
                                    }
                                    _ => {
                                        self.point(
                                            assign,
                                            target.target_span,
                                            scope,
                                            BTreeSet::new(),
                                        );
                                    }
                                }
                            } else {
                                let target_place =
                                    self.assignment_place(&target.place, target.target_span)?;
                                let assign = CfgPointKind::Assign {
                                    target: target_place,
                                    rhs_point,
                                    previous_origins: BTreeSet::new(),
                                    previous_loans: Vec::new(),
                                };
                                match value {
                                    ReferenceValue::Checked(origins) => {
                                        self.point(assign, target.target_span, scope, origins);
                                    }
                                    ReferenceValue::Uncomputed
                                        if matches!(
                                            output_types[position],
                                            ScalarType::CheckedReference { .. }
                                        ) =>
                                    {
                                        self.uncomputed_point(assign, target.target_span, scope);
                                    }
                                    _ => {
                                        self.point(
                                            assign,
                                            target.target_span,
                                            scope,
                                            BTreeSet::new(),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                ScalarBlockItem::While(loop_) => {
                    let loop_head = self.point(
                        CfgPointKind::LoopTest {
                            condition: loop_.condition.clone(),
                        },
                        expression_span(&loop_.condition),
                        scope,
                        BTreeSet::new(),
                    );
                    self.expression(&loop_.condition, scope)?;
                    let branch = self.point(
                        CfgPointKind::Branch {
                            condition: loop_.condition.clone(),
                        },
                        expression_span(&loop_.condition),
                        scope,
                        BTreeSet::new(),
                    );
                    let names = self.names.clone();
                    let origins = self.origins.clone();
                    let checked = self.checked.clone();
                    self.block(&loop_.body)?;
                    let back = self.cursor.expect("loop body terminal");
                    self.cfg.points[back.0].successors.push(loop_head);
                    self.names = names;
                    self.origins = origins;
                    self.checked = checked;
                    self.cursor = Some(branch);
                    self.point(CfgPointKind::Join, loop_.span, scope, BTreeSet::new());
                }
            }
        }
        if scope == ReferenceScopeId(0) {
            for (output, value_point) in return_values.into_iter().enumerate() {
                self.uncomputed_point(
                    CfgPointKind::ReturnOutput {
                        output,
                        value_point,
                        checked: matches!(
                            block.final_output_values[output].ty,
                            ScalarType::CheckedReference { .. }
                        ),
                    },
                    block.final_output_values[output].span,
                    scope,
                );
            }
        }
        self.point(CfgPointKind::ScopeExit, block.span, scope, BTreeSet::new());
        if scope == ReferenceScopeId(0) && !block.final_output_values.is_empty() {
            let return_span = ByteSpan::new(
                block
                    .final_output_values
                    .first()
                    .expect("output")
                    .span
                    .start,
                block.final_output_values.last().expect("output").span.end,
            );
            if let Some(value) = &result {
                match value {
                    ReferenceValue::Checked(origins) => {
                        self.point(CfgPointKind::Return, return_span, scope, origins.clone());
                    }
                    ReferenceValue::Uncomputed => {
                        self.uncomputed_point(CfgPointKind::Return, return_span, scope);
                    }
                    ReferenceValue::Other => {
                        self.point(CfgPointKind::Return, return_span, scope, BTreeSet::new());
                    }
                }
            } else {
                self.point(CfgPointKind::Return, return_span, scope, BTreeSet::new());
            }
        }
        for (name, id, previous) in declared.into_iter().rev() {
            self.origins.remove(&id);
            self.checked.remove(&id);
            self.binding_types.remove(&id);
            if let Some(previous) = previous {
                self.names.insert(name, previous);
            } else {
                self.names.remove(&name);
            }
        }
        Ok(result)
    }
}

#[derive(Clone, Default, PartialEq)]
struct ReferenceEdgeState {
    bindings: BTreeMap<ReferenceBindingId, BTreeSet<ReferenceOriginId>>,
    projected: Vec<ReferenceProjectedState>,
    uninitialized: HashSet<ReferenceBindingId>,
    value: Option<BTreeSet<ReferenceOriginId>>,
    released: BTreeSet<ReferenceBindingId>,
    invalidated: BTreeSet<ReferenceBindingId>,
    point_values: Vec<Option<BTreeSet<ReferenceOriginId>>>,
    return_outputs: BTreeMap<usize, BTreeSet<ReferenceOriginId>>,
    moved: Vec<(ReferencePlaceId, SourceSpan)>,
}

#[derive(Clone, PartialEq)]
struct ReferenceProjectedState {
    place: ReferencePlaceId,
    origins: BTreeSet<ReferenceOriginId>,
    uninitialized: bool,
}

fn reference_places_same(left: &ReferencePlaceId, right: &ReferencePlaceId) -> bool {
    left.binding == right.binding
        && left.projections.len() == right.projections.len()
        && left
            .projections
            .iter()
            .zip(&right.projections)
            .all(|(left, right)| match (left, right) {
                (
                    ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(left)),
                    ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(right)),
                ) => left == right,
                (
                    ReferencePlaceProjection::Index(ScalarExpression::Integer {
                        value: left, ..
                    }),
                    ReferencePlaceProjection::Index(ScalarExpression::Integer {
                        value: right, ..
                    }),
                ) => left == right,
                (
                    ReferencePlaceProjection::Dereference(left),
                    ReferencePlaceProjection::Dereference(right),
                ) => left == right,
                _ => false,
            })
}

fn available_place(place: &ReferencePlaceId) -> ReferencePlaceId {
    let mut resolved = place.clone();
    let mut projections = Vec::new();
    for projection in &place.projections {
        match projection {
            ReferencePlaceProjection::Dereference(loan) => {
                let owner = available_place(&loan.origin_place);
                resolved.binding = owner.binding;
                projections = owner.projections;
            }
            projection => projections.push(projection.clone()),
        }
    }
    resolved.projections = projections;
    resolved
}

fn unavailable_place(state: &ReferenceEdgeState, place: &ReferencePlaceId) -> bool {
    let place = available_place(place);
    state
        .moved
        .iter()
        .any(|(moved, _)| reference_places_overlap(moved, &place))
}

fn unavailable_origins(state: &ReferenceEdgeState, place: &ReferencePlaceId) -> Vec<SourceSpan> {
    let place = available_place(place);
    state
        .moved
        .iter()
        .filter(|(moved, _)| reference_places_overlap(moved, &place))
        .map(|(_, origin)| origin.clone())
        .collect()
}

fn reference_read_reachable(
    points: &[ReferenceFlowPoint],
    from: CfgPointId,
    binding: &ReferenceBindingId,
) -> bool {
    let mut pending = points[from.0].successors.clone();
    let mut visited = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id.0) {
            continue;
        }
        if matches!(&points[id.0].kind, CfgPointKind::Read { binding: read } if read == binding) {
            return true;
        }
        if matches!(&points[id.0].kind, CfgPointKind::Assign { target, .. } if target.binding == *binding && target.projections.is_empty())
            || matches!(&points[id.0].kind, CfgPointKind::Bind { binding: bound } if bound == binding)
            || matches!(&points[id.0].kind, CfgPointKind::ScopeExit if points[id.0].source_span.range == binding.block_span)
        {
            continue;
        }
        pending.extend(&points[id.0].successors);
    }
    false
}

fn loan_borrows_place(loan: &ReferenceLoanId, place: &ReferencePlaceId) -> bool {
    reference_places_overlap(&available_place(&loan.origin_place), place)
        || loan
            .parent
            .as_ref()
            .is_some_and(|parent| loan_borrows_place(parent, place))
}

fn conflicting_live_loan(
    points: &[ReferenceFlowPoint],
    facts: &[ReferenceOrigin],
    state: &ReferenceEdgeState,
    point: CfgPointId,
    place: &ReferencePlaceId,
    consuming_origin: Option<ReferenceOriginId>,
) -> Vec<SourceSpan> {
    let place = available_place(place);
    state
        .bindings
        .iter()
        .filter(|(binding, _)| reference_read_reachable(points, point, binding))
        .flat_map(|(_, origins)| origins.iter())
        .filter_map(|origin| {
            if Some(*origin) == consuming_origin {
                return None;
            }
            match &facts[origin.0] {
                ReferenceOrigin::Fresh { loan, .. }
                | ReferenceOrigin::BorrowedFrom { loan, .. }
                    if loan_borrows_place(loan, &place) =>
                {
                    Some(SourceSpan::new(
                        loan.origin_place.source.clone(),
                        loan.creation_span,
                    ))
                }
                _ => None,
            }
        })
        .collect()
}

fn write_projected_reference(
    state: &mut ReferenceEdgeState,
    place: &ReferencePlaceId,
    origins: &BTreeSet<ReferenceOriginId>,
) {
    state
        .projected
        .retain(|known| !reference_places_overlap(&known.place, place));
    state.projected.push(ReferenceProjectedState {
        place: place.clone(),
        origins: origins.clone(),
        uninitialized: false,
    });
}

fn transfer_reference_facts(
    cfg: &mut ScalarReferenceCfg,
    deferred_policies: &BTreeMap<usize, CopyPolicy>,
    contracts: &[ConcreteReferenceOutputContract],
) -> Vec<ReferenceAnalysisError> {
    let original_points = cfg.points.clone();
    let mut last_use = (0..cfg.points.len()).collect::<Vec<_>>();
    for point in &original_points {
        let dependencies = match &point.kind {
            CfgPointKind::Invalidate { address_point, .. }
            | CfgPointKind::Release {
                target: ReleaseTarget::Checked { address_point, .. },
            } => vec![*address_point],
            CfgPointKind::MoveThrough { pointer_point, .. }
            | CfgPointKind::DeferredMoveThrough { pointer_point, .. } => vec![*pointer_point],
            CfgPointKind::Assign { rhs_point, .. } => vec![*rhs_point],
            CfgPointKind::ReturnOutput { value_point, .. } => vec![*value_point],
            CfgPointKind::Call {
                argument_points, ..
            } => argument_points.clone(),
            CfgPointKind::AssignThrough {
                rhs_point,
                pointer_point,
                ..
            } => {
                vec![*rhs_point, *pointer_point]
            }
            CfgPointKind::ReadPlace { pointer_point, .. } => {
                pointer_point.iter().copied().collect()
            }
            _ => Vec::new(),
        };
        for dependency in dependencies {
            last_use[dependency.0] = point.id.0;
        }
    }
    let mut observed = vec![None::<ReferenceFlowPoint>; cfg.points.len()];
    let mut incoming = vec![Vec::<ReferenceEdgeState>::new(); cfg.points.len()];
    incoming[0].push(ReferenceEdgeState {
        point_values: vec![None; cfg.points.len()],
        ..ReferenceEdgeState::default()
    });
    let mut visited = vec![Vec::<ReferenceEdgeState>::new(); cfg.points.len()];
    let mut queued = vec![false; cfg.points.len()];
    queued[0] = true;
    let mut ready = std::collections::VecDeque::from([CfgPointId(0)]);
    let mut errors = Vec::new();
    while let Some(id) = ready.pop_front() {
        queued[id.0] = false;
        let multiple_paths = incoming[id.0].len() > 1;
        for mut state in std::mem::take(&mut incoming[id.0]) {
            if visited[id.0].contains(&state) {
                continue;
            }
            visited[id.0].push(state.clone());
            let mut move_alternatives = Vec::new();
            cfg.points[id.0] = original_points[id.0].clone();
            let invalidation_address = match &cfg.points[id.0].kind {
                CfgPointKind::Invalidate {
                    address_point,
                    address_span,
                    ..
                } => Some((
                    address_span.clone(),
                    state.point_values[address_point.0].clone(),
                )),
                CfgPointKind::Release {
                    target:
                        ReleaseTarget::Checked {
                            address_point,
                            address_span,
                            ..
                        },
                } => Some((
                    address_span.clone(),
                    state.point_values[address_point.0].clone(),
                )),
                _ => None,
            };
            let move_pointer = match &cfg.points[id.0].kind {
                CfgPointKind::MoveThrough { pointer_point, .. }
                | CfgPointKind::DeferredMoveThrough { pointer_point, .. } => {
                    Some(state.point_values[pointer_point.0].clone())
                }
                _ => None,
            };
            let assignment_rhs = match &cfg.points[id.0].kind {
                CfgPointKind::Assign { rhs_point, .. }
                | CfgPointKind::AssignThrough { rhs_point, .. } => {
                    Some(state.point_values[rhs_point.0].clone())
                }
                _ => None,
            };
            let assignment_pointer = match &cfg.points[id.0].kind {
                CfgPointKind::AssignThrough { pointer_point, .. } => {
                    Some(state.point_values[pointer_point.0].clone())
                }
                _ => None,
            };
            let place_read_pointer = match &cfg.points[id.0].kind {
                CfgPointKind::ReadPlace { pointer_point, .. } => {
                    pointer_point.map(|pointer_point| state.point_values[pointer_point.0].clone())
                }
                _ => None,
            };
            let call_origins = if let CfgPointKind::Call {
                expression: ScalarExpression::Call { type_arguments, .. },
                output,
                target:
                    ResolvedCallTarget::ModuleCallable {
                        source,
                        declaration_span,
                        concrete,
                    },
                argument_points,
                ..
            } = &cfg.points[id.0].kind
            {
                let contract = contracts.iter().find(|contract| {
                    contract.source == *source
                        && contract.declaration_span == *declaration_span
                        && contract.selection == *concrete
                        && contract.type_arguments
                            == type_arguments
                                .iter()
                                .map(|argument| argument.ty.clone())
                                .collect::<Vec<_>>()
                        && contract.output == *output
                });
                contract.map(|contract| {
                    let mut results = BTreeSet::new();
                    for origin in &contract.origins {
                        let actual = match origin {
                            ReferenceOrigin::BorrowedFrom { parameter, .. } => {
                                if let Some(argument) = state.point_values[argument_points[*parameter].0].as_ref() {
                                    results.extend(argument);
                                } else {
                                    errors.push(ReferenceAnalysisError::MissingContext {
                                        source_span: cfg.points[id.0].source_span.clone(),
                                        reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                                    });
                                }
                                continue;
                            }
                            ReferenceOrigin::Fresh { allocation, loan } => {
                                let mut loan = loan.clone();
                                let binding = ReferenceBindingId {
                                    source: cfg.points[id.0].source_span.source.clone(),
                                    function_span: cfg.points[0].source_span.range,
                                    declaration_span: cfg.points[id.0].source_span.range,
                                    block_span: cfg.points[0].source_span.range,
                                    kind: ReferenceBindingKind::CallOutput {
                                        point: id,
                                        output: *output,
                                    },
                                };
                                loan.origin_place.source = binding.source.clone();
                                loan.origin_place.function_span = binding.function_span;
                                loan.origin_place.declaration_span = binding.declaration_span;
                                loan.origin_place.block_span = binding.block_span;
                                loan.origin_place.binding = binding;
                                loan.creation_span = cfg.points[id.0].source_span.range;
                                ReferenceOrigin::Fresh {
                                    allocation: allocation.clone(),
                                    loan,
                                }
                            }
                            ReferenceOrigin::Null { .. } => ReferenceOrigin::Null {
                                span: cfg.points[id.0].source_span.range,
                            },
                            ReferenceOrigin::Invalid {
                                origin_span,
                                conflict_span,
                            } => ReferenceOrigin::Invalid {
                                origin_span: *origin_span,
                                conflict_span: *conflict_span,
                            },
                        };
                        let origin_id = if let Some(index) = cfg.facts.iter().position(|fact| fact == &actual) {
                            ReferenceOriginId(index)
                        } else {
                            let index = cfg.facts.len();
                            cfg.facts.push(actual);
                            ReferenceOriginId(index)
                        };
                        results.insert(origin_id);
                    }
                    results
                })
            } else {
                None
            };
            let point = &mut cfg.points[id.0];
            if let Some(origins) = call_origins {
                point.possible_origins = Some(origins);
            }
            if let CfgPointKind::ReadPlace {
                place,
                candidates,
                reads_origin,
                ..
            } = &mut point.kind
            {
                if let Some(origins) = place_read_pointer {
                    if let Some(origins) = origins.filter(|origins| !origins.is_empty()) {
                        if !*reads_origin {
                            point.possible_origins = Some(origins.clone());
                        }
                        for origin in origins {
                            match &cfg.facts[origin.0] {
                                ReferenceOrigin::Fresh { loan, .. }
                                | ReferenceOrigin::BorrowedFrom { loan, .. } => {
                                    let mut resolved = loan.origin_place.clone();
                                    resolved.place = place.clone();
                                    collect_reference_projections(
                                        place,
                                        Some(loan),
                                        &mut resolved.projections,
                                    );
                                    candidates.push(resolved);
                                }
                                ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => {
                                    if *reads_origin {
                                        errors.push(ReferenceAnalysisError::MissingContext {
                                            source_span: point.source_span.clone(),
                                            reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                                        });
                                    }
                                }
                            }
                        }
                    } else {
                        errors.push(ReferenceAnalysisError::MissingContext {
                            source_span: point.source_span.clone(),
                            reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                        });
                    }
                }
            }
            if let Some(origins) = assignment_pointer {
                if let Some(origins) = origins.filter(|origins| !origins.is_empty()) {
                    let CfgPointKind::AssignThrough {
                        place, candidates, ..
                    } = &mut point.kind
                    else {
                        unreachable!()
                    };
                    for origin in &origins {
                        let loan = match &cfg.facts[origin.0] {
                            ReferenceOrigin::Fresh { loan, .. }
                            | ReferenceOrigin::BorrowedFrom { loan, .. } => loan,
                            ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => {
                                errors.push(ReferenceAnalysisError::MissingContext {
                                source_span: point.source_span.clone(),
                                reason:
                                    ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                            });
                                continue;
                            }
                        };
                        let mut resolved_place = loan.origin_place.clone();
                        resolved_place.place = place.clone();
                        collect_reference_projections(place, None, &mut resolved_place.projections);
                        candidates.push(InvalidationCandidate {
                            origin: *origin,
                            place: resolved_place,
                            loan: loan.clone(),
                        });
                    }
                } else {
                    errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: point.source_span.clone(),
                        reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    });
                }
            }
            if let Some(origins) = move_pointer {
                match origins.filter(|origins| !origins.is_empty()) {
                    Some(origins) => {
                        let (place, ty) = match &point.kind {
                            CfgPointKind::MoveThrough { place, ty, .. }
                            | CfgPointKind::DeferredMoveThrough { place, ty, .. } => {
                                (place.clone(), ty.clone())
                            }
                            _ => unreachable!(),
                        };
                        let mut candidates = Vec::new();
                        for origin in &origins {
                            let loan = match &cfg.facts[origin.0] {
                                ReferenceOrigin::Fresh { loan, .. }
                                | ReferenceOrigin::BorrowedFrom { loan, .. } => loan,
                                ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => {
                                    errors.push(ReferenceAnalysisError::MissingContext {
                                    source_span: point.source_span.clone(),
                                    reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                                });
                                    continue;
                                }
                            };
                            let mut resolved_place = loan.origin_place.clone();
                            resolved_place.place = place.clone();
                            collect_reference_projections(
                                &place,
                                Some(loan),
                                &mut resolved_place.projections,
                            );
                            candidates.push(InvalidationCandidate {
                                origin: *origin,
                                place: resolved_place,
                                loan: loan.clone(),
                            });
                        }
                        let all_candidates_resolved = candidates.len() == origins.len();
                        point.possible_origins = Some(origins);
                        if !matches!(place, ScalarPlace::Dereference { .. })
                            && candidates.len() == 1
                            && all_candidates_resolved
                            && !multiple_paths
                            && matches!(point.kind, CfgPointKind::MoveThrough { .. })
                        {
                            let candidate = &candidates[0];
                            point.kind = CfgPointKind::Move {
                                place: candidate.place.clone(),
                                ty,
                                owner: candidate.place.binding.clone(),
                            };
                        } else if let CfgPointKind::MoveThrough {
                            candidates: event_candidates,
                            ..
                        } = &mut point.kind
                        {
                            *event_candidates = candidates;
                        } else if let CfgPointKind::DeferredMoveThrough {
                            candidates: event_candidates,
                            ..
                        } = &mut point.kind
                        {
                            *event_candidates = candidates;
                        }
                    }
                    None => errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: point.source_span.clone(),
                        reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    }),
                }
            }
            if let Some((address_span, origins)) = invalidation_address {
                if let Some(origins) = origins.filter(|origins| !origins.is_empty()) {
                    let mut candidates = Vec::new();
                    for origin in &origins {
                        let loan = match &cfg.facts[origin.0] {
                            ReferenceOrigin::Fresh { loan, .. }
                            | ReferenceOrigin::BorrowedFrom { loan, .. } => loan,
                            ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => {
                                errors.push(ReferenceAnalysisError::MissingContext {
                                source_span: address_span.clone(),
                                reason:
                                    ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                            });
                                continue;
                            }
                        };
                        candidates.push(InvalidationCandidate {
                            origin: *origin,
                            place: loan.origin_place.clone(),
                            loan: loan.clone(),
                        });
                    }
                    if let CfgPointKind::Invalidate {
                        candidates: event_candidates,
                        ..
                    } = &mut point.kind
                    {
                        *event_candidates = candidates;
                    } else if let CfgPointKind::Release {
                        target:
                            ReleaseTarget::Checked {
                                candidates: event_candidates,
                                ..
                            },
                    } = &mut point.kind
                    {
                        *event_candidates = candidates;
                    }
                    point.possible_origins = Some(origins);
                } else {
                    errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: address_span,
                        reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                    });
                }
            }
            let affected = match &point.kind {
                CfgPointKind::Invalidate { candidates, .. } => Some(candidates.as_slice()),
                CfgPointKind::Release {
                    target: ReleaseTarget::Checked { candidates, .. },
                } => Some(candidates.as_slice()),
                _ => None,
            };
            if let Some(candidates) = affected {
                for candidate in candidates {
                    let origins = conflicting_live_loan(
                        &original_points,
                        &cfg.facts,
                        &state,
                        id,
                        &candidate.place,
                        Some(candidate.origin),
                    );
                    if !origins.is_empty() {
                        errors.push(ReferenceAnalysisError::ConflictingPlace {
                            source_span: point.source_span.clone(),
                            origins,
                        });
                    }
                }
                if let CfgPointKind::Invalidate { operation, .. } = &point.kind {
                    for binding in candidates.iter().map(|candidate| &candidate.place.binding) {
                        match operation {
                            CoreOperationId::Invalidate => {
                                state.invalidated.insert(binding.clone());
                            }
                            CoreOperationId::Rebind => {
                                state.invalidated.remove(binding);
                            }
                            _ => unreachable!("resolved invalidation operation"),
                        }
                    }
                }
                if let CfgPointKind::Release {
                    target: ReleaseTarget::Checked { address_span, .. },
                } = &point.kind
                {
                    for binding in candidates
                        .iter()
                        .map(|candidate| &candidate.place.binding)
                        .collect::<BTreeSet<_>>()
                    {
                        if !cfg.allocations.contains(binding) {
                            errors.push(ReferenceAnalysisError::MissingContext {
                                source_span: address_span.clone(),
                                reason:
                                    ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                            });
                        } else if !state.released.insert(binding.clone()) {
                            errors.push(ReferenceAnalysisError::MissingContext {
                                source_span: point.source_span.clone(),
                                reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                            });
                        }
                    }
                }
                for origins in state.bindings.values_mut() {
                    origins.retain(|origin| {
                        let place = match &cfg.facts[origin.0] {
                            ReferenceOrigin::Fresh { loan, .. }
                            | ReferenceOrigin::BorrowedFrom { loan, .. } => &loan.origin_place,
                            ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => {
                                return true;
                            }
                        };
                        !candidates.iter().any(|candidate| {
                            place.binding == candidate.place.binding
                                && place.projections.starts_with(&candidate.place.projections)
                        })
                    });
                }
                state.projected.retain(|known| {
                    !candidates
                        .iter()
                        .any(|candidate| reference_places_overlap(&known.place, &candidate.place))
                });
            }
            if let CfgPointKind::Release {
                target: ReleaseTarget::Raw { binding },
            } = &point.kind
            {
                if !state.released.insert(binding.clone()) {
                    errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: point.source_span.clone(),
                        reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                    });
                }
                for origins in state.bindings.values_mut() {
                    origins.retain(|origin| match &cfg.facts[origin.0] {
                        ReferenceOrigin::Fresh { loan, .. }
                        | ReferenceOrigin::BorrowedFrom { loan, .. } => {
                            &loan.origin_place.binding != binding
                        }
                        ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => true,
                    });
                }
                state
                    .projected
                    .retain(|known| &known.place.binding != binding);
            }
            if let CfgPointKind::Assign {
                target,
                previous_origins,
                previous_loans,
                ..
            } = &mut point.kind
            {
                *previous_origins = if target.projections.is_empty() {
                    state
                        .bindings
                        .get(&target.binding)
                        .cloned()
                        .unwrap_or_default()
                } else {
                    state
                        .bindings
                        .values()
                        .flat_map(|origins| origins.iter().copied())
                        .filter(|origin| match &cfg.facts[origin.0] {
                            ReferenceOrigin::Fresh { loan, .. }
                            | ReferenceOrigin::BorrowedFrom { loan, .. } => {
                                reference_places_overlap(&loan.origin_place, target)
                            }
                            ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => false,
                        })
                        .collect()
                };
                *previous_loans = previous_origins
                    .iter()
                    .filter_map(|origin| match &cfg.facts[origin.0] {
                        ReferenceOrigin::Fresh { loan, .. }
                        | ReferenceOrigin::BorrowedFrom { loan, .. } => Some(loan.clone()),
                        ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => None,
                    })
                    .collect();
            }
            if let CfgPointKind::AssignThrough {
                candidates,
                previous_origins,
                previous_loans,
                ..
            } = &mut point.kind
            {
                *previous_origins = state
                    .bindings
                    .values()
                    .flat_map(|origins| origins.iter().copied())
                    .filter(|origin| match &cfg.facts[origin.0] {
                        ReferenceOrigin::Fresh { loan, .. }
                        | ReferenceOrigin::BorrowedFrom { loan, .. } => {
                            !candidates
                                .iter()
                                .any(|candidate| candidate.origin == *origin)
                                && candidates.iter().any(|candidate| {
                                    reference_places_overlap(&loan.origin_place, &candidate.place)
                                })
                        }
                        ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => false,
                    })
                    .collect();
                *previous_loans = previous_origins
                    .iter()
                    .filter_map(|origin| match &cfg.facts[origin.0] {
                        ReferenceOrigin::Fresh { loan, .. }
                        | ReferenceOrigin::BorrowedFrom { loan, .. } => Some(loan.clone()),
                        ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => None,
                    })
                    .collect();
            }
            let read_places = match &point.kind {
                CfgPointKind::ReadPlace { candidates, .. } => candidates.as_slice(),
                _ => &[],
            };
            for place in read_places {
                if unavailable_place(&state, place)
                    || state.released.contains(&place.binding)
                    || state.invalidated.contains(&place.binding)
                {
                    errors.push(ReferenceAnalysisError::ConflictingPlace {
                        source_span: point.source_span.clone(),
                        origins: unavailable_origins(&state, place),
                    });
                }
            }
            match &point.kind {
                CfgPointKind::Move {
                    place,
                    ty:
                        ScalarType::Struct(_)
                        | ScalarType::Array { .. }
                        | ScalarType::RuntimeArray { .. },
                    ..
                }
                | CfgPointKind::DeferredMove { place, .. }
                    if matches!(point.kind, CfgPointKind::Move { .. })
                        || deferred_policies.get(&id.0) == Some(&CopyPolicy::Move) =>
                {
                    let place = available_place(place);
                    let mut origins = unavailable_origins(&state, &place);
                    origins.extend(conflicting_live_loan(
                        &original_points,
                        &cfg.facts,
                        &state,
                        id,
                        &place,
                        None,
                    ));
                    if !origins.is_empty() {
                        errors.push(ReferenceAnalysisError::ConflictingPlace {
                            source_span: point.source_span.clone(),
                            origins,
                        });
                    }
                    if !state.moved.iter().any(|(moved, _)| moved == &place) {
                        state.moved.push((place, point.source_span.clone()));
                    }
                }
                CfgPointKind::MoveThrough { candidates, .. }
                | CfgPointKind::DeferredMoveThrough { candidates, .. }
                    if matches!(point.kind, CfgPointKind::MoveThrough { .. })
                        || deferred_policies.get(&id.0) == Some(&CopyPolicy::Move) =>
                {
                    for candidate in candidates {
                        let place = available_place(&candidate.place);
                        let mut origins = unavailable_origins(&state, &place);
                        origins.extend(conflicting_live_loan(
                            &original_points,
                            &cfg.facts,
                            &state,
                            id,
                            &place,
                            Some(candidate.origin),
                        ));
                        if !origins.is_empty() {
                            errors.push(ReferenceAnalysisError::ConflictingPlace {
                                source_span: point.source_span.clone(),
                                origins,
                            });
                        }
                        if !move_alternatives.iter().any(|(moved, _)| moved == &place) {
                            move_alternatives.push((
                                place,
                                SourceSpan::new(
                                    candidate.place.source.clone(),
                                    candidate.loan.creation_span,
                                ),
                            ));
                        }
                    }
                }
                CfgPointKind::Assign { target, .. } => {
                    let target = available_place(target);
                    if !target.projections.is_empty() {
                        let origins = conflicting_live_loan(
                            &original_points,
                            &cfg.facts,
                            &state,
                            id,
                            &target,
                            None,
                        );
                        if !origins.is_empty() {
                            errors.push(ReferenceAnalysisError::ConflictingPlace {
                                source_span: point.source_span.clone(),
                                origins,
                            });
                        }
                    }
                    state.moved.retain(|(moved, _)| {
                        moved.binding != target.binding
                            || !moved.projections.starts_with(&target.projections)
                    });
                }
                CfgPointKind::AssignThrough { candidates, .. } => {
                    for candidate in candidates {
                        let target = available_place(&candidate.place);
                        let origins = conflicting_live_loan(
                            &original_points,
                            &cfg.facts,
                            &state,
                            id,
                            &target,
                            Some(candidate.origin),
                        );
                        if !origins.is_empty() {
                            errors.push(ReferenceAnalysisError::ConflictingPlace {
                                source_span: point.source_span.clone(),
                                origins,
                            });
                        }
                        state.moved.retain(|(moved, _)| {
                            moved.binding != target.binding
                                || !moved.projections.starts_with(&target.projections)
                        });
                    }
                }
                _ => {}
            }
            let checked_value = match &point.kind {
                CfgPointKind::ReturnOutput { value_point, .. } => {
                    state.point_values[value_point.0].clone()
                }
                CfgPointKind::ReadPlace {
                    reads_origin: false,
                    ..
                } => None,
                CfgPointKind::ReadPlace {
                    candidates,
                    reads_origin: true,
                    ..
                } => {
                    let mut read = BTreeSet::new();
                    for candidate in candidates {
                        let matching = state
                            .projected
                            .iter()
                            .filter(|known| {
                                reference_places_same(&known.place, candidate)
                                    && !known.uninitialized
                            })
                            .collect::<Vec<_>>();
                        if matching.is_empty() {
                            errors.push(ReferenceAnalysisError::MissingOrigin {
                                source_span: point.source_span.clone(),
                                binding_id: candidate.binding.clone(),
                                reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                            });
                        }
                        for known in matching {
                            read.extend(&known.origins);
                        }
                    }
                    if read.is_empty() {
                        None
                    } else {
                        Some(read)
                    }
                }
                CfgPointKind::Read { binding } => {
                    if state
                        .moved
                        .iter()
                        .any(|(place, _)| &place.binding == binding)
                    {
                        errors.push(ReferenceAnalysisError::ConflictingPlace {
                            source_span: point.source_span.clone(),
                            origins: state
                                .moved
                                .iter()
                                .filter(|(place, _)| &place.binding == binding)
                                .map(|(_, origin)| origin.clone())
                                .collect(),
                        });
                    }
                    if state.released.contains(binding) || state.invalidated.contains(binding) {
                        errors.push(ReferenceAnalysisError::MissingOrigin {
                            source_span: point.source_span.clone(),
                            binding_id: binding.clone(),
                            reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                        });
                    }
                    if point
                        .possible_origins
                        .as_ref()
                        .is_some_and(BTreeSet::is_empty)
                    {
                        None
                    } else {
                        match state.bindings.get(binding).filter(|origins| {
                            !origins.is_empty() && !state.uninitialized.contains(binding)
                        }) {
                            Some(origins) => Some(origins.clone()),
                            None => {
                                errors.push(ReferenceAnalysisError::MissingOrigin {
                                    source_span: point.source_span.clone(),
                                    binding_id: binding.clone(),
                                    reason:
                                        ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                                });
                                None
                            }
                        }
                    }
                }
                CfgPointKind::Bind { binding }
                    if point.possible_origins.is_none()
                        || point
                            .possible_origins
                            .as_ref()
                            .is_some_and(|set| !set.is_empty()) =>
                {
                    let rhs = if point.possible_origins.is_none() {
                        state.value.as_ref()
                    } else {
                        point.possible_origins.as_ref()
                    };
                    match rhs.filter(|origins| !origins.is_empty()) {
                        Some(origins) => {
                            let origins = origins.clone();
                            state.bindings.insert(binding.clone(), origins.clone());
                            state.uninitialized.remove(binding);
                            Some(origins)
                        }
                        None => {
                            errors.push(ReferenceAnalysisError::MissingOrigin {
                                source_span: point.source_span.clone(),
                                binding_id: binding.clone(),
                                reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                            });
                            None
                        }
                    }
                }
                CfgPointKind::Assign {
                    target,
                    previous_origins,
                    ..
                } => {
                    let binding = &target.binding;
                    if target.projections.is_empty() {
                        state.bindings.remove(binding);
                        state
                            .projected
                            .retain(|known| &known.place.binding != binding);
                        state.uninitialized.remove(binding);
                        state.released.remove(binding);
                        state.invalidated.remove(binding);
                    } else {
                        for origins in state.bindings.values_mut() {
                            origins.retain(|origin| !previous_origins.contains(origin));
                        }
                        state
                            .projected
                            .retain(|known| !reference_places_overlap(&known.place, target));
                    }
                    if point.possible_origins.is_none()
                        || point
                            .possible_origins
                            .as_ref()
                            .is_some_and(|set| !set.is_empty())
                    {
                        match assignment_rhs
                            .flatten()
                            .filter(|origins| !origins.is_empty())
                        {
                            Some(origins) => {
                                if target.projections.is_empty() {
                                    state.bindings.insert(binding.clone(), origins.clone());
                                } else {
                                    write_projected_reference(&mut state, target, &origins);
                                }
                                Some(origins)
                            }
                            None => {
                                errors.push(ReferenceAnalysisError::MissingOrigin {
                                    source_span: point.source_span.clone(),
                                    binding_id: binding.clone(),
                                    reason:
                                        ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                                });
                                None
                            }
                        }
                    } else {
                        None
                    }
                }
                CfgPointKind::AssignThrough {
                    candidates,
                    previous_origins,
                    ..
                } => {
                    for origins in state.bindings.values_mut() {
                        origins.retain(|origin| !previous_origins.contains(origin));
                    }
                    let rhs = assignment_rhs
                        .flatten()
                        .filter(|origins| !origins.is_empty());
                    for candidate in candidates {
                        state.projected.retain(|known| {
                            !reference_places_overlap(&known.place, &candidate.place)
                        });
                    }
                    if point
                        .possible_origins
                        .as_ref()
                        .is_none_or(|origins| !origins.is_empty())
                    {
                        if let Some(origins) = &rhs {
                            for candidate in candidates {
                                write_projected_reference(&mut state, &candidate.place, origins);
                            }
                        } else {
                            errors.push(ReferenceAnalysisError::MissingContext {
                                source_span: point.source_span.clone(),
                                reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                            });
                        }
                    }
                    rhs
                }
                CfgPointKind::Evaluate(_)
                | CfgPointKind::Borrow { .. }
                | CfgPointKind::Call { .. } => {
                    if let CfgPointKind::Borrow { place, .. } = &point.kind {
                        if state.released.contains(&place.binding) {
                            errors.push(ReferenceAnalysisError::MissingContext {
                                source_span: point.source_span.clone(),
                                reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                            });
                        }
                    }
                    if point.possible_origins.is_none() {
                        state.value.clone()
                    } else {
                        point.possible_origins.clone().filter(|set| !set.is_empty())
                    }
                }
                CfgPointKind::Join if point.possible_origins.is_none() => state.value.clone(),
                CfgPointKind::Return => Some(
                    state
                        .return_outputs
                        .values()
                        .flat_map(|origins| origins.iter().copied())
                        .collect(),
                ),
                _ => None,
            };
            if matches!(point.kind, CfgPointKind::ReturnOutput { checked: true, .. })
                && checked_value.as_ref().is_none_or(BTreeSet::is_empty)
            {
                errors.push(ReferenceAnalysisError::MissingContext {
                    source_span: point.source_span.clone(),
                    reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                });
            }
            if let (CfgPointKind::ReturnOutput { checked: true, .. }, Some(origins)) =
                (&point.kind, &checked_value)
            {
                for origin in origins {
                    let loan = match &cfg.facts[origin.0] {
                        ReferenceOrigin::Fresh { loan, .. }
                        | ReferenceOrigin::BorrowedFrom { loan, .. } => loan,
                        ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => continue,
                    };
                    if unavailable_place(&state, &loan.origin_place)
                        || state.released.contains(&loan.origin_place.binding)
                        || state.invalidated.contains(&loan.origin_place.binding)
                    {
                        errors.push(ReferenceAnalysisError::ConflictingPlace {
                            source_span: point.source_span.clone(),
                            origins: vec![SourceSpan::new(
                                loan.origin_place.source.clone(),
                                loan.creation_span,
                            )],
                        });
                    }
                }
            }
            if let (CfgPointKind::ReturnOutput { output, .. }, Some(origins)) =
                (&point.kind, &checked_value)
            {
                if !origins.is_empty() {
                    state.return_outputs.insert(*output, origins.clone());
                }
            }
            if matches!(
                point.kind,
                CfgPointKind::Read { .. }
                    | CfgPointKind::ReadPlace {
                        reads_origin: true,
                        ..
                    }
                    | CfgPointKind::Bind { .. }
                    | CfgPointKind::Assign { .. }
                    | CfgPointKind::AssignThrough { .. }
                    | CfgPointKind::ReturnOutput { .. }
                    | CfgPointKind::Return
            ) || point.possible_origins.is_none()
            {
                point.possible_origins = checked_value.clone();
            }
            match &point.kind {
                CfgPointKind::ScopeExit => {
                    state
                        .bindings
                        .retain(|binding, _| binding.block_span != point.source_span.range);
                    state
                        .uninitialized
                        .retain(|binding| binding.block_span != point.source_span.range);
                    state
                        .moved
                        .retain(|(place, _)| place.binding.block_span != point.source_span.range);
                    state
                        .released
                        .retain(|binding| binding.block_span != point.source_span.range);
                    state
                        .invalidated
                        .retain(|binding| binding.block_span != point.source_span.range);
                    state
                        .projected
                        .retain(|known| known.place.binding.block_span != point.source_span.range);
                }
                CfgPointKind::Read { .. }
                | CfgPointKind::ReadPlace { .. }
                | CfgPointKind::Join
                | CfgPointKind::Evaluate(_)
                | CfgPointKind::Call { .. }
                | CfgPointKind::Borrow { .. }
                | CfgPointKind::Bind { .. }
                | CfgPointKind::Assign { .. } => state.value = checked_value,
                CfgPointKind::AssignThrough { .. } => state.value = checked_value,
                _ => {}
            }
            state.point_values[id.0] = point.possible_origins.clone();
            for (index, value) in state.point_values.iter_mut().enumerate().take(id.0 + 1) {
                if last_use[index] <= id.0 {
                    *value = None;
                }
            }
            if let Some(previous) = &mut observed[id.0] {
                if let Some(origins) = &point.possible_origins {
                    previous
                        .possible_origins
                        .get_or_insert_with(BTreeSet::new)
                        .extend(origins);
                }
                match (&mut previous.kind, &point.kind) {
                    (
                        CfgPointKind::Release {
                            target:
                                ReleaseTarget::Checked {
                                    candidates: known, ..
                                },
                        },
                        CfgPointKind::Release {
                            target: ReleaseTarget::Checked { candidates, .. },
                        },
                    )
                    | (
                        CfgPointKind::Invalidate {
                            candidates: known, ..
                        },
                        CfgPointKind::Invalidate { candidates, .. },
                    )
                    | (
                        CfgPointKind::AssignThrough {
                            candidates: known, ..
                        },
                        CfgPointKind::AssignThrough { candidates, .. },
                    )
                    | (
                        CfgPointKind::MoveThrough {
                            candidates: known, ..
                        },
                        CfgPointKind::MoveThrough { candidates, .. },
                    )
                    | (
                        CfgPointKind::DeferredMoveThrough {
                            candidates: known, ..
                        },
                        CfgPointKind::DeferredMoveThrough { candidates, .. },
                    ) => {
                        for candidate in candidates {
                            if !known.contains(candidate) {
                                known.push(candidate.clone());
                            }
                        }
                    }
                    (
                        CfgPointKind::ReadPlace {
                            candidates: known, ..
                        },
                        CfgPointKind::ReadPlace { candidates, .. },
                    ) => {
                        for candidate in candidates {
                            if !known.contains(candidate) {
                                known.push(candidate.clone());
                            }
                        }
                    }
                    (
                        CfgPointKind::Assign {
                            previous_origins: known,
                            previous_loans: loans,
                            ..
                        },
                        CfgPointKind::Assign {
                            previous_origins,
                            previous_loans,
                            ..
                        },
                    ) => {
                        known.extend(previous_origins);
                        for loan in previous_loans {
                            if !loans.contains(loan) {
                                loans.push(loan.clone());
                            }
                        }
                    }
                    _ => {}
                }
                if let (
                    CfgPointKind::AssignThrough {
                        previous_origins: known,
                        previous_loans: loans,
                        ..
                    },
                    CfgPointKind::AssignThrough {
                        previous_origins,
                        previous_loans,
                        ..
                    },
                ) = (&mut previous.kind, &point.kind)
                {
                    known.extend(previous_origins);
                    for loan in previous_loans {
                        if !loans.contains(loan) {
                            loans.push(loan.clone());
                        }
                    }
                }
            } else {
                observed[id.0] = Some(point.clone());
            }
            for successor in &point.successors {
                let slot = &mut incoming[successor.0];
                if move_alternatives.is_empty() {
                    let mut next = state.clone();
                    if successor.0 <= id.0 {
                        next.point_values[successor.0..].fill(None);
                    }
                    if !visited[successor.0].contains(&next) && !slot.contains(&next) {
                        slot.push(next);
                    }
                } else {
                    for (place, origin) in &move_alternatives {
                        let mut next = state.clone();
                        if !next.moved.iter().any(|(moved, _)| moved == place) {
                            next.moved.push((place.clone(), origin.clone()));
                        }
                        if successor.0 <= id.0 {
                            next.point_values[successor.0..].fill(None);
                        }
                        if !visited[successor.0].contains(&next) && !slot.contains(&next) {
                            slot.push(next);
                        }
                    }
                }
                if !slot.is_empty() && !queued[successor.0] {
                    queued[successor.0] = true;
                    ready.push_back(*successor);
                }
            }
        }
    }
    for (point, observed) in cfg.points.iter_mut().zip(observed) {
        if let Some(observed) = observed {
            *point = observed;
        }
    }
    let mut unique = Vec::new();
    for error in errors {
        if !unique.contains(&error) {
            unique.push(error);
        }
    }
    unique
}

fn reference_root(place: &ScalarPlace) -> &ScalarPlace {
    match place {
        ScalarPlace::Name { .. } | ScalarPlace::Dereference { .. } => place,
        ScalarPlace::Field { base, .. } | ScalarPlace::Index { base, .. } => reference_root(base),
    }
}

fn collect_reference_projections(
    place: &ScalarPlace,
    parent: Option<&ReferenceLoanId>,
    projections: &mut Vec<ReferencePlaceProjection>,
) {
    match place {
        ScalarPlace::Name { .. } => {}
        ScalarPlace::Dereference { .. } => {
            if let Some(parent) = parent {
                projections.push(ReferencePlaceProjection::Dereference(Box::new(
                    parent.clone(),
                )));
            }
        }
        ScalarPlace::Field { base, field, .. } => {
            collect_reference_projections(base, parent, projections);
            projections.push(ReferencePlaceProjection::Field(field.clone()));
        }
        ScalarPlace::Index { base, index, .. } => {
            collect_reference_projections(base, parent, projections);
            projections.push(ReferencePlaceProjection::Index(*index.clone()));
        }
    }
}

fn reference_places_overlap(left: &ReferencePlaceId, right: &ReferencePlaceId) -> bool {
    if left.binding != right.binding {
        return false;
    }
    for (left, right) in left.projections.iter().zip(&right.projections) {
        match (left, right) {
            (
                ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(left)),
                ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(right)),
            ) if left != right => return false,
            (
                ReferencePlaceProjection::Index(ScalarExpression::Integer { value: left, .. }),
                ReferencePlaceProjection::Index(ScalarExpression::Integer { value: right, .. }),
            ) if left != right => return false,
            _ => {}
        }
    }
    true
}

struct ValidatedCallResolver<'a> {
    source: &'a SourceIdentity,
    function_span: ByteSpan,
    module_callables: &'a BTreeMap<(Option<String>, String), (SourceIdentity, ByteSpan)>,
    extern_callables: &'a BTreeMap<(String, String), (SourceIdentity, ByteSpan)>,
    modules: &'a [ScalarModule],
    sites: Vec<ResolvedCallSite>,
}

impl ValidatedCallResolver<'_> {
    fn expression(
        &mut self,
        expression: &ScalarExpression,
        names: &BTreeMap<String, ReferenceBindingId>,
        types: &BTreeMap<String, ScalarType>,
    ) -> Result<(), ReferenceAnalysisError> {
        match expression {
            ScalarExpression::Call {
                receiver,
                receiver_span,
                name,
                arguments,
                overload_selection,
                span,
                ..
            } => {
                let target = if receiver.as_deref() == Some("core") {
                    CoreOperationId::from_name(name).map(ResolvedCallTarget::Core)
                } else if receiver.is_none() {
                    names
                        .get(name)
                        .cloned()
                        .map(ResolvedCallTarget::LocalCallable)
                        .or_else(|| {
                            self.module_callables.get(&(None, name.clone())).map(
                                |(source, declaration_span)| ResolvedCallTarget::ModuleCallable {
                                    source: source.clone(),
                                    declaration_span: *declaration_span,
                                    concrete: match overload_selection {
                                        Some(selection) => {
                                            ConcreteCallSelection::Overload(selection.clone())
                                        }
                                        None => ConcreteCallSelection::Declaration,
                                    },
                                },
                            )
                        })
                } else if let Some(receiver_type) =
                    receiver.as_ref().and_then(|binding| types.get(binding))
                {
                    let ScalarType::Struct(id) = receiver_type else {
                        return Err(ReferenceAnalysisError::MissingContext {
                            source_span: SourceSpan::new(self.source.clone(), *span),
                            reason: ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
                        });
                    };
                    let field = self
                        .modules
                        .iter()
                        .find_map(|module| {
                            module.structs.iter().find(|structure| structure.id == *id)
                        })
                        .and_then(|structure| {
                            resolved_struct_field(
                                receiver_type,
                                name,
                                std::slice::from_ref(structure),
                            )
                        })
                        .expect("validated callable field");
                    let binding_name = receiver.as_ref().expect("typed field receiver");
                    let binding = names.get(binding_name).expect("validated receiver binding");
                    let receiver_span = receiver_span.expect("validated receiver source span");
                    Some(ResolvedCallTarget::CallableField {
                        receiver_place: ReferencePlaceId {
                            source: self.source.clone(),
                            function_span: self.function_span,
                            declaration_span: binding.declaration_span,
                            block_span: binding.block_span,
                            binding: binding.clone(),
                            projections: Vec::new(),
                            place: ScalarPlace::Name {
                                name: binding_name.clone(),
                                span: receiver_span,
                            },
                        },
                        field: field.id.clone(),
                    })
                } else {
                    self.module_callables
                        .get(&(receiver.clone(), name.clone()))
                        .map(
                            |(source, declaration_span)| ResolvedCallTarget::ModuleCallable {
                                source: source.clone(),
                                declaration_span: *declaration_span,
                                concrete: match overload_selection {
                                    Some(selection) => {
                                        ConcreteCallSelection::Overload(selection.clone())
                                    }
                                    None => ConcreteCallSelection::Declaration,
                                },
                            },
                        )
                        .or_else(|| {
                            self.extern_callables
                                .get(&(receiver.clone().expect("qualified call"), name.clone()))
                                .map(|(source, declaration_span)| {
                                    ResolvedCallTarget::ExternCallable {
                                        source: source.clone(),
                                        declaration_span: *declaration_span,
                                    }
                                })
                        })
                };
                let target = target.ok_or_else(|| ReferenceAnalysisError::MissingContext {
                    source_span: SourceSpan::new(self.source.clone(), *span),
                    reason: ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
                })?;
                let site = ResolvedCallSite {
                    source_span: SourceSpan::new(self.source.clone(), *span),
                    target,
                };
                if let Some(existing) = self
                    .sites
                    .iter()
                    .find(|existing| existing.source_span == site.source_span)
                {
                    if existing.target != site.target {
                        return Err(ReferenceAnalysisError::MissingContext {
                            source_span: site.source_span,
                            reason: ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
                        });
                    }
                } else {
                    self.sites.push(site);
                }
                for argument in arguments {
                    self.expression(argument, names, types)?;
                }
            }
            ScalarExpression::Binary { left, right, .. } => {
                self.expression(left, names, types)?;
                self.expression(right, names, types)?;
            }
            ScalarExpression::Unary { operand, .. } => self.expression(operand, names, types)?,
            ScalarExpression::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.expression(condition, names, types)?;
                self.block(then_branch, names, types)?;
                self.block(else_branch, names, types)?;
            }
            ScalarExpression::UnitIf {
                condition,
                then_branch,
                ..
            } => {
                self.expression(condition, names, types)?;
                self.block(then_branch, names, types)?;
            }
            ScalarExpression::Block(block) => self.block(block, names, types)?,
            ScalarExpression::RawAddress { place, .. }
            | ScalarExpression::CheckedAddress { place, .. }
            | ScalarExpression::Dereference { place, .. }
            | ScalarExpression::IndexedRead { place, .. } => self.place(place, names, types)?,
            ScalarExpression::StructLiteral { fields, .. } => {
                for field in fields {
                    self.expression(&field.value, names, types)?;
                }
            }
            ScalarExpression::ArrayLiteral { elements, .. } => {
                for element in elements {
                    self.expression(element, names, types)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn place(
        &mut self,
        place: &ScalarPlace,
        names: &BTreeMap<String, ReferenceBindingId>,
        types: &BTreeMap<String, ScalarType>,
    ) -> Result<(), ReferenceAnalysisError> {
        match place {
            ScalarPlace::Name { .. } => {}
            ScalarPlace::Field { base, .. } => self.place(base, names, types)?,
            ScalarPlace::Index { base, index, .. } => {
                self.place(base, names, types)?;
                self.expression(index, names, types)?;
            }
            ScalarPlace::Dereference { pointer, .. } => self.expression(pointer, names, types)?,
        }
        Ok(())
    }

    fn block(
        &mut self,
        block: &ScalarBlock,
        outer: &BTreeMap<String, ReferenceBindingId>,
        outer_types: &BTreeMap<String, ScalarType>,
    ) -> Result<(), ReferenceAnalysisError> {
        let mut names = outer.clone();
        let mut types = outer_types.clone();
        for item in &block.items {
            match item {
                ScalarBlockItem::LocalBinding(binding) => {
                    self.expression(&binding.value, &names, &types)?;
                    if binding.output_origin == ScalarBindingOutputOrigin::IndependentExpressions {
                        for output in binding.output_values.iter().skip(1) {
                            self.expression(&output.value, &names, &types)?;
                        }
                    }
                    for receiver in &binding.receivers {
                        types.insert(receiver.name.clone(), receiver.ty.clone());
                        names.insert(
                            receiver.name.clone(),
                            ReferenceBindingId {
                                source: self.source.clone(),
                                function_span: self.function_span,
                                declaration_span: receiver.name_span,
                                block_span: block.span,
                                kind: ReferenceBindingKind::Declared,
                            },
                        );
                    }
                }
                ScalarBlockItem::Expression(expression) => {
                    self.expression(expression, &names, &types)?
                }
                ScalarBlockItem::Assignment(assignment) => {
                    for value in &assignment.values {
                        self.expression(value, &names, &types)?;
                    }
                    for target in &assignment.targets {
                        self.place(&target.place, &names, &types)?;
                    }
                    for target in &assignment.targets {
                        if let ScalarPlace::Name { name, .. } = &target.place {
                            if !names.contains_key(name) {
                                if let ScalarExpression::Name { name: source, .. } =
                                    &assignment.value
                                {
                                    if let Some(ty) = types.get(source).cloned() {
                                        types.insert(name.clone(), ty);
                                    }
                                }
                                names.insert(
                                    name.clone(),
                                    ReferenceBindingId {
                                        source: self.source.clone(),
                                        function_span: self.function_span,
                                        declaration_span: target.target_span,
                                        block_span: block.span,
                                        kind: ReferenceBindingKind::Declared,
                                    },
                                );
                            }
                        }
                    }
                }
                ScalarBlockItem::While(while_expression) => {
                    self.expression(&while_expression.condition, &names, &types)?;
                    self.block(&while_expression.body, &names, &types)?;
                }
            }
        }
        Ok(())
    }
}

fn resolve_module_call_sites(
    source: &SourceIdentity,
    items: &[ScalarItem],
    modules: &[ScalarModule],
) -> Result<Vec<ResolvedCallSite>, ReferenceAnalysisError> {
    let current = modules.iter().find(|module| &module.source == source);
    let mut module_callables = BTreeMap::new();
    let mut extern_callables = BTreeMap::new();
    for module in modules {
        let receiver = if &module.source == source {
            None
        } else {
            current.and_then(|current| {
                current
                    .namespace_bindings
                    .iter()
                    .find(|binding| binding.target == module.source)
                    .map(|binding| binding.binding.clone())
            })
        };
        if &module.source != source && receiver.is_none() {
            continue;
        }
        for item in &module.items {
            match item {
                ScalarItem::Function(function) => {
                    module_callables.insert(
                        (receiver.clone(), function.name.clone()),
                        (module.source.clone(), function.name_span),
                    );
                }
                ScalarItem::Binding(binding)
                    if matches!(binding.declared_type, ScalarType::Callable { .. }) =>
                {
                    module_callables.insert(
                        (receiver.clone(), binding.name.clone()),
                        (module.source.clone(), binding.name_span),
                    );
                }
                ScalarItem::Extern(extern_decl) if &module.source == source => {
                    for function in &extern_decl.functions {
                        extern_callables.insert(
                            (extern_decl.binding.clone(), function.name.clone()),
                            (module.source.clone(), function.name_span),
                        );
                    }
                }
                _ => {}
            }
        }
    }
    let mut sites = Vec::new();
    for item in items {
        if let ScalarItem::Function(function) = item {
            resolve_function_call_sites(
                source,
                function,
                &module_callables,
                &extern_callables,
                modules,
                &mut sites,
            )?;
        }
    }
    Ok(sites)
}

fn resolve_function_call_sites(
    source: &SourceIdentity,
    function: &ScalarFunction,
    module_callables: &BTreeMap<(Option<String>, String), (SourceIdentity, ByteSpan)>,
    extern_callables: &BTreeMap<(String, String), (SourceIdentity, ByteSpan)>,
    modules: &[ScalarModule],
    sites: &mut Vec<ResolvedCallSite>,
) -> Result<(), ReferenceAnalysisError> {
    for arm in &function.overload_arms {
        resolve_function_call_sites(
            source,
            arm,
            module_callables,
            extern_callables,
            modules,
            sites,
        )?;
    }
    let mut resolver = ValidatedCallResolver {
        source,
        function_span: function.span,
        module_callables,
        extern_callables,
        modules,
        sites: Vec::new(),
    };
    let mut parameter_names = BTreeMap::new();
    let mut parameter_types = BTreeMap::new();
    let ScalarType::Callable { parameters, .. } = &function.signature else {
        unreachable!("validated function signature")
    };
    for (index, (name, span)) in function
        .parameters
        .iter()
        .zip(&function.parameter_spans)
        .enumerate()
    {
        parameter_types.insert(name.clone(), parameters[index].clone());
        parameter_names.insert(
            name.clone(),
            ReferenceBindingId {
                source: source.clone(),
                function_span: function.span,
                declaration_span: *span,
                block_span: function.body.span,
                kind: ReferenceBindingKind::Declared,
            },
        );
    }
    resolver.block(&function.body, &parameter_names, &parameter_types)?;
    sites.extend(resolver.sites);
    Ok(())
}

fn record_module_reference_origins(
    source: &SourceIdentity,
    items: &mut [ScalarItem],
    call_sites: &[ResolvedCallSite],
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
    assignment_outputs: &HashMap<ResolvedAssignmentOutputId, ScalarType>,
) -> Vec<ReferenceAnalysisError> {
    let module_names = items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Binding(binding) => Some(binding.name.clone()),
            ScalarItem::Function(function) => Some(function.name.clone()),
            ScalarItem::Namespace(namespace) => Some(namespace.binding.clone()),
            ScalarItem::Extern(extern_decl) => Some(extern_decl.binding.clone()),
            ScalarItem::Executable(_) => None,
        })
        .collect::<HashSet<_>>();
    let mut errors = Vec::new();
    for item in items {
        if let ScalarItem::Function(function) = item {
            errors.extend(record_function_reference_origins(
                source,
                function,
                &module_names,
                call_sites,
                structs,
                enums,
                assignment_outputs,
            ));
        }
    }
    errors
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ReferenceCallableBody {
    Declaration,
    Overload(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReferenceOutputContract {
    source: SourceIdentity,
    declaration_span: ByteSpan,
    body: ReferenceCallableBody,
    output: usize,
    origins: Vec<ReferenceOrigin>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ConcreteReferenceOutputContract {
    source: SourceIdentity,
    declaration_span: ByteSpan,
    selection: ConcreteCallSelection,
    type_arguments: Vec<ScalarType>,
    output: usize,
    origins: Vec<ReferenceOrigin>,
}

fn collect_concrete_output_contracts(
    function: &ScalarFunction,
    templates: &[ReferenceOutputContract],
    concrete: &mut Vec<ConcreteReferenceOutputContract>,
) {
    for arm in &function.overload_arms {
        collect_concrete_output_contracts(arm, templates, concrete);
    }
    for point in &function.reference_cfg.initial_points {
        let CfgPointKind::Call {
            expression: ScalarExpression::Call { type_arguments, .. },
            output,
            target:
                ResolvedCallTarget::ModuleCallable {
                    source,
                    declaration_span,
                    concrete: selection,
                },
            ..
        } = &point.kind
        else {
            continue;
        };
        let body = match selection {
            ConcreteCallSelection::Declaration => ReferenceCallableBody::Declaration,
            ConcreteCallSelection::Overload(arm) => ReferenceCallableBody::Overload(arm.arm_index),
        };
        let Some(template) = templates.iter().find(|template| {
            template.source == *source
                && template.declaration_span == *declaration_span
                && template.body == body
                && template.output == *output
        }) else {
            continue;
        };
        let contract = ConcreteReferenceOutputContract {
            source: source.clone(),
            declaration_span: *declaration_span,
            selection: selection.clone(),
            type_arguments: type_arguments
                .iter()
                .map(|argument| argument.ty.clone())
                .collect(),
            output: *output,
            origins: template.origins.clone(),
        };
        if !concrete.contains(&contract) {
            concrete.push(contract);
        }
    }
}

fn collect_function_output_contracts(
    source: &SourceIdentity,
    function: &ScalarFunction,
    declaration_span: ByteSpan,
    body: ReferenceCallableBody,
    contracts: &mut Vec<ReferenceOutputContract>,
) {
    for point in &function.reference_cfg.points {
        let CfgPointKind::ReturnOutput { output, .. } = point.kind else {
            continue;
        };
        let Some(origins) = &point.possible_origins else {
            continue;
        };
        if !matches!(&function.signature, ScalarType::Callable { outputs, .. } if matches!(outputs.outputs[output].ty, ScalarType::CheckedReference { .. }))
        {
            continue;
        }
        contracts.push(ReferenceOutputContract {
            source: source.clone(),
            declaration_span,
            body: body.clone(),
            output,
            origins: origins
                .iter()
                .map(|id| function.reference_cfg.facts[id.0].clone())
                .collect(),
        });
    }
    for (index, arm) in function.overload_arms.iter().enumerate() {
        collect_function_output_contracts(
            source,
            arm,
            declaration_span,
            ReferenceCallableBody::Overload(index),
            contracts,
        );
    }
}

fn transfer_function_output_contracts(
    function: &mut ScalarFunction,
    contracts: &[ConcreteReferenceOutputContract],
) -> Vec<ReferenceAnalysisError> {
    let mut errors = Vec::new();
    for arm in &mut function.overload_arms {
        errors.extend(transfer_function_output_contracts(arm, contracts));
    }
    if !function.reference_cfg.initial_points.is_empty() {
        function.reference_cfg.points = function.reference_cfg.initial_points.clone();
        errors.extend(transfer_reference_facts(
            &mut function.reference_cfg,
            &BTreeMap::new(),
            contracts,
        ));
    }
    errors
}

fn resolve_reference_output_contracts(modules: &mut [ScalarModule]) -> Vec<ReferenceAnalysisError> {
    let mut contracts = Vec::new();
    loop {
        let mut concrete = Vec::new();
        for module in modules.iter() {
            for item in &module.items {
                if let ScalarItem::Function(function) = item {
                    collect_concrete_output_contracts(function, &contracts, &mut concrete);
                }
            }
        }
        let mut errors = Vec::new();
        for module in modules.iter_mut() {
            for item in &mut module.items {
                if let ScalarItem::Function(function) = item {
                    errors.extend(transfer_function_output_contracts(function, &concrete));
                }
            }
        }
        let mut next = Vec::new();
        for module in modules.iter() {
            for item in &module.items {
                if let ScalarItem::Function(function) = item {
                    collect_function_output_contracts(
                        &module.source,
                        function,
                        function.name_span,
                        ReferenceCallableBody::Declaration,
                        &mut next,
                    );
                }
            }
        }
        if next == contracts {
            return errors;
        }
        contracts = next;
    }
}

fn record_specialized_copy_policies(
    items: &mut [ScalarItem],
    modules: &[ScalarModule],
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
) -> Vec<ReferenceAnalysisError> {
    let mut errors = Vec::new();
    for item in items {
        if let ScalarItem::Function(function) = item {
            errors.extend(record_function_specialized_copy_policies(
                function, modules, structs, enums,
            ));
        }
    }
    errors
}

fn record_function_specialized_copy_policies(
    function: &mut ScalarFunction,
    modules: &[ScalarModule],
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
) -> Vec<ReferenceAnalysisError> {
    let mut errors = Vec::new();
    for arm in &mut function.overload_arms {
        errors.extend(record_function_specialized_copy_policies(
            arm, modules, structs, enums,
        ));
    }
    let generic_parameters = function
        .generic_parameters
        .iter()
        .map(|parameter| parameter.name.clone())
        .collect::<BTreeSet<_>>();
    for point in &function.reference_cfg.points {
        let CfgPointKind::Call {
            expression: ScalarExpression::Call { type_arguments, .. },
            output: 0,
            target:
                ResolvedCallTarget::ModuleCallable {
                    source,
                    declaration_span,
                    concrete,
                },
            ..
        } = &point.kind
        else {
            continue;
        };
        let error = || ReferenceAnalysisError::MissingContext {
            source_span: point.source_span.clone(),
            reason: ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
        };
        let Some(declaration) = modules
            .iter()
            .find(|module| module.source == *source)
            .and_then(|module| {
                module.items.iter().find_map(|item| match item {
                    ScalarItem::Function(candidate) if candidate.name_span == *declaration_span => {
                        Some(candidate)
                    }
                    _ => None,
                })
            })
        else {
            errors.push(error());
            continue;
        };
        let template = match concrete {
            ConcreteCallSelection::Overload(selection) => {
                declaration.overload_arms.get(selection.arm_index)
            }
            ConcreteCallSelection::Declaration => Some(declaration),
        };
        let Some(template) = template else {
            errors.push(error());
            continue;
        };
        if template.generic_parameters.is_empty() {
            continue;
        }
        let substitutions = match concrete {
            ConcreteCallSelection::Overload(selection) => selection.substitutions.clone(),
            ConcreteCallSelection::Declaration => {
                if type_arguments.len() != template.generic_parameters.len() {
                    errors.push(error());
                    continue;
                }
                template
                    .generic_parameters
                    .iter()
                    .zip(type_arguments)
                    .map(|(parameter, argument)| (parameter.name.clone(), argument.ty.clone()))
                    .collect()
            }
        };
        if template
            .generic_parameters
            .iter()
            .any(|parameter| !substitutions.contains_key(&parameter.name))
        {
            errors.push(error());
            continue;
        }
        let mut deferred = BTreeMap::new();
        for template_point in &template.reference_cfg.points {
            let ty = match &template_point.kind {
                CfgPointKind::DeferredMove { ty, .. }
                | CfgPointKind::DeferredMoveThrough { ty, .. }
                | CfgPointKind::DeferredArrayElement { ty, .. } => ty,
                _ => continue,
            };
            let concrete_ty = substitute_type(ty, &substitutions);
            match copy_policy(&concrete_ty, structs, enums, &generic_parameters) {
                Ok(CopyPolicy::Move)
                    if matches!(
                        template_point.kind,
                        CfgPointKind::DeferredArrayElement { .. }
                    ) =>
                {
                    errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: point.source_span.clone(),
                        reason: ReferenceAnalysisErrorReason::MoveFromArrayElement,
                    })
                }
                Ok(policy) => {
                    deferred.insert(template_point.id.0, policy);
                    function
                        .reference_cfg
                        .specialized_policies
                        .push(SpecializedCopyPolicy {
                            call_point: point.id,
                            template_source: source.clone(),
                            template_function_span: template.span,
                            template_point: template_point.id,
                            ty: concrete_ty,
                            policy,
                        })
                }
                Err(_) => errors.push(error()),
            }
        }
        if !deferred.is_empty() {
            let mut concrete_cfg = template.reference_cfg.clone();
            concrete_cfg.points = concrete_cfg.initial_points.clone();
            let transferred = transfer_reference_facts(&mut concrete_cfg, &deferred, &[]);
            errors.extend(transferred.into_iter().map(|error| {
                ReferenceAnalysisError::MissingContext {
                    source_span: point.source_span.clone(),
                    reason: error.reason(),
                }
            }));
        }
    }
    errors
}

fn record_function_reference_origins(
    source: &SourceIdentity,
    function: &mut ScalarFunction,
    module_names: &HashSet<String>,
    call_sites: &[ResolvedCallSite],
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
    assignment_outputs: &HashMap<ResolvedAssignmentOutputId, ScalarType>,
) -> Vec<ReferenceAnalysisError> {
    let mut errors = Vec::new();
    for arm in &mut function.overload_arms {
        errors.extend(record_function_reference_origins(
            source,
            arm,
            module_names,
            call_sites,
            structs,
            enums,
            assignment_outputs,
        ));
    }
    let mut builder = ReferenceFlowBuilder {
        source: source.clone(),
        function_span: function.span,
        names: BTreeMap::new(),
        module_names: module_names.clone(),
        call_sites: call_sites.to_vec(),
        origins: HashMap::new(),
        checked: HashSet::new(),
        binding_types: HashMap::new(),
        structs: structs.to_vec(),
        enums: enums.to_vec(),
        generic_parameters: function
            .generic_parameters
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect(),
        assignment_outputs: assignment_outputs.clone(),
        dereference_points: HashMap::new(),
        cfg: ScalarReferenceCfg {
            facts: Vec::new(),
            points: Vec::new(),
            initial_points: Vec::new(),
            allocations: BTreeSet::new(),
            specialized_policies: Vec::new(),
        },
        next_scope: 0,
        cursor: None,
    };
    builder.point(
        CfgPointKind::Entry,
        function.span,
        ReferenceScopeId(0),
        BTreeSet::new(),
    );
    for (name, span) in function.parameters.iter().zip(&function.parameter_spans) {
        builder.names.insert(
            name.clone(),
            ReferenceBindingId {
                source: source.clone(),
                function_span: function.span,
                declaration_span: *span,
                block_span: function.body.span,
                kind: ReferenceBindingKind::Declared,
            },
        );
    }
    if let ScalarType::Callable { parameters, .. } = &function.signature {
        for (index, ((name, span), ty)) in function
            .parameters
            .iter()
            .zip(&function.parameter_spans)
            .zip(parameters)
            .enumerate()
        {
            builder
                .binding_types
                .insert(builder.names[name].clone(), ty.clone());
            if let ScalarType::CheckedReference { mutability, .. } = ty {
                let binding = builder.names[name].clone();
                let loan = ReferenceLoanId {
                    mode: *mutability,
                    origin_place: ReferencePlaceId {
                        source: source.clone(),
                        function_span: function.span,
                        declaration_span: *span,
                        block_span: function.body.span,
                        binding: binding.clone(),
                        projections: Vec::new(),
                        place: ScalarPlace::Name {
                            name: name.clone(),
                            span: *span,
                        },
                    },
                    creation_span: *span,
                    parent: None,
                };
                let id = builder.intern(ReferenceOrigin::BorrowedFrom {
                    parameter: index,
                    loan,
                });
                builder.checked.insert(binding.clone());
                builder
                    .origins
                    .insert(binding.clone(), BTreeSet::from([id]));
                builder.point(
                    CfgPointKind::Bind { binding },
                    *span,
                    ReferenceScopeId(0),
                    BTreeSet::from([id]),
                );
            }
        }
    }
    match builder.block(&function.body) {
        Err(error) => errors.push(error),
        Ok(result) => {
            if let ScalarType::Callable { outputs, .. } = &function.signature {
                if matches!(
                    outputs.outputs.first(),
                    Some(ScalarOutput {
                        ty: ScalarType::CheckedReference { .. },
                        ..
                    })
                ) && result.is_none()
                    && !function.body.final_output_values.is_empty()
                {
                    let span = function.body.final_output_values[0].span;
                    errors.push(builder.missing_context(
                        span,
                        ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                    ));
                }
            }
        }
    }
    if errors.is_empty() && !builder.cfg.points.is_empty() {
        builder.cfg.initial_points = builder.cfg.points.clone();
        let _ = transfer_reference_facts(&mut builder.cfg, &BTreeMap::new(), &[]);
    }
    function.reference_cfg = builder.cfg;
    errors
}

fn expand_conditional_final_outputs(block: &mut ScalarBlock, outputs: &[ScalarOutput]) {
    let output_count = outputs.len();
    if block.final_output_values.len() != 1 || output_count < 2 {
        return;
    }
    let ScalarExpression::If {
        condition,
        then_branch,
        else_branch,
        ..
    } = &block.final_output_values[0].value
    else {
        return;
    };
    let mut then_branch = then_branch.clone();
    let mut else_branch = else_branch.clone();
    expand_conditional_final_outputs(&mut then_branch, outputs);
    expand_conditional_final_outputs(&mut else_branch, outputs);
    if then_branch.final_output_values.len() != output_count
        || else_branch.final_output_values.len() != output_count
    {
        return;
    }
    let condition = condition.clone();
    let conditional_span = span_of(&block.final_output_values[0].value);
    block.final_output_values = (0..output_count)
        .map(|position| {
            let value = ScalarExpression::If {
                condition: condition.clone(),
                then_branch: select_final_output(&then_branch, position, &outputs[position].ty),
                else_branch: select_final_output(&else_branch, position, &outputs[position].ty),
                span: conditional_span,
            };
            ScalarOutputValue {
                position,
                ty: ScalarType::Error,
                span: span_of(&value),
                value,
            }
        })
        .collect();
    let final_items = block
        .final_output_values
        .iter()
        .map(|output| ScalarBlockItem::Expression(output.value.clone()));
    let final_start = block.items.len() - 1;
    block.items.truncate(final_start);
    block.terminated_items.truncate(final_start);
    block.items.extend(final_items);
    block
        .terminated_items
        .extend((0..output_count).map(|_| false));
    block.expressions = block
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarBlockItem::Expression(expression) => Some(expression.clone()),
            _ => None,
        })
        .collect();
}

fn select_final_output(block: &ScalarBlock, position: usize, ty: &ScalarType) -> ScalarBlock {
    let mut selected = block.clone();
    let final_start = selected.items.len() - selected.final_output_values.len();
    selected.items.truncate(final_start);
    selected.terminated_items.truncate(final_start);
    selected.final_output_values = vec![block.final_output_values[position].clone()];
    selected.final_output_values[0].ty = ty.clone();
    selected.items.push(ScalarBlockItem::Expression(
        selected.final_output_values[0].value.clone(),
    ));
    selected.terminated_items.push(false);
    selected.expressions = selected
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarBlockItem::Expression(expression) => Some(expression.clone()),
            _ => None,
        })
        .collect();
    selected
}

fn derive_type(node: &CstNode) -> ScalarType {
    let actual = if matches!(node.kind(), SyntaxKind::TypeSpec | SyntaxKind::Punctuation) {
        node.descendants()
            .find(|child| {
                matches!(
                    child.kind(),
                    SyntaxKind::CallableType
                        | SyntaxKind::RawPointerType
                        | SyntaxKind::CheckedPointerType
                        | SyntaxKind::ParenthesizedType
                        | SyntaxKind::QualifiedType
                )
            })
            .unwrap_or_else(|| node.clone())
    } else {
        node.clone()
    };
    let mut ty = match actual.kind() {
        SyntaxKind::CheckedPointerType => {
            let inner = actual
                .children()
                .find(|child| {
                    matches!(
                        child.kind(),
                        SyntaxKind::ParenthesizedType
                            | SyntaxKind::QualifiedType
                            | SyntaxKind::Punctuation
                    )
                })
                .map(|child| derive_type(&child))
                .unwrap_or_else(|| {
                    let token = direct_token(&actual, SyntaxKind::TypeName)
                        .expect("checked reference type");
                    type_from_name(token.text(), token_span(&token))
                });
            ScalarType::CheckedReference {
                mutability: if actual.children_with_tokens().any(
                    |element| matches!(element, NodeOrToken::Token(token) if token.text() == "!"),
                ) {
                    ScalarReferenceMutability::Mutable
                } else {
                    ScalarReferenceMutability::Shared
                },
                inner: Box::new(inner),
            }
        }
        SyntaxKind::ParenthesizedType => {
            derive_type(&actual.children().next().expect("parenthesized type inner"))
        }
        SyntaxKind::RawPointerType => {
            let inner = actual
                .children()
                .find(|child| {
                    matches!(
                        child.kind(),
                        SyntaxKind::Punctuation
                            | SyntaxKind::QualifiedType
                            | SyntaxKind::RawPointerType
                    )
                })
                .map(|child| derive_type(&child))
                .unwrap_or_else(|| {
                    let token =
                        direct_token(&actual, SyntaxKind::TypeName).expect("raw pointer type");
                    type_from_name(token.text(), token_span(&token))
                });
            ScalarType::RawPointer(Box::new(inner))
        }
        SyntaxKind::QualifiedType => qualified_type(&actual),
        SyntaxKind::CallableType => {
            let output = actual
                .descendants()
                .find(|child| child.kind() == SyntaxKind::CallableOutput)
                .expect("callable output");
            let outputs = ScalarOutputSequence {
                outputs: output
                    .children()
                    .map(|child| ScalarOutput {
                        ty: derive_callable_array_type(&child),
                        span: wosy_syntax::byte_span(&child),
                    })
                    .collect(),
                span: wosy_syntax::byte_span(&actual),
            };
            let parameters = actual
                .children()
                .filter(|child| child.kind() != SyntaxKind::CallableOutput)
                .filter(|child| child.kind() == SyntaxKind::Punctuation)
                .filter(|child| child.text() != "(")
                .filter(|child| child.text() != ")")
                .map(|child| derive_callable_array_type(&child))
                .collect();
            ScalarType::Callable {
                outputs,
                parameters,
            }
        }
        _ => {
            let token = actual
                .descendants_with_tokens()
                .filter_map(|element| match element {
                    NodeOrToken::Token(token) if token.kind() == SyntaxKind::TypeName => {
                        Some(token)
                    }
                    _ => None,
                })
                .next()
                .expect("type token");
            type_from_name(token.text(), token_span(&token))
        }
    };
    for suffix in node
        .children()
        .filter(|child| child.kind() == SyntaxKind::FixedArraySuffix)
    {
        let length = suffix
            .children()
            .find(|child| child.kind() == SyntaxKind::FixedArrayLength)
            .expect("fixed array length");
        let token = direct_token(&length, SyntaxKind::Integer).expect("fixed array length token");
        let length_span = wosy_syntax::byte_span(&length);
        let negative = length
            .children_with_tokens()
            .any(|element| matches!(element, NodeOrToken::Token(token) if token.kind() == SyntaxKind::Punctuation && token.text() == "-"));
        if negative {
            return ScalarType::Error;
        }
        let Some(length) = parse_integer(token.text()).and_then(|value| u64::try_from(value).ok())
        else {
            return ScalarType::Error;
        };
        ty = ScalarType::Array {
            element: Box::new(ty),
            length,
            length_span,
            span: wosy_syntax::byte_span(node),
        };
    }
    for _ in node
        .children()
        .filter(|child| child.kind() == SyntaxKind::ErasedArraySuffix)
    {
        ty = ScalarType::RuntimeArray {
            element: Box::new(ty),
            span: wosy_syntax::byte_span(node),
        };
    }
    if !matches!(ty, ScalarType::Callable { .. }) {
        let erased_suffixes = node.text().to_string().match_indices("[]").count();
        for _ in runtime_array_depth(&ty)..erased_suffixes {
            ty = ScalarType::RuntimeArray {
                element: Box::new(ty),
                span: wosy_syntax::byte_span(node),
            };
        }
    }
    ty
}

fn runtime_array_depth(ty: &ScalarType) -> usize {
    match ty {
        ScalarType::RuntimeArray { element, .. } => 1 + runtime_array_depth(element),
        _ => 0,
    }
}

fn derive_callable_array_type(node: &CstNode) -> ScalarType {
    derive_type(node)
}

fn fixed_array_length_diagnostics(canonical: &CanonicalCstRoot) -> Vec<super::Diagnostic> {
    canonical
        .root
        .descendants()
        .filter(|node| node.kind() == SyntaxKind::FixedArrayLength)
        .filter_map(|length| {
            let token = direct_token(&length, SyntaxKind::Integer)?;
            let negative = length
                .children_with_tokens()
                .any(|element| matches!(element, NodeOrToken::Token(token) if token.kind() == SyntaxKind::Punctuation && token.text() == "-"));
            (!negative && parse_integer(token.text())
                .and_then(|value| u64::try_from(value).ok())
                .is_none()
                || negative)
                .then(|| super::Diagnostic {
                    code: "B0003".to_owned(),
                    severity: super::DiagnosticSeverity::Error,
                    message: "fixed array length must be an unsigned u64".to_owned(),
                    labels: vec![super::DiagnosticLabel {
                        kind: super::DiagnosticLabelKind::Primary,
                        span: SourceSpan::new(canonical.source.clone(), wosy_syntax::byte_span(&length)),
                        message: "invalid fixed array length".to_owned(),
                    }],
                    notes: Vec::new(),
                })
        })
        .collect()
}

fn qualified_type(node: &CstNode) -> ScalarType {
    let identifiers = node
        .children_with_tokens()
        .filter_map(|element| match element {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => Some(token),
            _ => None,
        })
        .collect::<Vec<_>>();
    ScalarType::Qualified {
        receiver: identifiers[0].text().to_owned(),
        receiver_span: token_span(&identifiers[0]),
        member: identifiers[1].text().to_owned(),
        member_span: token_span(&identifiers[1]),
        span: wosy_syntax::byte_span(node),
    }
}

fn type_from_name(value: &str, span: ByteSpan) -> ScalarType {
    match value {
        "unit" => ScalarType::Unit,
        "bool" => ScalarType::Bool,
        "i8" => ScalarType::I8,
        "i16" => ScalarType::I16,
        "i32" => ScalarType::I32,
        "i64" => ScalarType::I64,
        "i128" => ScalarType::I128,
        "u8" => ScalarType::U8,
        "u16" => ScalarType::U16,
        "u32" => ScalarType::U32,
        "u64" => ScalarType::U64,
        "u128" => ScalarType::U128,
        "f32" => ScalarType::F32,
        "f64" => ScalarType::F64,
        "char" => ScalarType::Char,
        "artifact_id" => ScalarType::ArtifactId,
        value => ScalarType::Named {
            name: value.to_owned(),
            span,
        },
    }
}

fn decode_string(value: &str) -> Result<String, (usize, usize)> {
    let Some(quoted) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    else {
        return Err((0, value.len()));
    };
    let mut decoded = String::new();
    let mut chars = quoted.char_indices();
    while let Some((index, character)) = chars.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        let Some((escape_index, escape)) = chars.next() else {
            return Err((index, 1));
        };
        match escape {
            'n' => decoded.push('\n'),
            'r' => decoded.push('\r'),
            't' => decoded.push('\t'),
            '0' => decoded.push('\0'),
            '"' => decoded.push('"'),
            '\'' => decoded.push('\''),
            '\\' => decoded.push('\\'),
            'u' => {
                let Some((brace_index, brace)) = chars.next() else {
                    return Err((index, escape_index + escape.len_utf8() - index));
                };
                if brace != '{' {
                    return Err((index, brace_index + brace.len_utf8() - index));
                }
                let mut code_point = 0u32;
                let mut digit_count = 0;
                let mut end = brace_index + brace.len_utf8();
                let mut closed = false;
                while let Some((digit_index, digit)) = chars.next() {
                    end = digit_index + digit.len_utf8();
                    if digit == '}' {
                        closed = true;
                        break;
                    }
                    let Some(digit_value) = digit.to_digit(16) else {
                        return Err((index, end - index));
                    };
                    code_point = match code_point
                        .checked_mul(16)
                        .and_then(|value| value.checked_add(digit_value))
                    {
                        Some(value) => value,
                        None => return Err((index, end - index)),
                    };
                    digit_count += 1;
                }
                if !closed || digit_count == 0 {
                    return Err((index, end - index));
                }
                let Some(character) = char::from_u32(code_point) else {
                    return Err((index, end - index));
                };
                decoded.push(character);
            }
            _ => return Err((index, escape_index + escape.len_utf8() - index)),
        }
    }
    Ok(decoded)
}

fn decode_char(value: &str) -> Result<char, (usize, usize)> {
    let decoded = value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
        .ok_or((0, value.len()))?;
    let decoded = decode_string(&format!("\"{}\"", decoded))
        .map_err(|(start, length)| (start.saturating_sub(1), length))?;
    let mut characters = decoded.chars();
    let character = characters
        .next()
        .ok_or((1, value.len().saturating_sub(2)))?;
    if characters.next().is_some() {
        return Err((1, value.len().saturating_sub(2)));
    }
    Ok(character)
}

fn string_diagnostics(canonical: &CanonicalCstRoot) -> Vec<super::Diagnostic> {
    canonical
        .root
        .descendants_with_tokens()
        .filter_map(|element| match element {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::String => {
                decode_string(token.text())
                    .err()
                    .map(|(start, length)| super::Diagnostic {
                        code: "B0003".to_owned(),
                        severity: super::DiagnosticSeverity::Error,
                        message: "invalid string literal".to_owned(),
                        labels: vec![super::DiagnosticLabel {
                            kind: super::DiagnosticLabelKind::Primary,
                            span: SourceSpan::new(
                                canonical.source.clone(),
                                string_error_span(&token, start, length),
                            ),
                            message: "invalid string literal".to_owned(),
                        }],
                        notes: Vec::new(),
                    })
            }
            _ => None,
        })
        .collect()
}

fn char_diagnostics(canonical: &CanonicalCstRoot) -> Vec<super::Diagnostic> {
    canonical
        .root
        .descendants_with_tokens()
        .filter_map(|element| match element {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::Char => {
                decode_char(token.text())
                    .err()
                    .map(|(start, length)| super::Diagnostic {
                        code: "B0003".to_owned(),
                        severity: super::DiagnosticSeverity::Error,
                        message: "invalid character literal".to_owned(),
                        labels: vec![super::DiagnosticLabel {
                            kind: super::DiagnosticLabelKind::Primary,
                            span: SourceSpan::new(
                                canonical.source.clone(),
                                string_error_span(&token, start, length),
                            ),
                            message: "invalid character literal".to_owned(),
                        }],
                        notes: Vec::new(),
                    })
            }
            _ => None,
        })
        .collect()
}

fn integer_diagnostics(canonical: &CanonicalCstRoot) -> Vec<super::Diagnostic> {
    canonical
        .root
        .descendants_with_tokens()
        .filter_map(|element| match element {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::Integer => {
                if parse_integer(token.text()).is_none() {
                    Some(super::Diagnostic {
                        code: "B0010".to_owned(),
                        severity: super::DiagnosticSeverity::Error,
                        message: "invalid integer literal".to_owned(),
                        labels: vec![super::DiagnosticLabel {
                            kind: super::DiagnosticLabelKind::Primary,
                            span: SourceSpan::new(canonical.source.clone(), token_span(&token)),
                            message: "invalid integer literal".to_owned(),
                        }],
                        notes: Vec::new(),
                    })
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect()
}

fn parse_float(text: &str) -> Option<f64> {
    if text.starts_with('_') || text.ends_with('_') || text.contains("__") {
        return None;
    }
    let mut previous = None;
    for character in text.chars() {
        if character == '_'
            && matches!(
                previous,
                None | Some('.') | Some('e') | Some('E') | Some('+') | Some('-')
            )
        {
            return None;
        }
        previous = Some(character);
    }
    let value = text.replace('_', "").parse::<f64>().ok()?;
    value.is_finite().then_some(value)
}

fn float_diagnostics(canonical: &CanonicalCstRoot) -> Vec<super::Diagnostic> {
    canonical
        .root
        .descendants_with_tokens()
        .filter_map(|element| match element {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::Float => {
                parse_float(token.text())
                    .is_none()
                    .then(|| super::Diagnostic {
                        code: "B0010".to_owned(),
                        severity: super::DiagnosticSeverity::Error,
                        message: "invalid finite floating-point literal".to_owned(),
                        labels: vec![super::DiagnosticLabel {
                            kind: super::DiagnosticLabelKind::Primary,
                            span: SourceSpan::new(canonical.source.clone(), token_span(&token)),
                            message: "invalid finite floating-point literal".to_owned(),
                        }],
                        notes: Vec::new(),
                    })
            }
            _ => None,
        })
        .collect()
}

fn string_error_span(token: &CstToken, start: usize, length: usize) -> ByteSpan {
    let span = token_span(token);
    ByteSpan::new(
        span.start + 1 + start as u32,
        span.start + 1 + (start + length) as u32,
    )
}

fn derive_block(node: &CstNode) -> ScalarBlock {
    derive_block_with_context(node, false)
}

fn derive_block_with_context(node: &CstNode, unsafe_context: bool) -> ScalarBlock {
    let block_items = node
        .children()
        .filter(|child| child.kind() == SyntaxKind::BlockItem)
        .collect::<Vec<_>>();
    let mut items = block_items
        .iter()
        .map(derive_block_item)
        .collect::<Vec<_>>();
    let mut final_output_values = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::FinalOutputList)
        .map(|final_list| {
            let output_list = direct_nodes(&final_list)
                .into_iter()
                .find(|child| child.kind() == SyntaxKind::OutputList)
                .expect("final output list");
            output_list
                .children()
                .filter(|child| child.kind() == SyntaxKind::Expression)
                .enumerate()
                .map(|(position, child)| {
                    let value = derive_expression(&child);
                    ScalarOutputValue {
                        position,
                        ty: ScalarType::Error,
                        span: span_of(&value),
                        value,
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut terminated_items = block_items
        .iter()
        .map(|item| {
            item.children_with_tokens().any(|element| {
                matches!(
                    element,
                    NodeOrToken::Token(token)
                        if token.kind() == SyntaxKind::Punctuation && token.text() == ";"
                )
            })
        })
        .collect::<Vec<_>>();
    if final_output_values.is_empty() {
        if let Some((item, ScalarBlockItem::Expression(value), false)) = block_items
            .last()
            .zip(items.last())
            .zip(terminated_items.last())
            .map(|((item, value), terminated)| (item, value, *terminated))
        {
            if let Some(expression) = direct_nodes(item).into_iter().find(|child| {
                child.kind() == SyntaxKind::Expression
                    && direct_nodes(child)
                        .into_iter()
                        .any(|actual| actual.kind() == SyntaxKind::Unsafe)
            }) {
                final_output_values.push(ScalarOutputValue {
                    position: 0,
                    ty: ScalarType::Error,
                    span: wosy_syntax::byte_span(&expression),
                    value: value.clone(),
                });
                items.pop();
                terminated_items.pop();
            }
        }
    }
    items.extend(
        final_output_values
            .iter()
            .map(|output| ScalarBlockItem::Expression(output.value.clone())),
    );
    terminated_items.extend(final_output_values.iter().map(|_| false));
    let expressions = items
        .iter()
        .filter_map(|item| match item {
            ScalarBlockItem::Expression(expression) => Some(expression.clone()),
            _ => None,
        })
        .collect();
    ScalarBlock {
        items,
        terminated_items,
        expressions,
        final_output_values,
        span: wosy_syntax::byte_span(node),
        unsafe_context,
    }
}

fn derive_block_item(node: &CstNode) -> ScalarBlockItem {
    let child = direct_nodes(node).into_iter().next().expect("block item");
    match child.kind() {
        SyntaxKind::LocalBinding => ScalarBlockItem::LocalBinding(derive_binding(&child)),
        SyntaxKind::While => ScalarBlockItem::While(derive_while(&child)),
        SyntaxKind::Expression => {
            let actual = direct_nodes(&child).into_iter().next().expect("expression");
            match actual.kind() {
                SyntaxKind::Assignment => ScalarBlockItem::Assignment(derive_assignment(&actual)),
                SyntaxKind::While => ScalarBlockItem::While(derive_while(&actual)),
                _ => ScalarBlockItem::Expression(derive_expression(&child)),
            }
        }
        _ => panic!("block item grammar"),
    }
}

fn derive_executable_item(node: &CstNode) -> ScalarBlockItem {
    let child = direct_nodes(node)
        .into_iter()
        .next()
        .expect("executable item");
    match child.kind() {
        SyntaxKind::Assignment => ScalarBlockItem::Assignment(derive_assignment(&child)),
        SyntaxKind::Expression => {
            let actual = direct_nodes(&child).into_iter().next().expect("expression");
            match actual.kind() {
                SyntaxKind::Assignment => ScalarBlockItem::Assignment(derive_assignment(&actual)),
                _ => ScalarBlockItem::Expression(derive_expression(&child)),
            }
        }
        _ => panic!("top-level item grammar"),
    }
}

fn derive_assignment(node: &CstNode) -> ScalarAssignment {
    let targets_node = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::AssignmentTargets)
        .expect("assignment targets node");
    let targets = targets_node
        .children()
        .filter(|child| child.kind() == SyntaxKind::AssignmentTarget)
        .map(|target_node| {
            let identifiers: Vec<CstToken> = target_node
                .descendants_with_tokens()
                .filter_map(|element| match element {
                    NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => {
                        Some(token)
                    }
                    _ => None,
                })
                .collect();
            let target = identifiers.last().expect("assignment target name");
            let place = derive_place(&target_node);
            ScalarAssignmentTarget {
                receiver: (identifiers.len() == 2).then(|| identifiers[0].text().to_owned()),
                target: target.text().to_owned(),
                receiver_span: (identifiers.len() == 2).then(|| token_span(&identifiers[0])),
                target_span: token_span(target),
                span: ByteSpan::new(
                    wosy_syntax::byte_span(&target_node).start,
                    scalar_place_target_span(&place).end,
                ),
                place,
            }
        })
        .collect::<Vec<_>>();
    let target_node = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::AssignmentTarget)
        .or_else(|| {
            node.descendants()
                .find(|child| child.kind() == SyntaxKind::AssignmentTarget)
        })
        .expect("assignment target node");
    let identifiers: Vec<CstToken> = target_node
        .descendants_with_tokens()
        .filter_map(|element| match element {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => Some(token),
            _ => None,
        })
        .collect();
    let target = identifiers.last().expect("assignment target name");
    let output_list = direct_nodes(node)
        .into_iter()
        .find(|child| child.kind() == SyntaxKind::OutputList)
        .expect("assignment output list");
    let values = output_list
        .children()
        .filter(|child| child.kind() == SyntaxKind::Expression)
        .map(|child| derive_expression(&child))
        .collect::<Vec<_>>();
    let value = values[0].clone();
    ScalarAssignment {
        receiver: (identifiers.len() == 2).then(|| identifiers[0].text().to_owned()),
        target: target.text().to_owned(),
        receiver_span: (identifiers.len() == 2).then(|| token_span(&identifiers[0])),
        target_span: token_span(&target),
        targets,
        value,
        values,
        span: wosy_syntax::byte_span(node),
    }
}

fn derive_while(node: &CstNode) -> ScalarWhile {
    let children = direct_nodes(node);
    ScalarWhile {
        condition: derive_expression(&children[0]),
        body: derive_block(&children[1]),
        span: wosy_syntax::byte_span(node),
    }
}

fn derive_expression(node: &CstNode) -> ScalarExpression {
    let actual = if node.kind() == SyntaxKind::Expression {
        direct_nodes(node)[0].clone()
    } else {
        node.clone()
    };
    match actual.kind() {
        SyntaxKind::IfExpr => {
            let children = direct_nodes(&actual);
            ScalarExpression::If {
                condition: Box::new(derive_expression(&children[0])),
                then_branch: derive_block(&children[1]),
                else_branch: derive_block(&children[2]),
                span: wosy_syntax::byte_span(&actual),
            }
        }
        SyntaxKind::UnitIfExpr => {
            let children = direct_nodes(&actual);
            ScalarExpression::UnitIf {
                condition: Box::new(derive_expression(&children[0])),
                then_branch: derive_block(&children[1]),
                span: wosy_syntax::byte_span(&actual),
            }
        }
        SyntaxKind::Unsafe => ScalarExpression::Block(derive_block_with_context(
            &direct_nodes(&actual)
                .into_iter()
                .find(|child| child.kind() == SyntaxKind::Block)
                .expect("unsafe block"),
            true,
        )),
        SyntaxKind::ArrayLiteral => ScalarExpression::ArrayLiteral {
            elements: direct_nodes(&actual)
                .into_iter()
                .filter(|child| child.kind() == SyntaxKind::Expression)
                .map(|child| derive_expression(&child))
                .collect(),
            span: wosy_syntax::byte_span(&actual),
        },
        SyntaxKind::Primary
            if actual.children_with_tokens().any(|element| {
                matches!(element, NodeOrToken::Token(token) if token.kind() == SyntaxKind::ArrayLiteral)
            }) => ScalarExpression::ArrayLiteral {
            elements: Vec::new(),
            span: wosy_syntax::byte_span(&actual),
        },
        SyntaxKind::Binary => {
            let children = semantic_children(&actual);
            let mut value = derive_element(&children[0]);
            let mut index = 1;
            while index < children.len() {
                let NodeOrToken::Token(operator_token) = &children[index] else {
                    panic!("binary operator token")
                };
                let operator = operator(operator_token.text());
                let right = derive_element(&children[index + 1]);
                let span = ByteSpan::new(span_of(&value).start, span_of(&right).end);
                value = ScalarExpression::Binary {
                    operator,
                    left: Box::new(value),
                    right: Box::new(right),
                    span,
                };
                index += 2;
            }
            value
        }
        SyntaxKind::Unary => {
            let children = semantic_children(&actual);
            let Some(first) = children.first() else {
                panic!("unary expression children")
            };
            let NodeOrToken::Token(operator_token) = first else {
                return derive_element(first);
            };
            let operator = match operator_token.text() {
                "!" => UnaryOperator::LogicalNot,
                "~" => UnaryOperator::BitwiseNot,
                "-" => UnaryOperator::Negate,
                _ => panic!("unary operator grammar"),
            };
            let operand = derive_element(children.get(1).expect("unary operand"));
            ScalarExpression::Unary {
                operator,
                span: ByteSpan::new(
                    operator_token.text_range().start().into(),
                    span_of(&operand).end,
                ),
                operand: Box::new(operand),
            }
        }
        SyntaxKind::Call => {
            let qualified = direct_nodes(&actual)
                .into_iter()
                .find(|child| child.kind() == SyntaxKind::QualifiedCall)
                .expect("qualified call");
            let identifiers: Vec<CstToken> = qualified
                .children_with_tokens()
                .filter_map(|element| match element {
                    NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => {
                        Some(token)
                    }
                    _ => None,
                })
                .collect();
            let receiver = identifiers
                .first()
                .filter(|_| identifiers.len() == 2)
                .map(|token| token.text().to_owned());
            let name_token = identifiers.last().expect("call member");
            let type_arguments = direct_nodes(&actual)
                .into_iter()
                .find(|child| child.kind() == SyntaxKind::GenericTypeArguments)
                .map(|generic| {
                    generic
                        .children()
                        .filter(|child| child.kind() == SyntaxKind::GenericTypeArgument)
                        .map(|argument| ScalarTypeArgument {
                            ty: derive_type(
                                &direct_nodes(&argument)
                                    .into_iter()
                                    .next()
                                    .expect("generic type argument type"),
                            ),
                            span: wosy_syntax::byte_span(&argument),
                        })
                        .collect()
                })
                .map_or_else(Vec::new, |arguments| arguments);
            let arguments = direct_nodes(&actual)
                .iter()
                .filter(|child| child.kind() == SyntaxKind::Expression)
                .map(derive_expression)
                .collect();
            ScalarExpression::Call {
                receiver,
                name: name_token.text().to_owned(),
                receiver_span: identifiers
                    .first()
                    .filter(|_| identifiers.len() == 2)
                    .map(token_span),
                name_span: token_span(name_token),
                type_arguments,
                arguments,
                overload_selection: None,
                span: wosy_syntax::byte_span(&actual),
            }
        }
        SyntaxKind::QualifiedMember => {
            let identifiers: Vec<CstToken> = actual
                .children_with_tokens()
                .filter_map(|element| match element {
                    NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => {
                        Some(token)
                    }
                    _ => None,
                })
                .collect();
            ScalarExpression::Member {
                receiver: if identifiers.len() == 3 {
                    format!("{}.{}", identifiers[0].text(), identifiers[1].text())
                } else {
                    identifiers[0].text().to_owned()
                },
                name: identifiers.last().expect("qualified member name").text().to_owned(),
                enum_tag: None,
                receiver_span: token_span(&identifiers[0]),
                name_span: token_span(identifiers.last().expect("qualified member name")),
                span: wosy_syntax::byte_span(&actual),
            }
        }
        SyntaxKind::FieldAccess => {
            if let Some(field) = actual
                .descendants()
                .find(|child| child.kind() == SyntaxKind::DereferencedField)
            {
                ScalarExpression::Dereference {
                    place: derive_place(&field),
                    span: wosy_syntax::byte_span(&actual),
                }
            } else {
                derive_element(&semantic_children(&actual)[0])
            }
        }
        SyntaxKind::IndexedPlace => ScalarExpression::IndexedRead {
            place: derive_place(&actual),
            span: wosy_syntax::byte_span(&actual),
        },
        SyntaxKind::Dereference | SyntaxKind::DereferencedField => ScalarExpression::Dereference {
            place: derive_place(&actual),
            span: wosy_syntax::byte_span(&actual),
        },
        SyntaxKind::CheckedAddress => {
            let target = direct_nodes(&actual)
                .into_iter()
                .find(|child| {
                    matches!(
                        child.kind(),
                        SyntaxKind::PlaceTarget | SyntaxKind::ParenthesizedDereferencePlace
                    )
                })
                .expect("checked address target");
            ScalarExpression::CheckedAddress {
                mutability: if actual.children_with_tokens().any(
                    |element| matches!(element, NodeOrToken::Token(token) if token.text() == "!"),
                ) {
                    ScalarReferenceMutability::Mutable
                } else {
                    ScalarReferenceMutability::Shared
                },
                place: derive_place(&target),
                span: wosy_syntax::byte_span(&actual),
            }
        }
        SyntaxKind::RawAddress => {
            let target = direct_nodes(&actual)
                .into_iter()
                .find(|child| child.kind() == SyntaxKind::PlaceTarget)
                .expect("raw address target");
            ScalarExpression::RawAddress {
                place: derive_place(&target),
                span: wosy_syntax::byte_span(&actual),
            }
        }
        SyntaxKind::StructLiteral => ScalarExpression::StructLiteral {
            fields: actual
                .children()
                .filter(|child| child.kind() == SyntaxKind::StructLiteralField)
                .map(|field| {
                    let name = direct_token(&field, SyntaxKind::Identifier).expect("field name");
                    let value = field
                        .descendants()
                        .find(|child| child.kind() == SyntaxKind::Expression)
                        .map(|child| derive_expression(&child))
                        .expect("field value");
                    ScalarStructLiteralField {
                        name: name.text().to_owned(),
                        name_span: token_span(&name),
                        value,
                        span: wosy_syntax::byte_span(&field),
                    }
                })
                .collect(),
            span: wosy_syntax::byte_span(&actual),
        },
        SyntaxKind::Parenthesized => derive_expression(&direct_nodes(&actual)[0]),
        SyntaxKind::Expression => derive_element(&semantic_children(&actual)[0]),
        _ => {
            let children = semantic_children(&actual);
            children
                .first()
                .map(derive_element)
                .unwrap_or_else(|| panic!("expression grammar {:?}", actual.kind()))
        }
    }
}

fn derive_place(node: &CstNode) -> ScalarPlace {
    match node.kind() {
        SyntaxKind::ParenthesizedDereferencePlace => derive_place(
            &direct_nodes(node)
                .into_iter()
                .find(|child| child.kind() == SyntaxKind::Dereference)
                .expect("parenthesized dereference place"),
        ),
        SyntaxKind::PlaceTarget => {
            derive_place(&direct_nodes(node).into_iter().next().expect("place target"))
        }
        SyntaxKind::AssignmentTarget => {
            if let Some(child) = direct_nodes(node).into_iter().next() {
                derive_place(&child)
            } else {
                let name = direct_token(node, SyntaxKind::Identifier).expect("place name");
                ScalarPlace::Name {
                    name: name.text().to_owned(),
                    span: token_span(&name),
                }
            }
        }
        SyntaxKind::Identifier => {
            let token = direct_token(node, SyntaxKind::Identifier).expect("place name");
            ScalarPlace::Name {
                name: token.text().to_owned(),
                span: token_span(&token),
            }
        }
        SyntaxKind::QualifiedMember => {
            let identifiers: Vec<_> = node
                .children_with_tokens()
                .filter_map(|element| match element {
                    NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => {
                        Some(token)
                    }
                    _ => None,
                })
                .collect();
            ScalarPlace::Field {
                base: Box::new(ScalarPlace::Name {
                    name: identifiers[0].text().to_owned(),
                    span: token_span(&identifiers[0]),
                }),
                field: ScalarFieldReference::Unresolved {
                    name: identifiers[1].text().to_owned(),
                    span: token_span(&identifiers[1]),
                },
                span: wosy_syntax::byte_span(node),
            }
        }
        SyntaxKind::DereferencedField => {
            let field = direct_token(node, SyntaxKind::Identifier).expect("field name");
            let parent = direct_nodes(node)
                .into_iter()
                .find(|child| child.kind() == SyntaxKind::Parenthesized)
                .expect("dereference receiver");
            let dereference = parent
                .descendants()
                .find(|child| child.kind() == SyntaxKind::Dereference)
                .expect("dereference expression");
            ScalarPlace::Field {
                base: Box::new(derive_place(&dereference)),
                field: ScalarFieldReference::Unresolved {
                    name: field.text().to_owned(),
                    span: token_span(&field),
                },
                span: wosy_syntax::byte_span(node),
            }
        }
        SyntaxKind::Dereference => ScalarPlace::Dereference {
            pointer: Box::new(derive_expression(
                &direct_nodes(node)
                    .into_iter()
                    .next()
                    .expect("dereference pointer"),
            )),
            span: wosy_syntax::byte_span(node),
        },
        SyntaxKind::IndexedPlace => {
            let children = direct_nodes(node);
            let mut place = children
                .iter()
                .find(|child| child.kind() != SyntaxKind::IndexSuffix)
                .map(derive_place)
                .unwrap_or_else(|| {
                    let name =
                        direct_token(node, SyntaxKind::Identifier).expect("indexed place name");
                    ScalarPlace::Name {
                        name: name.text().to_owned(),
                        span: token_span(&name),
                    }
                });
            for suffix in children
                .iter()
                .filter(|child| child.kind() == SyntaxKind::IndexSuffix)
            {
                let index = suffix
                    .children()
                    .find(|child| child.kind() == SyntaxKind::Expression)
                    .expect("indexed place expression");
                let index_span = wosy_syntax::byte_span(&index);
                place = ScalarPlace::Index {
                    base: Box::new(place),
                    index: Box::new(derive_expression(&index)),
                    span: ByteSpan::new(
                        wosy_syntax::byte_span(node).start,
                        wosy_syntax::byte_span(&suffix).end,
                    ),
                    index_span,
                };
            }
            place
        }
        _ => panic!("place grammar"),
    }
}

fn direct_nodes(node: &CstNode) -> Vec<CstNode> {
    node.children().collect()
}

fn semantic_children(node: &CstNode) -> Vec<NodeOrToken<CstNode, CstToken>> {
    node.children_with_tokens()
        .filter(|element| match element {
            NodeOrToken::Node(_) => true,
            NodeOrToken::Token(token) => matches!(
                token.kind(),
                SyntaxKind::Identifier
                    | SyntaxKind::TypeName
                    | SyntaxKind::Integer
                    | SyntaxKind::Float
                    | SyntaxKind::Char
                    | SyntaxKind::Boolean
                    | SyntaxKind::String
                    | SyntaxKind::Operator
            ),
        })
        .collect()
}

fn direct_token(node: &CstNode, kind: SyntaxKind) -> Option<CstToken> {
    node.children_with_tokens()
        .find_map(|element| match element {
            NodeOrToken::Token(token) if token.kind() == kind => Some(token),
            _ => None,
        })
}

fn derive_element(element: &NodeOrToken<CstNode, CstToken>) -> ScalarExpression {
    match element {
        NodeOrToken::Node(node) => derive_expression(node),
        NodeOrToken::Token(token) => match token.kind() {
            SyntaxKind::Boolean => ScalarExpression::Boolean {
                value: token.text() == "true",
                span: token_span(token),
            },
            SyntaxKind::Identifier => ScalarExpression::Name {
                name: token.text().to_owned(),
                span: token_span(token),
            },
            SyntaxKind::Integer => {
                let span = token_span(token);
                match parse_integer(token.text()) {
                    Some(value) => ScalarExpression::Integer { value, span },
                    None => ScalarExpression::InvalidInteger {
                        span,
                        error_span: None,
                    },
                }
            }
            SyntaxKind::Float => {
                let span = token_span(token);
                match parse_float(token.text()) {
                    Some(value) => ScalarExpression::Float {
                        value,
                        spelling: token.text().to_owned(),
                        span,
                    },
                    None => ScalarExpression::InvalidFloat { span },
                }
            }
            SyntaxKind::Char => {
                let span = token_span(token);
                match decode_char(token.text()) {
                    Ok(value) => ScalarExpression::Char {
                        value,
                        spelling: token.text().to_owned(),
                        span,
                    },
                    Err((start, length)) => ScalarExpression::InvalidInteger {
                        span,
                        error_span: Some(string_error_span(token, start, length)),
                    },
                }
            }
            SyntaxKind::String => {
                let span = token_span(token);
                match decode_string(token.text()) {
                    Ok(value) => ScalarExpression::Utf8 {
                        value: value.into_bytes(),
                        span,
                    },
                    Err((start, length)) => ScalarExpression::InvalidInteger {
                        span,
                        error_span: Some(string_error_span(token, start, length)),
                    },
                }
            }
            _ => panic!("expression token"),
        },
    }
}

fn parse_integer(text: &str) -> Option<BigInt> {
    let (digits, radix) = if text.starts_with("0x") || text.starts_with("0X") {
        (&text[2..], 16)
    } else if text.starts_with("0b") || text.starts_with("0B") {
        (&text[2..], 2)
    } else if text.starts_with("0o") || text.starts_with("0O") {
        (&text[2..], 8)
    } else {
        (text, 10)
    };
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') {
        return None;
    }
    let mut previous_underscore = false;
    for character in digits.chars() {
        if character == '_' {
            if previous_underscore {
                return None;
            }
            previous_underscore = true;
        } else {
            let valid = character.to_digit(radix).is_some();
            if !valid {
                return None;
            }
            previous_underscore = false;
        }
    }
    BigInt::parse_bytes(
        digits
            .as_bytes()
            .iter()
            .filter(|byte| **byte != b'_')
            .copied()
            .collect::<Vec<_>>()
            .as_slice(),
        radix,
    )
}

fn token_span(token: &CstToken) -> ByteSpan {
    let range = token.text_range();
    ByteSpan::new(u32::from(range.start()), u32::from(range.end()))
}

fn span_of(expression: &ScalarExpression) -> ByteSpan {
    match expression {
        ScalarExpression::Name { span, .. }
        | ScalarExpression::Member { span, .. }
        | ScalarExpression::Integer { span, .. }
        | ScalarExpression::Float { span, .. }
        | ScalarExpression::InvalidFloat { span }
        | ScalarExpression::InvalidInteger { span, .. }
        | ScalarExpression::Boolean { span, .. }
        | ScalarExpression::Char { span, .. }
        | ScalarExpression::Utf8 { span, .. }
        | ScalarExpression::Binary { span, .. }
        | ScalarExpression::Unary { span, .. }
        | ScalarExpression::Call { span, .. }
        | ScalarExpression::If { span, .. }
        | ScalarExpression::UnitIf { span, .. } => *span,
        ScalarExpression::RawAddress { span, .. }
        | ScalarExpression::CheckedAddress { span, .. }
        | ScalarExpression::Dereference { span, .. }
        | ScalarExpression::IndexedRead { span, .. }
        | ScalarExpression::StructLiteral { span, .. }
        | ScalarExpression::ArrayLiteral { span, .. } => *span,
        ScalarExpression::Block(block) => block.span,
    }
}

#[cfg(test)]
mod tests;
