use crate::model::{AppError, Result};

#[derive(Clone, Copy, Default)]
pub struct SqlMode {
    pub no_backslash_escapes: bool,
    pub ansi_quotes: bool,
}
impl SqlMode {
    pub fn parse(value: &str) -> Self {
        Self {
            no_backslash_escapes: value.split(',').any(|s| s == "NO_BACKSLASH_ESCAPES"),
            ansi_quotes: value.split(',').any(|s| s == "ANSI_QUOTES"),
        }
    }
}
#[derive(Debug)]
pub struct Statement {
    pub sql: String,
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub next: usize,
}
#[derive(Clone, Copy, PartialEq)]
enum ScanState {
    Normal,
    Quote(u8),
    LineComment,
    BlockComment,
}

/// MySQL lexical splitter. Offsets are UTF-8 byte offsets internally, never JS offsets.
pub fn next_statement(source: &str, offset: usize, mode: SqlMode) -> Result<Option<Statement>> {
    let bytes = source.as_bytes();
    let mut i = offset;
    let mut state = ScanState::Normal;
    let mut start = offset;
    let mut has_code = false;
    while i < bytes.len() {
        let b = bytes[i];
        match state {
            ScanState::Normal => {
                if b == b';' {
                    if has_code {
                        return make_statement(source, start, i, i + 1);
                    }
                    start = i + 1;
                } else if b == b'#'
                    || (b == b'-'
                        && bytes.get(i + 1) == Some(&b'-')
                        && bytes
                            .get(i + 2)
                            .is_none_or(|c| c.is_ascii_whitespace() || *c < 32))
                {
                    state = ScanState::LineComment;
                } else if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
                    // Executable version comments must be forwarded to the server.
                    if bytes.get(i + 2) == Some(&b'!') {
                        has_code = true;
                    }
                    state = ScanState::BlockComment;
                    i += 1;
                } else if matches!(b, b'\'' | b'"' | b'`') {
                    state = ScanState::Quote(b);
                    has_code = true;
                } else if !b.is_ascii_whitespace() {
                    if !has_code && b.is_ascii_alphabetic() {
                        let end = bytes[i..]
                            .iter()
                            .position(|c| !c.is_ascii_alphabetic())
                            .map_or(bytes.len(), |n| i + n);
                        if source[i..end].eq_ignore_ascii_case("delimiter") {
                            let mut error = AppError::new("UNSUPPORTED_DELIMITER", "第一版不支持 DELIMITER / 存储程序定义，请使用 MySQL 客户端执行此脚本。");
                            error.line = Some(line_at(source, i));
                            return Err(error);
                        }
                    }
                    has_code = true;
                }
            }
            ScanState::Quote(quote) => {
                let backslash = quote != b'`'
                    && !(quote == b'"' && mode.ansi_quotes)
                    && !mode.no_backslash_escapes;
                if b == b'\\' && backslash {
                    i += 1;
                } else if b == quote {
                    if bytes.get(i + 1) == Some(&quote) {
                        i += 1;
                    } else {
                        state = ScanState::Normal;
                    }
                }
            }
            ScanState::LineComment => {
                if b == b'\n' {
                    state = ScanState::Normal;
                }
            }
            ScanState::BlockComment => {
                if b == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    state = ScanState::Normal;
                    i += 1;
                }
            }
        }
        i += 1;
    }
    if matches!(state, ScanState::Quote(_) | ScanState::BlockComment) {
        let mut error = AppError::new("SQL_PARSE", "SQL 中存在未闭合的引号或块注释。");
        error.line = Some(line_at(source, start));
        return Err(error);
    }
    if has_code {
        make_statement(source, start, bytes.len(), bytes.len())
    } else {
        Ok(None)
    }
}
fn make_statement(
    source: &str,
    start: usize,
    end: usize,
    next: usize,
) -> Result<Option<Statement>> {
    let text = &source[start..end];
    let trim = text.len() - text.trim_start().len();
    let start = start + trim;
    Ok(Some(Statement {
        sql: source[start..end].trim_end().into(),
        line: line_at(source, start),
        start,
        end,
        next,
    }))
}
fn line_at(source: &str, at: usize) -> usize {
    source[..at].bytes().filter(|b| *b == b'\n').count() + 1
}

/// Check DELIMITER before sending any SQL. SET expressions may compute sql_mode
/// from variables or functions, so statically explore its four lexical modes.
/// The actual execution always splits using the freshly read server mode.
pub fn preflight(source: &str, mode: SqlMode) -> Result<()> {
    let bits = u8::from(mode.no_backslash_escapes) | (u8::from(mode.ansi_quotes) << 1);
    let mut pending = vec![(0, bits)];
    let mut visited = std::collections::HashSet::new();
    let mut complete = false;
    let mut parse_error = None;
    while let Some((offset, bits)) = pending.pop() {
        if !visited.insert((offset, bits)) {
            continue;
        }
        let mode = SqlMode {
            no_backslash_escapes: bits & 1 != 0,
            ansi_quotes: bits & 2 != 0,
        };
        match next_statement(source, offset, mode) {
            Ok(Some(statement)) => {
                if may_set_mode(&statement.sql) {
                    for mode in 0..4 {
                        pending.push((statement.next, mode));
                    }
                } else {
                    pending.push((statement.next, bits));
                }
            }
            Ok(None) => complete = true,
            Err(error) if error.code == "UNSUPPORTED_DELIMITER" => return Err(error),
            Err(error) => parse_error = Some(error),
        }
    }
    if complete {
        Ok(())
    } else {
        Err(parse_error.unwrap_or_else(|| AppError::new("SQL_PARSE", "无法解析脚本")))
    }
}
fn may_set_mode(sql: &str) -> bool {
    let mut text = sql.trim_start();
    loop {
        if text.starts_with("/*!") {
            return true;
        }
        if text.starts_with("/*") {
            if let Some(end) = text.find("*/") {
                text = text[end + 2..].trim_start();
                continue;
            }
        } else if text.starts_with('#')
            || (text.starts_with("--")
                && text
                    .as_bytes()
                    .get(2)
                    .is_none_or(|c| c.is_ascii_whitespace()))
        {
            if let Some(end) = text.find('\n') {
                text = text[end + 1..].trim_start();
                continue;
            }
        }
        break;
    }
    let word = text
        .split(|c: char| !c.is_ascii_alphabetic())
        .next()
        .unwrap_or("");
    word.eq_ignore_ascii_case("SET") || word.eq_ignore_ascii_case("CALL")
}
pub fn at_cursor(source: &str, cursor_utf16: usize, mode: SqlMode) -> Result<String> {
    let mut units = 0;
    let cursor = source
        .char_indices()
        .find_map(|(index, c)| {
            if units >= cursor_utf16 {
                Some(index)
            } else {
                units += c.len_utf16();
                None
            }
        })
        .unwrap_or(source.len());
    let mut offset = 0;
    let mut last = None;
    while let Some(stmt) = next_statement(source, offset, mode)? {
        if cursor <= stmt.end || (cursor >= stmt.start && cursor < stmt.next) {
            return Ok(stmt.sql);
        }
        offset = stmt.next;
        last = Some(stmt.sql);
    }
    last.ok_or_else(|| AppError::new("EMPTY_SQL", "请先输入 SQL。"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn split(s: &str, mode: SqlMode) -> Vec<String> {
        let mut out = Vec::new();
        let mut offset = 0;
        while let Some(stmt) = next_statement(s, offset, mode).unwrap() {
            offset = stmt.next;
            out.push(stmt.sql);
        }
        out
    }
    #[test]
    fn quotes_comments_and_unicode() {
        let s = "# 开始;\nSELECT 'a;b', \"c;d\", `e;f`, 'it''s;ok'; -- comment;\nSELECT '中文😀'; /* ; */";
        assert_eq!(split(s, SqlMode::default()).len(), 2);
        assert!(at_cursor(s, s.encode_utf16().count(), SqlMode::default())
            .unwrap()
            .contains("中文😀"));
    }
    #[test]
    fn mysql_dash_comment_requires_whitespace() {
        assert_eq!(split("SELECT 1--2; SELECT 3", SqlMode::default()).len(), 2);
        assert_eq!(
            split("-- comment;\n; /* comment */", SqlMode::default()).len(),
            0
        );
    }
    #[test]
    fn sql_modes_change_escaping() {
        assert_eq!(
            split(
                "SELECT 'a\\'; SELECT 2",
                SqlMode {
                    no_backslash_escapes: true,
                    ansi_quotes: false
                }
            )
            .len(),
            2
        );
        assert!(next_statement("SELECT 'a\\'; SELECT 2", 0, SqlMode::default()).is_err());
        assert_eq!(
            split(
                "SELECT \"a\\\"; SELECT 2",
                SqlMode {
                    no_backslash_escapes: false,
                    ansi_quotes: true
                }
            )
            .len(),
            2
        );
        preflight(
            "SET sql_mode='NO_BACKSLASH_ESCAPES'; SELECT 'a\\'; SELECT 2",
            SqlMode::default(),
        )
        .unwrap();
    }
    #[test]
    fn rejects_delimiter_before_execution() {
        let e = preflight(
            "SELECT 1;\nDELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT 2; END$$",
            SqlMode::default(),
        )
        .unwrap_err();
        assert_eq!(e.code, "UNSUPPORTED_DELIMITER");
        assert_eq!(e.line, Some(2));
        preflight(
            "SELECT 'DELIMITER $$'; -- DELIMITER $$\nSELECT 2",
            SqlMode::default(),
        )
        .unwrap();
    }
    #[test]
    fn version_comment_and_unclosed_input() {
        assert_eq!(
            split("/*!80000 SET @x=1 */; SELECT @x", SqlMode::default()).len(),
            2
        );
        assert!(next_statement("SELECT 'abc", 0, SqlMode::default()).is_err());
        assert!(next_statement("SELECT 1 /*", 0, SqlMode::default()).is_err());
    }
    #[test]
    fn cursor_and_source_lines() {
        let source = "SELECT 1;\n\nSELECT 2;";
        let a = next_statement(source, 0, SqlMode::default())
            .unwrap()
            .unwrap();
        let b = next_statement(source, a.next, SqlMode::default())
            .unwrap()
            .unwrap();
        assert_eq!(b.line, 3);
        assert_eq!(
            at_cursor(source, 12, SqlMode::default()).unwrap(),
            "SELECT 2"
        );
    }
}
