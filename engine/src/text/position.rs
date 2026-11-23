use line_index::{LineCol, LineIndex, TextSize, WideEncoding, WideLineCol};

use crate::protocol::schema::{CanonicalPos, CanonicalRange, LspPosition, OffsetEncoding};

/// Translates between canonical 0-based `(line, utf8_byte_col)` positions
/// and LSP `offset_encoding` (`utf-8`, `utf-16`, `utf-32`), using an ASCII fast-path.
pub struct PositionMapper<'a> {
    text: &'a str,
    is_ascii: bool,
    index: Option<LineIndex>,
}

impl<'a> PositionMapper<'a> {
    pub fn new(text: &'a str) -> Self {
        let is_ascii = text.is_ascii();
        let index = if is_ascii {
            None
        } else {
            Some(LineIndex::new(text))
        };
        Self {
            text,
            is_ascii,
            index,
        }
    }

    pub fn byte_offset_to_canonical(&self, byte_offset: usize) -> CanonicalPos {
        let clamped = byte_offset.min(self.text.len());
        if self.is_ascii {
            let prefix = &self.text[..clamped];
            let line = prefix.bytes().filter(|&b| b == b'\n').count() as u32;
            let line_start = prefix.rfind('\n').map(|i| i + 1).unwrap_or(0);
            let col = (clamped - line_start) as u32;
            return CanonicalPos { line, col };
        }
        let idx = self.index.as_ref().expect("non-ascii builds LineIndex");
        let lc = idx.line_col(TextSize::from(clamped as u32));
        CanonicalPos {
            line: lc.line,
            col: lc.col,
        }
    }

    pub fn canonical_to_byte_offset(&self, pos: CanonicalPos) -> usize {
        if self.is_ascii {
            let mut current_line = 0u32;
            let mut line_start = 0usize;
            for (i, b) in self.text.bytes().enumerate() {
                if current_line == pos.line {
                    break;
                }
                if b == b'\n' {
                    current_line += 1;
                    line_start = i + 1;
                }
            }
            if current_line < pos.line {
                return self.text.len();
            }
            let line_end = self.text[line_start..]
                .find('\n')
                .map(|rel| line_start + rel)
                .unwrap_or(self.text.len());
            return (line_start + pos.col as usize).min(line_end);
        }
        let idx = self.index.as_ref().expect("non-ascii builds LineIndex");
        idx.offset(LineCol {
            line: pos.line,
            col: pos.col,
        })
        .map(usize::from)
        .unwrap_or(self.text.len())
    }

    pub fn lsp_to_canonical(&self, pos: LspPosition, encoding: OffsetEncoding) -> CanonicalPos {
        if self.is_ascii || encoding == OffsetEncoding::Utf8 {
            return CanonicalPos {
                line: pos.line,
                col: pos.character,
            };
        }
        let idx = self.index.as_ref().expect("non-ascii builds LineIndex");
        let wide_enc = match encoding {
            OffsetEncoding::Utf16 => WideEncoding::Utf16,
            OffsetEncoding::Utf32 => WideEncoding::Utf32,
            OffsetEncoding::Utf8 => unreachable!(),
        };
        let wide = WideLineCol {
            line: pos.line,
            col: pos.character,
        };
        if let Some(lc) = idx.to_utf8(wide_enc, wide) {
            CanonicalPos {
                line: lc.line,
                col: lc.col,
            }
        } else {
            CanonicalPos {
                line: pos.line,
                col: pos.character,
            }
        }
    }

    pub fn canonical_to_lsp(&self, pos: CanonicalPos, encoding: OffsetEncoding) -> LspPosition {
        if self.is_ascii || encoding == OffsetEncoding::Utf8 {
            return LspPosition {
                line: pos.line,
                character: pos.col,
            };
        }
        let idx = self.index.as_ref().expect("non-ascii builds LineIndex");
        let wide_enc = match encoding {
            OffsetEncoding::Utf16 => WideEncoding::Utf16,
            OffsetEncoding::Utf32 => WideEncoding::Utf32,
            OffsetEncoding::Utf8 => unreachable!(),
        };
        let lc = LineCol {
            line: pos.line,
            col: pos.col,
        };
        if let Some(wide) = idx.to_wide(wide_enc, lc) {
            LspPosition {
                line: wide.line,
                character: wide.col,
            }
        } else {
            LspPosition {
                line: pos.line,
                character: pos.col,
            }
        }
    }

    pub fn byte_range_to_canonical(&self, start_byte: usize, end_byte: usize) -> CanonicalRange {
        CanonicalRange {
            start: self.byte_offset_to_canonical(start_byte),
            end: self.byte_offset_to_canonical(end_byte),
        }
    }
}
