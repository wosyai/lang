use super::{CfgPointId, ProducerCfgOwnerId, ScalarReferenceMutability, SourceSpan};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct LoanSupportId(pub usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct LoanClassId(pub usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct LoanTargetId(pub usize);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LoanGrantSite {
    pub owner: ProducerCfgOwnerId,
    pub point: CfgPointId,
    pub source_span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LoanClassFact {
    pub site: LoanGrantSite,
    pub target: LoanTargetId,
    pub mode: ScalarReferenceMutability,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum LoanSupport {
    Unavailable,
    Null(SourceSpan),
    Invalid(SourceSpan),
    Available(LoanClassId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LoanRelation {
    supports: Vec<LoanSupport>,
    classes: Vec<LoanClassFact>,
    ancestors: BTreeSet<(LoanClassId, LoanClassId)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LoanPair {
    Equal,
    Ancestor,
    Descendant,
    Unrelated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LoanParentWitness {
    pub site: LoanGrantSite,
    pub parent_support: LoanSupportId,
    pub parent: LoanClassId,
    pub child: LoanClassId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LoanTargetSubstitution {
    pub from: LoanTargetId,
    pub to: LoanTargetId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LoanSiteSubstitution {
    pub from: LoanGrantSite,
    pub to: LoanGrantSite,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum LoanRelationError {
    Unavailable(LoanSupportId),
    Null {
        support: LoanSupportId,
        source_span: SourceSpan,
    },
    Invalid {
        support: LoanSupportId,
        source_span: SourceSpan,
    },
    MoveOnlyCopy(LoanSupportId),
    SameSupportTransfer(LoanSupportId),
    InvalidSupportOrder,
    MissingTarget(LoanTargetId),
    DuplicateTarget(LoanTargetId),
    ForeignTarget(LoanTargetId),
    MissingSite(LoanGrantSite),
    DuplicateSite(LoanGrantSite),
    ForeignSite(LoanGrantSite),
}

impl LoanRelation {
    pub(super) fn new(support_count: usize) -> Self {
        Self {
            supports: vec![LoanSupport::Unavailable; support_count],
            classes: Vec::new(),
            ancestors: BTreeSet::new(),
        }
    }

    pub(super) fn supports(&self) -> &[LoanSupport] {
        &self.supports
    }

    pub(super) fn classes(&self) -> &[LoanClassFact] {
        &self.classes
    }

    pub(super) fn ancestors(&self) -> &BTreeSet<(LoanClassId, LoanClassId)> {
        &self.ancestors
    }

    fn available(&self, support: LoanSupportId) -> Result<LoanClassId, LoanRelationError> {
        match &self.supports[support.0] {
            LoanSupport::Available(class) => Ok(*class),
            LoanSupport::Unavailable => Err(LoanRelationError::Unavailable(support)),
            LoanSupport::Null(source_span) => Err(LoanRelationError::Null {
                support,
                source_span: source_span.clone(),
            }),
            LoanSupport::Invalid(source_span) => Err(LoanRelationError::Invalid {
                support,
                source_span: source_span.clone(),
            }),
        }
    }

    fn canonicalize(&mut self) -> BTreeMap<LoanClassId, LoanClassId> {
        let mut renaming = BTreeMap::new();
        let mut classes = Vec::new();
        for support in &mut self.supports {
            if let LoanSupport::Available(old) = support {
                let new = *renaming.entry(*old).or_insert_with(|| {
                    let new = LoanClassId(classes.len());
                    classes.push(self.classes[old.0].clone());
                    new
                });
                *old = new;
            }
        }
        self.ancestors = self
            .ancestors
            .iter()
            .filter_map(
                |(left, right)| match (renaming.get(left), renaming.get(right)) {
                    (Some(left), Some(right)) => Some((*left, *right)),
                    _ => None,
                },
            )
            .collect();
        self.classes = classes;
        renaming
    }

    pub(super) fn create_root(
        &mut self,
        destination: LoanSupportId,
        fact: LoanClassFact,
    ) -> LoanClassId {
        let fresh = LoanClassId(self.classes.len());
        self.classes.push(fact);
        self.supports[destination.0] = LoanSupport::Available(fresh);
        self.canonicalize()[&fresh]
    }

    pub(super) fn create_child(
        &mut self,
        parent_support: LoanSupportId,
        destination: LoanSupportId,
        fact: LoanClassFact,
    ) -> Result<LoanParentWitness, LoanRelationError> {
        let parent = self.available(parent_support)?;
        let child = LoanClassId(self.classes.len());
        for index in 0..self.classes.len() {
            let ancestor = LoanClassId(index);
            if ancestor == parent || self.ancestors.contains(&(ancestor, parent)) {
                self.ancestors.insert((ancestor, child));
            }
        }
        let site = fact.site.clone();
        self.classes.push(fact);
        self.supports[destination.0] = LoanSupport::Available(child);
        let renaming = self.canonicalize();
        Ok(LoanParentWitness {
            site,
            parent_support,
            parent,
            child: renaming[&child],
        })
    }

    pub(super) fn copy(
        &mut self,
        source: LoanSupportId,
        destination: LoanSupportId,
    ) -> Result<(), LoanRelationError> {
        let class = self.available(source)?;
        if self.classes[class.0].mode == ScalarReferenceMutability::Mutable {
            return Err(LoanRelationError::MoveOnlyCopy(source));
        }
        self.supports[destination.0] = LoanSupport::Available(class);
        self.canonicalize();
        Ok(())
    }

    pub(super) fn transfer(
        &mut self,
        source: LoanSupportId,
        destination: LoanSupportId,
    ) -> Result<(), LoanRelationError> {
        if source == destination {
            return Err(LoanRelationError::SameSupportTransfer(source));
        }
        let class = self.available(source)?;
        self.supports[destination.0] = LoanSupport::Available(class);
        self.supports[source.0] = LoanSupport::Unavailable;
        self.canonicalize();
        Ok(())
    }

    pub(super) fn clear(&mut self, support: LoanSupportId) {
        self.supports[support.0] = LoanSupport::Unavailable;
        self.canonicalize();
    }

    pub(super) fn set_null(&mut self, destination: LoanSupportId, source_span: SourceSpan) {
        self.supports[destination.0] = LoanSupport::Null(source_span);
        self.canonicalize();
    }

    pub(super) fn set_invalid(&mut self, destination: LoanSupportId, source_span: SourceSpan) {
        self.supports[destination.0] = LoanSupport::Invalid(source_span);
        self.canonicalize();
    }

    pub(super) fn pair(
        &self,
        left: LoanSupportId,
        right: LoanSupportId,
    ) -> Result<LoanPair, LoanRelationError> {
        let left = self.available(left)?;
        let right = self.available(right)?;
        Ok(if left == right {
            LoanPair::Equal
        } else if self.ancestors.contains(&(left, right)) {
            LoanPair::Ancestor
        } else if self.ancestors.contains(&(right, left)) {
            LoanPair::Descendant
        } else {
            LoanPair::Unrelated
        })
    }

    pub(super) fn remap(
        &self,
        support_order: &[LoanSupportId],
        target_substitutions: &[LoanTargetSubstitution],
        site_substitutions: &[LoanSiteSubstitution],
    ) -> Result<Self, LoanRelationError> {
        let order: BTreeSet<_> = support_order.iter().copied().collect();
        if support_order.len() != self.supports.len()
            || order.len() != self.supports.len()
            || order.iter().any(|support| support.0 >= self.supports.len())
        {
            return Err(LoanRelationError::InvalidSupportOrder);
        }
        let represented_targets: BTreeSet<_> =
            self.classes.iter().map(|fact| fact.target).collect();
        let mut targets = BTreeMap::new();
        for row in target_substitutions {
            if !represented_targets.contains(&row.from) {
                return Err(LoanRelationError::ForeignTarget(row.from));
            }
            if targets.insert(row.from, row.to).is_some() {
                return Err(LoanRelationError::DuplicateTarget(row.from));
            }
        }
        for target in represented_targets {
            if !targets.contains_key(&target) {
                return Err(LoanRelationError::MissingTarget(target));
            }
        }
        let mut sites: Vec<&LoanSiteSubstitution> = Vec::new();
        for row in site_substitutions {
            if !self.classes.iter().any(|fact| fact.site == row.from) {
                return Err(LoanRelationError::ForeignSite(row.from.clone()));
            }
            if sites.iter().any(|previous| previous.from == row.from) {
                return Err(LoanRelationError::DuplicateSite(row.from.clone()));
            }
            sites.push(row);
        }
        let mut classes = Vec::new();
        for fact in &self.classes {
            let Some(site) = sites.iter().find(|row| row.from == fact.site) else {
                return Err(LoanRelationError::MissingSite(fact.site.clone()));
            };
            classes.push(LoanClassFact {
                site: site.to.clone(),
                target: targets[&fact.target],
                mode: fact.mode,
            });
        }
        let mut result = Self {
            supports: support_order
                .iter()
                .map(|support| self.supports[support.0].clone())
                .collect(),
            classes,
            ancestors: self.ancestors.clone(),
        };
        result.canonicalize();
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
