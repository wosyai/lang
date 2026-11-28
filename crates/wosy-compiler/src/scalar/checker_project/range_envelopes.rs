use super::*;
use crate::scalar::reference::{BlockOrdinal, OccurrenceEdge, OccurrenceNode, OccurrenceParent, PlainNameIdentity};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Envelope {
    pub(super) low: BigInt,
    pub(super) high: BigInt,
}

impl Envelope {
    fn whole(ty: &ScalarType) -> Option<Self> {
        let (low, high) = super::super::typecheck::integer_bounds(ty)?;
        Some(Self { low, high })
    }

    fn exact(value: BigInt) -> Self {
        Self { low: value.clone(), high: value }
    }

    fn hull(self, rhs: Self) -> Self {
        Self { low: self.low.min(rhs.low), high: self.high.max(rhs.high) }
    }

    fn fit(self, ty: &ScalarType) -> Option<Self> {
        let full = Self::whole(ty)?;
        Some(if self.low < full.low || self.high > full.high { full } else { self })
    }

    fn binary(self, op: &super::super::BinaryOperator, rhs: Self, ty: &ScalarType) -> Option<Self> {
        use super::super::BinaryOperator::*;
        let bounds = match op {
            Add => Self { low: self.low + rhs.low, high: self.high + rhs.high },
            Subtract => Self { low: self.low - rhs.high, high: self.high - rhs.low },
            Multiply if self.low == self.high && rhs.low == rhs.high => Self::exact(self.low * rhs.low),
            Divide | Remainder if rhs.low == rhs.high && rhs.low.bits() != 0 => {
                // Signed division truncates toward zero. A negative divisor
                // reverses the endpoints; never infer from MIN / -1.
                let entire = Self::whole(ty)?;
                if rhs.low == BigInt::from(-1) && self.low <= entire.low {
                    return Some(entire);
                }
                if matches!(op, Divide) {
                    let a = self.low / &rhs.low;
                    let b = self.high / &rhs.low;
                    if a <= b { Self { low: a, high: b } } else { Self { low: b, high: a } }
                } else if self.low == self.high {
                    Self::exact(self.low % rhs.low)
                } else {
                    // The remainder has the dividend's sign and magnitude
                    // strictly smaller than the absolute divisor.
                    let zero = BigInt::from(0);
                    let limit = if rhs.low < zero { -rhs.low } else { rhs.low } - 1u8;
                    let low = self.low.min(zero.clone()).max(-&limit);
                    let high = self.high.max(zero).min(limit);
                    Self { low, high }
                }
            }
            Multiply | Divide | Remainder => Self::whole(ty)?,
            _ => return None,
        };
        bounds.fit(ty)
    }
}

pub(super) type Inputs = Vec<Option<Envelope>>;
type Key = (PreparedCallable, Rc<Inputs>, bool);
type Lanes = Option<Rc<Inputs>>;

// Constructed only from a prepared call occurrence and its dominating guards.
pub(super) struct IncomingRangeEdge {
    caller: PreparedCallable,
    callee: PreparedCallable,
    pub(super) site: OccurrenceOrdinal,
    pub(super) inputs: Option<Inputs>,
}

#[derive(Clone, Copy)]
enum Need { Int, Bool(bool) }

#[derive(Clone)]
enum Answer { Int(Option<Envelope>), Bool(Lanes) }

impl Answer {
    fn bound(self) -> Option<Envelope> { if let Self::Int(bound) = self { bound } else { None } }
    fn lanes(self) -> Option<Lanes> { if let Self::Bool(lanes) = self { Some(lanes) } else { None } }
}

#[derive(Clone, Copy)]
struct Frame<'a> {
    body: &'a FunctionCallSites,
    target: PreparedCallable,
    ids: &'a [ReferenceBindingId],
    arrival: bool,
}

impl<'a> Frame<'a> {
    fn formal(&self, e: OccurrenceOrdinal) -> Option<usize> {
        let PlainNameIdentity::Lexical(binding) = self.body.prepared_plain_name(e)? else { return None; };
        self.ids.iter().position(|formal| formal == binding)
    }

    fn output(&self, block: BlockOrdinal) -> Option<OccurrenceOrdinal> {
        let [item] = self.body.structure.blocks.get(block.0)?.items.as_slice() else { return None; };
        let BlockItemKind::FinalOutput(slot) = item.kind else { return None; };
        (slot.output == 0).then_some(slot.expression)
    }

    fn conditional(&self, e: OccurrenceOrdinal) -> Option<(OccurrenceOrdinal, OccurrenceOrdinal, OccurrenceOrdinal)> {
        let ConditionalRegion::Value { condition, then_block, else_block, resume, .. } = self.body.conditionals.get(&e)? else { return None; };
        if *resume != e || self.body.structure.checked_types.expressions.get(condition)? != &ScalarType::Bool {
            return None;
        }
        Some((*condition, self.output(*then_block)?, self.output(*else_block)?))
    }
}

pub(super) struct PreparedEnvelopes<'a> {
    project: &'a ScalarProject,
    calls: &'a ProjectCallSiteResults,
    integer: HashMap<Key, Answer>,
    boolean: HashMap<(Key,bool), Answer>,
    #[cfg(test)]
    work: [usize;3],
}

impl<'a> PreparedEnvelopes<'a> {
    pub(super) fn new(project: &'a ScalarProject, calls: &'a ProjectCallSiteResults) -> Self {
        Self { project, calls, integer: HashMap::new(), boolean: HashMap::new(),
            #[cfg(test)]
            work: [0;3],
        }
    }

    fn frame_without_formal_mutation(&self,target: PreparedCallable) -> Option<Frame<'a>> {
        use crate::scalar::reference::{DirectDestinationIdentity, NamedPlaceRootIdentity};
        let body = self.calls.function_body(target.0,target.1,None)?;
        let ids = body.boundary.checker_parameter_bindings.as_deref()?;
        if ids.len() != body.boundary.parameters.len() || body.contextual_formal().is_some()
            || !body.arms.is_empty() || body.structure.blocks.iter().any(|b| b.unsafe_context)
        { return None; }
        let formals = ids.iter().collect::<HashSet<_>>();
        for at in (0..body.occurrences.len()).map(OccurrenceOrdinal) {
            if matches!(body.prepared_direct_destination(at),Some(DirectDestinationIdentity::Existing(id)) if formals.contains(id))
                || matches!(body.prepared_named_place_root(at),Some(NamedPlaceRootIdentity::Lexical(id)) if formals.contains(id))
            { return None; }
        }
        Some(Frame { body,target,ids,arrival:false })
    }

    fn frame(&self, target: PreparedCallable) -> Option<Frame<'a>> {
        let function = self.calls.function_body(target.0, target.1, None)?;
        if function.contextual_formal().is_some() || !function.arms.is_empty() || !function.assignments.is_empty()
            || !function.callable_escapes.is_empty() || !function.allocation_length_calls.is_empty()
            || !function.structure.module_storage_uses.is_empty()
            || !function.structure.checker_directives.is_empty()
            || function.structure.blocks.iter().any(|block| block.unsafe_context)
        { return None; }
        let ids = function.boundary.checker_parameter_bindings.as_deref()?;
        if ids.len() != function.boundary.parameters.len() { return None; }
        Some(Frame { body: function, target, ids, arrival:false })
    }

    fn min_max(&self, frame: Frame<'a>, condition: OccurrenceOrdinal, yes: OccurrenceOrdinal, no: OccurrenceOrdinal,
        input: &Inputs) -> Option<Envelope> {
        // Recognize the source relation, not a helper's spelling.
        let (left,op,right) = frame.body.structure.prepared_range_binary.get(&condition)?;
        let (lhs,rhs,success,failure) = (frame.formal(*left)?,frame.formal(*right)?,frame.formal(yes)?,frame.formal(no)?);
        let picks_left_on_true = (success,failure) == (lhs,rhs);
        let picks_right_on_true = (success,failure) == (rhs,lhs);
        if !picks_left_on_true && !picks_right_on_true { return None; }
        use super::super::BinaryOperator::*;
        let max = match op {
            Less | LessEqual => picks_right_on_true,
            Greater | GreaterEqual => picks_left_on_true,
            _ => return None,
        };
        let (l,r) = (input.get(lhs)?.as_ref()?,input.get(rhs)?.as_ref()?);
        let pick = if max { std::cmp::max } else { std::cmp::min };
        Some(Envelope { low: pick(l.low.clone(),r.low.clone()), high: pick(l.high.clone(),r.high.clone()) })
    }

    fn facts(&mut self, frame: &Frame<'a>, site: OccurrenceOrdinal, want: bool, input: Inputs) -> Option<Option<Inputs>> {
        self.evaluate(*frame,site,Need::Bool(want),Rc::new(input))?.lanes()
            .map(|row| row.map(Rc::unwrap_or_clone))
    }

    fn evaluate(&mut self, root: Frame<'a>, at: OccurrenceOrdinal, need: Need, inputs: Rc<Inputs>) -> Option<Answer> {
        enum Step<'a> {
            Visit(Frame<'a>,OccurrenceOrdinal,Need,Rc<Inputs>),
            Restrict(Frame<'a>,OccurrenceOrdinal,Need),
            Join(Need,&'a ScalarType),
            Arithmetic(&'a BinaryOperator,&'a ScalarType),
            Compare(Frame<'a>,OccurrenceOrdinal,OccurrenceOrdinal,&'a BinaryOperator,bool,Rc<Inputs>),
            Args(PreparedCallable,Vec<Option<OccurrenceOrdinal>>,Frame<'a>,Need,Rc<Inputs>,&'a ScalarType),
            Record(Key,Need),
            Forward(Vec<Option<OccurrenceOrdinal>>,Frame<'a>,Rc<Inputs>),
        }
        use Answer::{Bool, Int};
        use Step::*;
        let mut steps = vec![Visit(root,at,need,inputs)];
        let mut values = Vec::<Answer>::new();
        let mut active = HashSet::from([root.target]);
        while let Some(step) = steps.pop() {
            #[cfg(test)]
            {
                self.work[0] += 1;
                self.work[2] = self.work[2].max(steps.len()+1);
            }
            match step {
                Visit(frame,e,needed,input) => {
                    let st = &frame.body.structure;
                    let ty = st.checked_types.expressions.get(&e)?;
                    match needed {
                        Need::Int => {
                            if !is_integer_type(ty) { return None; }
                            if let Some(value) = st.prepared_range_literals.get(&e) {
                                if !integer_literal_fits_type(value,ty) { return None; }
                                values.push(Int(Some(Envelope::exact(value.clone()))));
                                continue;
                            }
                            if let Some(i) = frame.formal(e) {
                                values.push(Int(Some(input.get(i)?.clone()?)));
                                continue;
                            }
                            if let Some((left,op,right)) = st.prepared_range_binary.get(&e) {
                                steps.extend([Arithmetic(op,ty),Visit(frame,*right,Need::Int,Rc::clone(&input)),
                                    Visit(frame,*left,Need::Int,input)]);
                                continue;
                            }
                        }
                        Need::Bool(want) => {
                            if *ty != ScalarType::Bool { return None; }
                            if let Some(value) = st.observed_bool_literals.get(&e) {
                                values.push(Bool((want == *value).then_some(input)));
                                continue;
                            }
                            if frame.formal(e).is_some() { values.push(Bool(Some(input))); continue; }
                            if let Some(inner) = st.observed_bool_negations.get(&e) {
                                steps.push(Visit(frame,*inner,Need::Bool(!want),input));
                                continue;
                            }
                            if let Some(branch) = frame.body.short_circuits.get(&e) {
                                let live = branch.evaluate_right_when;
                                if want != live {
                                    steps.extend([Join(needed,ty),Visit(frame,branch.left,Need::Bool(!live),Rc::clone(&input))]);
                                }
                                steps.extend([Restrict(frame,branch.right,needed),Visit(frame,branch.left,Need::Bool(live),input)]);
                                continue;
                            }
                            if let Some((left,op,right)) = st.prepared_range_binary.get(&e) {
                                steps.extend([Compare(frame,*left,*right,op,want,Rc::clone(&input)),
                                    Visit(frame,*right,Need::Int,Rc::clone(&input)),Visit(frame,*left,Need::Int,input)]);
                                continue;
                            }
                        }
                    }
                    if let Some((condition,yes,no)) = frame.conditional(e) {
                        if matches!(needed, Need::Int) {
                            if let Some(bound) = self.min_max(frame,condition,yes,no,&input) {
                                values.push(Int(Some(bound)));
                                continue;
                            }
                        }
                        steps.extend([Join(needed,ty),Restrict(frame,no,needed),Visit(frame,condition,Need::Bool(false),Rc::clone(&input)),
                            Restrict(frame,yes,needed),Visit(frame,condition,Need::Bool(true),input)]);
                        continue;
                    }
                    let child = frame.body.checked_call_signatures.get(&e).and_then(|signature| {
                        let target = super::authenticate_prepared_direct_edge(self.project,self.calls,frame.target.0,frame.body,e,signature)?;
                        Some((target,signature))
                    });
                    let Some((child,signature)) = child else {
                        // Unknown typed values are an unconstrained range *on
                        // arrival*. This is not a claim that a producing
                        // expression executes, is pure, or returns normally.
                        if frame.arrival && matches!(needed, Need::Int) {
                            values.push(Int(Some(Envelope::whole(ty)?)));
                            continue;
                        }
                        return None;
                    };
                    let mut slots = Vec::with_capacity(signature.parameters.len());
                    for (slot,ty) in signature.parameters.iter().enumerate() {
                        let OccurrenceNode::Expression(argument) = st.edges.get(&(
                            OccurrenceParent::Expression(e),OccurrenceEdge::CallArgument(slot)
                        ))? else { return None; };
                        if st.checked_types.expressions.get(argument)? != ty { return None; }
                        if is_integer_type(ty) { slots.push(Some(*argument)); }
                        else if *ty == ScalarType::Bool && (frame.arrival || frame.formal(*argument).is_some()
                            || st.observed_bool_literals.contains_key(argument)) {
                            slots.push(None);
                        } else { return None; }
                    }
                    steps.push(Args(child,slots.clone(),frame,needed,Rc::clone(&input),ty));
                    steps.extend(slots.into_iter().rev().flatten().map(|arg| Visit(frame,arg,Need::Int,Rc::clone(&input))));
                }
                Restrict(frame,next,need) => {
                    match values.pop()?.lanes()? {
                        Some(row) => steps.push(Visit(frame,next,need,row)),
                        None => values.push(match need { Need::Int => Int(None), Need::Bool(_) => Bool(None) }),
                    }
                }
                Join(need,ty) => {
                    let (r,l) = (values.pop()?,values.pop()?);
                    values.push(match (need,l,r) {
                        (Need::Int,Int(Some(l)),Int(Some(r))) => Int(Some(l.hull(r))),
                        (Need::Int,Int(l),Int(r)) => Int(l.or(r).or_else(|| Envelope::whole(ty))),
                        (Need::Bool(_),Bool(l),Bool(r)) => Bool(join(l,r)),
                        _ => return None,
                    });
                }
                Arithmetic(op,ty) => {
                    let (r,l) = (values.pop()?.bound()?,values.pop()?.bound()?);
                    values.push(Int(Some(l.binary(op,r,ty)?)));
                }
                Compare(frame,left,right,op,want,mut input) => {
                    let (r,l) = (values.pop()?.bound()?,values.pop()?.bound()?);
                    let mut viable = true;
                    for (site,other,reverse) in [(left,&r,false),(right,&l,true)] {
                        if let Some(axis) = frame.formal(site) {
                            let bound = Rc::make_mut(&mut input).get_mut(axis)?.as_mut()?;
                            intersect_comparison(bound,op,want,other,reverse)?;
                            viable &= bound.low <= bound.high;
                        }
                    }
                    values.push(Bool(viable.then_some(input)));
                }
                Args(target,slots,frame,needed,outer,ty) => {
                    let args = slots.iter().rev().map(|slot| if slot.is_some() { Some(Some(values.pop()?.bound()?)) } else { Some(None) })
                        .collect::<Option<Vec<_>>>()?.into_iter().rev().collect::<Vec<_>>();
                    let key = (target,Rc::new(args),frame.arrival);
                    if matches!(needed,Need::Bool(_)) { steps.push(Forward(slots,frame,outer)); }
                    let cached = match needed {
                        Need::Int => self.integer.get(&key).cloned(),
                        Need::Bool(want) => self.boolean.get(&(key.clone(),want)).cloned(),
                    };
                    if let Some(known) = cached {
                        #[cfg(test)] { self.work[1] += 1; }
                        values.push(known);
                        continue;
                    }
                    let definition = (!active.contains(&target)).then(|| self.frame(target)).flatten()
                        .and_then(|body| Some((body,body.output(body.body.structure.root)?)));
                    let Some((mut definition,output)) = definition else {
                        if frame.arrival && matches!(needed, Need::Int) {
                            values.push(Int(Some(Envelope::whole(ty)?)));
                            continue;
                        }
                        return None;
                    };
                    active.insert(target);
                    definition.arrival = frame.arrival;
                    let local = Rc::clone(&key.1);
                    steps.extend([Record(key,needed),Visit(definition,output,needed,local)]);
                }
                Record(key,needed) => {
                    active.remove(&key.0);
                    let value = values.last()?.clone();
                    match needed {
                        Need::Int => self.integer.insert(key,value),
                        Need::Bool(want) => self.boolean.insert((key,want),value),
                    };
                }
                Forward(slots,frame,mut outer) => {
                    let narrowed = values.pop()?.lanes()?;
                    let mut viable = narrowed.is_some();
                    if let Some(row) = narrowed {
                        if slots.len() != row.len() { return None; }
                        for (slot,bounds) in slots.into_iter().zip(row.iter()) {
                            let (Some(arg),Some(bounds)) = (slot,bounds) else { continue; };
                            if let Some(axis) = frame.formal(arg) {
                                let current = Rc::make_mut(&mut outer).get_mut(axis)?.as_mut()?;
                                current.low = std::cmp::max(current.low.clone(),bounds.low.clone());
                                current.high = std::cmp::min(current.high.clone(),bounds.high.clone());
                                viable &= current.low <= current.high;
                            }
                        }
                    }
                    values.push(Bool(viable.then_some(outer)));
                }
            }
        }
        if values.len() != 1 { return None; }
        values.pop()
    }

    #[cfg(test)]
    pub(super) fn work(&self) -> [usize;3] { self.work }

    fn refine_at_site(&mut self, frame: Frame<'a>, site: OccurrenceOrdinal,
        parents: &HashMap<OccurrenceParent,(OccurrenceParent,OccurrenceEdge)>,
        known: &mut HashMap<OccurrenceParent,Lanes>) -> Option<Lanes> {
        let mut cursor = OccurrenceParent::Expression(site);
        let mut suffix = Vec::new();
        while !known.contains_key(&cursor) {
            let &(parent,edge) = parents.get(&cursor)?;
            suffix.push((cursor,parent,edge));
            cursor = parent;
        }
        let mut current = known.get(&cursor)?.clone();
        for (child,parent,edge) in suffix.into_iter().rev() {
            let mut guard = None;
            if let OccurrenceParent::Expression(at) = parent {
                match edge {
                    OccurrenceEdge::ThenBranch | OccurrenceEdge::ElseBranch => {
                        let condition = match frame.body.conditionals.get(&at)? {
                            ConditionalRegion::Unit { condition, .. } | ConditionalRegion::Value { condition, .. } => *condition,
                        };
                        guard = Some((condition,edge == OccurrenceEdge::ThenBranch));
                    }
                    OccurrenceEdge::BinaryRight => {
                        if let Some(region) = frame.body.short_circuits.get(&at) {
                            guard = Some((region.left,region.evaluate_right_when));
                        }
                    }
                    _ => {}
                }
            }
            if let (Some((guard,want)),Some(row)) = (guard,&current) {
                // An unknown guard does not rule out its arm. Cache the
                // shared unchanged prefix as well as checked refinements.
                if let Some(answer) = self.evaluate(frame,guard,Need::Bool(want),Rc::clone(row)) {
                    current = answer.lanes()?;
                }
            }
            known.insert(child,current.clone());
        }
        Some(current)
    }

    /// Bounds on the arguments of this exact incoming call in a given entry
    /// envelope. This never narrows any other activation of the target.
    fn edge_inputs(&mut self, frame: Frame<'a>, site: OccurrenceOrdinal,
        parents: &HashMap<OccurrenceParent,(OccurrenceParent,OccurrenceEdge)>,
        known: &mut HashMap<OccurrenceParent,Lanes>) -> Option<Option<Inputs>> {
        let Some(row) = self.refine_at_site(frame,site,parents,known)? else { return Some(None); };
        let st = &frame.body.structure;
        let signature = frame.body.checked_call_signatures.get(&site)?;
        let args = signature.parameters.iter().enumerate().map(|(slot,ty)| {
            let OccurrenceNode::Expression(arg) = st.edges.get(&(
                OccurrenceParent::Expression(site),OccurrenceEdge::CallArgument(slot)))? else { return None; };
            if st.checked_types.expressions.get(arg)? != ty { return None; }
            if *ty == ScalarType::Bool { return Some(None); }
            let entire = Envelope::whole(ty)?;
            // A typed value outside this vocabulary is unknown, not a
            // new externally supplied prerequisite.
            Some(Some(self.evaluate(frame,*arg,Need::Int,Rc::clone(&row)).and_then(Answer::bound).unwrap_or(entire)))
        }).collect::<Option<Inputs>>()?;
        Some(Some(args))
    }

    /// All call sites here are prepared identities of this one frame, not
    /// an assertion that all ways to enter its callee have been enumerated.
    pub(super) fn direct_edge_envelopes(
        &mut self, owner: PreparedCallable, callee: PreparedCallable, input: &Inputs,
    ) -> Option<Vec<IncomingRangeEdge>> {
        self.direct_edge_envelopes_with_unknown_places(owner,callee,input,false)
    }

    pub(super) fn direct_arrival_edge_envelopes(
        &mut self, owner: PreparedCallable, callee: PreparedCallable, input: &Inputs,
    ) -> Option<Vec<IncomingRangeEdge>> {
        self.direct_edge_envelopes_with_unknown_places(owner,callee,input,true)
    }

    fn direct_edge_envelopes_with_unknown_places(
        &mut self, owner: PreparedCallable, callee: PreparedCallable, input: &Inputs, arrival: bool,
    ) -> Option<Vec<IncomingRangeEdge>> {
        let mut frame = self.frame_without_formal_mutation(owner)?;
        frame.arrival = arrival;
        if !contained(&initial_inputs(frame.body)?,input) { return None; }
        let parents = occurrence_parents(frame.body)?;
        let mut known = HashMap::from([(OccurrenceParent::Block(frame.body.structure.root),Some(Rc::new(input.clone())))]);
        let mut edges = Vec::new();
        for site in &frame.body.call_occurrences {
            if frame.body.structure.checker_direct_targets.get(&site.ordinal) != Some(&callee) { continue; }
            let sig = frame.body.checked_call_signatures.get(&site.ordinal)?;
            if super::authenticate_prepared_direct_edge(self.project,self.calls,owner.0,frame.body,site.ordinal,sig)? != callee { return None; }
            edges.push(IncomingRangeEdge { caller:owner, callee, site:site.ordinal,
                inputs:self.edge_inputs(frame,site.ordinal,&parents,&mut known)? });
        }
        (!edges.is_empty()).then_some(edges)
    }

    /// Exact monotone self-edge checking after initial rows are proved by
    /// their incoming caller; the default top external row does not
    /// silently borrow that caller's bound.
    pub(super) fn guarded_self_induction(
        &mut self, target: PreparedCallable, invariant: &Inputs, initial: &IncomingRangeEdge,
    ) -> Option<Vec<OccurrenceOrdinal>> {
        if initial.callee != target || initial.caller == target || !contained(invariant,initial.inputs.as_ref()?) { return None; }
        let mut returned = Vec::new();
        for next in self.direct_edge_envelopes(target,target,invariant)? {
            if next.inputs.as_ref().is_some_and(|row| !contained(invariant,row)) { return None; }
            returned.push(next.site);
        }
        Some(returned)
    }

    /// Close only obligations universal under exact same-call branch facts.
    /// Every remaining authored site and any unsupported history use the
    /// existing search; proving one call site never certifies other callers.
    pub(super) fn close_guarded_formals(&mut self, target: PreparedCallable) -> Option<Vec<ByteSpan>> {
        let body = self.calls.function_body(target.0,target.1,None)?;
        if !body.checker_returns_unit() { return None; }
        self.requirements_in_envelope(target,initial_inputs(body)?)
    }

    fn requirements_in_envelope(&mut self, target: PreparedCallable, entry: Inputs) -> Option<Vec<ByteSpan>> {
        let frame = self.frame_without_formal_mutation(target)?;
        let b = frame.body;
        let st = &b.structure;
        if st.checker_directives.is_empty() || b.conditionals.is_empty()
            || !contained(&initial_inputs(b)?,&entry)
        { return None; }
        let parents = occurrence_parents(frame.body)?;
        let mut known = HashMap::from([(OccurrenceParent::Block(st.root),Some(Rc::new(entry)))]);
        let mut covered = Vec::new();
        for (&expression,directive) in &st.checker_directives {
            if directive.operation != CheckerOperation::Require { return None; }
            let inputs = self.refine_at_site(frame,expression,&parents,&mut known)??;
            self.requirement_accepts(frame,expression,&inputs)?;
            covered.push(b.occurrences.get(expression.0)?.span);
        }
        Some(covered)
    }

    fn requirement_accepts(&self, frame: Frame<'a>, site: OccurrenceOrdinal, input: &Inputs) -> Option<()> {
        use crate::scalar::reference::CheckerOrdinaryBoolean;
        let b = frame.body;
        let st = &b.structure;
        let directive = st.checker_directives.get(&site)?;
        if directive.operation != CheckerOperation::Require { return None; }
        let accepts = |c: &super::super::checker_flow::IntegerPredicate| {
            let axis = frame.ids.iter().position(|id| id == &c.binding)?;
            let range = input.get(axis)?.as_ref()?;
            if b.boundary.parameters.get(axis)?.ty != c.ty { return None; }
            IntegerRelation::from_operator(&c.operator)?
                .accepts_closed(&range.low,&range.high,&c.bound).then_some(())
        };
        match directive.predicate.as_ref() {
            Some(CheckerPredicate::Integer(c)) => accepts(c),
            None => {
                let mut todo = vec![st.checker_ordinary_booleans.get(&site)?];
                while let Some(p) = todo.pop() {
                    match p {
                        CheckerOrdinaryBoolean::Integer(c) => accepts(c)?,
                        CheckerOrdinaryBoolean::And(pair) => todo.extend(pair.iter().rev()),
                        _ => return None,
                    }
                }
                Some(())
            }
            _ => None,
        }
    }

    /// An exact literal edge does not depend on caller storage, preceding
    /// host calls, or the guard leading to it. It is conditional on the actual
    /// call being reached and says nothing about other ways into its callee.
    pub(super) fn literal_call_seeds(&self, caller: PreparedCallable, callee: PreparedCallable) -> Option<Vec<IncomingRangeEdge>> {
        let body = self.calls.function_body(caller.0,caller.1,None)?;
        let st = &body.structure;
        let mut edges = Vec::new();
        for call in &body.call_occurrences {
            let site = call.ordinal;
            if st.checker_direct_targets.get(&site) != Some(&callee) { continue; }
            let sig = body.checked_call_signatures.get(&site)?;
            if super::authenticate_prepared_direct_edge(self.project,self.calls,caller.0,body,site,sig)? != callee { return None; }
            let inputs = sig.parameters.iter().enumerate().map(|(slot,ty)| {
                let OccurrenceNode::Expression(argument) = st.edges.get(&(
                    OccurrenceParent::Expression(site),OccurrenceEdge::CallArgument(slot)))? else { return None; };
                if st.checked_types.expressions.get(argument)? != ty || !is_integer_type(ty) { return None; }
                let value = st.prepared_range_literals.get(argument)?;
                integer_literal_fits_type(value,ty).then(|| Some(Envelope::exact(value.clone())))
            }).collect::<Option<Inputs>>()?;
            edges.push(IncomingRangeEdge { caller,callee,site,inputs:Some(inputs) });
        }
        (!edges.is_empty()).then_some(edges)
    }

    /// Conditional on reaching a prepared occurrence, bound its typed
    /// arguments with no claims about the caller's effects or any branch.
    /// In particular, a local/current module value starts as its full type;
    /// a pure max/min in that argument can still establish an outer bound.
    pub(super) fn unconstrained_arrival_edges(&mut self, caller: PreparedCallable, callee: PreparedCallable)
        -> Option<Vec<IncomingRangeEdge>> {
        let body = self.calls.function_body(caller.0,caller.1,None)?;
        let st = &body.structure;
        let ids = body.boundary.checker_parameter_bindings.as_deref()?;
        if ids.len() != body.boundary.parameters.len() || !body.arms.is_empty() || body.contextual_formal().is_some() {
            return None;
        }
        let input = Rc::new(initial_inputs(body)?);
        let frame = Frame { body,target:caller,ids,arrival:true };
        let mut edges = Vec::new();
        for call in &body.call_occurrences {
            let site = call.ordinal;
            if st.checker_direct_targets.get(&site) != Some(&callee) { continue; }
            let signature = body.checked_call_signatures.get(&site)?;
            if super::authenticate_prepared_direct_edge(self.project,self.calls,caller.0,body,site,signature)? != callee { return None; }
            let values = signature.parameters.iter().enumerate().map(|(slot,ty)| {
                let OccurrenceNode::Expression(argument) = st.edges.get(&(
                    OccurrenceParent::Expression(site),OccurrenceEdge::CallArgument(slot)))? else { return None; };
                if st.checked_types.expressions.get(argument)? != ty { return None; }
                if *ty == ScalarType::Bool { return Some(None); }
                let value = self.evaluate(frame,*argument,Need::Int,Rc::clone(&input)).and_then(Answer::bound);
                Some(Some(value.unwrap_or(Envelope::whole(ty)?)))
            }).collect::<Option<Inputs>>()?;
            edges.push(IncomingRangeEdge { caller,callee,site,inputs:Some(values) });
        }
        (!edges.is_empty()).then_some(edges)
    }

    /// Ranges on a direct child edge of this activation only. Opaque effects
    /// elsewhere cannot be used to claim no indirect reentry or termination.
    pub(super) fn forward_incoming_edge(&mut self, incoming: &IncomingRangeEdge, callee: PreparedCallable)
        -> Option<Vec<IncomingRangeEdge>> {
        self.direct_edge_envelopes(incoming.callee,callee,incoming.inputs.as_ref()?)
    }

    /// Transport the incoming by-value row to this activation's ordinary
    /// child sites. Reads of unmodeled captured storage use their types,
    /// never startup values; the owner must still have immutable formals.
    /// The result is conditional on arrival, not a history or a return proof.
    pub(super) fn forward_arrival_edge(&mut self, incoming: &IncomingRangeEdge, callee: PreparedCallable)
        -> Option<Vec<IncomingRangeEdge>> {
        // Empty arms were proven unreachable at the preceding site.
        let Some(inputs) = incoming.inputs.as_ref() else { return Some(Vec::new()); };
        self.direct_arrival_edge_envelopes(incoming.callee,callee,inputs)
    }

    /// Induction covers the original self-call edges and derives the child
    /// bound on every direct site in those iterations. No dynamic callback
    /// route or separately seeded activation borrows this result.
    pub(super) fn forward_inductive_edges(&mut self, seed: &IncomingRangeEdge, invariant: &Inputs, callee: PreparedCallable)
        -> Option<(Vec<OccurrenceOrdinal>,Vec<IncomingRangeEdge>)> {
        let steps = self.guarded_self_induction(seed.callee,invariant,seed)?;
        Some((steps,self.direct_edge_envelopes(seed.callee,callee,invariant)?))
    }

    /// Prove only the first directive, before any possible effect. A later
    /// unsafe host call is not used as evidence or covered by this proof.
    pub(super) fn incoming_entry_requirement(&self, edge: &IncomingRangeEdge) -> Option<ByteSpan> {
        let body = self.calls.function_body(edge.callee.0,edge.callee.1,None)?;
        let input = edge.inputs.as_ref()?;
        if !body.checker_returns_unit() || !body.arms.is_empty() || body.contextual_formal().is_some()
            || !contained(&initial_inputs(body)?,input) { return None; }
        let ids = body.boundary.checker_parameter_bindings.as_deref()?;
        if ids.len() != body.boundary.parameters.len() { return None; }
        let block = body.structure.blocks.get(body.structure.root.0)?;
        if block.unsafe_context { return None; }
        let BlockItemKind::Evaluation { expression } = block.items.first()?.kind else { return None; };
        self.requirement_accepts(Frame { body,target:edge.callee,ids,arrival:false },expression,input)?;
        Some(body.occurrences.get(expression.0)?.span)
    }

    /// A returning or nonreturning activation at one specific incoming
    /// source occurrence. This result is not a package entry roster, and
    /// cannot remove ordinary search for different callers.
    pub(super) fn prove_inductive_activation(&mut self, initial: &IncomingRangeEdge,
        invariant: &Inputs) -> Option<(Vec<OccurrenceOrdinal>, Vec<ByteSpan>)> {
        let target = initial.callee;
        self.verify_no_indirect_reentry(target)?;
        let steps = self.guarded_self_induction(target,invariant,initial)?;
        let requirements = self.requirements_in_envelope(target,invariant.clone())?;
        Some((steps,requirements))
    }

    fn verify_no_indirect_reentry(&self, root: PreparedCallable) -> Option<()> {
        use super::super::CoreOperationId;
        let mut pending = vec![root];
        let mut visited = HashSet::new();
        while let Some(target) = pending.pop() {
            if !visited.insert(target) { continue; }
            let body = self.calls.function_body(target.0,target.1,None)?;
            if !body.arms.is_empty() || !body.callable_escapes.is_empty() || !body.allocation_length_calls.is_empty()
                || !body.allocation_length_call_targets.is_empty() || body.structure.blocks.iter().any(|block| block.unsafe_context)
            { return None; }
            for call in &body.call_occurrences {
                let at = call.ordinal;
                let signature = body.checked_call_signatures.get(&at)?;
                match &body.resolved_call_site(at)?.target {
                    ResolvedCallTarget::Core(CoreOperationId::IntTrunc | CoreOperationId::IntExtend
                        | CoreOperationId::UintToFloat | CoreOperationId::SintToFloat
                        | CoreOperationId::FloatToSintTrunc | CoreOperationId::FloatToUintTrunc
                        | CoreOperationId::FloatTrunc | CoreOperationId::FloatExtend) => {}
                    ResolvedCallTarget::ModuleCallable { concrete: ConcreteCallSelection::Declaration, .. } => {
                        let child = super::authenticate_prepared_direct_edge(self.project,self.calls,target.0,body,at,signature)?;
                        if child == root {
                            if target != root { return None; }
                        } else { pending.push(child); }
                    }
                    _ => return None,
                }
            }
        }
        Some(())
    }

    #[cfg(test)]
    pub(super) fn formal_envelopes(&mut self, target: PreparedCallable, want: bool) -> Option<Inputs> {
        let frame = self.frame(target)?;
        self.formal_envelopes_at(target,want,initial_inputs(frame.body)?)?
    }

    #[cfg(test)]
    pub(super) fn formal_envelopes_at(&mut self, target: PreparedCallable, want: bool, input: Inputs) -> Option<Option<Inputs>> {
        let frame = self.frame(target)?;
        if !matches!(frame.body.boundary.results.as_slice(), [result] if result.ty == ScalarType::Bool) { return None; }
        self.facts(&frame, frame.output(frame.body.structure.root)?, want, input)
    }
}

pub(super) fn occurrence_parents(body:&FunctionCallSites) -> Option<HashMap<OccurrenceParent,(OccurrenceParent,OccurrenceEdge)>> {
    let mut parents = HashMap::new();
    for (&(parent,edge), &child) in &body.structure.edges {
        let child = match child {
            OccurrenceNode::Expression(at) => OccurrenceParent::Expression(at),
            OccurrenceNode::Place(at) => OccurrenceParent::Place(at),
            OccurrenceNode::Block(at) => OccurrenceParent::Block(at),
        };
        if parents.insert(child,(parent,edge)).is_some() { return None; }
    }
    Some(parents)
}

fn contained(outer: &Inputs, inner: &Inputs) -> bool {
    outer.len() == inner.len() && outer.iter().zip(inner).all(|(limit,candidate)| match (limit,candidate) {
        (Some(a),Some(b)) => b.low <= b.high && a.low <= b.low && a.high >= b.high,
        (None,None) => true,
        _ => false,
    })
}

fn initial_inputs(body: &FunctionCallSites) -> Option<Inputs> {
    body.boundary.parameters.iter().map(|p| if p.ty == ScalarType::Bool { Some(None) }
        else { Envelope::whole(&p.ty).map(Some) }).collect()
}

fn join(left: Lanes, right: Lanes) -> Lanes {
    match (left,right) {
        (Some(left),Some(right)) => {
            if Rc::ptr_eq(&left,&right) { return Some(left); }
            Some(Rc::new(Rc::unwrap_or_clone(left).into_iter().zip(Rc::unwrap_or_clone(right)).map(|(l,r)| match (l,r) {
                (Some(l),Some(r)) => Some(l.hull(r)),
                _ => None,
            }).collect()))
        }
        (left,right) => left.or(right),
    }
}

fn intersect_comparison(
    target: &mut Envelope, op: &super::super::BinaryOperator, want: bool, other: &Envelope, reverse: bool,
) -> Option<()> {
    use super::super::BinaryOperator::*;
    if let Some((hi, strict)) = match (op,want,reverse) {
        (Less,true,false) | (LessEqual,false,true) | (Greater,true,true) | (GreaterEqual,false,false) => Some((true,true)),
        (LessEqual,true,false) | (Less,false,true) | (GreaterEqual,true,true) | (Greater,false,false) => Some((true,false)),
        (Greater,true,false) | (GreaterEqual,false,true) | (Less,true,true) | (LessEqual,false,false) => Some((false,true)),
        (GreaterEqual,true,false) | (Greater,false,true) | (LessEqual,true,true) | (Less,false,false) => Some((false,false)),
        (Equal,true,_) | (NotEqual,false,_) => {
            target.low = std::cmp::max(target.low.clone(),other.low.clone());
            target.high = std::cmp::min(target.high.clone(),other.high.clone());
            None
        }
        (Equal,false,_) | (NotEqual,true,_) => None,
        _ => return None,
    } {
        if hi {
            target.high = std::cmp::min(target.high.clone(), &other.high - u8::from(strict));
        } else {
            target.low = std::cmp::max(target.low.clone(), &other.low + u8::from(strict));
        }
    }
    Some(())
}

#[test]
fn signed_fixed_divisor_envelopes_cover_every_returning_value() {
    use super::super::BinaryOperator::{Divide, Remainder};
    let range = |low: i32, high: i32| Envelope { low: low.into(), high: high.into() };
    for divisor in [-12i32, -4, -1, 1, 4, 12] {
        for lo in -9..=9 {
            for hi in lo..=9 {
                let argument = range(lo, hi);
                let denominator = range(divisor, divisor);
                let quo = argument.clone().binary(&Divide, denominator.clone(), &ScalarType::I32).unwrap();
                let rem = argument.binary(&Remainder, denominator, &ScalarType::I32).unwrap();
                let quotients = [lo / divisor, hi / divisor];
                assert_eq!(quo, range(*quotients.iter().min().unwrap(), *quotients.iter().max().unwrap()));
                for x in lo..=hi {
                    let exact = BigInt::from(x % divisor);
                    assert!(rem.low <= exact && exact <= rem.high);
                }
            }
        }
    }
    for operation in [Divide,Remainder] {
        assert_eq!(range(i32::MIN,i32::MIN).binary(&operation,range(-1,-1),&ScalarType::I32),
            Some(range(i32::MIN,i32::MAX)));
        assert_eq!(range(1,9).binary(&operation,range(0,0),&ScalarType::I32),
            Some(range(i32::MIN,i32::MAX)));
    }
}
