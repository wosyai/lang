//! Render a verified diagnostic response against its immutable local source.
use super::diagnostics::{diagnostic_deadline, require_diagnostic_time, within_diagnostic_deadline};
use super::{
    checked_file_key, load_usage, read_selection, CapturedBuildDiagnostics, DiagnosticExchange,
    RequiredSourceReplay, Result, SourceReplay, SqliteSourceState, StoredNodes,
};
use codespan_reporting::term::termcolor::WriteColor;
use rusqlite::TransactionBehavior;
use std::{collections::{BTreeMap, HashMap}, future::{poll_fn, Future}, pin::pin, task::Poll, time::Duration};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::Instant;
use wosy_compiler::Diagnostic;
use wosy_project::PackageRelativePath;
use wosy_protocol::{
    walk_cst, BuildDiagnosticResponse, BuildSessionRefusal, CstEvent, CstExpansionBudget,
    BuildSourceSelection, FileEntry, FileKey,
};
use wosy_syntax::SourceIdentity;

pub enum ProjectDiagnosticExchange {
    StaleSelection,
    ReplayRequired(RequiredSourceReplay),
    Refused(BuildSessionRefusal),
    Report(RootedDiagnosticReport),
}

pub enum AcknowledgedBuildDiagnostics {
    NotAcknowledged,
    StaleSelection,
    ReplayNotAccepted,
    Refused(BuildSessionRefusal),
    Report(RootedDiagnosticReport),
}

impl SqliteSourceState {
    /// Analyze the acknowledged source, using a trusted adapter to open a
    /// fresh, exclusively owned stream for its exact project/target/root.
    /// After a terminal replay request it may open one more stream with the
    /// same captured SQLite generation's bounded canonical source. A later
    /// source (even identical bytes newly acknowledged) is never substituted.
    ///
    /// One deadline covers both openings and framed exchanges. The adapter
    /// must cooperate with asynchronous cancellation; a synchronous call
    /// or a running poll cannot be preempted. Replay and authored projection
    /// retain their separate read budgets. This neither advances the
    /// selection nor emits artifacts.
    pub async fn acknowledged_build_diagnostics<Open, Opening, Stream>(
        &mut self, target: &str, profile: &str, timeout: Duration, budget: CstExpansionBudget, mut open: Open,
    ) -> Result<AcknowledgedBuildDiagnostics>
    where
        Open: FnMut(BuildSourceSelection, Option<SourceReplay>) -> Opening,
        Opening: Future<Output = Result<Stream>>,
        Stream: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        use AcknowledgedBuildDiagnostics as Outcome;
        let deadline = diagnostic_deadline(timeout)?;
        let Some(captured) = self.capture_build_diagnostics(target, profile)? else {
            return Ok(Outcome::NotAcknowledged);
        };
        let answer = within_diagnostic_deadline(deadline, async {
            let first = open_exchange(self, &captured, deadline, None, &mut open).await?;
            let DiagnosticExchange::ReplayRequired(challenge) = first else { return Ok(first) };
            let Some(replay) = challenge.prepare(self)? else { return Ok(DiagnosticExchange::StaleSelection) };
            open_exchange(self, &captured, deadline, Some(replay), &mut open).await
        }).await?;
        Ok(match captured.finish_project_exchange(self, answer, budget)? {
            ProjectDiagnosticExchange::StaleSelection => Outcome::StaleSelection,
            ProjectDiagnosticExchange::ReplayRequired(_) => Outcome::ReplayNotAccepted,
            ProjectDiagnosticExchange::Refused(reason) => Outcome::Refused(reason),
            ProjectDiagnosticExchange::Report(report) => Outcome::Report(report),
        })
    }
}

async fn open_exchange<Open, Opening, Stream>(
    local: &mut SqliteSourceState, captured: &CapturedBuildDiagnostics, deadline: Instant,
    replay: Option<SourceReplay>, open: &mut Open,
) -> Result<DiagnosticExchange>
where
    Open: FnMut(BuildSourceSelection, Option<SourceReplay>) -> Opening,
    Opening: Future<Output = Result<Stream>>,
    Stream: AsyncRead + AsyncWrite + Unpin + 'static,
{
    require_diagnostic_time(deadline)?;
    let mut opening = pin!(open(captured.selection().clone(), replay));
    let stream = poll_fn(|cx| match require_diagnostic_time(deadline) {
        Ok(()) => opening.as_mut().poll(cx),
        Err(expired) => Poll::Ready(Err(expired)),
    }).await?;
    require_diagnostic_time(deadline)?;
    captured.converse_current(local, stream, deadline).await
}

/// Renderable diagnostics from one opened source-only session. The owned
/// snippets describe the authenticated revision, not later workspace edits.
/// No artifact, Build verdict, or Check result is implied.
pub struct RootedDiagnosticReport {
    response: BuildDiagnosticResponse<Diagnostic>,
    sources: HashMap<SourceIdentity, String>,
}

impl RootedDiagnosticReport {
    pub fn response(&self) -> &BuildDiagnosticResponse<Diagnostic> { &self.response }

    pub fn emit_to(&self, output: &mut impl WriteColor) -> Result<()> {
        let BuildDiagnosticResponse::BuildDiagnostics { diagnostics, .. } = &self.response;
        for diagnostic in diagnostics {
            crate::diagnostic_output::emit_project_diagnostic(output, diagnostic, |source| {
                self.sources.get(source).map(String::as_str)
            })?;
        }
        Ok(())
    }
}

impl CapturedBuildDiagnostics {
    /// Use the existing typed leased conversation and the ordinary project
    /// renderer. All labels are checked before a report can reach the writer.
    /// The caller supplies a budget for expanded authored CST occurrences
    /// and bytes; the frame timeout only bounds the asynchronous exchange.
    pub async fn exchange_project_diagnostics<T: AsyncRead + AsyncWrite + Unpin + 'static>(
        &self, local: &mut SqliteSourceState, stream: T, timeout: Duration, budget: CstExpansionBudget,
    ) -> Result<ProjectDiagnosticExchange> {
        let answer = self.exchange_unprojected(local, stream, timeout).await?;
        self.finish_project_exchange(local, answer, budget)
    }

    fn finish_project_exchange(
        &self, local: &mut SqliteSourceState, answer: DiagnosticExchange, budget: CstExpansionBudget,
    ) -> Result<ProjectDiagnosticExchange> {
        use ProjectDiagnosticExchange as Outcome;
        match answer {
            DiagnosticExchange::StaleSelection => Ok(Outcome::StaleSelection),
            DiagnosticExchange::Diagnostics(response) =>
                Ok(self.project_report(local, response, budget)?.map_or(Outcome::StaleSelection, Outcome::Report)),
            DiagnosticExchange::ReplayRequired(replay) if self.is_current(local)? => Ok(Outcome::ReplayRequired(replay)),
            DiagnosticExchange::Refused(reason) if self.is_current(local)? => Ok(Outcome::Refused(reason)),
            _ => Ok(Outcome::StaleSelection),
        }
    }

    fn project_report(
        &self, local: &mut SqliteSourceState, response: BuildDiagnosticResponse<Diagnostic>, mut budget: CstExpansionBudget,
    ) -> Result<Option<RootedDiagnosticReport>> {
        let tx = local.db.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let selected = self.selection();
        if !self.source.matches(read_selection(&tx, &selected.target)?) {
            return Ok(None);
        }
        let BuildDiagnosticResponse::BuildDiagnostics { diagnostics, profile, .. } = &response;
        if let Some(usage) = load_usage(&tx, selected.root)? { usage.admit(local.limits)?; }
        let mut read = StoredNodes::new(&tx)?;
        let (config, files) = SqliteSourceState::header_using(&local.project, &selected.target, selected.root,
            |id| read.charged_record(id, false, |bytes| budget.read(bytes)))?;
        if !config.profiles.contains_key(profile) { return Err("diagnostic profile is not in the selected source".into()); }
        let mut requested = BTreeMap::new();
        for label in diagnostics.iter().flat_map(|diagnostic| &diagnostic.labels) {
            let source = &label.span.source;
            let key = FileKey {
                package: source.package.clone(),
                path: PackageRelativePath::new(source.path.clone().into())?,
            };
            if requested.insert(key, source).is_some_and(|old| old != source) {
                return Err("diagnostic labels requested conflicting source revisions".into());
            }
        }

        let mut sources = HashMap::new();
        if !requested.is_empty() {
            if files.len() > local.limits.nodes { return Err("diagnostic source file index exceeds the node budget".into()); }
            let parser = config.parser.id();
            for id in files {
                let node = read.charged_record(id, false, |bytes| budget.read(bytes))?;
                let file = FileEntry::from_node(id, node)?;
                let key = checked_file_key(&config, parser, &file)?;
                let Some(source) = requested.get(&key) else { continue; };
                if file.metadata.source.project != source.project || file.metadata.source.package != source.package
                    || file.metadata.source.path.as_path().to_str() != Some(source.path.as_str())
                    || file.metadata.revision.hex() != source.revision
                {
                    return Err("diagnostic source does not match the authored file identity".into());
                }
                if sources.contains_key(*source) { return Err("duplicate diagnostic file in source index".into()); }
                let text = materialize_file(&mut read, &mut budget, &file)?;
                sources.insert((*source).clone(), text);
            }
        }

        for label in diagnostics.iter().flat_map(|diagnostic| &diagnostic.labels) {
            let text = sources.get(&label.span.source).ok_or("diagnostic label is outside the selected source")?;
            let span = label.span.range;
            if span.start > span.end || !text.is_char_boundary(span.start as usize) || !text.is_char_boundary(span.end as usize) {
                return Err("diagnostic label is outside authored UTF-8 boundaries".into());
            }
        }
        drop(read);
        tx.commit()?;
        Ok(Some(RootedDiagnosticReport { response, sources }))
    }
}

fn materialize_file(read: &mut StoredNodes<'_>, budget: &mut CstExpansionBudget, file: &FileEntry) -> Result<String> {
    let bytes = file.metadata.bytes as usize;
    budget.file(bytes)?;
    let mut state = (read, budget, String::with_capacity(bytes));
    walk_cst(file.authored, &mut state, |(read, budget, _), id| {
        read.charged_record(id, true, |bytes| budget.read(bytes))
    }, |(_, budget, text), event| {
        match event {
            CstEvent::Start { depth, children, .. } => budget.children(depth, children)?,
            CstEvent::Token { text: token, .. } => {
                if token.len() > bytes - text.len() { return Err("diagnostic file exceeds its authored extent".into()); }
                text.push_str(token);
            }
            CstEvent::Finish => {}
        }
        Ok(())
    })?;
    let text = state.2;
    if text.len() != bytes || blake3::hash(text.as_bytes()).as_bytes() != &file.metadata.revision.0 {
        return Err("diagnostic text does not match its authored revision".into());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_recovery::ClientSourceLimits;
    use wosy_compiler::{DiagnosticLabel, DiagnosticLabelKind, DiagnosticSeverity};
    use wosy_protocol::{BuildAnalysisOrigin, BuildAnalysisPhase, NodeId, BUILD_DIAGNOSTIC_VERSION};
    use wosy_syntax::{ByteSpan, SourceSpan};

    #[test]
    #[ignore = "requires a disposable SQLite project with a selected app root and a UTF-8 source comment"]
    fn verify_report_admission_before_a_writer_is_available() {
        let project = std::path::PathBuf::from(std::env::var_os("WOSY_REPORT_PROJECT").unwrap());
        let mut local = SqliteSourceState::open(&project, ClientSourceLimits {
            nodes: 25_000, encoded_bytes: 16 << 20,
        }).unwrap();
        let capture = local.capture_build_diagnostics("app", "dev").unwrap().unwrap();
        {
            let mut records = StoredNodes::new(&local.db).unwrap();
            let root = capture.selection().root;
            let mut limited = CstExpansionBudget::new(0, 1, 1, 0);
            assert!(records.charged_record(root, true, |bytes| limited.read(bytes)).is_err());
            assert!(records.checked.nodes.is_empty());
            assert_eq!(records.count, 1);
            let mut enough = CstExpansionBudget::new(1024, 1, 1, 0);
            records.charged_record(root, true, |bytes| enough.read(bytes)).unwrap();
            assert!(records.checked.nodes.contains_key(&root));
            assert_eq!(records.count, 2);
            assert!(records.charged_record(root, true, |bytes| limited.read(bytes)).is_err());
            assert_eq!(records.count, 2);
        }
        let text = std::fs::read_to_string(project.join("src/main.w")).unwrap();
        let unicode = text.find('☕').unwrap() as u32;
        let source = SourceIdentity::new(capture.selection().project.clone(), "app".into(), "src/main.w".into(),
            NodeId(*blake3::hash(text.as_bytes()).as_bytes()).hex());
        let report = |source: SourceIdentity, range: ByteSpan| BuildDiagnosticResponse::BuildDiagnostics {
            version: BUILD_DIAGNOSTIC_VERSION, source_root: capture.selection().root,
            profile: "dev".into(), preparation: NodeId([12; 32]), receipt: NodeId([13; 32]),
            phase: BuildAnalysisPhase::Analyzed, origin: BuildAnalysisOrigin::Derived,
            diagnostics: vec![Diagnostic {
                code: "fixture".into(), severity: DiagnosticSeverity::Error, message: "fixture".into(),
                notes: vec![], labels: vec![DiagnosticLabel { kind: DiagnosticLabelKind::Primary,
                    span: SourceSpan::new(source, range), message: "exact authored span".into(),
                }],
            }],
        };
        let budget = || CstExpansionBudget::new(16 << 20, 20_000, 1024, 8 << 20);
        let range = ByteSpan::new(unicode, unicode + 3);
        assert!(capture.project_report(&mut local, report(source.clone(), range),
            CstExpansionBudget::new(0, 20_000, 1024, 8 << 20)).is_err());
        assert!(capture.project_report(&mut local, report(source.clone(), range), budget()).unwrap().is_some());
        let mut wrong = source.clone();
        wrong.revision = NodeId([99; 32]).hex();
        assert!(capture.project_report(&mut local, report(wrong, range), budget()).is_err());
        let mut wrong = source.clone();
        wrong.path = "src/ordinary-but-unseen.w".into();
        assert!(capture.project_report(&mut local, report(wrong, range), budget()).is_err());
        let mut wrong = source.clone();
        wrong.project.push_str("-different");
        assert!(capture.project_report(&mut local, report(wrong, range), budget()).is_err());
        assert!(capture.project_report(&mut local, report(source, ByteSpan::new(unicode + 1, unicode + 3)),
            budget()).is_err());
    }
}
