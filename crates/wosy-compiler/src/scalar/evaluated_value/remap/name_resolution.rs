use super::*;
mod original_inventory;
mod counterpart;
pub(super) use counterpart::validate as validate_counterpart;
pub(super) use original_inventory::{forest as validate_original_forest, owned as validate_owned_originals};

impl ValueSchemaRemap {
    pub(super) fn name_declaration(&self, declaration: &SourceNameDeclarationRef) -> Result<SourceNameDeclarationRef, ValueSchemaRemapError> {
        Ok(SourceNameDeclarationRef {
            owner: self.producer(&declaration.owner)?,
            site: match &declaration.site {
                SourceNameDeclarationSite::Parameter { ordinal } => SourceNameDeclarationSite::Parameter { ordinal: *ordinal },
                SourceNameDeclarationSite::Receiver { occurrence, ordinal } => SourceNameDeclarationSite::Receiver { occurrence: occurrence.clone(), ordinal: *ordinal },
                SourceNameDeclarationSite::Assignment { occurrence, ordinal } => SourceNameDeclarationSite::Assignment { occurrence: occurrence.clone(), ordinal: *ordinal },
                SourceNameDeclarationSite::Capture { event, ordinal } => SourceNameDeclarationSite::Capture { event: self.source_event(event)?, ordinal: *ordinal },
            },
        })
    }

    pub(super) fn name_read(&self, fact: &SourceNameReadFact) -> Result<SourceNameReadFact, ValueSchemaRemapError> {
        Ok(SourceNameReadFact {
            occurrence: fact.occurrence.clone(), expression: fact.expression.clone(),
            event: self.source_event(&fact.event)?, value: self.value(&fact.value)?,
            binding: self.binding(&fact.event.owner, &fact.binding)?,
            declaration: self.name_declaration(&fact.declaration)?,
            original_declaration: fact.original_declaration.clone(), role: fact.role.clone(),
        })
    }
}
