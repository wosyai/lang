use serde::{Deserialize, Serialize};
use wosy_syntax::SourceSpan;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Note,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DiagnosticLabelKind {
    Primary,
    Secondary,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticLabel {
    pub kind: DiagnosticLabelKind,
    pub span: SourceSpan,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub labels: Vec<DiagnosticLabel>,
    pub notes: Vec<String>,
}

impl Diagnostic {
    pub fn new(
        code: String,
        severity: DiagnosticSeverity,
        message: String,
        labels: Vec<DiagnosticLabel>,
        notes: Vec<String>,
    ) -> Self {
        Self {
            code,
            severity,
            message,
            labels,
            notes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Diagnostic, DiagnosticLabel, DiagnosticLabelKind, DiagnosticSeverity};
    use wosy_syntax::{ByteSpan, SourceIdentity, SourceSpan};

    #[test]
    fn diagnostic_serializes_source_ranges_and_labels() {
        let source = SourceIdentity::new(
            "project-a".to_owned(),
            "main".to_owned(),
            "src/main.w".to_owned(),
            "revision-1".to_owned(),
        );
        let diagnostic = Diagnostic::new(
            "S0001".to_owned(),
            DiagnosticSeverity::Error,
            "invalid source".to_owned(),
            vec![DiagnosticLabel {
                kind: DiagnosticLabelKind::Primary,
                span: SourceSpan::new(source, ByteSpan::new(10, 15)),
                message: "source starts here".to_owned(),
            }],
            vec!["repair the declaration".to_owned()],
        );

        let value = serde_json::to_value(&diagnostic).expect("diagnostic serialization");

        assert_eq!(value["code"], "S0001");
        assert_eq!(value["labels"][0]["kind"], "Primary");
        assert_eq!(value["labels"][0]["span"]["range"]["start"], 10);
    }
}
