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

mod reference;
use reference::{
    expand_conditional_final_outputs, record_module_reference_origins,
    record_program_reference_origins, record_specialized_copy_policies, resolve_module_call_sites,
    resolve_reference_output_contracts,
};
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
