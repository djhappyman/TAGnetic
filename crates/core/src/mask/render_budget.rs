//! Conservative work bounds from the actual shared mask AST. These count a
//! cumulative allocation allowance, not only the final rendered string. A
//! caller reserves this allowance before entering the existing renderer.
use super::{
    FileContext, Function, Mask, MaskError, Segment, TagField, TagMap, MAX_PATTERN_OUTPUT,
};

#[derive(Debug, thiserror::Error)]
pub enum BoundedRenderError {
    #[error("Pattern rendering exceeds its admitted working budget")]
    TooLarge,
    #[error(transparent)]
    Mask(MaskError),
}

impl From<MaskError> for BoundedRenderError {
    fn from(mut error: MaskError) -> Self {
        // The legacy numeric diagnostic includes the complete tag argument.
        // Bound both its retained allocation and wire/log representation for
        // this admitted API; leave legacy render methods unchanged.
        if let MaskError::BadArgument(detail) = &mut error {
            if detail.len() > 1024 {
                let mut end = 1024;
                while !detail.is_char_boundary(end) {
                    end -= 1;
                }
                let mut bounded = detail[..end].to_owned();
                bounded.push('…');
                *detail = bounded;
            }
        }
        Self::Mask(error)
    }
}

impl Mask {
    /// Conservative allocation allowance for rendering this AST with these
    /// borrowed inputs. Includes argument/section strings, output growth,
    /// function scratch and error formatting. `None` means arithmetic overflow.
    /// It does not include caller-owned tags or the already compiled mask.
    ///
    /// Both value and filename rendering can use this bound. It assumes the
    /// filename sanitisation/padding path even for unsanitised value rendering.
    pub fn render_work_bytes(&self, tags: &TagMap, file: &FileContext<'_>) -> Option<usize> {
        estimate(&self.segments, tags, file)?.work.checked_add(4096)
    }

    /// Render only after the AST-derived allocation allowance fits `max_work`.
    /// Callers must independently acquire their coordinator's matching lease.
    pub fn render_with_budget(
        &self,
        tags: &TagMap,
        file: &FileContext<'_>,
        max_work: usize,
    ) -> Result<String, BoundedRenderError> {
        self.check_render_work(tags, file, max_work)?;
        Ok(self.render_with(tags, file)?)
    }

    /// Unsanitised field-value rendering with the same work admission contract.
    pub fn render_value_with_budget(
        &self,
        tags: &TagMap,
        file: &FileContext<'_>,
        max_work: usize,
    ) -> Result<String, BoundedRenderError> {
        self.check_render_work(tags, file, max_work)?;
        Ok(self.render_value_with(tags, file)?)
    }

    fn check_render_work(
        &self,
        tags: &TagMap,
        file: &FileContext<'_>,
        max_work: usize,
    ) -> Result<(), BoundedRenderError> {
        if self.extract_only {
            return Err(MaskError::ExtractOnly.into());
        }
        if self
            .render_work_bytes(tags, file)
            .is_none_or(|bytes| bytes > max_work)
        {
            return Err(BoundedRenderError::TooLarge);
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Estimate {
    // An argument/result which can survive the renderer's 1 MiB checks.
    output: usize,
    // Cumulative allocations, including temporaries already freed. This
    // overestimates simultaneous live memory and accounts all AST branches.
    work: usize,
}

fn value_bytes(field: &TagField, tags: &TagMap) -> usize {
    tags.get(field)
        .or_else(|| {
            matches!(field, TagField::Custom(_))
                .then(|| {
                    tags.iter()
                        .find(|(key, _)| key.same_name(field))
                        .map(|(_, value)| value)
                })
                .flatten()
        })
        .map_or(0, String::len)
}

fn estimate(segments: &[Segment], tags: &TagMap, file: &FileContext<'_>) -> Option<Estimate> {
    let mut output = 0usize;
    let mut work = 0usize;
    for segment in segments {
        let part = match segment {
            Segment::Literal(text) => Estimate {
                output: text.len(),
                work: 0,
            },
            Segment::Placeholder(field, width, _) => {
                let bytes = value_bytes(field, tags).max(*width);
                // Sanitisation, zero padding and an unsatisfied custom-field
                // diagnostic may each allocate; six copies dominate those paths.
                Estimate {
                    output: bytes.min(MAX_PATTERN_OUTPUT),
                    work: bytes.checked_mul(6)?.checked_add(4096)?,
                }
            }
            Segment::Section(inner) => estimate(inner, tags, file)?,
            Segment::Call(function, arguments) => {
                let mut argument_bytes = 0usize;
                let mut argument_work = 0usize;
                for argument in arguments {
                    let bound = estimate(argument, tags, file)?;
                    argument_bytes = argument_bytes.checked_add(bound.output)?;
                    argument_work = argument_work.checked_add(bound.work)?;
                }
                let (result, scratch) = function_bound(*function, argument_bytes, arguments)?;
                Estimate {
                    output: result.min(MAX_PATTERN_OUTPUT),
                    work: argument_work
                        .checked_add(scratch)?
                        .checked_add(result.checked_mul(2)?)?,
                }
            }
            Segment::File(_) => {
                // Lossy OS path conversion expands an encoded byte by at most
                // three UTF-8 bytes; numeric/date/property strings are smaller.
                let bytes = file
                    .path
                    .map_or(0, |path| path.as_os_str().len())
                    .checked_mul(3)?
                    .checked_add(256)?;
                Estimate {
                    output: bytes.min(MAX_PATTERN_OUTPUT),
                    work: bytes.checked_mul(8)?,
                }
            }
            Segment::Side => {
                let bytes = value_bytes(&TagField::MediaType, tags)
                    .checked_add(value_bytes(&TagField::DiscNumber, tags))?;
                Estimate {
                    output: 4,
                    work: bytes.checked_mul(8)?.checked_add(256)?,
                }
            }
            Segment::Position => {
                let bytes = tags.iter().find_map(|(field, value)| {
                    matches!(field, TagField::Custom(name) if name == super::POSITION_CUSTOM_FIELD).then_some(value.len())
                }).unwrap_or(0);
                Estimate {
                    output: bytes.min(MAX_PATTERN_OUTPUT),
                    work: bytes.checked_mul(2)?.checked_add(256)?,
                }
            }
            Segment::Skip => Estimate { output: 0, work: 0 },
        };
        // A push which would exceed the renderer's limit fails before growing
        // its buffer, so only a successful prefix needs an output allocation.
        output = output.checked_add(part.output)?.min(MAX_PATTERN_OUTPUT);
        work = work.checked_add(part.work)?;
    }
    Some(Estimate {
        output,
        // Geometric String growth is bounded by twice the successful prefix;
        // use four copies to include alloc/realloc overlap conservatively.
        work: work.checked_add(output.checked_mul(4)?)?.checked_add(256)?,
    })
}

fn function_bound(
    function: Function,
    inputs: usize,
    arguments: &[Vec<Segment>],
) -> Option<(usize, usize)> {
    use Function::*;
    let output = match function {
        Lower | Upper | Caps | Caps2 => inputs.checked_mul(3)?,
        Left | Right | Substr | Trim | TrimLeft | TrimRight | Reverse | GetPart | StripPrefix
        | CutMix => inputs,
        SwapPrefix => inputs.checked_add(2)?,
        PadLeft | PadRight | Num => {
            // A literal count has a cheap exact numeric bound. Unknown counts
            // use the renderer's enforced count ceiling before allocation.
            let padding = arguments
                .get(1)
                .and_then(|argument| match argument.as_slice() {
                    [Segment::Literal(value)] => value.trim().parse::<usize>().ok(),
                    _ => None,
                })
                .filter(|count| *count <= MAX_PATTERN_OUTPUT / 4)
                .and_then(|count| count.checked_mul(4))
                .unwrap_or(MAX_PATTERN_OUTPUT);
            inputs.checked_add(padding)?
        }
        // Nonempty replacement counts cannot exceed the first input's byte
        // count. The square of aggregate input bounds their product; the
        // existing helper refuses outputs beyond 1 MiB before allocation.
        Replace => inputs.saturating_mul(inputs).min(MAX_PATTERN_OUTPUT),
        Insert => inputs,
        Len | StrPos | Add | Sub | Mul | Div | Mod | Min | Max | Round => 1024,
        If | If2 => inputs,
        Equal | NotEqual | And | Or | Not | Greater | Longer | IsNumber | In => 1,
    };
    // Current helpers collect at most four bytes per input character and keep
    // at most head/tail/result copies. 16x includes geometric Vec growth and
    // debug-escaped numeric errors. Prefix-candidate and argument Vecs are
    // bounded separately by the AST's actual argument count. This exhaustive
    // match must be reconsidered if Function gains a new allocating operation.
    let scratch = inputs
        .checked_mul(16)?
        .checked_add(arguments.len().checked_mul(128)?)?
        .checked_add(output.checked_mul(4)?)?
        .checked_add(4096)?;
    Some((output, scratch))
}
