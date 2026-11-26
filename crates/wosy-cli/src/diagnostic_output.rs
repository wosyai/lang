//! The ordinary project diagnostic rendering shared by both client routes.
use codespan_reporting::{
    diagnostic::{Diagnostic as Report, Label},
    files::{Error, SimpleFiles},
    term::{self, termcolor::WriteColor},
};
use wosy_compiler::{Diagnostic, DiagnosticLabelKind};
use wosy_syntax::SourceIdentity;

pub fn emit_project_diagnostic<'a>(
    output: &mut impl WriteColor, diagnostic: &Diagnostic, mut source: impl FnMut(&SourceIdentity) -> Option<&'a str>,
) -> Result<(), Error> {
    let mut files = SimpleFiles::new();
    let labels = diagnostic.labels.iter().filter_map(|label| {
        let text = source(&label.span.source)?;
        let file = files.add(&label.span.source.path, text);
        let range = label.span.range.start as usize..label.span.range.end as usize;
        Some(match label.kind {
            DiagnosticLabelKind::Primary => Label::primary(file, range),
            DiagnosticLabelKind::Secondary => Label::secondary(file, range),
        }.with_message(label.message.clone()))
    }).collect();
    let report = Report::error()
        .with_code(diagnostic.code.clone())
        .with_message(diagnostic.message.clone())
        .with_labels(labels)
        .with_notes(diagnostic.notes.clone());
    term::emit(output, &term::Config::default(), &files, &report)
}
