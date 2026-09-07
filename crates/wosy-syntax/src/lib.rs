use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SourceIdentity {
    pub project: String,
    pub package: String,
    pub path: String,
    pub revision: String,
}

impl SourceIdentity {
    pub fn new(project: String, package: String, path: String, revision: String) -> Self {
        Self {
            project,
            package,
            path,
            revision,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ByteSpan {
    pub start: u32,
    pub end: u32,
}

impl ByteSpan {
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SourceSpan {
    pub source: SourceIdentity,
    pub range: ByteSpan,
}

impl SourceSpan {
    pub fn new(source: SourceIdentity, range: ByteSpan) -> Self {
        Self { source, range }
    }
}

#[cfg(test)]
mod tests {
    use super::{ByteSpan, SourceIdentity, SourceSpan};

    #[test]
    fn source_span_serializes_as_structured_data() {
        let source = SourceIdentity::new(
            "project-a".to_owned(),
            "main".to_owned(),
            "src/main.w".to_owned(),
            "revision-1".to_owned(),
        );
        let span = SourceSpan::new(source, ByteSpan::new(4, 12));

        let value = serde_json::to_value(&span).expect("source span serialization");

        assert_eq!(value["source"]["path"], "src/main.w");
        assert_eq!(value["range"]["start"], 4);
        assert_eq!(value["range"]["end"], 12);
    }
}
