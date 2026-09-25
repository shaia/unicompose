//! The XCompose file format, read the way libxkbcommon 1.6 reads it
//! (`src/compose/parser.c`), so a file means the same here as on Linux.
//!
//! One deliberate difference: a string-only rule on the last line of a file with no final
//! newline is accepted, where libxkbcommon reports "unexpected token" and drops it.

use std::fmt;

use uc_keysym::Keysym;

use crate::includes::{Expand, Includes};
use crate::rules::{Output, RuleSet};

/// Longest left-hand side, in keysyms.
const MAX_KEYS: usize = 10;
/// WinCompose has no limit; its emoji rules spell out names such as "beaming face with smiling eyes".
const MAX_KEYS_WINCOMPOSE: usize = 128;
/// Deepest `include` nesting.
const MAX_INCLUDE_DEPTH: usize = 5;
/// A file with more errors than this fails to load.
const MAX_ERRORS: usize = 10;
/// Longest right-hand side string, in bytes (libxkbcommon's buffer less its NUL).
const MAX_STRING_BYTES: usize = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub file: String,
    pub line: u32,
    pub severity: Severity,
    pub message: String,
}

impl Diagnostic {
    /// An error about a whole file, such as one that cannot be read, or about one line.
    pub fn error(file: &str, line: u32, message: &str) -> Self {
        Diagnostic { file: file.to_owned(), line, severity: Severity::Error, message: message.to_owned() }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let severity = match self.severity {
            Severity::Warning => "warning",
            Severity::Error => "error",
        };
        write!(f, "{}:{}: {severity}: {}", self.file, self.line, self.message)
    }
}

/// A file that loaded, perhaps with lines skipped.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub rules: RuleSet,
    pub diagnostics: Vec<Diagnostic>,
}

/// A file that failed to load: too many errors, or an include that could not be read.
#[derive(Debug, Clone)]
pub struct LoadError {
    pub diagnostics: Vec<Diagnostic>,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.diagnostics.iter().rev().find(|d| d.severity == Severity::Error) {
            Some(last) => write!(f, "cannot load Compose file: {last}"),
            None => f.write_str("cannot load Compose file"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Which flavour of the format to accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dialect {
    /// Exactly what libxkbcommon accepts.
    #[default]
    Xkbcommon,
    /// Also what WinCompose accepts: any single character as a key name (`<!>`, `< >`,
    /// `<->`), and a byte-order mark at the start of a file.
    WinCompose,
}

/// Parses `text`, named `file` in diagnostics, following `include` lines through
/// `includes`.
pub fn load(text: &str, file: &str, includes: &mut dyn Includes) -> Result<Loaded, LoadError> {
    let mut rules = RuleSet::new();
    let diagnostics = load_into(&mut rules, text, file, includes, Dialect::Xkbcommon)?;
    Ok(Loaded { rules, diagnostics })
}

/// Parses `text` into existing `rules`, so later files override earlier ones. On failure,
/// `rules` keeps whatever lines were read before the failure, as libxkbcommon's does.
pub fn load_into(
    rules: &mut RuleSet,
    text: &str,
    file: &str,
    includes: &mut dyn Includes,
    dialect: Dialect,
) -> Result<Vec<Diagnostic>, LoadError> {
    let text = match dialect {
        Dialect::WinCompose => text.strip_prefix('\u{feff}').unwrap_or(text),
        Dialect::Xkbcommon => text,
    };
    let mut parser = Parser { rules, includes, dialect };
    let mut diagnostics = Vec::new();
    if parser.parse(text, file, 0, &mut diagnostics) {
        Ok(diagnostics)
    } else {
        Err(LoadError { diagnostics })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    EndOfLine,
    EndOfFile,
    /// `<name>`
    Keysym(String),
    Colon,
    Bang,
    Tilde,
    Str(String),
    Ident(String),
    Include,
    /// Already reported.
    Error,
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
    /// Line of the token just returned.
    token_line: u32,
}

impl<'a> Lexer<'a> {
    fn new(text: &'a str) -> Self {
        Lexer { src: text.as_bytes(), pos: 0, line: 1, token_line: 1 }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        if b == b'\n' {
            self.line += 1;
        }
        Some(b)
    }

    fn eat(&mut self, b: u8) -> bool {
        let found = self.peek() == Some(b);
        if found {
            self.bump();
        }
        found
    }

    fn at_eol_or_eof(&self) -> bool {
        matches!(self.peek(), None | Some(b'\n'))
    }

    fn skip_to_eol(&mut self) {
        while !self.at_eol_or_eof() {
            self.pos += 1;
        }
    }

    /// Skips blanks; true if that consumed a newline.
    fn skip_blanks(&mut self) -> bool {
        while let Some(b) = self.peek().filter(|b| is_space(*b)) {
            self.bump();
            if b == b'\n' {
                return true;
            }
        }
        false
    }

    fn next(&mut self, diagnostics: &mut Reporter) -> Token {
        loop {
            if self.skip_blanks() {
                return Token::EndOfLine;
            }
            if self.eat(b'#') {
                self.skip_to_eol();
                continue;
            }
            break;
        }
        self.token_line = self.line;
        let Some(first) = self.peek() else { return Token::EndOfFile };
        match first {
            b'<' => {
                self.bump();
                let start = self.pos;
                while self.peek() != Some(b'>') && !self.at_eol_or_eof() {
                    self.pos += 1;
                }
                let name = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
                if !self.eat(b'>') {
                    diagnostics.error(self.token_line, "unterminated keysym literal");
                    return Token::Error;
                }
                Token::Keysym(name)
            }
            b':' => {
                self.bump();
                Token::Colon
            }
            b'!' => {
                self.bump();
                Token::Bang
            }
            b'~' => {
                self.bump();
                Token::Tilde
            }
            b'"' => {
                self.bump();
                self.string(diagnostics)
            }
            b if b.is_ascii_alphabetic() || b == b'_' => {
                let start = self.pos;
                while self.peek().is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_') {
                    self.pos += 1;
                }
                let ident = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
                if ident == "include" {
                    Token::Include
                } else {
                    Token::Ident(ident)
                }
            }
            _ => {
                self.skip_to_eol();
                diagnostics.error(self.token_line, "unrecognized token");
                Token::Error
            }
        }
    }

    /// The rest of a string literal, after its opening quote.
    fn string(&mut self, diagnostics: &mut Reporter) -> Token {
        let mut buf = Vec::new();
        while !self.at_eol_or_eof() && self.peek() != Some(b'"') {
            if !self.eat(b'\\') {
                buf.push(self.bump().expect("not at end of file"));
                continue;
            }
            if self.eat(b'\\') {
                buf.push(b'\\');
            } else if self.eat(b'"') {
                buf.push(b'"');
            } else if self.eat(b'x') || self.eat(b'X') {
                match self.hex_escape() {
                    Some(b) if b != 0 => buf.push(b),
                    _ => diagnostics
                        .warning(self.line, "illegal hexadecimal escape sequence in string literal"),
                }
            } else {
                match self.octal_escape() {
                    Some(Some(b)) if b != 0 => buf.push(b),
                    Some(_) => {
                        diagnostics.warning(self.line, "illegal octal escape sequence in string literal")
                    }
                    // Unknown escape: libxkbcommon drops the backslash and keeps the character.
                    None => diagnostics.warning(self.line, "unknown escape sequence in string literal"),
                }
            }
        }
        if !self.eat(b'"') {
            diagnostics.error(self.token_line, "unterminated string literal");
            return Token::Error;
        }
        match String::from_utf8(buf) {
            Ok(text) => Token::Str(text),
            Err(_) => {
                diagnostics.error(self.token_line, "string literal is not a valid UTF-8 string");
                Token::Error
            }
        }
    }

    /// Up to two hex digits; `None` if there are none.
    fn hex_escape(&mut self) -> Option<u8> {
        let mut value = 0u8;
        let mut digits = 0;
        while digits < 2 {
            let Some(d) = self.peek().and_then(|b| (b as char).to_digit(16)) else { break };
            self.bump();
            value = value * 16 + d as u8;
            digits += 1;
        }
        (digits > 0).then_some(value)
    }

    /// Up to three octal digits. `None` if there are none, `Some(None)` if the value does
    /// not fit in a byte.
    fn octal_escape(&mut self) -> Option<Option<u8>> {
        let mut value = 0u8;
        let mut digits = 0;
        while digits < 3 {
            let Some(d) = self.peek().filter(|b| (b'0'..=b'7').contains(b)) else { break };
            self.bump();
            if value >= 0o40 {
                return Some(None);
            }
            value = value * 8 + (d - b'0');
            digits += 1;
        }
        (digits > 0).then_some(Some(value))
    }

    /// The quoted path after `include`, with `%` escapes expanded.
    fn include_path(&mut self, includes: &dyn Includes, diagnostics: &mut Reporter) -> Token {
        if self.skip_blanks() {
            return Token::EndOfLine;
        }
        self.token_line = self.line;
        if !self.eat(b'"') {
            diagnostics.error(self.token_line, "include statement must be followed by a path");
            return Token::Error;
        }
        let mut path = Vec::new();
        while !self.at_eol_or_eof() && self.peek() != Some(b'"') {
            if !self.eat(b'%') {
                path.push(self.bump().expect("not at end of file"));
                continue;
            }
            let (expand, name) = match self.bump() {
                Some(b'%') => {
                    path.push(b'%');
                    continue;
                }
                Some(b'H') => (Expand::Home, "%H"),
                Some(b'L') => (Expand::LocaleFile, "%L"),
                Some(b'S') => (Expand::SystemDir, "%S"),
                other => {
                    let shown = other.map_or(String::new(), |b| (b as char).to_string());
                    diagnostics
                        .error(self.token_line, &format!("unknown % format ({shown}) in include statement"));
                    return Token::Error;
                }
            };
            match includes.expand(expand) {
                Some(value) => path.extend_from_slice(value.as_bytes()),
                None => {
                    diagnostics.error(self.token_line, &format!("cannot expand {name} in include statement"));
                    return Token::Error;
                }
            }
        }
        if !self.eat(b'"') {
            diagnostics.error(self.token_line, "unterminated include statement");
            return Token::Error;
        }
        Token::Str(String::from_utf8_lossy(&path).into_owned())
    }
}

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c')
}

/// Collects diagnostics for one file.
struct Reporter<'a> {
    file: &'a str,
    out: &'a mut Vec<Diagnostic>,
    errors: usize,
}

impl Reporter<'_> {
    fn push(&mut self, line: u32, severity: Severity, message: &str) {
        self.out.push(Diagnostic { file: self.file.to_owned(), line, severity, message: message.to_owned() });
    }

    fn warning(&mut self, line: u32, message: &str) {
        self.push(line, Severity::Warning, message);
    }

    fn error(&mut self, line: u32, message: &str) {
        self.push(line, Severity::Error, message);
    }
}

/// How a production ended, when it did not add a rule.
enum Abandon {
    /// Skip the rest of the line.
    Skip(Token),
    /// Count an error, then skip the rest of the line.
    Error(Token),
}

struct Parser<'i> {
    rules: &'i mut RuleSet,
    includes: &'i mut dyn Includes,
    dialect: Dialect,
}

impl Parser<'_> {
    /// Resolves a `<name>` on the left-hand side.
    fn key_name(&self, name: &str) -> Option<Keysym> {
        uc_keysym::from_name(name).or_else(|| {
            let mut chars = name.chars();
            match (self.dialect, chars.next(), chars.next()) {
                (Dialect::WinCompose, Some(c), None) if !c.is_control() => Some(uc_keysym::from_char(c)),
                _ => None,
            }
        })
    }

    /// Parses one file into `self.rules`. False if the whole load must fail.
    fn parse(&mut self, text: &str, file: &str, depth: usize, out: &mut Vec<Diagnostic>) -> bool {
        let mut lexer = Lexer::new(text);
        let mut report = Reporter { file, out, errors: 0 };
        loop {
            let mut tok = lexer.next(&mut report);
            while tok == Token::EndOfLine {
                tok = lexer.next(&mut report);
            }
            let abandon = match tok {
                Token::EndOfFile => return true,
                Token::Include => match lexer.include_path(&*self.includes, &mut report) {
                    Token::Str(path) => match lexer.next(&mut report) {
                        Token::EndOfLine | Token::EndOfFile => {
                            let line = lexer.token_line;
                            if !self.include(&path, depth, line, &mut report) {
                                return false;
                            }
                            continue;
                        }
                        other => unexpected(other, lexer.token_line, &mut report),
                    },
                    other => unexpected(other, lexer.token_line, &mut report),
                },
                tok => match self.production(&mut lexer, tok, &mut report) {
                    Ok(()) => continue,
                    Err(abandon) => abandon,
                },
            };
            let mut tok = match abandon {
                Abandon::Skip(tok) => tok,
                Abandon::Error(tok) => {
                    report.errors += 1;
                    if report.errors > MAX_ERRORS {
                        report.error(lexer.token_line, "too many errors");
                        report.error(lexer.token_line, "failed to parse file");
                        return false;
                    }
                    tok
                }
            };
            while !matches!(tok, Token::EndOfLine | Token::EndOfFile) {
                tok = lexer.next(&mut report);
            }
            if tok == Token::EndOfFile {
                return true;
            }
        }
    }

    fn include(&mut self, path: &str, depth: usize, line: u32, report: &mut Reporter) -> bool {
        if depth >= MAX_INCLUDE_DEPTH {
            report.error(
                line,
                &format!(
                    "maximum include depth ({MAX_INCLUDE_DEPTH}) exceeded; maybe there is an include loop?"
                ),
            );
            return false;
        }
        let text = match self.includes.read(path) {
            Ok(text) => text,
            Err(e) => {
                report.error(line, &format!("failed to open included Compose file \"{path}\": {e}"));
                return false;
            }
        };
        let ok = self.parse(&text, path, depth + 1, report.out);
        if !ok {
            report.error(line, "failed to parse file");
        }
        ok
    }

    /// One `keys : output` line, starting at `tok`.
    fn production(
        &mut self,
        lexer: &mut Lexer,
        mut tok: Token,
        report: &mut Reporter,
    ) -> Result<(), Abandon> {
        let mut keys: Vec<Keysym> = Vec::new();
        // Left-hand side: optional modifiers before each keysym (parsed, then ignored:
        // libxkbcommon does not match on them either), up to the colon.
        loop {
            match tok {
                Token::Colon if keys.is_empty() => {
                    report.warning(
                        lexer.token_line,
                        "expected at least one keysym on left-hand side; skipping line",
                    );
                    return Err(Abandon::Skip(tok));
                }
                Token::Colon => break,
                Token::Ident(ref name) if name == "None" => tok = lexer.next(report),
                Token::Bang => {
                    tok = lexer.next(report);
                    tok = modifiers(lexer, tok, report)?;
                }
                Token::Ident(_) | Token::Tilde => tok = modifiers(lexer, tok, report)?,
                _ => {}
            }
            let Token::Keysym(name) = tok else { return Err(unexpected(tok, lexer.token_line, report)) };
            let Some(keysym) = self.key_name(&name) else {
                report.error(lexer.token_line, &format!("unrecognized keysym \"{name}\" on left-hand side"));
                return Err(Abandon::Error(Token::Keysym(name)));
            };
            let max_keys = match self.dialect {
                Dialect::Xkbcommon => MAX_KEYS,
                Dialect::WinCompose => MAX_KEYS_WINCOMPOSE,
            };
            if keys.len() == max_keys {
                report.warning(
                    lexer.token_line,
                    &format!("too many keysyms ({}) on left-hand side; skipping line", max_keys + 1),
                );
                return Err(Abandon::Skip(Token::Keysym(name)));
            }
            keys.push(keysym);
            tok = lexer.next(report);
        }

        // Right-hand side: an optional string, then an optional keysym, which ends it.
        let mut output = Output { text: None, keysym: None };
        let line = lexer.token_line;
        loop {
            match lexer.next(report) {
                Token::Str(text) => {
                    if output.text.is_some() {
                        report.warning(
                            lexer.token_line,
                            "right-hand side can have at most one string; skipping line",
                        );
                        return Err(Abandon::Skip(Token::Str(text)));
                    }
                    if text.len() > MAX_STRING_BYTES {
                        report.warning(lexer.token_line, "right-hand side string is too long; skipping line");
                        return Err(Abandon::Skip(Token::Str(text)));
                    }
                    output.text = Some(text);
                }
                Token::Ident(name) => {
                    let Some(keysym) = uc_keysym::from_name(&name) else {
                        report.error(
                            lexer.token_line,
                            &format!("unrecognized keysym \"{name}\" on right-hand side"),
                        );
                        return Err(Abandon::Error(Token::Ident(name)));
                    };
                    output.keysym = Some(keysym);
                    break;
                }
                Token::EndOfLine | Token::EndOfFile if output.text.is_none() => {
                    report.warning(
                        lexer.token_line,
                        "right-hand side must have at least one of string or keysym; skipping line",
                    );
                    return Err(Abandon::Skip(Token::EndOfLine));
                }
                Token::EndOfLine | Token::EndOfFile => break,
                other => return Err(unexpected(other, lexer.token_line, report)),
            }
        }
        let ended_on_keysym = output.keysym.is_some();
        if let Some(warning) = self.rules.add(&keys, output).warning() {
            report.warning(line, warning);
        }
        if ended_on_keysym && self.dialect == Dialect::WinCompose {
            // WinCompose ignores anything after the keysym, such as a second one
            // (`U2764 UFE0F`); libxkbcommon would read it as the start of another rule.
            let mut tok = lexer.next(report);
            while !matches!(tok, Token::EndOfLine | Token::EndOfFile) {
                tok = lexer.next(report);
            }
        }
        Ok(())
    }
}

/// Reads `[~]Name` modifiers until the next keysym, which it returns.
fn modifiers(lexer: &mut Lexer, mut tok: Token, report: &mut Reporter) -> Result<Token, Abandon> {
    loop {
        if tok == Token::Tilde {
            tok = lexer.next(report);
            if !matches!(tok, Token::Ident(_)) {
                return Err(unexpected(tok, lexer.token_line, report));
            }
        }
        let Token::Ident(name) = &tok else { return Ok(tok) };
        if !matches!(name.as_str(), "Shift" | "Ctrl" | "Alt" | "Meta" | "Lock" | "Caps") {
            report.error(lexer.token_line, &format!("unrecognized modifier \"{name}\""));
            return Err(Abandon::Error(tok));
        }
        tok = lexer.next(report);
    }
}

fn unexpected(tok: Token, line: u32, report: &mut Reporter) -> Abandon {
    if tok != Token::Error {
        report.error(line, "unexpected token");
    }
    Abandon::Error(tok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::includes::MapIncludes;
    use crate::rules::Rule;

    fn parse(text: &str) -> Loaded {
        load(text, "test", &mut MapIncludes::default()).unwrap()
    }

    fn rules(text: &str) -> Vec<(Vec<Keysym>, Option<String>, Option<Keysym>)> {
        parse(text)
            .rules
            .rules()
            .into_iter()
            .map(|Rule { keys, output }| (keys, output.text, output.keysym))
            .collect()
    }

    fn messages(text: &str) -> Vec<String> {
        parse(text).diagnostics.iter().map(ToString::to_string).collect()
    }

    const MULTI: Keysym = uc_keysym::sym::MULTI_KEY;

    #[test]
    fn reads_rules_with_string_and_keysym() {
        let text = "# comment\n<Multi_key> <o> <quotedbl>\t: \"ö\"\todiaeresis # LATIN SMALL LETTER O WITH DIAERESIS\n";
        assert_eq!(rules(text), [(vec![MULTI, 0x6f, 0x22], Some("ö".into()), Some(0xf6))]);
        assert!(messages(text).is_empty());
    }

    #[test]
    fn colon_may_touch_the_last_keysym_and_output_may_be_either_part() {
        let text = "<Multi_key> <a>: \"x\"\n<Multi_key> <b> : U2192\n";
        assert_eq!(
            rules(text),
            [(vec![MULTI, 0x61], Some("x".into()), None), (vec![MULTI, 0x62], None, Some(0x0100_2192))]
        );
    }

    #[test]
    fn string_escapes() {
        let text = r#"<a> : "\"\\\x41\101\x\q"
"#;
        assert_eq!(rules(text)[0].1.as_deref(), Some("\"\\AAq"));
        let warnings = messages(text);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("hexadecimal") && warnings[1].contains("unknown escape"));
    }

    #[test]
    fn octal_escapes_can_build_utf8() {
        assert_eq!(rules("<a> : \"\\316\\261\"\n")[0].1.as_deref(), Some("α"));
        let loaded = parse("<a> : \"\\377\"\n<b> : \"ok\"\n");
        assert_eq!(loaded.rules.len(), 1);
        assert!(loaded.diagnostics[0].message.contains("UTF-8"));
    }

    #[test]
    fn modifiers_are_accepted_and_ignored() {
        let text = "!Ctrl ~Shift <a> None <b> : \"x\"\n";
        assert_eq!(rules(text), [(vec![0x61, 0x62], Some("x".into()), None)]);
        assert!(messages("Hyper <a> : \"x\"\n")[0].contains("unrecognized modifier"));
    }

    #[test]
    fn bad_lines_are_skipped_with_their_line_number() {
        let text = "<nope> : \"x\"\n: \"y\"\n<a> <b>\n<a> : \"one\" \"two\"\n<a>\t: \n<c> : \"ok\"\n";
        assert_eq!(rules(text), [(vec![0x63], Some("ok".into()), None)]);
        let lines: Vec<u32> = parse(text).diagnostics.iter().map(|d| d.line).collect();
        assert_eq!(lines, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn eleven_keys_is_one_too_many() {
        let ten = "<a> ".repeat(10);
        assert_eq!(rules(&format!("{ten}: \"x\"\n")).len(), 1);
        assert!(messages(&format!("{ten}<a> : \"x\"\n"))[0].contains("too many keysyms"));
    }

    #[test]
    fn last_line_without_newline_still_counts() {
        assert_eq!(rules("<a> : \"x\"").len(), 1);
        assert_eq!(rules("<a> : x").len(), 1);
    }

    #[test]
    fn too_many_errors_fail_the_load() {
        let text = "<nope> : \"x\"\n".repeat(MAX_ERRORS + 1);
        let error = load(&text, "test", &mut MapIncludes::default()).unwrap_err();
        assert!(error.to_string().contains("failed to parse file"), "{error}");
        assert!(load(&"<nope> : \"x\"\n".repeat(MAX_ERRORS), "test", &mut MapIncludes::default()).is_ok());
    }

    #[test]
    fn includes_are_expanded_and_read_in_place() {
        let mut includes = MapIncludes::default()
            .with_home("/home/me")
            .with_file("/home/me/extra", "<a> : \"from include\"\n<b> : \"b\"\n");
        let text = "<a> : \"first\"\ninclude \"%H/extra\"\n<b> : \"last\"\n";
        let loaded = load(text, "main", &mut includes).unwrap();
        assert_eq!(loaded.rules.get(&[0x61]).unwrap().text.as_deref(), Some("from include"));
        assert_eq!(loaded.rules.get(&[0x62]).unwrap().text.as_deref(), Some("last"));
        let files: Vec<&str> = loaded.diagnostics.iter().map(|d| d.file.as_str()).collect();
        assert_eq!(files, ["/home/me/extra", "main"], "each override is reported where it happens");
    }

    #[test]
    fn include_failures_fail_the_load() {
        let missing = load("include \"/nowhere\"\n", "main", &mut MapIncludes::default()).unwrap_err();
        assert!(missing.to_string().contains("/nowhere"), "{missing}");
        let unset = load("include \"%H/x\"\n", "main", &mut MapIncludes::default()).unwrap();
        assert!(unset.diagnostics[0].message.contains("%H"), "an unexpandable path skips only its line");
        let mut looping = MapIncludes::default().with_file("/loop", "include \"/loop\"\n");
        let error = load("include \"/loop\"\n", "main", &mut looping).unwrap_err();
        assert!(error.diagnostics.iter().any(|d| d.message.contains("include depth")), "{error}");
    }
}
