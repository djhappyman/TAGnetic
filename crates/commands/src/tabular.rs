//! Strict, bounded CSV/TSV mapping into the existing field-plan engine.
//! Cells are data, never formulas. Empty cells require an explicit policy.
use super::*;
use crate::field_operations::invalid;
use tagrex_core::resources::{Budget, Charge, Lease};
use tagrex_core::text_codec::{self, TextEncoding};

const MAX_INPUT: usize = 8 * 1024 * 1024;
const MAX_FIELD: usize = 1024 * 1024;
const MAX_COLUMNS: usize = 256;
const MAX_RECORDS: usize = 100_000;
const MAX_RETAINED: usize = 64 * 1024 * 1024;

#[cfg(test)]
#[path = "../tests/support/tabular_parser_adversarial.rs"]
mod adversarial_tests;

fn charge(
    bytes: &mut usize,
    additional: usize,
    lease: &mut Lease,
) -> Result<(), TabularImportError> {
    let next = bytes
        .checked_add(additional)
        .filter(|size| *size <= MAX_RETAINED)
        .ok_or_else(|| invalid("Tabular import intermediates exceed 64 MiB."))?;
    lease.try_resize(Charge {
        transient: next.saturating_add(2 * MAX_INPUT),
        jobs: 1,
        ..Charge::default()
    })?;
    *bytes = next;
    Ok(())
}

/// A zero-based column index or an exact, unique header name.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ImportColumn {
    Index { index: usize },
    Header { name: String },
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmptyCell {
    #[default]
    Skip,
    Clear,
    Reject,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnMapping {
    pub column: ImportColumn,
    pub field: String,
    #[serde(default)]
    pub empty: EmptyCell,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TabularImportOptions {
    /// An ASCII delimiter; comma and tab are the usual choices.
    pub delimiter: char,
    /// An ASCII quote, doubled inside quoted cells.
    pub quote: char,
    pub headers: bool,
    pub encoding: String,
    pub mode: TextImportMode,
    pub match_column: Option<ImportColumn>,
    pub mappings: Vec<ColumnMapping>,
}
impl Default for TabularImportOptions {
    fn default() -> Self {
        Self {
            delimiter: ',',
            quote: '"',
            headers: true,
            encoding: "utf-8".into(),
            mode: TextImportMode::Sequential,
            match_column: None,
            mappings: Vec::new(),
        }
    }
}

/// Source coordinates are one-based; `column` identifies the logical cell.
#[derive(Debug, Error)]
pub enum TabularImportError {
    #[error(transparent)]
    Resource(#[from] tagrex_core::resources::AdmissionError),
    #[error("Record {record}, line {line}, column {column}: {reason}")]
    Record {
        record: usize,
        line: usize,
        column: usize,
        reason: String,
    },
    #[error(transparent)]
    Native(#[from] AppError),
}
impl From<TabularImportError> for ErrorDto {
    fn from(error: TabularImportError) -> Self {
        match error {
            TabularImportError::Native(inner) => inner.into(),
            TabularImportError::Resource(inner) => AppError::Resource(inner).into(),
            other => Self::plain(other.to_string()),
        }
    }
}
fn record_error(record: usize, line: usize, column: usize, reason: &str) -> TabularImportError {
    TabularImportError::Record {
        record,
        line,
        column,
        reason: reason.into(),
    }
}

#[derive(Debug)]
struct Record {
    cells: Vec<String>,
    number: usize,
    line: usize,
}
struct Records<'a> {
    input: &'a [u8],
    offset: usize,
    line: usize,
    number: usize,
    delimiter: u8,
    quote: u8,
}
impl<'a> Records<'a> {
    fn new(text: &'a str, delimiter: u8, quote: u8) -> Self {
        Self {
            input: text.as_bytes(),
            offset: 0,
            line: 1,
            number: 0,
            delimiter,
            quote,
        }
    }
    fn next(
        &mut self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<Record>, TabularImportError> {
        let mut next_check = self.offset;
        // Empty physical lines outside quoted records are ignored. Whitespace
        // and a quoted empty field remain meaningful input.
        while self.offset < self.input.len() && matches!(self.input[self.offset], b'\r' | b'\n') {
            self.check_cancel(&mut next_check, cancelled)?;
            self.newline();
        }
        if self.offset == self.input.len() {
            return Ok(None);
        }
        self.number += 1;
        if self.number > MAX_RECORDS + 1 {
            return Err(record_error(self.number, self.line, 1, "too many records"));
        }
        let line = self.line;
        let mut cells = Vec::new();
        loop {
            if cells.len() == MAX_COLUMNS {
                return Err(record_error(
                    self.number,
                    self.line,
                    cells.len() + 1,
                    "too many columns",
                ));
            }
            let quoted = self.input.get(self.offset) == Some(&self.quote);
            if quoted {
                self.offset += 1;
            }
            let mut cell = Vec::new();
            let mut closed = !quoted;
            while self.offset < self.input.len() {
                self.check_cancel(&mut next_check, cancelled)?;
                let byte = self.input[self.offset];
                if quoted && !closed {
                    if byte == self.quote {
                        self.offset += 1;
                        if self.input.get(self.offset) == Some(&self.quote) {
                            cell.push(self.quote);
                            self.offset += 1;
                        } else {
                            closed = true;
                        }
                    } else {
                        if byte == b'\r' || byte == b'\n' {
                            let begin = self.offset;
                            self.newline();
                            cell.extend_from_slice(&self.input[begin..self.offset]);
                        } else {
                            cell.push(byte);
                            self.offset += 1;
                        }
                    }
                } else {
                    if byte == self.delimiter || byte == b'\r' || byte == b'\n' {
                        break;
                    }
                    if quoted || byte == self.quote {
                        return Err(record_error(
                            self.number,
                            self.line,
                            cells.len() + 1,
                            "unexpected text or quote outside a quoted cell",
                        ));
                    }
                    cell.push(byte);
                    self.offset += 1;
                }
                if cell.len() > MAX_FIELD {
                    return Err(record_error(
                        self.number,
                        self.line,
                        cells.len() + 1,
                        "cell exceeds 1 MiB",
                    ));
                }
            }
            if !closed {
                return Err(record_error(
                    self.number,
                    self.line,
                    cells.len() + 1,
                    "unclosed quoted cell",
                ));
            }
            // Delimiters and quotes are ASCII, so valid UTF-8 input cannot be
            // split inside a codepoint. Still handle conversion defensively.
            cells.push(String::from_utf8(cell).map_err(|_| {
                record_error(
                    self.number,
                    self.line,
                    cells.len() + 1,
                    "invalid UTF-8 cell",
                )
            })?);
            if self.input.get(self.offset) == Some(&self.delimiter) {
                self.offset += 1;
                continue;
            }
            if self.offset < self.input.len() {
                self.newline();
            }
            return Ok(Some(Record {
                cells,
                number: self.number,
                line,
            }));
        }
    }
    fn newline(&mut self) {
        let cr = self.input[self.offset] == b'\r';
        self.offset += 1;
        if cr && self.input.get(self.offset) == Some(&b'\n') {
            self.offset += 1;
        }
        self.line += 1;
    }
    fn check_cancel(
        &self,
        next_check: &mut usize,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), TabularImportError> {
        // Two-byte escapes/newlines can skip every equality/modulo checkpoint.
        if self.offset >= *next_check {
            *next_check = self.offset.saturating_add(4096);
            if cancelled() {
                return Err(AppError::from(tagrex_core::plan::PlanError::Cancelled).into());
            }
        }
        Ok(())
    }
}

fn column_index(column: &ImportColumn, headers: Option<&Record>) -> Result<usize, AppError> {
    match column {
        ImportColumn::Index { index }
            if *index < MAX_COLUMNS && headers.is_none_or(|header| *index < header.cells.len()) =>
        {
            Ok(*index)
        }
        ImportColumn::Index { .. } => Err(invalid("Column index must be below 256.")),
        ImportColumn::Header { name } => headers
            .and_then(|r| r.cells.iter().position(|cell| cell == name))
            .ok_or_else(|| invalid(format!("Missing exact header: {name}."))),
    }
}

impl App {
    /// Parse spreadsheet data and return an ordinary staged plan plus row
    /// diagnostics. This does not mutate the live stage or any audio file.
    pub fn preview_tabular_import(
        &self,
        paths: &[PathBuf],
        input: &[u8],
        options: &TabularImportOptions,
        working: Option<&PlanDto>,
    ) -> Result<TextImportPreview, TabularImportError> {
        self.preview_tabular_import_with_cancel(paths, input, options, working, || false)
    }

    pub fn preview_tabular_import_with_cancel(
        &self,
        paths: &[PathBuf],
        input: &[u8],
        options: &TabularImportOptions,
        working: Option<&PlanDto>,
        cancelled: impl Fn() -> bool,
    ) -> Result<TextImportPreview, TabularImportError> {
        let _policy = self.model_policy.enter();
        let policy_revision = tagrex_core::model::read_policy_revision();
        let check_cancel = || -> Result<(), TabularImportError> {
            if cancelled() {
                Err(AppError::from(tagrex_core::plan::PlanError::Cancelled).into())
            } else if policy_revision == u64::MAX
                || tagrex_core::model::read_policy_revision() != policy_revision
            {
                Err(invalid(
                    "Metadata settings changed during the table import; preview it again.",
                )
                .into())
            } else {
                Ok(())
            }
        };
        check_cancel()?;
        if input.len() > MAX_INPUT || paths.len() > MAX_RECORDS {
            return Err(
                invalid("Tabular imports allow 8 MiB input and 100000 selected entries.").into(),
            );
        }
        // Decode and the current record may each expand to MAX_INPUT. Shared
        // admission precedes those allocations; retained rows grow this lease.
        let mut admission = Budget::process().try_acquire(Charge {
            transient: 3 * MAX_INPUT,
            jobs: 1,
            ..Charge::default()
        })?;
        if !options.delimiter.is_ascii()
            || !options.quote.is_ascii()
            || options.delimiter == options.quote
            || matches!(options.delimiter, '\0' | '\r' | '\n')
            || options.quote.is_control()
            || options.delimiter.is_ascii_control() && options.delimiter != '\t'
        {
            return Err(invalid("Choose distinct ASCII delimiter and quote characters; only tab may be a control delimiter.").into());
        }
        if options.mappings.is_empty() || options.mappings.len() > MAX_COLUMNS {
            return Err(invalid("Supply between 1 and 256 column mappings.").into());
        }
        if (options.mode == TextImportMode::Sequential) != options.match_column.is_none() {
            return Err(invalid(
                "Filename/path matching requires a match column; sequential matching must omit it.",
            )
            .into());
        }
        let mut fields = Vec::new();
        for mapping in &options.mappings {
            let field = TagField::from_name(&mapping.field)
                .ok_or_else(|| invalid("Invalid import destination."))?;
            if fields
                .iter()
                .any(|previous: &TagField| previous.same_name(&field))
            {
                return Err(
                    invalid("Two columns cannot assign the same logical destination.").into(),
                );
            }
            fields.push(field);
        }
        // Reuse the destination validator, including reserved structural fields.
        let operations: Vec<_> = options
            .mappings
            .iter()
            .map(|mapping| FieldOperation::Format {
                field: mapping.field.clone(),
                pattern: String::new(),
            })
            .collect();
        self.preview_field_operations(&[], &operations, None)?;
        let encoding =
            TextEncoding::from_label(&options.encoding).map_err(|e| invalid(e.to_string()))?;
        let text = text_codec::decode_with_cancel(input, encoding, MAX_INPUT, &cancelled).map_err(
            |e| {
                if cancelled() {
                    AppError::from(tagrex_core::plan::PlanError::Cancelled)
                } else {
                    invalid(e.to_string())
                }
            },
        )?;
        if text.contains('\0') {
            return Err(invalid("NUL is not allowed in tabular text.").into());
        }
        let mut records = Records::new(&text, options.delimiter as u8, options.quote as u8);
        let header = if options.headers {
            let header = records
                .next(&cancelled)?
                .ok_or_else(|| invalid("The header record is missing."))?;
            let mut names = HashSet::new();
            if header
                .cells
                .iter()
                .any(|name| name.is_empty() || !names.insert(name))
            {
                return Err(record_error(
                    header.number,
                    header.line,
                    1,
                    "headers must be nonempty and unique",
                ));
            }
            Some(header)
        } else {
            None
        };
        let mappings: Vec<_> = options
            .mappings
            .iter()
            .zip(fields)
            .map(|(mapping, field)| {
                Ok((
                    column_index(&mapping.column, header.as_ref())?,
                    field,
                    mapping.empty,
                ))
            })
            .collect::<Result<_, AppError>>()?;
        let key_column = options
            .match_column
            .as_ref()
            .map(|column| column_index(column, header.as_ref()))
            .transpose()?;
        // Bound path copies, row diagnostics, mapped values and field keys
        // before retaining them. The caller input and final plan have separate
        // budgets; this is not a whole-process memory guarantee.
        let mut retained = text.len();
        for path in paths {
            charge(
                &mut retained,
                path.as_os_str().len().saturating_mul(5).saturating_add(512),
                &mut admission,
            )?;
        }
        let mut unique = HashSet::new();
        let paths: Vec<_> = paths
            .iter()
            .filter(|path| unique.insert(path.as_path()))
            .cloned()
            .collect();
        let mut index = HashMap::new();
        for path in &paths {
            check_cancel()?;
            if path.to_str().is_none() {
                return Err(invalid("Tabular matching requires Unicode paths.").into());
            }
            let key = match options.mode {
                TextImportMode::Sequential => continue,
                TextImportMode::Filename => path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| invalid("Missing filename."))?,
                TextImportMode::Path => path.to_str().ok_or_else(|| invalid("Invalid path."))?,
            };
            if index.insert(key, path).is_some() {
                return Err(
                    invalid("Ambiguous selected filenames; use full-path matching.").into(),
                );
            }
        }
        let mut assignments = HashMap::new();
        let mut matched = HashSet::new();
        let mut rows = Vec::new();
        let mut expected_width = header.as_ref().map(|record| record.cells.len());
        while let Some(record) = records.next(&cancelled)? {
            check_cancel()?;
            charge(&mut retained, 256, &mut admission)?;
            if rows.len() == MAX_RECORDS {
                return Err(record_error(
                    record.number,
                    record.line,
                    1,
                    "too many data records",
                ));
            }
            let width = *expected_width.get_or_insert(record.cells.len());
            if record.cells.len() != width {
                return Err(record_error(
                    record.number,
                    record.line,
                    record.cells.len() + 1,
                    "record width differs from the header or first record",
                ));
            }
            let get = |column: usize| {
                record.cells.get(column).ok_or_else(|| {
                    record_error(
                        record.number,
                        record.line,
                        column + 1,
                        "mapped column is missing",
                    )
                })
            };
            // Validate mapped cells even when this row does not match a file.
            let mut tags = TagMap::new();
            for (column, field, empty) in &mappings {
                let value = get(*column)?;
                if value.is_empty() {
                    match empty {
                        EmptyCell::Skip => continue,
                        EmptyCell::Reject => {
                            return Err(record_error(
                                record.number,
                                record.line,
                                column + 1,
                                "empty cell is forbidden by this mapping",
                            ))
                        }
                        EmptyCell::Clear => {}
                    }
                }
                charge(
                    &mut retained,
                    value
                        .len()
                        .saturating_add(field.to_storage_key().len())
                        .saturating_add(128),
                    &mut admission,
                )?;
                tags.insert(field.clone(), value.clone());
            }
            let path = match key_column {
                Some(column) => index.get(get(column)?.as_str()).copied(),
                None => paths.get(rows.len()),
            };
            let Some(path) = path else {
                rows.push(TextImportRow {
                    line: record.line,
                    path: None,
                    fields: Vec::new(),
                    status: if key_column.is_some() {
                        TextImportStatus::FileNotSelected
                    } else {
                        TextImportStatus::NoRemainingFile
                    },
                });
                continue;
            };
            if !matched.insert(path.clone()) {
                return Err(record_error(
                    record.number,
                    record.line,
                    key_column.unwrap_or(0) + 1,
                    "multiple records assign one selected file",
                ));
            }
            let fields = tags.keys().map(TagField::to_storage_key).collect();
            assignments.insert(path.clone(), tags);
            rows.push(TextImportRow {
                line: record.line,
                path: Some(path.to_string_lossy().into()),
                fields,
                status: TextImportStatus::Matched,
            });
        }
        let unmatched_files = paths
            .iter()
            .filter(|path| !matched.contains(*path))
            .map(|path| path.to_string_lossy().into())
            .collect();
        // Reserve the field engine's changed-text/Undo allowance while its
        // output coexists with parsed assignments. The engine retains its own
        // validation and limits; this reservation covers participating work.
        admission.try_resize(Charge {
            transient: retained
                .saturating_add(2 * MAX_INPUT)
                .saturating_add(MAX_RETAINED),
            jobs: 1,
            ..Charge::default()
        })?;
        let plan = self.preview_assignments(&paths, assignments, working, &cancelled)?;
        check_cancel()?;
        Ok(TextImportPreview {
            plan,
            rows,
            unmatched_files,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(text: &str, delimiter: u8) -> Result<Vec<Vec<String>>, TabularImportError> {
        let mut parser = Records::new(text, delimiter, b'"');
        let mut out = Vec::new();
        while let Some(record) = parser.next(&|| false)? {
            out.push(record.cells);
        }
        Ok(out)
    }
    #[test]
    fn parses_independent_python_csv_corpus_without_changing_cells() {
        let corpus: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/tabular-python-csv.json"))
                .unwrap();
        let rows = parse(corpus["csv"].as_str().unwrap(), b',').unwrap();
        let expected: Vec<Vec<String>> = serde_json::from_value(corpus["rows"].clone()).unwrap();
        assert_eq!(rows, expected);
    }
    #[test]
    fn rejects_malformed_quotes_and_reports_source_coordinates() {
        for text in [
            "a,b\n\"unterminated",
            "a,b\n\"closed\"oops,x",
            "a,b\nun\"quoted,x",
        ] {
            let error = parse(text, b',').unwrap_err();
            assert!(matches!(
                error,
                TabularImportError::Record {
                    record: 2,
                    line: 2,
                    column: 1,
                    ..
                }
            ));
        }
        let mut records = Records::new("a,b\r\n\"multi\r\nline\",c\r\nx,y", b',', b'"');
        assert_eq!(records.next(&|| false).unwrap().unwrap().line, 1);
        assert_eq!(records.next(&|| false).unwrap().unwrap().line, 2);
        assert_eq!(records.next(&|| false).unwrap().unwrap().line, 4);
    }
    #[test]
    fn tsv_preserves_empty_cells_spaces_unicode_and_multiline_values() {
        assert_eq!(
            parse("\n\"\"\t A \t\"a\tb\"\t\"α\n🎵\"\t", b'\t').unwrap(),
            vec![vec!["", " A ", "a\tb", "α\n🎵", ""]]
        );
    }
    #[test]
    fn parser_stops_on_capacity_and_cancellation_inside_large_record() {
        assert!(parse(&format!("\"{}\"", "a".repeat(MAX_FIELD + 1)), b',').is_err());
        assert!(parse(&",".repeat(MAX_COLUMNS), b',').is_err());
        let text = "x".repeat(100_000);
        let mut records = Records::new(&text, b',', b'"');
        let calls = Cell::new(0);
        let error = records
            .next(&|| {
                calls.set(calls.get() + 1);
                calls.get() > 2
            })
            .unwrap_err();
        assert!(matches!(
            error,
            TabularImportError::Native(AppError::Plan(tagrex_core::plan::PlanError::Cancelled))
        ));
        assert!(records.offset < text.len());
    }
    #[test]
    fn hostile_short_inputs_never_panic_or_lose_parser_progress() {
        let alphabet = *b"a,\"\r\n";
        // Enumerate all 78,125 length-seven strings, including malformed input.
        for mut seed in 0..78_125usize {
            let mut input = [0; 7];
            for byte in &mut input {
                *byte = alphabet[seed % alphabet.len()];
                seed /= alphabet.len();
            }
            let text = std::str::from_utf8(&input).unwrap();
            let mut parser = Records::new(text, b',', b'"');
            for _ in 0..8 {
                let before = parser.offset;
                match parser.next(&|| false) {
                    Ok(Some(_)) => assert!(parser.offset > before),
                    Ok(None) | Err(_) => break,
                }
                assert!(parser.offset <= input.len());
            }
        }
    }

    #[test]
    fn record_pressure_respects_explicit_limits() {
        let text = "x\n".repeat(MAX_RECORDS + 2);
        let mut parser = Records::new(&text, b',', b'"');
        for _ in 0..MAX_RECORDS + 1 {
            assert!(parser.next(&|| false).unwrap().is_some());
        }
        assert!(parser.next(&|| false).is_err());
    }

    #[test]
    #[ignore = "manual 10000-file native CSV pressure/latency measurement"]
    fn tabular_ten_thousand_file_pressure_probe() {
        let dir = Fixture::new();
        let seed = dir.wav("seed.wav");
        let bytes = std::fs::read(&seed).unwrap();
        let mut paths = Vec::new();
        let mut text = String::from("File,Title\r\n");
        for index in 0..10_000 {
            let path = dir.0.join(format!("{index}.wav"));
            std::fs::write(&path, &bytes).unwrap();
            paths.push(path);
        }
        for index in (0..10_000).rev() {
            text.push_str(&format!("{index}.wav,\"Title {index}, 🎵\"\r\n"));
        }
        let options = TabularImportOptions {
            mode: TextImportMode::Filename,
            match_column: Some(ImportColumn::Header {
                name: "File".into(),
            }),
            ..options()
        };
        let usage_before = Budget::process().usage().0;
        let app = dir.app();
        let mut timings = Vec::new();
        for _ in 0..6 {
            let start = Instant::now();
            let preview = app
                .preview_tabular_import(&paths, text.as_bytes(), &options, None)
                .unwrap();
            timings.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(preview.plan.changes.len(), 10_000);
            assert!(preview.unmatched_files.is_empty());
            assert_eq!(
                preview.plan.changes[0].tag_changes[0].new.as_deref(),
                Some("Title 0, 🎵")
            );
            assert_eq!(
                preview.plan.changes[9999].tag_changes[0].new.as_deref(),
                Some("Title 9999, 🎵")
            );
        }
        let usage_warm = Budget::process().usage().0;
        assert_eq!(usage_warm.transient, usage_before.transient);
        assert_eq!(usage_warm.jobs, usage_before.jobs);
        drop(app);
        assert_eq!(Budget::process().usage().0, usage_before);
        println!(
            "TABULAR_PRESSURE {}",
            serde_json::json!({"files":10000,"input_bytes":text.len(),"first_ms":timings[0],"warm_ms":&timings[1..],"retained_metadata_bytes":usage_warm.retained.saturating_sub(usage_before.retained),"transient_leaked":false,"audio_writes":false})
        );
    }

    #[test]
    #[ignore = "independent synthetic MP3/FLAC/M4A verification; requires ffmpeg and ffprobe"]
    fn tabular_independent_audio_apply_restart_undo() {
        use std::process::Command;
        fn run(program: &str, args: &[&std::ffi::OsStr]) -> Vec<u8> {
            let output = Command::new(program).args(args).output().unwrap();
            assert!(
                output.status.success(),
                "{program}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            output.stdout
        }
        fn hashes(path: &Path) -> serde_json::Value {
            serde_json::from_slice(&run(
                "ffprobe",
                &[
                    "-v".as_ref(),
                    "error".as_ref(),
                    "-select_streams".as_ref(),
                    "a".as_ref(),
                    "-show_packets".as_ref(),
                    "-show_entries".as_ref(),
                    "packet=data_hash".as_ref(),
                    "-show_data_hash".as_ref(),
                    "sha256".as_ref(),
                    "-of".as_ref(),
                    "json".as_ref(),
                    path.as_os_str(),
                ],
            ))
            .unwrap()
        }
        fn tags(path: &Path) -> BTreeMap<String, String> {
            let value: serde_json::Value = serde_json::from_slice(&run(
                "ffprobe",
                &[
                    "-v".as_ref(),
                    "error".as_ref(),
                    "-show_entries".as_ref(),
                    "format_tags=title,artist,comment".as_ref(),
                    "-of".as_ref(),
                    "json".as_ref(),
                    path.as_os_str(),
                ],
            ))
            .unwrap();
            value["format"]["tags"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(key, value)| {
                    (
                        key.to_ascii_lowercase(),
                        value.as_str().unwrap().to_string(),
                    )
                })
                .collect()
        }
        let dir = Fixture::new();
        for (extension, codec) in [("mp3", "libmp3lame"), ("flac", "flac"), ("m4a", "aac")] {
            let path = dir.0.join(format!("independent.{extension}"));
            run(
                "ffmpeg",
                &[
                    "-v".as_ref(),
                    "error".as_ref(),
                    "-f".as_ref(),
                    "lavfi".as_ref(),
                    "-i".as_ref(),
                    "anullsrc=r=44100:cl=mono".as_ref(),
                    "-t".as_ref(),
                    "0.1".as_ref(),
                    "-c:a".as_ref(),
                    codec.as_ref(),
                    "-metadata".as_ref(),
                    "title=Saved".as_ref(),
                    "-metadata".as_ref(),
                    "artist=Original".as_ref(),
                    path.as_os_str(),
                ],
            );
            let baseline = tags(&path);
            let audio = hashes(&path);
            let bytes = std::fs::read(&path).unwrap();
            assert!(!audio["packets"].as_array().unwrap().is_empty());
            let mut app = dir.app();
            let options = TabularImportOptions {
                mappings: vec![
                    mapping("Title", "title", EmptyCell::Skip),
                    mapping("Notes", "comment", EmptyCell::Skip),
                ],
                ..options()
            };
            let preview = app
                .preview_tabular_import(
                    std::slice::from_ref(&path),
                    "Title,Notes\r\n\"Björk, 🎵\",\"line one\r\nline two\"".as_bytes(),
                    &options,
                    None,
                )
                .unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            let batch = app.apply(&preview.plan).unwrap();
            drop(app);
            let applied = tags(&path);
            assert_eq!(applied.get("title").unwrap(), "Björk, 🎵");
            assert_eq!(applied.get("comment").unwrap(), "line one\r\nline two");
            assert_eq!(applied.get("artist"), baseline.get("artist"));
            assert_eq!(hashes(&path), audio);
            dir.app().undo(batch.id).unwrap();
            assert_eq!(tags(&path), baseline);
            assert_eq!(hashes(&path), audio);
        }
    }

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "tagrex-tabular-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn wav(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            let mut wav = Vec::new();
            wav.extend(b"RIFF");
            wav.extend(356u32.to_le_bytes());
            wav.extend(b"WAVEfmt ");
            wav.extend(16u32.to_le_bytes());
            wav.extend(1u16.to_le_bytes());
            wav.extend(1u16.to_le_bytes());
            wav.extend(8000u32.to_le_bytes());
            wav.extend(16000u32.to_le_bytes());
            wav.extend(2u16.to_le_bytes());
            wav.extend(16u16.to_le_bytes());
            wav.extend(b"data");
            wav.extend(320u32.to_le_bytes());
            wav.extend([0; 320]);
            std::fs::write(&path, wav).unwrap();
            let mut file = TagEngine::read(&path).unwrap();
            file.tags.insert(TagField::Title, "Saved".into());
            file.tags.insert(TagField::Artist, "Original artist".into());
            TagEngine::write(&file).unwrap();
            path
        }
        fn app(&self) -> App {
            App::open(&self.0, &self.0.join("journal.sqlite")).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn mapping(header: &str, field: &str, empty: EmptyCell) -> ColumnMapping {
        ColumnMapping {
            column: ImportColumn::Header {
                name: header.into(),
            },
            field: field.into(),
            empty,
        }
    }
    fn options() -> TabularImportOptions {
        TabularImportOptions {
            mappings: vec![mapping("Title", "title", EmptyCell::Skip)],
            ..Default::default()
        }
    }
    #[test]
    fn table_callbacks_cannot_publish_after_owning_app_policy_changes() {
        let dir = Fixture::new();
        let paths = [dir.wav("first.wav"), dir.wav("second.wav")];
        let original: Vec<_> = paths
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect();
        let app = dir.app();
        let text = b"Title\nFirst import\nSecond import";
        let calls = Cell::new(0usize);
        let baseline = app
            .preview_tabular_import_with_cancel(&paths, text, &options(), None, || {
                calls.set(calls.get() + 1);
                false
            })
            .unwrap();
        assert_eq!(baseline.plan.changes.len(), 2);
        // Sweep every public callback boundary, including callbacks between
        // saved-row reads and the final callback immediately before return.
        for change_at in 1..=calls.get() {
            let visited = Cell::new(0usize);
            let changed = Cell::new(false);
            let result = app.preview_tabular_import_with_cancel(
                &paths,
                text,
                &options(),
                None,
                || {
                    visited.set(visited.get() + 1);
                    if visited.get() == change_at {
                        app.apply_settings(&SettingsDto {
                            multi_value_separator: format!(" / {change_at} / "),
                            ..Default::default()
                        });
                        changed.set(true);
                    }
                    false
                },
            );
            assert!(changed.get(), "callback {change_at} was not reached");
            let error = result.expect_err("an obsolete interpretation must not publish a plan");
            assert!(error.to_string().contains("Metadata settings changed"));
        }
        for (path, bytes) in paths.iter().zip(original) {
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
    }

    #[test]
    fn table_callbacks_keep_peer_app_policy_changes_isolated() {
        let dir = Fixture::new();
        let paths = [dir.wav("first.wav"), dir.wav("second.wav")];
        let original: Vec<_> = paths
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect();
        let app = dir.app();
        let peer_dir = Fixture::new();
        let peer = peer_dir.app();
        let calls = Cell::new(0usize);
        let preview = app
            .preview_tabular_import_with_cancel(
                &paths,
                b"Title\nFirst import\nSecond import",
                &options(),
                None,
                || {
                    calls.set(calls.get() + 1);
                    peer.apply_settings(&SettingsDto {
                        multi_value_separator: format!(" / peer {} / ", calls.get()),
                        ..Default::default()
                    });
                    false
                },
            )
            .unwrap();
        assert!(calls.get() > 2);
        for ((path, expected), bytes) in paths
            .iter()
            .zip(["First import", "Second import"])
            .zip(original)
        {
            let change = preview
                .plan
                .changes
                .iter()
                .find(|row| Path::new(&row.path) == path)
                .unwrap();
            let title = change
                .tag_changes
                .iter()
                .find(|field| field.field == "title")
                .unwrap();
            assert_eq!(title.old.as_deref(), Some("Saved"));
            assert_eq!(title.new.as_deref(), Some(expected));
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
    }

    #[test]
    fn direct_field_recipe_pins_its_owner_across_saved_row_callbacks() {
        let dir = Fixture::new();
        let path = dir.wav("recipe.wav");
        let original = std::fs::read(&path).unwrap();
        let app = dir.app();
        let operations = [FieldOperation::Format {
            field: "title".into(),
            pattern: "Imported".into(),
        }];
        let calls = Cell::new(0usize);
        let result = app.preview_field_operations_with_cancel(
            std::slice::from_ref(&path),
            &operations,
            None,
            || {
                calls.set(calls.get() + 1);
                // The first row has been read when its operation is polled.
                if calls.get() == 2 {
                    app.apply_settings(&SettingsDto {
                        multi_value_separator: " / changed / ".into(),
                        ..Default::default()
                    });
                }
                false
            },
        );
        assert_eq!(calls.get(), 2);
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Metadata settings changed"));
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn import_matches_rows_and_preserves_staged_edits_apply_and_restart_undo() {
        let dir = Fixture::new();
        let a = dir.wav("a.wav");
        let b = dir.wav("b.wav");
        let original_a = std::fs::read(&a).unwrap();
        let original_b = std::fs::read(&b).unwrap();
        let mut app = dir.app();
        let working = app
            .preview_tag_edits(&[TagEditDto {
                path: a.to_string_lossy().into(),
                field: "comment".into(),
                value: Some("Unrelated staged edit".into()),
            }])
            .unwrap();
        let options = TabularImportOptions {
            mode: TextImportMode::Filename,
            match_column: Some(ImportColumn::Header {
                name: "File".into(),
            }),
            ..options()
        };
        let preview = app
            .preview_tabular_import(
                &[a.clone(), b.clone()],
                b"File,Title\r\nb.wav,Second\r\na.wav,\"First, with comma\"\r\nabsent.wav,Ignored",
                &options,
                Some(&working),
            )
            .unwrap();
        assert_eq!(preview.rows.len(), 3);
        assert_eq!(preview.rows[2].status, TextImportStatus::FileNotSelected);
        assert_eq!(std::fs::read(&a).unwrap(), original_a);
        assert_eq!(std::fs::read(&b).unwrap(), original_b);
        let batch = app.apply(&preview.plan).unwrap();
        assert_eq!(
            TagEngine::read(&a).unwrap().tags[&TagField::Title],
            "First, with comma"
        );
        assert_eq!(
            TagEngine::read(&a).unwrap().tags[&TagField::Comment],
            "Unrelated staged edit"
        );
        assert_eq!(
            TagEngine::read(&b).unwrap().tags[&TagField::Title],
            "Second"
        );
        drop(app);
        dir.app().undo(batch.id).unwrap();
        assert_eq!(TagEngine::read(&a).unwrap().tags[&TagField::Title], "Saved");
        assert!(!TagEngine::read(&a)
            .unwrap()
            .tags
            .contains_key(&TagField::Comment));
        assert_eq!(TagEngine::read(&b).unwrap().tags[&TagField::Title], "Saved");
    }
    #[test]
    fn empty_policy_is_explicit_and_noop_import_reverts_only_its_staged_field() {
        let dir = Fixture::new();
        let path = dir.wav("a.wav");
        let app = dir.app();
        let working = app
            .preview_tag_edits(&[TagEditDto {
                path: path.to_string_lossy().into(),
                field: "title".into(),
                value: Some("Staged".into()),
            }])
            .unwrap();
        let paths = [path];
        let skip = app
            .preview_tabular_import(&paths, b"Title\n\"\"", &options(), Some(&working))
            .unwrap();
        assert_eq!(
            skip.plan.changes[0].tag_changes[0].new.as_deref(),
            Some("Staged")
        );
        let mut clear_options = options();
        clear_options.mappings[0].empty = EmptyCell::Clear;
        let clear = app
            .preview_tabular_import(&paths, b"Title\n\"\"", &clear_options, Some(&working))
            .unwrap();
        assert_eq!(clear.plan.changes[0].tag_changes[0].new, None);
        let reverted = app
            .preview_tabular_import(&paths, b"Title\nSaved", &options(), Some(&working))
            .unwrap();
        assert!(reverted.plan.changes.is_empty());
        clear_options.mappings[0].empty = EmptyCell::Reject;
        assert!(app
            .preview_tabular_import(&paths, b"Title\n\"\"", &clear_options, None)
            .is_err());
    }

    #[test]
    fn mapped_fields_respect_locks_and_stale_working_baselines() {
        let dir = Fixture::new();
        let path = dir.wav("selected.wav");
        let other = dir.wav("unselected.wav");
        let app = dir.app();
        app.set_locked_fields(&["artist".into()]);
        let options = TabularImportOptions {
            mappings: vec![
                mapping("Title", "title", EmptyCell::Skip),
                mapping("Artist", "artist", EmptyCell::Skip),
            ],
            ..options()
        };
        let preview = app
            .preview_tabular_import(
                std::slice::from_ref(&path),
                b"Title,Artist\nImported,Blocked",
                &options,
                None,
            )
            .unwrap();
        assert!(preview.plan.changes[0]
            .tag_changes
            .iter()
            .any(|field| field.field == "title"));
        assert!(!preview.plan.changes[0]
            .tag_changes
            .iter()
            .any(|field| field.field == "artist"));
        assert!(preview
            .plan
            .locked_skipped
            .iter()
            .any(|field| field.field == "artist"));
        assert_eq!(
            TagEngine::read(&other).unwrap().tags[&TagField::Title],
            "Saved"
        );
        let mut external = TagEngine::read(&path).unwrap();
        external.tags.insert(TagField::Title, "External".into());
        TagEngine::write(&external).unwrap();
        let attempt = app.preview_tabular_import(
            std::slice::from_ref(&path),
            b"Title,Artist\nRetry,Blocked",
            &options,
            Some(&preview.plan),
        );
        assert!(matches!(
            attempt,
            Err(TabularImportError::Native(AppError::Plan(
                tagrex_core::plan::PlanError::Stale(_)
            )))
        ));
        assert_eq!(
            TagEngine::read(&path).unwrap().tags[&TagField::Title],
            "External"
        );
    }
    #[test]
    fn validates_headers_aliases_width_matches_encoding_and_empty_selection() {
        let dir = Fixture::new();
        let path = dir.wav("a.wav");
        let app = dir.app();
        for text in [
            "Title,Title\na,b",
            "Title,Other\na",
            "Title\na,b",
            "Unknown\na",
            "Title\na\"b",
        ] {
            assert!(
                app.preview_tabular_import(&[], text.as_bytes(), &options(), None)
                    .is_err(),
                "{text}"
            );
        }
        let mut duplicate = options();
        duplicate
            .mappings
            .push(mapping("Track", "TRACKNUMBER", EmptyCell::Skip));
        duplicate
            .mappings
            .push(mapping("Track", "track", EmptyCell::Skip));
        assert!(app
            .preview_tabular_import(&[], b"Title,Track", &duplicate, None)
            .is_err());
        assert!(app
            .preview_tabular_import(&[], &[0xff], &options(), None)
            .is_err());
        let matched = TabularImportOptions {
            mode: TextImportMode::Filename,
            match_column: Some(ImportColumn::Header {
                name: "File".into(),
            }),
            ..options()
        };
        assert!(app
            .preview_tabular_import(
                std::slice::from_ref(&path),
                b"File,Title\na.wav,one\na.wav,two",
                &matched,
                None
            )
            .is_err());
        let bytes =
            text_codec::encode("Title\nBjörk 🎵", TextEncoding::Utf16Le, true, MAX_INPUT).unwrap();
        let utf16 = TabularImportOptions {
            encoding: "utf-16le".into(),
            ..options()
        };
        let preview = app
            .preview_tabular_import(&[path], &bytes, &utf16, None)
            .unwrap();
        assert_eq!(
            preview.plan.changes[0].tag_changes[0].new.as_deref(),
            Some("Björk 🎵")
        );
    }
    #[test]
    fn cancels_without_mutation_and_rejects_mapping_expansion_before_large_retention() {
        let dir = Fixture::new();
        let path = dir.wav("a.wav");
        let app = dir.app();
        let before = std::fs::read(&path).unwrap();
        let input = format!("Title\n{}", "a".repeat(MAX_FIELD));
        let calls = Cell::new(0);
        assert!(app
            .preview_tabular_import_with_cancel(
                std::slice::from_ref(&path),
                input.as_bytes(),
                &options(),
                None,
                || {
                    calls.set(calls.get() + 1);
                    calls.get() > 10
                }
            )
            .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let huge = TabularImportOptions {
            mappings: (0..256)
                .map(|i| mapping("Title", &format!("custom:Field{i}"), EmptyCell::Skip))
                .collect(),
            ..options()
        };
        let error = app
            .preview_tabular_import(&[], input.as_bytes(), &huge, None)
            .unwrap_err();
        assert!(error.to_string().contains("64 MiB"), "{error}");
    }

    #[test]
    fn legacy_preview_context_stops_between_reads_and_leaves_disk_and_next_job_intact() {
        let dir = Fixture::new();
        let first = dir.wav("one.wav");
        let second = dir.wav("two.wav");
        let before = std::fs::read(&first).unwrap();
        let app = dir.app();
        let calls = std::rc::Rc::new(Cell::new(0));
        let probe = calls.clone();
        let paths = [first.clone(), second];
        let result = with_preview_cancellation(
            move || {
                probe.set(probe.get() + 1);
                probe.get() > 2
            },
            || app.preview_tags_from_name("%title%", &paths),
        );
        assert!(result.is_err());
        assert!(calls.get() <= 4);
        assert_eq!(std::fs::read(&first).unwrap(), before);
        assert!(app.preview_tags_from_name("%title%", &paths).is_ok());
        let stopped = with_preview_cancellation(|| true, || app.read_track_rows(&paths));
        assert!(matches!(
            stopped,
            Err(AppError::Plan(tagrex_core::plan::PlanError::Cancelled))
        ));
        assert!(app.read_track_rows(&paths).is_ok());
    }

    #[test]
    fn cancelled_folder_extras_scan_preserves_the_previous_files_and_retries_cleanly() {
        let dir = Fixture::new();
        let source = dir.0.join("source");
        let destination = dir.0.join("destination");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&destination).unwrap();
        let track = dir.wav("source/track.wav");
        for index in 0..320 {
            std::fs::write(source.join(format!("note-{index}.txt")), b"notes").unwrap();
        }
        let before = std::fs::read(&track).unwrap();
        let app = dir.app();
        app.apply_settings(&SettingsDto {
            carry_folder_extras: true,
            carry_sidecars: false,
            ..Default::default()
        });
        let calls = std::rc::Rc::new(Cell::new(0));
        let probe = calls.clone();
        let preview = || {
            app.preview_move(
                "%filename%",
                std::slice::from_ref(&track),
                Some(&destination),
                false,
                true,
            )
        };
        let result = with_preview_cancellation(
            move || {
                probe.set(probe.get() + 1);
                probe.get() > 30
            },
            preview,
        );
        assert!(matches!(
            result,
            Err(AppError::Plan(tagrex_core::plan::PlanError::Cancelled))
        ));
        assert!(calls.get() < 40);
        assert_eq!(std::fs::read(&track).unwrap(), before);
        assert!(!destination.join("track.wav").exists());
        let completed = preview().unwrap();
        assert_eq!(completed.changes[0].folder_extras.len(), 320);
        assert_eq!(std::fs::read_dir(&source).unwrap().count(), 321);
    }

    #[cfg(unix)]
    #[test]
    fn folder_extras_refuse_linked_subtrees_and_bound_real_directory_depth() {
        let dir = Fixture::new();
        let source = dir.0.join("source");
        let other = dir.0.join("other");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&other).unwrap();
        std::fs::write(other.join("notes.txt"), b"unrelated").unwrap();
        std::os::unix::fs::symlink(&other, source.join("linked")).unwrap();
        let empty = std::collections::BTreeSet::new();
        let mut extras = Vec::new();
        let mut budget = super::super::ExtrasScanBudget::default();
        assert!(!super::super::collect_folder_extras(
            &source,
            &source,
            &empty,
            &empty,
            &mut extras,
            0,
            &mut budget
        )
        .unwrap());
        assert!(extras.is_empty());
        assert_eq!(
            std::fs::read(other.join("notes.txt")).unwrap(),
            b"unrelated"
        );
        std::fs::remove_file(source.join("linked")).unwrap();
        let mut nested = source.clone();
        for _ in 0..130 {
            nested.push("d");
            std::fs::create_dir(&nested).unwrap();
        }
        let error = super::super::collect_folder_extras(
            &source,
            &source,
            &empty,
            &empty,
            &mut extras,
            0,
            &mut super::super::ExtrasScanBudget::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("128 nested levels"));
        assert!(extras.is_empty());
    }
}
