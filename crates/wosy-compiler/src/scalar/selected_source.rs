use super::*;
#[cfg(test)]
pub(super) mod snapshot23_observation {
use super::*;
use std::io::Write;

#[derive(Clone, Copy)]
enum ObserverFault {
    SourceText,
    SourceLimit,
    OwnerDepth,
    OwnerLimit,
    ReferenceLimit,
    IntegerMagnitude,
    BindingDepth,
    PlaceDepth,
    ExpressionCoverage,
    ProjectionLimit,
    FrozenLimit,
    Scope,
    Constructor,
    RowLimit,
    Serialization,
    Output,
    Lock,
    Admission,
    Unwind,
    StorageCoverage,
}

#[derive(Clone, Copy, Eq, PartialEq, Serialize)]
enum ReferenceDomain {
    Configuration,
    Contributor,
    Monitor,
    Slot,
    View,
    Shape,
    Supported,
    Residual,
    Binding,
    Operation,
    Frame,
    Frozen,
    Place,
    Field,
    Projection,
    Value,
    PlaceSyntax,
}

#[derive(Serialize)]
enum Payload {
    Source { key: usize, identity: SourceIdentity },
    Owner { key: usize, fields: [usize; 8] },
    Reference { key: usize, domain: ReferenceDomain, fields: [usize; 32] },
    Event { tag: usize, fields: [usize; 32] },
    Integer { key: usize, sign: u8, magnitude: Vec<u8> },
    Text { key: usize, value: String },
    Site { factory: usize, file: &'static str, line: u32, column: u32 },
}

#[derive(Serialize)]
struct Row {
    marker: &'static str,
    pid: u32,
    sequence: usize,
    scope: [usize; 8],
    payload: Payload,
}

struct Buffer {
    bytes: [u8; 2048],
    used: usize,
}

impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > 2047 - self.used {
            return Err(std::io::Error::from(std::io::ErrorKind::WriteZero));
        }
        self.bytes[self.used..self.used + bytes.len()].copy_from_slice(bytes);
        self.used += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct Core {
    sources: Vec<SourceIdentity>,
    owners: Vec<ProducerCfgOwnerId>,
    references: Vec<(ReferenceDomain, [usize; 32])>,
    integers: Vec<(u8, Vec<u8>)>,
    texts: Vec<String>,
    threads: Vec<std::thread::ThreadId>,
    workers: usize,
    transfers: usize,
    call_rows: [usize; 8],
    rows: usize,
}

impl Core {
    const fn new() -> Self {
        Self {
            sources: Vec::new(),
            owners: Vec::new(),
            references: Vec::new(),
            integers: Vec::new(),
            texts: Vec::new(),
            threads: Vec::new(),
            workers: 0,
            transfers: 0,
            call_rows: [0; 8],
            rows: 0,
        }
    }

    fn row(&mut self, scope: [usize; 8], payload: Payload) -> Result<(), ObserverFault> {
        if self.rows == 2880 {
            return Err(ObserverFault::RowLimit);
        }
        if scope[5] > 8 {
            return Err(ObserverFault::Scope);
        }
        if scope[5] != 0 && self.call_rows[scope[5] - 1] == 256 {
            return Err(ObserverFault::RowLimit);
        }
        let row = Row {
            marker: "T0126_SNAPSHOT23_S2",
            pid: std::process::id(),
            sequence: self.rows,
            scope,
            payload,
        };
        let mut buffer = Buffer { bytes: [0; 2048], used: 0 };
        serde_json::to_writer(&mut buffer, &row).map_err(|_| ObserverFault::Serialization)?;
        buffer.bytes[buffer.used] = b'\n';
        buffer.used += 1;
        let mut output = std::io::stderr().lock();
        if FAULTED.load(Ordering::SeqCst) {
            return Err(ObserverFault::Scope);
        }
        output.write_all(&buffer.bytes[..buffer.used])
            .map_err(|_| ObserverFault::Output)?;
        self.rows += 1;
        if scope[5] != 0 {
            self.call_rows[scope[5] - 1] += 1;
        }
        Ok(())
    }

    fn source(&mut self, scope: [usize; 8], source: &SourceIdentity) -> Result<usize, ObserverFault> {
        if [&source.project, &source.package, &source.path, &source.revision]
            .iter().any(|text| text.len() > 64)
        {
            return Err(ObserverFault::SourceText);
        }
        if let Some(key) = self.sources.iter().position(|known| known == source) {
            return Ok(key);
        }
        if self.sources.len() == 32 {
            return Err(ObserverFault::SourceLimit);
        }
        let key = self.sources.len();
        self.row(scope, Payload::Source { key, identity: source.clone() })?;
        self.sources.push(source.clone());
        Ok(key)
    }

    fn owner(&mut self, scope: [usize; 8], owner: &ProducerCfgOwnerId) -> Result<usize, ObserverFault> {
        let mut current = owner;
        let mut depth = 0;
        loop {
            depth += 1;
            if depth > 8 {
                return Err(ObserverFault::OwnerDepth);
            }
            let source = match current {
                ProducerCfgOwnerId::Module(source) => source,
                ProducerCfgOwnerId::Function { source, .. } => source,
                ProducerCfgOwnerId::Literal { literal, .. } => &literal.source,
            };
            if [&source.project, &source.package, &source.path, &source.revision]
                .iter().any(|text| text.len() > 64)
            {
                return Err(ObserverFault::SourceText);
            }
            match current {
                ProducerCfgOwnerId::Literal { enclosing, .. } => current = enclosing,
                ProducerCfgOwnerId::Module(_) | ProducerCfgOwnerId::Function { .. } => break,
            }
        }
        if let Some(key) = self.owners.iter().position(|known| known == owner) {
            return Ok(key);
        }
        let fields = match owner {
            ProducerCfgOwnerId::Module(source) => [0, self.source(scope, source)?, 0, 0, 0, 0, 0, 0],
            ProducerCfgOwnerId::Function { source, declaration, body } => {
                let (kind, ordinal) = match body {
                    ProducerFunctionBodyId::Declaration => (0, 0),
                    ProducerFunctionBodyId::Overload(ordinal) => (1, *ordinal),
                };
                [1, self.source(scope, source)?, declaration.start as usize,
                 declaration.end as usize, kind, ordinal, 0, 0]
            }
            ProducerCfgOwnerId::Literal { enclosing, capture_point, literal } => {
                let parent = self.owner(scope, enclosing)?;
                [2, self.source(scope, &literal.source)?, literal.range.start as usize,
                 literal.range.end as usize, 0, 0, parent, capture_point.0]
            }
        };
        if self.owners.len() == 256 {
            return Err(ObserverFault::OwnerLimit);
        }
        let key = self.owners.len();
        self.row(scope, Payload::Owner { key, fields })?;
        self.owners.push(owner.clone());
        Ok(key)
    }

    fn reference(&mut self, scope: [usize; 8], domain: ReferenceDomain, fields: [usize; 32]) -> Result<usize, ObserverFault> {
        if let Some(key) = self.references.iter().position(|known| *known == (domain, fields)) {
            return Ok(key);
        }
        if self.references.len() + self.integers.len() + self.texts.len() == 512 {
            return Err(ObserverFault::ReferenceLimit);
        }
        let key = self.references.len();
        self.row(scope, Payload::Reference { key, domain, fields })?;
        self.references.push((domain, fields));
        Ok(key)
    }

    fn integer(&mut self, scope: [usize; 8], value: &BigInt) -> Result<usize, ObserverFault> {
        let operand = integer_operand(value)?;
        if let Some(key) = self.integers.iter().position(|known| known.0 == operand.sign && known.1 == operand.magnitude) {
            return Ok(key);
        }
        if self.references.len() + self.integers.len() + self.texts.len() == 512 {
            return Err(ObserverFault::ReferenceLimit);
        }
        let key = self.integers.len();
        self.row(scope, Payload::Integer { key, sign: operand.sign, magnitude: operand.magnitude.clone() })?;
        self.integers.push((operand.sign, operand.magnitude));
        Ok(key)
    }

    fn text(&mut self, scope: [usize; 8], value: &str) -> Result<usize, ObserverFault> {
        if value.len() > 64 {
            return Err(ObserverFault::SourceText);
        }
        if let Some(key) = self.texts.iter().position(|known| known == value) {
            return Ok(key);
        }
        if self.references.len() + self.integers.len() + self.texts.len() == 512 {
            return Err(ObserverFault::ReferenceLimit);
        }
        let key = self.texts.len();
        self.row(scope, Payload::Text { key, value: value.to_owned() })?;
        self.texts.push(value.to_owned());
        Ok(key)
    }

    fn indexed(&mut self, scope: [usize; 8], domain: ReferenceDomain, owner: &ProducerCfgOwnerId, index: usize) -> Result<usize, ObserverFault> {
        let mut fields = [0; 32];
        fields[0] = self.owner(scope, owner)?;
        fields[1] = index;
        self.reference(scope, domain, fields)
    }
}

#[derive(Serialize)]
struct IntegerOperand {
    sign: u8,
    magnitude: Vec<u8>,
}

fn integer_operand(value: &BigInt) -> Result<IntegerOperand, ObserverFault> {
    if value.bits() > 128 {
        return Err(ObserverFault::IntegerMagnitude);
    }
    let (sign, magnitude) = value.to_bytes_be();
    let sign = match sign {
        num_bigint::Sign::Minus => 0,
        num_bigint::Sign::NoSign => 1,
        num_bigint::Sign::Plus => 2,
    };
    Ok(IntegerOperand { sign, magnitude })
}

impl Core {
    fn binding(&mut self, scope: [usize; 8], binding: &ReferenceBindingId, depth: usize) -> Result<usize, ObserverFault> {
        if depth >= 8 {
            return Err(ObserverFault::BindingDepth);
        }
        let mut fields = [0; 32];
        fields[0] = self.source(scope, &binding.source)?;
        fields[1] = binding.function_span.start as usize;
        fields[2] = binding.function_span.end as usize;
        fields[3] = binding.declaration_span.start as usize;
        fields[4] = binding.declaration_span.end as usize;
        fields[5] = binding.block_span.start as usize;
        fields[6] = binding.block_span.end as usize;
        match &binding.kind {
            ReferenceBindingKind::Declared => fields[7] = 0,
            ReferenceBindingKind::BorrowedReferent { parameter } => {
                fields[7] = 1;
                fields[8] = *parameter;
            }
            ReferenceBindingKind::CallOutput { point, output } => {
                fields[7] = 2;
                fields[8] = point.0;
                fields[9] = *output;
            }
            ReferenceBindingKind::AssignmentPrevious { snapshot, output } => {
                fields[7] = 3;
                fields[8] = snapshot.0;
                fields[9] = *output;
            }
            ReferenceBindingKind::Captured { environment_start, environment_end } => {
                fields[7] = 4;
                fields[8] = *environment_start as usize;
                fields[9] = *environment_end as usize;
            }
            ReferenceBindingKind::Projected { field } => {
                fields[7] = 5;
                fields[8] = *field;
            }
            ReferenceBindingKind::ImportedCapture { call_start, call_end, output, producer } => {
                fields[7] = 6;
                fields[8] = *call_start as usize;
                fields[9] = *call_end as usize;
                fields[10] = *output;
                fields[11] = self.binding(scope, producer, depth + 1)?;
            }
        }
        self.reference(scope, ReferenceDomain::Binding, fields)
    }

    fn field(&mut self, scope: [usize; 8], field: &ScalarFieldReference) -> Result<usize, ObserverFault> {
        let mut fields = [0; 32];
        match field {
            ScalarFieldReference::Unresolved { name, span } => {
                fields[0] = 0;
                fields[1] = self.text(scope, name)?;
                fields[2] = span.start as usize;
                fields[3] = span.end as usize;
            }
            ScalarFieldReference::Resolved(field) => {
                fields[0] = 1;
                fields[1] = self.source(scope, &field.structure.source)?;
                fields[2] = field.structure.index;
                fields[3] = field.index;
            }
        }
        self.reference(scope, ReferenceDomain::Field, fields)
    }

    fn projection(&mut self, scope: [usize; 8], projection: &ReferencePlaceProjection) -> Result<usize, ObserverFault> {
        let mut fields = [0; 32];
        match projection {
            ReferencePlaceProjection::Field(field) => {
                fields[0] = 0;
                fields[1] = self.field(scope, field)?;
            }
            ReferencePlaceProjection::Index(ScalarExpression::Integer { value, span }) => {
                fields[0] = 1;
                fields[1] = self.integer(scope, value)?;
                fields[2] = span.start as usize;
                fields[3] = span.end as usize;
            }
            ReferencePlaceProjection::Index(_) | ReferencePlaceProjection::Dereference(_) => {
                return Err(ObserverFault::ExpressionCoverage);
            }
        }
        self.reference(scope, ReferenceDomain::Projection, fields)
    }

    fn syntax(&mut self, scope: [usize; 8], place: &ScalarPlace, depth: usize) -> Result<usize, ObserverFault> {
        if depth >= 8 {
            return Err(ObserverFault::PlaceDepth);
        }
        let mut fields = [0; 32];
        match place {
            ScalarPlace::Name { name, span } => {
                fields[0] = 0;
                fields[1] = self.text(scope, name)?;
                fields[2] = span.start as usize;
                fields[3] = span.end as usize;
            }
            ScalarPlace::Field { base, field, span } => {
                fields[0] = 1;
                fields[1] = self.syntax(scope, base, depth + 1)?;
                fields[2] = self.field(scope, field)?;
                fields[3] = span.start as usize;
                fields[4] = span.end as usize;
            }
            ScalarPlace::Index { base, index, span, index_span } => {
                let ScalarExpression::Integer { value, span: expression_span } = index.as_ref() else {
                    return Err(ObserverFault::ExpressionCoverage);
                };
                fields[0] = 2;
                fields[1] = self.syntax(scope, base, depth + 1)?;
                fields[2] = self.integer(scope, value)?;
                fields[3] = expression_span.start as usize;
                fields[4] = expression_span.end as usize;
                fields[5] = span.start as usize;
                fields[6] = span.end as usize;
                fields[7] = index_span.start as usize;
                fields[8] = index_span.end as usize;
            }
            ScalarPlace::Dereference { .. } => return Err(ObserverFault::ExpressionCoverage),
        }
        self.reference(scope, ReferenceDomain::PlaceSyntax, fields)
    }

    fn place(&mut self, scope: [usize; 8], place: &ReferencePlaceId) -> Result<usize, ObserverFault> {
        if place.projections.len() > 8 {
            return Err(ObserverFault::ProjectionLimit);
        }
        let mut fields = [0; 32];
        fields[0] = self.source(scope, &place.source)?;
        fields[1] = place.function_span.start as usize;
        fields[2] = place.function_span.end as usize;
        fields[3] = place.declaration_span.start as usize;
        fields[4] = place.declaration_span.end as usize;
        fields[5] = place.block_span.start as usize;
        fields[6] = place.block_span.end as usize;
        fields[7] = self.binding(scope, &place.binding, 0)?;
        fields[8] = place.projections.len();
        for (ordinal, projection) in place.projections.iter().enumerate() {
            fields[9 + ordinal] = self.projection(scope, projection)?;
        }
        fields[17] = self.syntax(scope, &place.place, 0)?;
        self.reference(scope, ReferenceDomain::Place, fields)
    }

    fn operation(&mut self, scope: [usize; 8], operation: &AssignmentOperationId) -> Result<usize, ObserverFault> {
        if operation.occurrence.output_projection.len() > 16 {
            return Err(ObserverFault::ProjectionLimit);
        }
        let mut fields = [0; 32];
        fields[0] = self.owner(scope, &operation.producer)?;
        fields[1] = self.source(scope, &operation.occurrence.source_span.source)?;
        fields[2] = operation.occurrence.source_span.range.start as usize;
        fields[3] = operation.occurrence.source_span.range.end as usize;
        fields[4] = operation.occurrence.output_projection.len();
        fields[5..5 + operation.occurrence.output_projection.len()]
            .copy_from_slice(&operation.occurrence.output_projection);
        self.reference(scope, ReferenceDomain::Operation, fields)
    }

    fn frozen(&mut self, scope: [usize; 8], selection: &FrozenSelectionOccurrence) -> Result<usize, ObserverFault> {
        let frame = self.indexed(scope, ReferenceDomain::Frame,
            &selection.application.frame.owner, selection.application.frame.variable.0)?;
        let mut fields = [0; 32];
        fields[0] = frame;
        fields[1] = self.operation(scope, &selection.application.operation)?;
        fields[2] = self.operation(scope, &selection.destination.operation)?;
        fields[3] = selection.destination.target;
        self.reference(scope, ReferenceDomain::Frozen, fields)
    }

    fn value(&mut self, scope: [usize; 8], value: &EvaluatedValueId) -> Result<usize, ObserverFault> {
        if value.resolved.output_projection.len() > 16 {
            return Err(ObserverFault::ProjectionLimit);
        }
        let mut fields = [0; 32];
        fields[0] = self.owner(scope, &value.resolved.producer)?;
        fields[1] = self.source(scope, &value.resolved.occurrence.source)?;
        fields[2] = value.resolved.occurrence.range.start as usize;
        fields[3] = value.resolved.occurrence.range.end as usize;
        fields[4] = value.point.0;
        fields[5] = value.resolved.output;
        fields[6] = value.resolved.output_projection.len();
        fields[7..7 + value.resolved.output_projection.len()]
            .copy_from_slice(&value.resolved.output_projection);
        self.reference(scope, ReferenceDomain::Value, fields)
    }
}

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[derive(Clone, Copy, Eq, PartialEq)]
struct ErrorIdentity {
    source: usize,
    start: u32,
    end: u32,
    reason: ReferenceAnalysisErrorReason,
}

#[derive(Clone, Copy)]
struct Worker {
    thread: usize,
    ordinal: usize,
    pass: usize,
    stage: usize,
    trace: usize,
    calls: usize,
}

#[derive(Clone, Copy)]
struct Constructor {
    token: usize,
    identity: ErrorIdentity,
    mapped: Option<(usize, usize)>,
}

#[derive(Clone, Copy)]
struct Call {
    ordinal: usize,
    constructors: usize,
    constructor: Option<Constructor>,
    helper_count: usize,
    result: bool,
    receiver: bool,
    outcome: usize,
}

#[derive(Clone, Copy)]
struct Helper {
    ordinal: usize,
    rejected: Option<usize>,
    result: bool,
    success: bool,
}

#[derive(Clone, Copy)]
struct State {
    worker: Option<Worker>,
    call: Option<Call>,
    helper: Option<Helper>,
    last_helper: Option<Helper>,
    mapped: Option<(usize, usize)>,
    phase: usize,
}

impl State {
    const fn new() -> Self {
        Self {
            worker: None,
            call: None,
            helper: None,
            last_helper: None,
            mapped: None,
            phase: 0,
        }
    }

    fn scope(&self) -> [usize; 8] {
        let Some(worker) = self.worker else { return [0; 8]; };
        let transfer = match self.call { Some(call) => call.ordinal, None => 0 };
        let helper = match self.helper { Some(helper) => helper.ordinal, None => 0 };
        [worker.thread, worker.ordinal, worker.pass, worker.stage, worker.trace,
         transfer, helper, self.phase]
    }
}

static CORE: Mutex<Core> = Mutex::new(Core::new());
static FAULTED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State::new()) };
}

fn fault(fault: ObserverFault) {
    if FAULTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let code = match fault {
        ObserverFault::SourceText => 0,
        ObserverFault::SourceLimit => 1,
        ObserverFault::OwnerDepth => 2,
        ObserverFault::OwnerLimit => 3,
        ObserverFault::ReferenceLimit => 4,
        ObserverFault::IntegerMagnitude => 5,
        ObserverFault::BindingDepth => 6,
        ObserverFault::PlaceDepth => 7,
        ObserverFault::ExpressionCoverage => 8,
        ObserverFault::ProjectionLimit => 9,
        ObserverFault::FrozenLimit => 10,
        ObserverFault::Scope => 11,
        ObserverFault::Constructor => 12,
        ObserverFault::RowLimit => 13,
        ObserverFault::Serialization => 14,
        ObserverFault::Output => 15,
        ObserverFault::Lock => 16,
        ObserverFault::Admission => 17,
        ObserverFault::Unwind => 18,
        ObserverFault::StorageCoverage => 19,
    };
    let _delivery = writeln!(std::io::stderr().lock(),
        "{{\"marker\":\"T0126_SNAPSHOT23_S2_FAULT\",\"pid\":{},\"code\":{}}}",
        std::process::id(), code);
}

macro_rules! observe {
    ($state:ident, $core:ident, $body:block) => {
        observe!(false, $state, $core, $body)
    };
    ($entry:literal, $state:ident, $core:ident, $body:block) => {{
        if !FAULTED.load(Ordering::SeqCst) {
            let result = STATE.with(|cell| {
                let mut $state = cell.try_borrow_mut().map_err(|_| ObserverFault::Lock)?;
                if !$entry && $state.worker.is_none() { return Ok(()); }
                let mut $core = CORE.try_lock().map_err(|_| ObserverFault::Lock)?;
                $body
                Ok::<(), ObserverFault>(())
            });
            if let Err(error) = result {
                fault(error);
            }
        }
    }};
}

impl Core {
    fn error(&mut self, scope: [usize; 8], error: &ReferenceAnalysisError) -> Result<ErrorIdentity, ObserverFault> {
        let ReferenceAnalysisError::MissingContext { source_span, reason } = error else {
            return Err(ObserverFault::Constructor);
        };
        Ok(ErrorIdentity {
            source: self.source(scope, &source_span.source)?,
            start: source_span.range.start,
            end: source_span.range.end,
            reason: *reason,
        })
    }

    fn event(&mut self, scope: [usize; 8], tag: usize, values: &[usize]) -> Result<(), ObserverFault> {
        if values.len() > 32 {
            return Err(ObserverFault::Scope);
        }
        let mut fields = [0; 32];
        fields[..values.len()].copy_from_slice(values);
        self.row(scope, Payload::Event { tag, fields })
    }
}

enum SiteToken {
    Location { factory: usize, location: &'static std::panic::Location<'static> },
    Literal { factory: usize, site: usize },
}

fn reason_code(reason: ReferenceAnalysisErrorReason) -> usize {
    match reason {
        ReferenceAnalysisErrorReason::MissingResolvedBinding => 0,
        ReferenceAnalysisErrorReason::MissingResolvedCallTarget => 1,
        ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin => 2,
        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin => 3,
        ReferenceAnalysisErrorReason::MoveFromArrayElement => 4,
        ReferenceAnalysisErrorReason::MovedPlace => 5,
        ReferenceAnalysisErrorReason::EscapingCapturedReference => 6,
        ReferenceAnalysisErrorReason::MissingGlobalCallableProducer => 7,
        ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence => 8,
        ReferenceAnalysisErrorReason::InvalidPointerTypeCorrespondence => 9,
    }
}

fn construct(site: SiteToken, error: &ReferenceAnalysisError) {
    observe!(state, core, {
        if state.call.is_none() { return Ok(()); }
        let scope = state.scope();
        let identity = core.error(scope, error)?;
        let token = core.rows;
        match site {
            SiteToken::Location { factory, location } => {
                if location.file().len() > 128 {
                    return Err(ObserverFault::SourceText);
                }
                core.row(scope, Payload::Site {
                    factory, file: location.file(), line: location.line(), column: location.column(),
                })?;
            }
            SiteToken::Literal { factory, site } => core.event(scope, 20, &[factory, site])?,
        }
        let mapped = state.mapped.take();
        let Some(call) = &mut state.call else { return Err(ObserverFault::Scope); };
        call.constructors += 1;
        let count = call.constructors;
        call.constructor = Some(Constructor { token, identity, mapped });
        let (helper, predicate) = match mapped { Some(pair) => pair, None => (0, 0) };
        core.event(scope, 21, &[token, count, identity.source,
            identity.start as usize, identity.end as usize, reason_code(identity.reason), helper, predicate])?;
    });
}

pub(in crate::scalar) fn constructed(factory: usize, location: &'static std::panic::Location<'static>, error: &ReferenceAnalysisError) {
    construct(SiteToken::Location { factory, location }, error);
}

pub(in crate::scalar) fn constructed_site(factory: usize, site: usize, error: &ReferenceAnalysisError) {
    construct(SiteToken::Literal { factory, site }, error);
}

fn restore(previous: State) {
    let restored = STATE.with(|cell| {
        let mut state = cell.try_borrow_mut().map_err(|_| ObserverFault::Lock)?;
        *state = previous;
        Ok::<(), ObserverFault>(())
    });
    if let Err(error) = restored { fault(error); }
}

pub(in crate::scalar) struct WorkerGuard {
    previous: Option<State>,
    ordinal: Option<usize>,
}

pub(in crate::scalar) fn worker(owner: &ProducerCfgOwnerId, pass: usize, function: bool, stage: usize, trace: usize) -> WorkerGuard {
    let mut guard = WorkerGuard { previous: None, ordinal: None };
    if std::env::var_os("T0126_SNAPSHOT23_S2") != Some(std::ffi::OsString::from("admit")) {
        return guard;
    }
    observe!(true, state, core, {
        guard.previous = Some(*state);
        *state = State::new();
        if !function || !matches!(owner,
            ProducerCfgOwnerId::Function { source, declaration, body }
                if source.project == "project" && source.package == "package"
                    && source.path == "src/main.w" && source.revision == "r1"
                    && declaration.start == 13 && declaration.end == 20
                    && *body == ProducerFunctionBodyId::Declaration)
        {
            return Ok(());
        }
        if core.workers == 16 || stage > 1 { return Err(ObserverFault::Scope); }
        if std::env::var_os("RUST_MIN_STACK") != Some(std::ffi::OsString::from("536870912")) {
            return Err(ObserverFault::Admission);
        }
        let thread = std::thread::current().id();
        let thread_key = match core.threads.iter().position(|known| *known == thread) {
            Some(key) => key + 1,
            None => {
                if core.threads.len() == 8 { return Err(ObserverFault::Scope); }
                core.threads.push(thread);
                core.threads.len()
            }
        };
        core.workers += 1;
        state.worker = Some(Worker {
            thread: thread_key, ordinal: core.workers, pass, stage, trace, calls: 0,
        });
        guard.ordinal = Some(core.workers);
        let scope = state.scope();
        let owner_key = core.owner(scope, owner)?;
        core.event(scope, 1, &[owner_key, 536870912])?;
    });
    guard
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        let Some(previous) = self.previous.take() else { return; };
        if std::thread::panicking() {
            fault(ObserverFault::Unwind);
        } else if let Some(ordinal) = self.ordinal {
            observe!(state, core, {
                let Some(worker) = state.worker else { return Err(ObserverFault::Scope); };
                if worker.ordinal != ordinal || state.call.is_some() { return Err(ObserverFault::Scope); }
                core.event(state.scope(), 2, &[worker.calls])?;
            });
        }
        restore(previous);
    }
}

pub(in crate::scalar) struct CallGuard {
    previous: Option<State>,
    ordinal: Option<usize>,
}

impl CallGuard {
    pub(in crate::scalar) fn admitted(&self) -> bool {
        self.ordinal.is_some() && !FAULTED.load(Ordering::SeqCst)
    }
}

pub(in crate::scalar) fn call(owner: &ProducerCfgOwnerId, point: &ReferenceFlowPoint, incoming: &SelectedSourceState, frozen: &[SelectedSnapshotInput]) -> CallGuard {
    let mut guard = CallGuard { previous: None, ordinal: None };
    observe!(state, core, {
        let Some(worker) = state.worker else { return Ok(()); };
        if point.id != CfgPointId(23)
            || !matches!(&point.kind, CfgPointKind::AssignmentSnapshot { destination_point, .. } if *destination_point == CfgPointId(22))
            || point.source_span.range.start != 117 || point.source_span.range.end != 122
            || !matches!(owner, ProducerCfgOwnerId::Function { source, declaration, body }
                if *source == point.source_span.source && source.project == "project"
                    && source.package == "package" && source.path == "src/main.w"
                    && source.revision == "r1" && declaration.start == 13
                    && declaration.end == 20 && *body == ProducerFunctionBodyId::Declaration)
        {
            return Ok(());
        }
        if state.call.is_some() || core.transfers == 8 { return Err(ObserverFault::Scope); }
        let mut worker = worker;
        worker.calls += 1;
        state.worker = Some(worker);
        guard.previous = Some(*state);
        core.transfers += 1;
        guard.ordinal = Some(core.transfers);
        state.call = Some(Call {
            ordinal: core.transfers, constructors: 0, constructor: None,
            helper_count: 0, result: false, receiver: false, outcome: 0,
        });
        state.helper = None;
        state.last_helper = None;
        state.mapped = None;
        state.phase = 1;
        let scope = state.scope();
        if frozen.len() > 8 { return Err(ObserverFault::FrozenLimit); }
        let owner_key = core.owner(scope, owner)?;
        core.event(scope, 3, &[owner_key, point.id.0, 22, frozen.len()])?;
        let CfgPointKind::AssignmentSnapshot { holder, observed, .. } = &point.kind else {
            return Err(ObserverFault::Scope);
        };
        let holder = core.binding(scope, holder, 0)?;
        core.event(scope, 11, &[holder, usize::from(*observed)])?;
        match incoming {
            SelectedSourceState::ObservationOnly => core.event(scope, 4, &[0])?,
            SelectedSourceState::Active { configuration, contributor, predecessor, monitor } => {
                let configuration = core.indexed(scope, ReferenceDomain::Configuration, &configuration.owner, configuration.index)?;
                let contributor = core.indexed(scope, ReferenceDomain::Contributor, &contributor.owner, contributor.index)?;
                let predecessor_owner = core.owner(scope, &predecessor.owner)?;
                let monitor = core.indexed(scope, ReferenceDomain::Monitor, &monitor.owner, monitor.index)?;
                core.event(scope, 4, &[1, configuration, contributor, predecessor_owner, predecessor.point.0, monitor])?;
            }
        }
        for (ordinal, input) in frozen.iter().enumerate() {
            let selection = core.frozen(scope, &input.selection)?;
            let place = core.place(scope, &input.place)?;
            core.event(scope, 5, &[ordinal, selection, place])?;
        }
    });
    guard
}

pub(in crate::scalar) fn storage(ordinal: usize, access: usize) {
    observe!(state, core, {
        if state.call.is_none() { return Ok(()); }
        if ordinal >= 8 || access > 3 { return Err(ObserverFault::Scope); }
        core.event(state.scope(), 6, &[ordinal, access])?;
        if access != 0 { return Err(ObserverFault::StorageCoverage); }
    });
}

pub(in crate::scalar) fn returned(result: &Result<Vec<SelectedSourceState>, ReferenceAnalysisError>) {
    observe!(state, core, {
        if state.call.is_none() { return Ok(()); }
        state.phase = 2;
        let scope = state.scope();
        match result {
            Ok(outputs) => {
                let Some(call) = &mut state.call else { return Err(ObserverFault::Scope); };
                if call.result || call.receiver { return Err(ObserverFault::Scope); }
                call.result = true;
                call.outcome = 0;
                core.event(scope, 7, &[0, outputs.len(), call.constructors])?;
                if call.constructors != 0 { return Err(ObserverFault::Constructor); }
            }
            Err(error) => {
                let identity = core.error(scope, error)?;
                let Some(call) = &mut state.call else { return Err(ObserverFault::Scope); };
                if call.result || call.receiver { return Err(ObserverFault::Scope); }
                call.result = true;
                core.event(scope, 7, &[1, call.constructors, identity.source,
                    identity.start as usize, identity.end as usize, reason_code(identity.reason)])?;
                let Some(constructor) = call.constructor else { return Err(ObserverFault::Constructor); };
                if call.constructors != 1 || constructor.identity != identity { return Err(ObserverFault::Constructor); }
                call.outcome = match constructor.mapped { Some(_) => 1, None => 2 };
                core.event(scope, 8, &[constructor.token, call.outcome])?;
            }
        }
    });
}

pub(in crate::scalar) fn receive_ok(outputs: &[SelectedSourceState]) {
    observe!(state, core, {
        if state.call.is_none() { return Ok(()); }
        state.phase = 3;
        let scope = state.scope();
        let Some(call) = &mut state.call else { return Err(ObserverFault::Scope); };
        if !call.result || call.receiver || call.outcome != 0 { return Err(ObserverFault::Scope); }
        call.receiver = true;
        core.event(scope, 9, &[0, outputs.len()])?;
    });
}

pub(in crate::scalar) fn receive_error(error: &ReferenceAnalysisError, pre_push_ordinal: usize) {
    observe!(state, core, {
        if state.call.is_none() { return Ok(()); }
        state.phase = 3;
        let scope = state.scope();
        let identity = core.error(scope, error)?;
        let Some(call) = &mut state.call else { return Err(ObserverFault::Scope); };
        let Some(constructor) = call.constructor else { return Err(ObserverFault::Constructor); };
        if !call.result || call.receiver || call.outcome == 0 || constructor.identity != identity {
            return Err(ObserverFault::Scope);
        }
        call.receiver = true;
        core.event(scope, 9, &[1, pre_push_ordinal, constructor.token, call.outcome,
            identity.source, identity.start as usize, identity.end as usize, reason_code(identity.reason)])?;
    });
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        let Some(previous) = self.previous.take() else { return; };
        if std::thread::panicking() {
            fault(ObserverFault::Unwind);
        } else {
            observe!(state, core, {
                let Some(call) = state.call else { return Err(ObserverFault::Scope); };
                if Some(call.ordinal) != self.ordinal || !call.result || !call.receiver {
                    return Err(ObserverFault::Scope);
                }
                state.phase = 4;
                core.event(state.scope(), 10, &[call.outcome, call.constructors])?;
            });
        }
        restore(previous);
    }
}

pub(in crate::scalar) struct HelperGuard {
    previous: Option<Helper>,
    ordinal: Option<usize>,
}

pub(in crate::scalar) fn projected(configuration: &SourceConfigurationFact, place: &ReferencePlaceId) -> HelperGuard {
    let mut guard = HelperGuard { previous: None, ordinal: None };
    observe!(state, core, {
        let Some(call) = &mut state.call else { return Ok(()); };
        if call.helper_count == 64 { return Err(ObserverFault::Scope); }
        call.helper_count += 1;
        let ordinal = call.helper_count;
        guard.previous = state.helper;
        guard.ordinal = Some(ordinal);
        state.helper = Some(Helper { ordinal, rejected: None, result: false, success: false });
        let scope = state.scope();
        let configuration = core.indexed(scope, ReferenceDomain::Configuration,
            &configuration.id.owner, configuration.id.index)?;
        let place = core.place(scope, place)?;
        core.event(scope, 30, &[configuration, place])?;
    });
    guard
}

pub(in crate::scalar) fn coordinates(configuration: &SourceConfigurationFact, role: &SourceCoordinateRole, count: usize) {
    observe!(state, core, {
        if state.helper.is_none() { return Ok(()); }
        let scope = state.scope();
        let SourceCoordinateRole::Binding(binding) = role else { return Err(ObserverFault::Scope); };
        let configuration = core.indexed(scope, ReferenceDomain::Configuration,
            &configuration.id.owner, configuration.id.index)?;
        let binding = core.binding(scope, binding, 0)?;
        core.event(scope, 31, &[configuration, binding, count])?;
    });
}

pub(in crate::scalar) fn slots(coordinate: &SourceSlotFact, count: usize) {
    observe!(state, core, {
        if state.helper.is_none() { return Ok(()); }
        let scope = state.scope();
        let id = core.indexed(scope, ReferenceDomain::Slot, &coordinate.id.owner, coordinate.id.index)?;
        let shape = core.indexed(scope, ReferenceDomain::Shape, &coordinate.shape.producer, coordinate.shape.index)?;
        core.event(scope, 32, &[id, shape, count])?;
    });
}

pub(in crate::scalar) fn slot(slot: &SourceSelectedSlotFact) {
    observe!(state, core, {
        if state.helper.is_none() { return Ok(()); }
        let scope = state.scope();
        let coordinate = core.indexed(scope, ReferenceDomain::Slot, &slot.coordinate.owner, slot.coordinate.index)?;
        let view = core.indexed(scope, ReferenceDomain::View, &slot.view.owner, slot.view.index)?;
        let availability = match &slot.availability {
            ValueAvailability::Available => 0,
            ValueAvailability::Uninitialized { .. } => 1,
            ValueAvailability::Moved { .. } => 2,
            ValueAvailability::PartiallyUnavailable { .. } => 3,
        };
        core.event(scope, 33, &[coordinate, view, availability])?;
    });
}

pub(in crate::scalar) fn step(tables: &EvaluatedValueTables, requested: &SourceValueViewRef, projection: &ReferencePlaceProjection, ordinal: usize) {
    observe!(state, core, {
        if state.helper.is_none() { return Ok(()); }
        let scope = state.scope();
        let requested_key = core.indexed(scope, ReferenceDomain::View, &requested.owner, requested.index)?;
        let projection = core.projection(scope, projection)?;
        let actual = tables.source_views.get(requested.index);
        core.event(scope, 34, &[ordinal, requested_key, projection, tables.source_views.len(),
            usize::from(actual.is_some()), usize::from(actual.is_some_and(|view| view.id == *requested))])?;
        if let Some(view) = actual {
            let actual_key = core.indexed(scope, ReferenceDomain::View, &view.id.owner, view.id.index)?;
            let shape = core.indexed(scope, ReferenceDomain::Shape, &view.shape.producer, view.shape.index)?;
            let payload = match &view.payload {
                SourcePayloadView::Plain => 0,
                SourcePayloadView::Unwritten => 1,
                SourcePayloadView::Formal(_) => 2,
                SourcePayloadView::Raw { .. } => 3,
                SourcePayloadView::StaticRaw(_) => 4,
                SourcePayloadView::Aggregate(_) => 5,
                SourcePayloadView::Backing { .. } => 6,
                SourcePayloadView::Checked { .. } => 7,
            };
            core.event(scope, 35, &[ordinal, actual_key, shape, payload])?;
            if let SourcePayloadView::Backing { instance, element } = &view.payload {
                let (domain, owner, index) = match instance {
                    SourceInstanceRef::Supported(id) => (ReferenceDomain::Supported, &id.owner, id.index),
                    SourceInstanceRef::Residual(id) => (ReferenceDomain::Residual, &id.owner, id.index),
                };
                let instance = core.indexed(scope, domain, owner, index)?;
                let element = core.indexed(scope, ReferenceDomain::Shape, &element.producer, element.index)?;
                core.event(scope, 36, &[ordinal, usize::from(matches!(domain, ReferenceDomain::Residual)), instance, element])?;
            }
        }
    });
}

pub(in crate::scalar) fn members(total: usize, selected: &[&SourceMemberView]) {
    observe!(state, core, {
        if state.helper.is_none() { return Ok(()); }
        let scope = state.scope();
        core.event(scope, 37, &[total, selected.len()])?;
        if let [member] = selected {
            let source = core.value(scope, &member.source)?;
            let view = core.indexed(scope, ReferenceDomain::View, &member.view.owner, member.view.index)?;
            let (kind, operand) = match member.projection.as_slice() {
                [ValueProjection::FixedIndex(index)] => (0, core.integer(scope, &BigInt::from(*index))?),
                [ValueProjection::Field(field)] => {
                    let mut fields = [0; 32];
                    fields[0] = 1;
                    fields[1] = core.source(scope, &field.structure.source)?;
                    fields[2] = field.structure.index;
                    fields[3] = field.index;
                    (1, core.reference(scope, ReferenceDomain::Field, fields)?)
                }
                _ => return Err(ObserverFault::ExpressionCoverage),
            };
            core.event(scope, 38, &[source, view, kind, operand])?;
        }
    });
}

pub(in crate::scalar) fn rejected(predicate: usize) {
    observe!(state, core, {
        let Some(helper) = &mut state.helper else { return Ok(()); };
        if helper.rejected.is_some() || helper.result || !(1..=5).contains(&predicate) {
            return Err(ObserverFault::Scope);
        }
        helper.rejected = Some(predicate);
        core.event(state.scope(), 39, &[predicate])?;
    });
}

pub(in crate::scalar) fn unit_result(success: bool) {
    observe!(state, core, {
        let Some(helper) = &mut state.helper else { return Ok(()); };
        if helper.result || success == helper.rejected.is_some() { return Err(ObserverFault::Scope); }
        helper.result = true;
        helper.success = success;
        core.event(state.scope(), 40, &[usize::from(success)])?;
    });
}

impl Drop for HelperGuard {
    fn drop(&mut self) {
        let Some(ordinal) = self.ordinal else { return; };
        if std::thread::panicking() {
            fault(ObserverFault::Unwind);
        } else {
            observe!(state, core, {
                let Some(helper) = state.helper else { return Err(ObserverFault::Scope); };
                if helper.ordinal != ordinal || !helper.result { return Err(ObserverFault::Scope); }
                core.event(state.scope(), 41, &[ordinal, usize::from(helper.success)])?;
                state.last_helper = Some(helper);
            });
        }
        let restored = STATE.with(|cell| {
            let mut state = cell.try_borrow_mut().map_err(|_| ObserverFault::Lock)?;
            state.helper = self.previous;
            Ok::<(), ObserverFault>(())
        });
        if let Err(error) = restored { fault(error); }
    }
}

pub(in crate::scalar) fn mapped_projected_error() {
    observe!(state, core, {
        if state.call.is_none() { return Ok(()); }
        let Some(helper) = state.last_helper else { return Err(ObserverFault::Scope); };
        let Some(predicate) = helper.rejected else { return Err(ObserverFault::Scope); };
        if !helper.result || helper.success || state.mapped.is_some() { return Err(ObserverFault::Scope); }
        state.mapped = Some((helper.ordinal, predicate));
        core.event(state.scope(), 42, &[helper.ordinal, predicate])?;
    });
}
}
pub(super) mod read_choice;
mod parameter_input;
mod backing_producer;
#[cfg(test)]
mod backing_producer_tests;
mod checked;
pub(super) mod local_loan;
#[cfg(test)]
mod local_loan_tests;
pub(super) mod source_use;
#[cfg(test)]
mod source_use_tests;
mod validation;
use checked::checked_source;
mod owner_consumer;
mod owner_receiving;
#[cfg(test)]
mod owner_receiving_tests;
pub(super) mod port_monitor;
#[cfg(test)]
mod runtime_proof_negative_tests;
mod selected_grant;
#[cfg(test)]
mod selected_grant_tests;
mod selected_move;
#[cfg(test)]
mod selected_move_tests;
pub(super) use validation::validate;
#[cfg(test)]
mod owner_consumer_tests;
#[cfg(test)]
mod port_monitor_tests;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SelectedSourceState {
    ObservationOnly,
    Active {
        configuration: SourceConfigurationRef,
        contributor: JointProducerTupleRef,
        predecessor: SourceEventId,
        monitor: JointPortMonitorRef,
    },
}

pub(super) struct SelectedSnapshotInput {
    pub selection: FrozenSelectionOccurrence,
    pub place: ReferencePlaceId,
}

#[cfg_attr(test, track_caller)]
fn invalid(source: &SourceSpan) -> ReferenceAnalysisError {
    let error = ReferenceAnalysisError::MissingContext {
        source_span: source.clone(),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    };
    #[cfg(test)]
    snapshot23_observation::constructed(0, std::panic::Location::caller(), &error);
    error
}

impl reference::ReferenceFlowBuilder {
    pub(super) fn register_source_binding(
        &mut self,
        binding: &ReferenceBindingId,
        ty: &ScalarType,
    ) -> Result<(), ReferenceAnalysisError> {
        let source = SourceSpan::new(binding.source.clone(), binding.declaration_span);
        let shape = intern_value_shape(
            &mut self.cfg.evaluated_values.shapes,
            &self.cfg.producer_owner,
            ty,
            &self.structs,
            &self.enums,
            &self.generic_scope,
            &self.generic_declarations,
        )
        .map_err(|_| invalid(&source))?;
        if let Some(previous) = self
            .cfg
            .evaluated_values
            .source_binding_shapes
            .iter()
            .find(|row| row.binding == *binding)
        {
            if previous.shape != shape {
                return Err(invalid(&source));
            }
        } else {
            self.cfg
                .evaluated_values
                .source_binding_shapes
                .push(SourceBindingShapeFact {
                    binding: binding.clone(),
                    shape,
                });
        }
        Ok(())
    }
}

pub(super) fn register_literal_members(
    cfg: &mut ScalarReferenceCfg,
    structs: &[ScalarStruct],
    generic_scope: &ProducerCfgOwnerId,
    generic_declarations: &[ScalarGenericParameter],
) -> Result<(), ReferenceAnalysisError> {
    backing_producer::register(cfg)?;
    intern_value_shape(
        &mut cfg.evaluated_values.shapes,
        &cfg.producer_owner,
        &ScalarType::Unit,
        structs,
        &[],
        generic_scope,
        generic_declarations,
    )
    .map_err(|_| invalid(&cfg.points[0].source_span))?;
    if matches!(cfg.producer_owner, ProducerCfgOwnerId::Module(_))
        && !cfg
            .evaluated_values
            .boundary_signatures
            .iter()
            .any(|row| row.owner == cfg.producer_owner)
    {
        let entries = cfg
            .points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::Entry))
            .collect::<Vec<_>>();
        let [entry] = entries.as_slice() else {
            return Err(invalid(&cfg.points[0].source_span));
        };
        cfg.evaluated_values
            .boundary_signatures
            .push(BoundarySignatureFact {
                owner: cfg.producer_owner.clone(),
                entry: SourceEventId {
                    owner: cfg.producer_owner.clone(),
                    point: entry.id,
                },
                frame: FrameVariableRef {
                    owner: cfg.producer_owner.clone(),
                    variable: SupportedInstanceVariable(0),
                },
                ports: Vec::new(),
            });
    }
    let literals = cfg
        .evaluated_values
        .values
        .iter()
        .filter(|value| {
            matches!(
                &cfg.initial_points[value.id.point.0].kind,
                CfgPointKind::Evaluate(
                    ScalarExpression::StructLiteral { .. } | ScalarExpression::ArrayLiteral { .. }
                )
            )
        })
        .map(|value| value.id.clone())
        .collect::<Vec<_>>();
    for value in literals {
        let members = evaluated_aggregate_member::literal_aggregate_members(cfg, structs, &value)?;
        let fact = SourceLiteralMembersFact { value, members };
        if !cfg.evaluated_values.source_literal_members.contains(&fact) {
            cfg.evaluated_values.source_literal_members.push(fact);
        }
    }
    Ok(())
}

use source_boundary::coordinate::intern_coordinate as coordinate;

fn intern_view(
    tables: &mut EvaluatedValueTables,
    owner: &ProducerCfgOwnerId,
    shape: ValueShapeId,
    payload: SourcePayloadView,
    dependencies: Vec<EvaluatedDependencyFact>,
) -> SourceValueViewRef {
    if let Some(row) = tables.source_views.iter().find(|row| {
        row.id.owner == *owner
            && row.shape == shape
            && row.payload == payload
            && row.dependencies == dependencies
    }) {
        return row.id.clone();
    }
    let id = SourceValueViewRef {
        owner: owner.clone(),
        index: tables.source_views.len(),
    };
    tables.source_views.push(SourceValueViewFact {
        id: id.clone(),
        shape,
        payload,
        dependencies,
    });
    id
}

fn configuration(
    tables: &mut EvaluatedValueTables,
    mut row: SourceConfigurationFact,
) -> SourceConfigurationRef {
    row.slots.sort_by_key(|slot| slot.coordinate.index);
    if let Some(previous) = tables.source_configurations.iter().find(|previous| {
        let mut candidate = row.clone();
        candidate.id = previous.id.clone();
        candidate == **previous
    }) {
        return previous.id.clone();
    }
    row.id.index = tables.source_configurations.len();
    let id = row.id.clone();
    tables.source_configurations.push(row);
    id
}

fn contributor(
    tables: &mut EvaluatedValueTables,
    mut row: JointProducerTupleFact,
) -> JointProducerTupleRef {
    if let Some(previous) = tables.source_contributors.iter().find(|previous| {
        let mut candidate = row.clone();
        candidate.id = previous.id.clone();
        candidate == **previous
    }) {
        return previous.id.clone();
    }
    row.id.index = tables.source_contributors.len();
    let id = row.id.clone();
    tables.source_contributors.push(row);
    id
}

fn install(
    configuration: &mut SourceConfigurationFact,
    coordinate: SourceSlotRef,
    view: SourceValueViewRef,
    availability: ValueAvailability,
) {
    configuration
        .slots
        .retain(|slot| slot.coordinate != coordinate);
    configuration.slots.push(SourceSelectedSlotFact {
        coordinate,
        view,
        availability,
    });
}

fn selected_slot<'a>(
    tables: &EvaluatedValueTables,
    configuration: &'a SourceConfigurationFact,
    role: &SourceCoordinateRole,
) -> Result<&'a SourceSelectedSlotFact, ()> {
    let coordinates = tables
        .source_slots
        .iter()
        .filter(|row| &row.role == role && row.id.owner == configuration.id.owner)
        .collect::<Vec<_>>();
    #[cfg(test)]
    snapshot23_observation::coordinates(configuration, role, coordinates.len());
    let [coordinate] = coordinates.as_slice() else {
        #[cfg(test)]
        snapshot23_observation::rejected(1);
        return Err(());
    };
    let slots = configuration
        .slots
        .iter()
        .filter(|slot| slot.coordinate == coordinate.id)
        .collect::<Vec<_>>();
    #[cfg(test)]
    snapshot23_observation::slots(coordinate, slots.len());
    let [slot] = slots.as_slice() else {
        #[cfg(test)]
        snapshot23_observation::rejected(2);
        return Err(());
    };
    Ok(slot)
}

pub(super) fn root(
    cfg: &mut ScalarReferenceCfg,
) -> Result<SelectedSourceState, ReferenceAnalysisError> {
    let signatures = cfg
        .evaluated_values
        .boundary_signatures
        .iter()
        .filter(|signature| signature.owner == cfg.producer_owner)
        .collect::<Vec<_>>();
    let [signature] = signatures.as_slice() else {
        if signatures.is_empty() && cfg.top_level.points.is_empty() {
            return Ok(SelectedSourceState::ObservationOnly);
        }
        return Err(invalid(&cfg.points[0].source_span));
    };
    let signature = (*signature).clone();
    let owner = cfg.producer_owner.clone();
    let mut row = SourceConfigurationFact {
        id: SourceConfigurationRef {
            owner: owner.clone(),
            index: 0,
        },
        frame: signature.frame.clone(),
        slots: Vec::new(),
        instances: Vec::new(),
        created: Vec::new(),
        owns: Vec::new(),
        depends: Vec::new(),
        has_target: Vec::new(),
        functional: Vec::new(),
        same_target: Vec::new(),
        different_target: Vec::new(),
        formal_inputs: Vec::new(),
        pending: Vec::new(),
        saved: Vec::new(),
        owner_views: Vec::new(),
        grants: Vec::new(),
        grant_ancestry: Vec::new(),
    };
    for (index, port) in signature.ports.iter().enumerate() {
        if matches!(
            port.role,
            BoundaryPortRole::Output(_) | BoundaryPortRole::Effect { .. }
        ) {
            continue;
        }
        let formal = BoundaryFormalPortRef {
            owner: owner.clone(),
            port: index,
        };
        let shape = cfg
            .evaluated_values
            .shapes
            .iter()
            .find(|shape| shape.id == port.shape)
            .ok_or_else(|| invalid(&cfg.points[0].source_span))?;
        let payload = if matches!(shape.payload, ValuePayloadShape::Plain(_)) {
            SourcePayloadView::Plain
        } else {
            SourcePayloadView::Formal(source_boundary::formal_value::root(&cfg.evaluated_values, &formal, &port.shape)
                .map_err(|_| invalid(&cfg.points[0].source_span))?.id.clone())
        };
        let view = intern_view(
            &mut cfg.evaluated_values,
            &owner,
            port.shape.clone(),
            payload,
            Vec::new(),
        );
        install(
            &mut row,
            port.coordinate.clone(),
            view.clone(),
            ValueAvailability::Available,
        );
        let binding = match &port.role {
            BoundaryPortRole::Parameter { binding, .. }
            | BoundaryPortRole::Capture { binding, .. }
            | BoundaryPortRole::Global(binding) => binding,
            _ => unreachable!(),
        };
        let binding_coordinate = coordinate(
            &mut cfg.evaluated_values,
            &owner,
            SourceCoordinateRole::Binding(binding.clone()),
            port.shape.clone(),
            &cfg.points[0].source_span,
        )?;
        install(
            &mut row,
            binding_coordinate,
            view,
            ValueAvailability::Available,
        );
        row.formal_inputs.push(SourceFormalInputFact {
            port: formal,
            coordinate: port.coordinate.clone(),
            shape: port.shape.clone(),
        });
    }
    let input = configuration(&mut cfg.evaluated_values, row);
    let tuple = contributor(
        &mut cfg.evaluated_values,
        JointProducerTupleFact {
            id: JointProducerTupleRef {
                owner: owner.clone(),
                index: 0,
            },
            event: signature.entry.clone(),
            input: input.clone(),
            output: input.clone(),
            kind: JointProducerRuleKind::Root,
            children: Vec::new(),
            preserved: Vec::new(),
            fresh: Vec::new(),
            outputs: Vec::new(),
            transports: Vec::new(),
            productions: Vec::new(),
        },
    );
    Ok(SelectedSourceState::Active {
        monitor: port_monitor::root(&mut cfg.evaluated_values, &tuple),
        configuration: input,
        contributor: tuple,
        predecessor: signature.entry,
    })
}

fn fresh(
    tables: &mut EvaluatedValueTables,
    row: &mut SourceConfigurationFact,
    support: SourceSlotRef,
    shape: ValueShapeId,
    role: SourceObjectRole,
    event: SourceEventId,
) -> SourceInstanceRef {
    let used = row
        .instances
        .iter()
        .filter_map(|instance| match instance {
            SourceInstanceRef::Supported(id) => Some(tables.source_instances[id.index].variable.0),
            SourceInstanceRef::Residual(_) => None,
        })
        .collect::<BTreeSet<_>>();
    let variable = (0..)
        .find(|variable| !used.contains(variable))
        .expect("vacant finite source support variable");
    let candidate = SourceSupportedClassFact {
        id: SourceSupportedClassRef {
            owner: row.id.owner.clone(),
            index: 0,
        },
        variable: SupportedInstanceVariable(variable),
        frame: row.frame.clone(),
        support,
        shape: shape.clone(),
        role: role.clone(),
        origin: SourceInstanceOrigin::Creation(event.clone()),
    };
    let id = if let Some(previous) = tables.source_instances.iter().find(|previous| {
        let mut compared = candidate.clone();
        compared.id = previous.id.clone();
        compared == **previous
    }) {
        previous.id.clone()
    } else {
        let mut candidate = candidate;
        candidate.id.index = tables.source_instances.len();
        let id = candidate.id.clone();
        tables.source_instances.push(candidate);
        id
    };
    let instance = SourceInstanceRef::Supported(id);
    row.instances.push(instance.clone());
    row.created.push(SourceCreatedAtFact {
        instance: instance.clone(),
        event,
        shape,
        role,
    });
    instance
}

fn current_owner(
    tables: &EvaluatedValueTables,
    row: &SourceConfigurationFact,
    capability: &OwnerCapabilityId,
) -> Result<SourceInstanceRef, ()> {
    let instances = row
        .instances
        .iter()
        .filter(|instance| match instance {
            SourceInstanceRef::Supported(id) => {
                tables.source_instances[id.index].role
                    == SourceObjectRole::Owner(capability.clone())
            }
            SourceInstanceRef::Residual(_) => false,
        })
        .collect::<Vec<_>>();
    let [instance] = instances.as_slice() else {
        return Err(());
    };
    Ok((*instance).clone())
}

fn projected_view(
    tables: &EvaluatedValueTables,
    row: &SourceConfigurationFact,
    place: &ReferencePlaceId,
) -> Result<SourceValueViewRef, ()> {
    #[cfg(test)]
    let _snapshot23_helper = snapshot23_observation::projected(row, place);
    let slot = selected_slot(
        tables,
        row,
        &SourceCoordinateRole::Binding(place.binding.clone()),
    ).map_err(|()| {
        #[cfg(test)]
        snapshot23_observation::unit_result(false);
        ()
    })?;
    #[cfg(test)]
    snapshot23_observation::slot(slot);
    if slot.availability != ValueAvailability::Available {
        #[cfg(test)]
        {
            snapshot23_observation::rejected(3);
            snapshot23_observation::unit_result(false);
        }
        return Err(());
    }
    let mut view = slot.view.clone();
    for (_projection_ordinal, projection) in place.projections.iter().enumerate() {
        #[cfg(test)]
        snapshot23_observation::step(tables, &view, projection, _projection_ordinal);
        let SourcePayloadView::Aggregate(members) = &tables.source_views[view.index].payload else {
            #[cfg(test)]
            {
                snapshot23_observation::rejected(4);
                snapshot23_observation::unit_result(false);
            }
            return Err(());
        };
        let selections = members
            .iter()
            .filter(|member| match (projection, member.projection.as_slice()) {
                (
                    ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(field)),
                    [ValueProjection::Field(actual)],
                ) => field == actual,
                (
                    ReferencePlaceProjection::Index(ScalarExpression::Integer { value, .. }),
                    [ValueProjection::FixedIndex(index)],
                ) => value == &BigInt::from(*index),
                _ => false,
            })
            .collect::<Vec<_>>();
        #[cfg(test)]
        snapshot23_observation::members(members.len(), &selections);
        let [selection] = selections.as_slice() else {
            #[cfg(test)]
            {
                snapshot23_observation::rejected(5);
                snapshot23_observation::unit_result(false);
            }
            return Err(());
        };
        view = selection.view.clone();
    }
    #[cfg(test)]
    snapshot23_observation::unit_result(true);
    Ok(view)
}

fn current_cell(
    tables: &EvaluatedValueTables,
    row: &SourceConfigurationFact,
    slot: &SourceSlotRef,
) -> Result<SourceInstanceRef, ()> {
    let cells = row
        .instances
        .iter()
        .filter(|instance| match instance {
            SourceInstanceRef::Supported(id) => {
                let class = &tables.source_instances[id.index];
                class.role == SourceObjectRole::BindingCell && class.support == *slot
            }
            SourceInstanceRef::Residual(_) => false,
        })
        .collect::<Vec<_>>();
    let [cell] = cells.as_slice() else {
        return Err(());
    };
    Ok((*cell).clone())
}

fn install_cell_dependency(
    row: &mut SourceConfigurationFact,
    cell: SourceInstanceRef,
    payload: &SourcePayloadView,
) {
    row.depends
        .retain(|edge| edge.source != cell || edge.label != SourceDependencyLabel::CellContent);
    row.functional
        .retain(|edge| edge.source != cell || edge.label != SourceDependencyLabel::CellContent);
    let target = match payload {
        SourcePayloadView::Raw { referent, .. } => referent,
        SourcePayloadView::Backing { instance, .. } => instance,
        _ => return,
    };
    row.depends.push(SourceDependsFact {
        source: cell.clone(),
        label: SourceDependencyLabel::CellContent,
        target: target.clone(),
        truth: SourcePairTruth::AllPairs,
    });
    row.functional.push(SourceFunctionalFact {
        source: cell,
        label: SourceDependencyLabel::CellContent,
        truth: SourceFunctionalTruth::EverySourceOne,
    });
}

fn dominates(points: &[ReferenceFlowPoint], header: CfgPointId, predecessor: CfgPointId) -> bool {
    let mut pending = vec![CfgPointId(0)];
    let mut visited = BTreeSet::new();
    while let Some(point) = pending.pop() {
        if point == header {
            continue;
        }
        if point == predecessor {
            return false;
        }
        if visited.insert(point) {
            pending.extend(&points[point.0].successors);
        }
    }
    true
}

fn rule_kind(
    points: &[ReferenceFlowPoint],
    predecessor: &SourceEventId,
    point: &ReferenceFlowPoint,
) -> Result<JointProducerRuleKind, ReferenceAnalysisError> {
    if predecessor.point == point.id && matches!(point.kind, CfgPointKind::Entry) {
        return Ok(JointProducerRuleKind::Root);
    }
    let previous = points
        .get(predecessor.point.0)
        .ok_or_else(|| invalid(&point.source_span))?;
    if !previous.successors.contains(&point.id) {
        return Err(invalid(&point.source_span));
    }
    if matches!(point.kind, CfgPointKind::ReturnOutput { .. }) {
        return Ok(JointProducerRuleKind::Return);
    }
    if dominates(points, point.id, predecessor.point) {
        return Ok(JointProducerRuleKind::Backedge {
            predecessor: predecessor.clone(),
        });
    }
    if matches!(
        previous.kind,
        CfgPointKind::Branch { .. } | CfgPointKind::LoopTest { .. }
    ) {
        return Ok(JointProducerRuleKind::Branch {
            predecessor: predecessor.clone(),
        });
    }
    Ok(JointProducerRuleKind::Step)
}

pub(super) fn transfer(
    tables: &mut EvaluatedValueTables,
    owner: &ProducerCfgOwnerId,
    original: &[ReferenceFlowPoint],
    point: &ReferenceFlowPoint,
    incoming: &SelectedSourceState,
    backings: &[RuntimeBackingFact],
    checked: &Option<BTreeSet<ReferenceOriginId>>,
    origins: &[ReferenceOrigin],
    frozen: &[SelectedSnapshotInput],
) -> Result<Vec<SelectedSourceState>, ReferenceAnalysisError> {
    let SelectedSourceState::Active {
        configuration: input,
        predecessor,
        monitor: input_monitor,
        ..
    } = incoming
    else {
        return Ok(vec![SelectedSourceState::ObservationOnly]);
    };
    let mut row = tables
        .source_configurations
        .get(input.index)
        .filter(|row| row.id == *input)
        .ok_or_else(|| invalid(&point.source_span))?
        .clone();
    let event = SourceEventId {
        owner: owner.clone(),
        point: point.id,
    };
    let before = row.instances.clone();
    let input_row = row.clone();
    let move_selection = selected_move::prepare(
        tables,
        original,
        &input_row,
        &mut row,
        point,
        &event,
        input_monitor,
    )?;
    let mut transports = Vec::new();
    let mut production_inputs = Vec::new();
    let mut backing_ports = Vec::new();
    let mut result_transport = None;
    let mut monitor_complete = true;
    let outputs = tables
        .values
        .iter()
        .filter(|value| value.id.point == point.id)
        .map(|value| (value.id.clone(), value.shape.clone()))
        .collect::<Vec<_>>();
    let mut result = None;
    let mut grant_creation = None;
    let introductions = tables
        .owner_capabilities
        .iter()
        .filter(|capability| capability.introduction == event)
        .cloned()
        .collect::<Vec<_>>();
    for capability in introductions {
        let shapes = tables
            .shapes
            .iter()
            .filter(|shape| {
                matches!(
                    shape.payload,
                    ValuePayloadShape::Plain(PlainValueKind::Unit)
                )
            })
            .collect::<Vec<_>>();
        let [shape] = shapes.as_slice() else {
            return Err(invalid(&point.source_span));
        };
        let shape = shape.id.clone();
        let slot = coordinate(
            tables,
            owner,
            SourceCoordinateRole::Owner(capability.id.clone()),
            shape.clone(),
            &point.source_span,
        )?;
        fresh(
            tables,
            &mut row,
            slot,
            shape,
            SourceObjectRole::Owner(capability.id),
            event.clone(),
        );
    }
    match &point.kind {
        CfgPointKind::Evaluate(expression) => {
            if let [(value, shape)] = outputs.as_slice() {
                let schema = tables
                    .shapes
                    .iter()
                    .find(|row| row.id == *shape)
                    .ok_or_else(|| invalid(&point.source_span))?
                    .clone();
                if matches!(schema.payload, ValuePayloadShape::Plain(_)) {
                    result = Some(intern_view(
                        tables,
                        owner,
                        shape.clone(),
                        SourcePayloadView::Plain,
                        Vec::new(),
                    ));
                } else if matches!(expression, ScalarExpression::Name { name, .. } if name == "null")
                    && matches!(schema.payload, ValuePayloadShape::Raw { .. })
                {
                    result = Some(intern_view(
                        tables,
                        owner,
                        shape.clone(),
                        SourcePayloadView::StaticRaw(point.source_span.clone()),
                        Vec::new(),
                    ));
                } else if let ScalarExpression::RawAddress { .. } = expression {
                    let productions = tables
                        .raw_address_productions
                        .iter()
                        .filter(|production| production.value == *value)
                        .cloned()
                        .collect::<Vec<_>>();
                    let [production] = productions.as_slice() else {
                        return Err(invalid(&point.source_span));
                    };
                    match &production.source {
                        RawAddressInputSource::CompilerStorage => {
                            let inputs = tables
                                .raw_address_inputs
                                .iter()
                                .filter(|input| input.value == *value)
                                .cloned()
                                .collect::<Vec<_>>();
                            let [input] = inputs.as_slice() else {
                                return Err(invalid(&point.source_span));
                            };
                            let selected = selected_slot(
                                tables,
                                &row,
                                &SourceCoordinateRole::Binding(input.place.binding.clone()),
                            )
                            .map_err(|_| invalid(&point.source_span))?
                            .clone();
                            let view = tables.source_views[selected.view.index].clone();
                            let referent = match view.payload {
                                SourcePayloadView::Backing { instance, .. } => instance,
                                _ => {
                                    let classes = row
                                        .created
                                        .iter()
                                        .filter(|created| {
                                            created.role == SourceObjectRole::BindingCell
                                                && match &created.instance {
                                                    SourceInstanceRef::Supported(id) => {
                                                        tables.source_instances[id.index].support
                                                            == selected.coordinate
                                                    }
                                                    SourceInstanceRef::Residual(_) => false,
                                                }
                                        })
                                        .collect::<Vec<_>>();
                                    let [class] = classes.as_slice() else {
                                        return Err(invalid(&point.source_span));
                                    };
                                    class.instance.clone()
                                }
                            };
                            let obligations = row
                                .owns
                                .iter()
                                .filter(|owns| {
                                    owns.payload == referent
                                        && owns.state == SourceObligationState::Outstanding
                                })
                                .collect::<Vec<_>>();
                            let mut selected_owners = Vec::new();
                            for obligation in &obligations {
                                let selected = (
                                    obligation.capability.clone(),
                                    obligation.owner_instance.clone(),
                                    obligation.kind.clone(),
                                );
                                if !selected_owners.contains(&selected) {
                                    selected_owners.push(selected);
                                }
                            }
                            let [(source_owner, owner_instance, kind)] = selected_owners.as_slice()
                            else {
                                return Err(invalid(&point.source_span));
                            };
                            let capability = tables
                                .owner_capabilities
                                .iter()
                                .find(|capability| capability.id == *source_owner)
                                .ok_or_else(|| invalid(&point.source_span))?;
                            if !capability.endpoints.iter().all(|endpoint| {
                                obligations.iter().any(|obligation| {
                                    obligation.owner_instance == *owner_instance
                                        && obligation.endpoint == *endpoint
                                })
                            }) {
                                return Err(invalid(&point.source_span));
                            }
                            let scopes = tables
                                .owner_capabilities
                                .iter()
                                .filter(|capability| {
                                    matches!(capability.role, OwnerRole::Scope(_))
                                        && original[capability.introduction.point.0].scope
                                            == point.scope
                                })
                                .collect::<Vec<_>>();
                            let [scope] = scopes.as_slice() else {
                                return Err(invalid(&point.source_span));
                            };
                            let created = row
                                .created
                                .iter()
                                .find(|created| created.instance == referent)
                                .ok_or_else(|| invalid(&point.source_span))?;
                            let mut dependencies = view.dependencies.clone();
                            dependencies.push(EvaluatedDependencyFact {
                                projection: Vec::new(),
                                source: if created.role == SourceObjectRole::BindingCell {
                                    DependencySource::BindingCell(input.place.binding.clone())
                                } else {
                                    DependencySource::Allocation(OwnerRecordFamilyId {
                                        producer: owner.clone(),
                                        creation: created.event.point,
                                        projection: Vec::new(),
                                    })
                                },
                                action: match kind {
                                    RuntimeBackingKind::FixedValue => {
                                        DependencyAction::BorrowFixed {
                                            referent_lifetime: FixedLifetime::Capability(
                                                source_owner.clone(),
                                            ),
                                            use_owner: scope.id.clone(),
                                        }
                                    }
                                    RuntimeBackingKind::OwnedAllocation => {
                                        DependencyAction::TransferUnique {
                                            source_owner: source_owner.clone(),
                                            receiving_owner: scope.id.clone(),
                                        }
                                    }
                                },
                                witness: point.source_span.clone(),
                            });
                            let provenance = RawValueProvenance::CompilerStorage {
                                source_place: input.place.clone(),
                                creation: created.event.point,
                            };
                            result = Some(intern_view(
                                tables,
                                owner,
                                shape.clone(),
                                SourcePayloadView::Raw {
                                    provenance,
                                    referent,
                                },
                                dependencies,
                            ));
                        }
                        RawAddressInputSource::ThroughPointer { pointer, .. } => {
                            let selected = selected_slot(
                                tables,
                                &row,
                                &SourceCoordinateRole::Evaluated(pointer.clone()),
                            )
                            .map_err(|_| invalid(&point.source_span))?;
                            let view = tables.source_views[selected.view.index].clone();
                            if let SourcePayloadView::Raw {
                                provenance,
                                referent,
                            } = view.payload
                            {
                                production_inputs.push(selected.coordinate.clone());
                                result = Some(intern_view(
                                    tables,
                                    owner,
                                    shape.clone(),
                                    SourcePayloadView::Raw {
                                        provenance,
                                        referent,
                                    },
                                    view.dependencies,
                                ));
                            }
                        }
                    }
                } else if matches!(
                    expression,
                    ScalarExpression::StructLiteral { .. } | ScalarExpression::ArrayLiteral { .. }
                ) {
                    let members = tables
                        .source_literal_members
                        .iter()
                        .find(|members| members.value == *value)
                        .ok_or_else(|| invalid(&point.source_span))?
                        .members
                        .clone();
                    let mut selected = Vec::new();
                    let mut dependencies = Vec::new();
                    let mut complete = true;
                    for member in members {
                        match selected_slot(
                            tables,
                            &row,
                            &SourceCoordinateRole::Evaluated(member.value.clone()),
                        ) {
                            Ok(slot) => {
                                production_inputs.push(slot.coordinate.clone());
                                let view = &tables.source_views[slot.view.index];
                                if matches!(view.payload, SourcePayloadView::Formal(_)) {
                                    complete = false;
                                }
                                for dependency in &view.dependencies {
                                    let mut dependency = dependency.clone();
                                    let mut projection = member.projection.clone();
                                    projection.extend(dependency.projection);
                                    dependency.projection = projection;
                                    dependencies.push(dependency);
                                }
                                selected.push(SourceMemberView {
                                    projection: member.projection,
                                    source: member.value,
                                    view: slot.view.clone(),
                                });
                            }
                            Err(()) => complete = false,
                        }
                    }
                    if complete {
                        result = Some(intern_view(
                            tables,
                            owner,
                            shape.clone(),
                            SourcePayloadView::Aggregate(selected),
                            dependencies,
                        ));
                    }
                }
            }
        }
        CfgPointKind::Read { binding } => {
            if let Ok(selected) = selected_slot(
                tables,
                &row,
                &SourceCoordinateRole::Binding(binding.clone()),
            ) {
                if selected.availability == ValueAvailability::Available {
                    result_transport =
                        Some((selected.coordinate.clone(), JointValueTransportKind::Copy));
                    production_inputs.push(selected.coordinate.clone());
                    result = Some(selected.view.clone());
                }
            }
        }
        CfgPointKind::AssignmentSnapshot { holder, .. } => {
            for selection in frozen {
                let view = projected_view(tables, &row, &selection.place)
                    .map_err(|_| {
                        #[cfg(test)]
                        snapshot23_observation::mapped_projected_error();
                        invalid(&point.source_span)
                    })?;
                let slot = selected_slot(
                    tables,
                    &row,
                    &SourceCoordinateRole::Binding(selection.place.binding.clone()),
                )
                .map_err(|_| invalid(&point.source_span))?;
                let source_port = slot.coordinate.clone();
                let availability = slot.availability.clone();
                let saved_coordinate = coordinate(
                    tables,
                    owner,
                    SourceCoordinateRole::Saved(selection.selection.clone()),
                    tables.source_views[view.index].shape.clone(),
                    &point.source_span,
                )?;
                match port_monitor::place_transport(tables, &input_row, &selection.place) {
                    Ok(kind) => {
                        result_transport = Some((source_port.clone(), kind.clone()));
                        transports.push(JointValueTransportFact {
                            source: source_port.clone(),
                            destination: saved_coordinate.clone(),
                            kind: match kind {
                                JointValueTransportKind::Copy => JointValueTransportKind::Snapshot,
                                project => project,
                            },
                            child: JointValueChildPort::LocalInput,
                        });
                    }
                    Err(()) => monitor_complete = false,
                }
                let saved = SourceSavedViewFact {
                    coordinate: saved_coordinate.clone(),
                    snapshot: event.clone(),
                    selection: selection.selection.clone(),
                    holder: holder.clone(),
                    view: view.clone(),
                    availability: availability.clone(),
                };
                row.saved
                    .retain(|old| old.snapshot != event || old.selection != saved.selection);
                row.saved.push(saved);
                install(&mut row, saved_coordinate, view.clone(), availability);
                production_inputs.push(source_port.clone());
                result = Some(view);
            }
        }
        CfgPointKind::AssignmentResult { snapshot_point, .. } => {
            let saved = row
                .saved
                .iter()
                .filter(|saved| saved.snapshot.point == *snapshot_point)
                .collect::<Vec<_>>();
            let [saved] = saved.as_slice() else {
                return Err(invalid(&point.source_span));
            };
            result = Some(saved.view.clone());
            production_inputs.push(saved.coordinate.clone());
            result_transport = Some((saved.coordinate.clone(), JointValueTransportKind::Result));
        }
        CfgPointKind::AssignmentDispose { snapshot_point } => {
            let ended = row
                .saved
                .iter()
                .filter(|saved| saved.snapshot.point == *snapshot_point)
                .map(|saved| saved.coordinate.clone())
                .collect::<Vec<_>>();
            row.slots.retain(|slot| !ended.contains(&slot.coordinate));
            row.saved
                .retain(|saved| saved.snapshot.point != *snapshot_point);
        }
        CfgPointKind::ReadPlace {
            candidates,
            pointer_point: None,
            ..
        } => {
            if let [place] = candidates.as_slice() {
                if let Ok(view) = projected_view(tables, &row, place) {
                    let source = selected_slot(
                        tables,
                        &input_row,
                        &SourceCoordinateRole::Binding(place.binding.clone()),
                    )
                    .map_err(|_| invalid(&point.source_span))?
                    .coordinate
                    .clone();
                    production_inputs.push(source.clone());
                    match port_monitor::place_transport(tables, &input_row, place) {
                        Ok(kind) => result_transport = Some((source, kind)),
                        Err(()) => monitor_complete = false,
                    }
                    result = Some(view);
                }
            }
        }
        CfgPointKind::Bind { binding } if tables.owner_capabilities.iter().any(|owner|
            owner.role == OwnerRole::Parameter(binding.clone()) && owner.introduction == event) => {
            transports.push(parameter_input::preserve(tables, &row, point)?);
        }
        CfgPointKind::Bind { binding } => {
            let shapes = tables
                .source_binding_shapes
                .iter()
                .filter(|shape| shape.binding == *binding)
                .cloned()
                .collect::<Vec<_>>();
            if let [schema] = shapes.as_slice() {
                let slot = coordinate(
                    tables,
                    owner,
                    SourceCoordinateRole::Binding(binding.clone()),
                    schema.shape.clone(),
                    &point.source_span,
                )?;
                let capabilities = tables.owner_capabilities.iter().filter(|capability| matches!(&capability.role, OwnerRole::Binding(actual) | OwnerRole::Parameter(actual) if actual == binding)).cloned().collect::<Vec<_>>();
                if let [capability] = capabilities.as_slice() {
                    let owner_instance = current_owner(tables, &row, &capability.id)
                        .map_err(|_| invalid(&point.source_span))?;
                    let instance = fresh(
                        tables,
                        &mut row,
                        slot.clone(),
                        schema.shape.clone(),
                        SourceObjectRole::BindingCell,
                        event.clone(),
                    );
                    for endpoint in &capability.endpoints {
                        row.owns.push(SourceOwnsFact {
                            capability: capability.id.clone(),
                            owner_instance: owner_instance.clone(),
                            payload: instance.clone(),
                            endpoint: endpoint.clone(),
                            kind: RuntimeBackingKind::FixedValue,
                            state: SourceObligationState::Outstanding,
                        });
                    }
                }
                let source = tables
                    .binding_inputs
                    .iter()
                    .find(|input| input.consumer == point.id)
                    .cloned();
                if let Some(input) = source {
                    let policy = reference::binding_disposition(tables, &input, binding)?.policy;
                    let kind = match policy {
                        CopyPolicy::Copy => Some(JointValueTransportKind::Copy),
                        CopyPolicy::Move => Some(JointValueTransportKind::Move),
                        CopyPolicy::Deferred => { monitor_complete = false; None }
                    };
                    let source = input.source;
                    if let Some(kind) = kind {
                    if let Ok(selected) =
                        selected_slot(tables, &row, &SourceCoordinateRole::Evaluated(source))
                    {
                        let selected = selected.clone();
                        if tables.source_views[selected.view.index].shape == schema.shape {
                            let cell = current_cell(tables, &row, &slot)
                                .map_err(|_| invalid(&point.source_span))?;
                            install_cell_dependency(
                                &mut row,
                                cell,
                                &tables.source_views[selected.view.index].payload,
                            );
                            transports.push(JointValueTransportFact {
                                source: selected.coordinate.clone(),
                                destination: slot.clone(),
                                kind,
                                child: JointValueChildPort::LocalInput,
                            });
                            install(&mut row, slot.clone(), selected.view, selected.availability);
                        }
                    }
                    }
                }
                if !row.slots.iter().any(|selected| selected.coordinate == slot) {
                    let view = intern_view(
                        tables,
                        owner,
                        schema.shape.clone(),
                        SourcePayloadView::Unwritten,
                        Vec::new(),
                    );
                    install(
                        &mut row,
                        slot,
                        view,
                        ValueAvailability::Uninitialized {
                            source: point.source_span.clone(),
                        },
                    );
                }
            }
        }
        CfgPointKind::Backing {
            target,
            transfer: RuntimeBackingTransfer::Allocate | RuntimeBackingTransfer::CreateFixed,
            ..
        } => {
            let producer = backing_producer::resolve(tables, owner, point)?;
            let schemas = tables
                .source_binding_shapes
                .iter()
                .filter(|shape| shape.binding == *target)
                .cloned()
                .collect::<Vec<_>>();
            let [schema] = schemas.as_slice() else {
                return Err(invalid(&point.source_span));
            };
            if schema.shape != producer.shape {
                return Err(invalid(&point.source_span));
            }
            let shape = tables
                .shapes
                .iter()
                .find(|shape| shape.id == schema.shape)
                .ok_or_else(|| invalid(&point.source_span))?
                .clone();
            let element = match shape.payload {
                ValuePayloadShape::RuntimeArray { element }
                | ValuePayloadShape::FixedArray { element, .. } => element,
                _ => return Err(invalid(&point.source_span)),
            };
            let capabilities = tables
                .owner_capabilities
                .iter()
                .filter(|capability| capability.role == OwnerRole::Binding(target.clone()))
                .cloned()
                .collect::<Vec<_>>();
            let [capability] = capabilities.as_slice() else {
                return Err(invalid(&point.source_span));
            };
            let owner_instance = current_owner(tables, &row, &capability.id)
                .map_err(|_| invalid(&point.source_span))?;
            let slot = coordinate(
                tables,
                owner,
                SourceCoordinateRole::Binding(target.clone()),
                schema.shape.clone(),
                &point.source_span,
            )?;
            let instance = fresh(
                tables,
                &mut row,
                slot.clone(),
                schema.shape.clone(),
                SourceObjectRole::Payload,
                event.clone(),
            );
            let facts = point
                .possible_backings
                .iter()
                .filter(|backing| backing.owner == *target)
                .collect::<Vec<_>>();
            let [backing] = facts.as_slice() else {
                return Err(invalid(&point.source_span));
            };
            let dependencies = match backing.kind {
                RuntimeBackingKind::OwnedAllocation => {
                    let OwnerParent::Nested(storage_owner) = &capability.parent else {
                        return Err(invalid(&point.source_span));
                    };
                    current_owner(tables, &row, storage_owner)
                        .map_err(|_| invalid(&point.source_span))?;
                    vec![EvaluatedDependencyFact {
                        projection: Vec::new(),
                        source: DependencySource::Allocation(OwnerRecordFamilyId {
                            producer: producer.event.owner.clone(),
                            creation: producer.event.point,
                            projection: Vec::new(),
                        }),
                        action: DependencyAction::TransferUnique {
                            source_owner: capability.id.clone(),
                            receiving_owner: storage_owner.clone(),
                        },
                        witness: point.source_span.clone(),
                    }]
                }
                RuntimeBackingKind::FixedValue => Vec::new(),
            };
            let view = intern_view(
                tables,
                owner,
                schema.shape.clone(),
                SourcePayloadView::Backing {
                    instance: instance.clone(),
                    element,
                },
                dependencies,
            );
            let cell =
                current_cell(tables, &row, &slot).map_err(|_| invalid(&point.source_span))?;
            install_cell_dependency(&mut row, cell, &tables.source_views[view.index].payload);
            let observation = SourceBackingObservation {
                view: view.clone(),
                backing: (*backing).clone(),
            };
            if !tables.source_backing_observations.contains(&observation) {
                tables.source_backing_observations.push(observation);
            }
            for endpoint in &capability.endpoints {
                row.owns.push(SourceOwnsFact {
                    capability: capability.id.clone(),
                    owner_instance: owner_instance.clone(),
                    payload: instance.clone(),
                    endpoint: endpoint.clone(),
                    kind: backing.kind.clone(),
                    state: SourceObligationState::Outstanding,
                });
            }
            backing_ports.push(slot.clone());
            result = Some(view.clone());
            install(&mut row, slot, view, ValueAvailability::Available);
        }
        CfgPointKind::Assign {
            target, rhs_point, ..
        } if target.projections.is_empty() => {
            let values = tables
                .values
                .iter()
                .filter(|value| value.id.point == *rhs_point)
                .collect::<Vec<_>>();
            if let [value] = values.as_slice() {
                if let Ok(selected) = selected_slot(
                    tables,
                    &row,
                    &SourceCoordinateRole::Evaluated(value.id.clone()),
                ) {
                    let selected = selected.clone();
                    let slot = coordinate(
                        tables,
                        owner,
                        SourceCoordinateRole::Binding(target.binding.clone()),
                        value.shape.clone(),
                        &point.source_span,
                    )?;
                    let cell = current_cell(tables, &row, &slot)
                        .map_err(|_| invalid(&point.source_span))?;
                    install_cell_dependency(
                        &mut row,
                        cell,
                        &tables.source_views[selected.view.index].payload,
                    );
                    transports.push(JointValueTransportFact {
                        source: selected.coordinate.clone(),
                        destination: slot.clone(),
                        kind: JointValueTransportKind::Copy,
                        child: JointValueChildPort::LocalInput,
                    });
                    install(&mut row, slot, selected.view, selected.availability);
                }
            }
        }
        CfgPointKind::ScopeExit => {
            for obligation in &mut row.owns {
                if obligation.endpoint.event == event
                    && obligation.state == SourceObligationState::Outstanding
                {
                    obligation.state = SourceObligationState::Discharged;
                }
            }
            row.slots.retain(|slot| !matches!(&tables.source_slots[slot.coordinate.index].role, SourceCoordinateRole::Binding(binding) if binding.block_span == point.source_span.range));
        }
        _ => {}
    }
    let control_outputs = tables.productions.iter().filter(|production| {
        production.producer == *owner
            && matches!(production.source, OutputProductionSource::Conditional { .. } | OutputProductionSource::Block { .. })
    }).flat_map(|production| &production.outputs).cloned().collect::<Vec<_>>();
    if let [(value, _)] = outputs.as_slice() {
        if control_outputs.contains(value) {
            if let Ok(selected) = selected_slot(tables, &input_row, &SourceCoordinateRole::Evaluated(value.clone())) {
                let selected = selected.clone();
                production_inputs = vec![selected.coordinate.clone()];
                result_transport = Some((selected.coordinate, JointValueTransportKind::Copy));
                result = Some(selected.view);
            }
        }
    }
    for successor in &point.successors {
        let tuple = reference::conditional_output_production::edge_outputs(tables, owner, point.id, *successor)?;
        let selected = tuple.iter().map(|(source, destination)| {
            selected_slot(tables, &input_row, &SourceCoordinateRole::Evaluated(source.clone()))
                .map(|source| (source.clone(), destination.clone()))
        }).collect::<Result<Vec<_>, ()>>();
        if let Ok(selected) = selected {
            for (source, destination) in selected {
                let fact = tables.values.iter().find(|fact| fact.id == destination)
                    .ok_or_else(|| invalid(&point.source_span))?;
                let shape = fact.shape.clone();
                let destination = coordinate(tables, owner, SourceCoordinateRole::Evaluated(destination), shape, &point.source_span)?;
                transports.push(JointValueTransportFact {
                    source: source.coordinate,
                    destination: destination.clone(),
                    kind: JointValueTransportKind::Copy,
                    child: JointValueChildPort::LocalInput,
                });
                install(&mut row, destination, source.view, source.availability);
            }
        } else {
            let pending = SourcePendingProducerFact {
                event: SourceEventId { owner: owner.clone(), point: *successor },
                outputs: tuple.iter().map(|(_, destination)| destination.clone()).collect(),
            };
            if !row.pending.contains(&pending) {
                row.pending.push(pending);
            }
        }
    }
    if let [(value, shape)] = outputs.as_slice() {
        if matches!(
            tables
                .shapes
                .iter()
                .find(|schema| schema.id == *shape)
                .ok_or_else(|| invalid(&point.source_span))?
                .payload,
            ValuePayloadShape::Checked { .. }
        ) {
            if result.is_none() {
                result = checked_source(
                    tables, &row, owner, point, original, checked, origins, value, shape,
                )?;
            }
            if matches!(point.kind, CfgPointKind::Borrow { .. }) {
                let constructed = selected_grant::borrow(
                    tables,
                    &mut row,
                    point,
                    &event,
                    input_monitor,
                    value,
                    shape,
                    result.clone(),
                )?;
                result = constructed.0;
                grant_creation = constructed.1;
                production_inputs = constructed.2;
            }
            if let Some(view) = &result {
                if let SourcePayloadView::Checked { referents, .. } =
                    &tables.source_views[view.index].payload
                {
                    for referent in referents.iter().filter(|_| {
                        result_transport.is_none()
                            && !matches!(
                                point.kind,
                                CfgPointKind::Borrow {
                                    request: ReferenceBorrowRequest::Through { .. }
                                }
                            )
                    }) {
                        if !production_inputs.contains(&referent.coordinate) {
                            production_inputs.push(referent.coordinate.clone());
                        }
                    }
                }
            }
        }
    }
    if let Some(view) = &result {
        if !selected_grant::has_pending(tables, view) {
            row.pending.retain(|pending| pending.event != event);
        } else {
            let pending = SourcePendingProducerFact {
                event: event.clone(),
                outputs: outputs.iter().map(|(value, _)| value.clone()).collect(),
            };
            if !row.pending.contains(&pending) {
                row.pending.push(pending);
            }
        }
        for (value, shape) in &outputs {
            let slot = coordinate(
                tables,
                owner,
                SourceCoordinateRole::Evaluated(value.clone()),
                shape.clone(),
                &point.source_span,
            )?;
            install(&mut row, slot, view.clone(), ValueAvailability::Available);
            if let Some((source, kind)) = &result_transport {
                let destination = selected_slot(
                    tables,
                    &row,
                    &SourceCoordinateRole::Evaluated(value.clone()),
                )
                .map_err(|_| invalid(&point.source_span))?
                .coordinate
                .clone();
                transports.push(JointValueTransportFact {
                    source: source.clone(),
                    destination,
                    kind: kind.clone(),
                    child: JointValueChildPort::LocalInput,
                });
            }
        }
    } else if !outputs.is_empty() {
        let pending = SourcePendingProducerFact {
            event: event.clone(),
            outputs: outputs.iter().map(|(value, _)| value.clone()).collect(),
        };
        if !row.pending.contains(&pending) {
            row.pending.push(pending);
        }
    }
    for slot in &row.slots {
        if !transports
            .iter()
            .any(|transport| transport.destination == slot.coordinate)
            && input_row.slots.iter().any(|old| old == slot)
        {
            transports.push(JointValueTransportFact {
                source: slot.coordinate.clone(),
                destination: slot.coordinate.clone(),
                kind: JointValueTransportKind::Preserve,
                child: JointValueChildPort::LocalInput,
            });
        }
    }
    let receiving_transition = if matches!(point.kind, CfgPointKind::ReturnOutput { .. }) {
        owner_receiving::prepare(
            tables,
            original,
            &event,
            &input_row,
            &mut row,
            input_monitor,
        )?
    } else {
        None
    };
    owner_receiving::refresh_views(tables, &mut row, &point.source_span)?;
    let output = configuration(tables, row.clone());
    row.id = output.clone();
    if let Some(mut transition) = receiving_transition {
        transition.output = output.clone();
        if !tables.owner_receiving_transitions.contains(&transition) {
            tables.owner_receiving_transitions.push(transition);
        }
    }
    let kind = rule_kind(original, predecessor, point)?;
    let mut production_ports = Vec::new();
    if result.is_some() && monitor_complete {
        for (value, _) in &outputs {
            production_ports.push(
                selected_slot(
                    tables,
                    &row,
                    &SourceCoordinateRole::Evaluated(value.clone()),
                )
                .map_err(|_| invalid(&point.source_span))?
                .coordinate
                .clone(),
            );
        }
        production_ports.extend(backing_ports.clone());
    }
    let tuple = contributor(
        tables,
        JointProducerTupleFact {
            id: JointProducerTupleRef {
                owner: owner.clone(),
                index: 0,
            },
            event: event.clone(),
            input: input.clone(),
            output: output.clone(),
            kind,
            children: vec![JointProducerInputPort {
                event: predecessor.clone(),
                configuration: input.clone(),
                ports: input_row
                    .slots
                    .iter()
                    .map(|slot| slot.coordinate.clone())
                    .collect(),
            }],
            preserved: before.clone(),
            fresh: row
                .instances
                .iter()
                .filter(|instance| !before.contains(instance))
                .cloned()
                .collect(),
            outputs: outputs.iter().map(|(value, _)| value.clone()).collect(),
            productions: production_ports
                .into_iter()
                .map(|destination| JointValueProductionPort {
                    destination,
                    inputs: production_inputs.clone(),
                })
                .collect(),
            transports,
        },
    );
    let mut productions = Vec::new();
    if let Some(view) = result {
        let path = EvaluatedPathFact {
            id: EvaluatedControlPathId {
                producer: owner.clone(),
                ordinal: tables.source_paths.len(),
            },
            event: event.clone(),
            frame: row.frame.clone(),
            configuration: output.clone(),
            contributor: tuple.clone(),
        };
        let path = if let Some(previous) = tables.source_paths.iter().find(|previous| {
            previous.event == path.event
                && previous.configuration == path.configuration
                && previous.contributor == path.contributor
        }) {
            previous.id.clone()
        } else {
            let id = path.id.clone();
            tables.source_paths.push(path);
            id
        };
        let selected = tables.source_views[view.index].clone();
        let payload = match selected.payload {
            SourcePayloadView::Plain => Some(EvaluatedPayloadFact::Plain),
            SourcePayloadView::Raw { provenance, .. } => {
                Some(EvaluatedPayloadFact::Raw { provenance })
            }
            SourcePayloadView::StaticRaw(source) => Some(EvaluatedPayloadFact::Raw {
                provenance: RawValueProvenance::Static { source },
            }),
            SourcePayloadView::Aggregate(members) => Some(EvaluatedPayloadFact::FixedAggregate {
                members: members
                    .into_iter()
                    .map(|member| EvaluatedMemberFact {
                        projection: member.projection,
                        value: member.source,
                    })
                    .collect(),
            }),
            SourcePayloadView::Backing { element, .. } => {
                let facts = tables
                    .source_backing_observations
                    .iter()
                    .filter(|fact| fact.view == view)
                    .collect::<Vec<_>>();
                let [fact] = facts.as_slice() else {
                    return Err(invalid(&point.source_span));
                };
                Some(EvaluatedPayloadFact::RuntimeBacking {
                    backing: fact.backing.clone(),
                    elements: ElementViewFact {
                        element,
                        backing: fact.backing.value.clone(),
                        selections: Vec::new(),
                    },
                })
            }
            SourcePayloadView::Formal(_) => None,
            SourcePayloadView::Unwritten => None,
            SourcePayloadView::Checked { state, .. } => Some(EvaluatedPayloadFact::Checked {
                origins: checked.clone().ok_or_else(|| invalid(&point.source_span))?,
                state,
            }),
        };
        if let Some(payload) = payload {
            for (value, _) in outputs {
                let fact = tables
                    .values
                    .iter_mut()
                    .find(|fact| fact.id == value)
                    .ok_or_else(|| invalid(&point.source_span))?;
                let existing = fact
                    .cases
                    .iter()
                    .find(|case| {
                        case.path == path
                            && case.payload == payload
                            && case.dependencies == selected.dependencies
                    })
                    .map(|case| case.id.clone());
                let case_id = if let Some(existing) = existing {
                    existing
                } else {
                    let case = EvaluatedValueCase {
                        id: ValueCaseId {
                            value: value.clone(),
                            alternative: fact.cases.len(),
                        },
                        path: path.clone(),
                        availability: ValueAvailability::Available,
                        payload: payload.clone(),
                        dependencies: selected.dependencies.clone(),
                    };
                    let id = case.id.clone();
                    fact.cases.push(case);
                    id
                };
                let port = selected_slot(
                    tables,
                    &row,
                    &SourceCoordinateRole::Evaluated(value.clone()),
                )
                .map_err(|_| invalid(&point.source_span))?
                .coordinate
                .clone();
                let mut ports = vec![port];
                ports.extend(backing_ports.clone());
                for port in ports {
                    let observation = SourceCaseViewFact {
                        case: case_id.clone(),
                        view: view.clone(),
                        port,
                        path: path.clone(),
                        configuration: output.clone(),
                        contributor: tuple.clone(),
                    };
                    if !tables.source_case_views.contains(&observation) {
                        tables.source_case_views.push(observation.clone());
                    }
                    if monitor_complete {
                        productions.push(JointPortProductionFact {
                            observation,
                            inputs: production_inputs.clone(),
                        });
                    }
                }
            }
        }
    }
    if let Some((child, parent)) = grant_creation {
        let witness = SourceGrantCreationWitness {
            event: event.clone(),
            input: input.clone(),
            output: output.clone(),
            contributor: tuple.clone(),
            child,
            parent,
        };
        if !tables.source_grant_creations.contains(&witness) {
            tables.source_grant_creations.push(witness);
        }
    }
    let moves = match move_selection {
        Some((input, selection)) => vec![selected_move::register(tables, input, selection, &tuple)],
        None => Vec::new(),
    };
    let monitor = port_monitor::transition(
        tables,
        &tuple,
        input_monitor,
        Vec::new(),
        productions,
        moves,
        &point.source_span,
    )?;
    if matches!(point.kind, CfgPointKind::AssignmentSnapshot { .. }) {
        port_monitor::retain_saved(tables, &row, &event, &monitor, frozen, &point.source_span)?;
    }
    if matches!(point.kind, CfgPointKind::ReturnOutput { .. }) {
        owner_consumer::construct_return(tables, original, &event, &row, &tuple, &monitor)?;
    }
    Ok(vec![SelectedSourceState::Active {
        configuration: output,
        contributor: tuple,
        predecessor: event,
        monitor,
    }])
}
