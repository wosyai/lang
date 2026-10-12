use super::*;
mod source_inputs;
mod lexical_inventory;
pub(in crate::scalar::reference) mod literal_seed;
pub(in crate::scalar::reference) mod initial_transfer;
pub(super) use lexical_inventory::record;
pub(in crate::scalar) use lexical_inventory::{validate, validate_null_constructors, direct_constructors, allocation_constructors, checked_pointer_cast_constructors};
pub(super) use source_inputs::{canonical_module_root, complete_module_source, global_declarations, literal_context, module_context, named_context, validate_literal_source, validate_named_source};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NameResolutionContext {
    Building,
    Complete(Box<SourceNameResolutionContext>),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceNameResolutionContext {
    pub owner: ProducerCfgOwnerId,
    pub expressions: Vec<ResolvedExpressionOutputFact>,
    pub root: SourceNameResolutionRoot,
    pub globals: Vec<SourceNameGlobalDeclaration>,
    pub parents: Vec<SourceEventId>,
    pub ancestors: Vec<SourceNameResolutionContextRoot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceNameResolutionContextRoot {
    pub owner: ProducerCfgOwnerId,
    pub root: SourceNameResolutionRoot,
    pub globals: Vec<SourceNameGlobalDeclaration>,
    pub parents: Vec<SourceEventId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceNameFunctionRoot {
    pub name: String,
    pub name_span: ByteSpan,
    pub span: ByteSpan,
    pub signature: ScalarType,
    pub parameters: Vec<String>,
    pub parameter_spans: Vec<ByteSpan>,
    pub generic_declarations: Vec<ScalarGenericParameter>,
    pub body: ScalarBlock,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceNameFunctionDeclaration {
    pub owner: ProducerCfgOwnerId,
    pub name: String,
    pub name_span: ByteSpan,
    pub span: ByteSpan,
    pub signature: ScalarType,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceNameModuleItem {
    Binding(ScalarBinding),
    Executable(ScalarBlockItem),
    Function(SourceNameFunctionDeclaration),
    Namespace(ScalarNamespace),
    Extern(ScalarExtern),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceNameResolutionRoot {
    Named(SourceNameFunctionRoot),
    Literal {
        parent: SourceEventId,
        occurrence: SourceSpan,
        signature: ScalarType,
        parameters: Vec<String>,
        parameter_spans: Vec<ByteSpan>,
        captures: Vec<closure::ClosureCapture>,
        body: ScalarBlock,
    },
    Module {
        items: Vec<SourceNameModuleItem>,
        top_level: ScalarTopLevelCorrespondence,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceNameGlobalDeclaration {
    pub owner: ProducerCfgOwnerId,
    pub binding: ScalarBinding,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceNameDeclarationSite {
    Parameter { ordinal: usize },
    Receiver { occurrence: ScalarBindingOccurrence, ordinal: usize },
    Assignment { occurrence: ScalarAssignmentOccurrence, ordinal: usize },
    Capture { event: SourceEventId, ordinal: usize },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceNameDeclarationRef {
    pub owner: ProducerCfgOwnerId,
    pub site: SourceNameDeclarationSite,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceNameResolutionRole {
    Parameter,
    Local,
    Global,
    Capture,
    InferredAssignment,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceNameReadFact {
    pub occurrence: SourceSpan,
    pub expression: ScalarExpression,
    pub event: SourceEventId,
    pub value: EvaluatedValueId,
    pub binding: ReferenceBindingId,
    pub declaration: SourceNameDeclarationRef,
    pub original_declaration: SourceNameDeclarationRef,
    pub role: SourceNameResolutionRole,
}

#[cfg(test)]
mod null_recording_tests;

#[cfg(test)]
mod allocation_recording_tests;
