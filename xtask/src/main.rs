//! Development tasks.
//!
//! - `cargo xtask data KEYSYMDEF_H COMPOSE_PRE` regenerates the checked-in data:
//!   `crates/uc-keysym/src/generated.rs` from xorgproto's `include/X11/keysymdef.h`
//!   (<https://gitlab.freedesktop.org/xorg/proto/xorgproto>), and
//!   `crates/uc-xcompose/data/en_US.UTF-8.Compose` from libX11's
//!   `nls/en_US.UTF-8/Compose.pre` (<https://gitlab.freedesktop.org/xorg/lib/libx11>),
//!   preprocessed the way libX11's build does it. The inputs are paths, not URLs, so a run
//!   never depends on the network.
//! - `cargo xtask assets` redraws the icon and the MSIX logos in `packaging/assets`.
//! - `package`, `bundle`, `checksums` and `winget` build the release files; see `package.rs`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod images;
mod package;

#[allow(dead_code)] // `Art::Active` and friends are for the tray, not the installers.
#[path = "../../crates/uc-app/src/icon_art.rs"]
mod icon_art;

type Error = Box<dyn std::error::Error>;

const USAGE: &str = "usage:
  cargo xtask data <keysymdef.h> <Compose.pre>
  cargo xtask assets
  cargo xtask package --target <triple> --bins <dir> [--out <dir>] [--publisher <subject>]
  cargo xtask bundle [--out <dir>]
  cargo xtask checksums [--out <dir>]
  cargo xtask winget --url-base <url> [--out <dir>]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["data", keysymdef, compose] => data(Path::new(keysymdef), Path::new(compose)),
        ["assets"] => assets(),
        ["package", ref rest @ ..] => package::package(rest),
        ["bundle", ref rest @ ..] => package::bundle(rest),
        ["checksums", ref rest @ ..] => package::checksums(rest),
        ["winget", ref rest @ ..] => package::winget(rest),
        _ => Err(USAGE.into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

pub(crate) fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask sits in the workspace root").to_owned()
}

/// The app icon at the sizes Windows asks for, and the MSIX logos, from `icon_art`.
fn assets() -> Result<(), Error> {
    use icon_art::{pixels, Art};
    let dir = root().join("packaging/assets");
    std::fs::create_dir_all(&dir)?;
    let sizes = [16, 20, 24, 32, 40, 48, 64, 256];
    let ico = images::ico(&sizes.iter().map(|&s| (s, pixels(Art::App, s))).collect::<Vec<_>>());
    std::fs::write(dir.join("unicompose.ico"), ico)?;
    for (name, size) in [("Square44x44Logo.png", 44), ("Square150x150Logo.png", 150), ("StoreLogo.png", 50)] {
        std::fs::write(dir.join(name), images::png(&pixels(Art::App, size), size, size))?;
    }
    println!("wrote {}", dir.display());
    Ok(())
}

fn data(keysymdef: &Path, compose: &Path) -> Result<(), Error> {
    let header = std::fs::read_to_string(keysymdef)?;
    let out = root().join("crates/uc-keysym/src/generated.rs");
    let (source, count) = keysyms(&header)?;
    std::fs::write(&out, source)?;
    println!("wrote {} ({count} keysym names)", out.display());

    let pre = std::fs::read_to_string(compose)?;
    let out = root().join("crates/uc-xcompose/data/en_US.UTF-8.Compose");
    let text = preprocess_compose(&pre)?;
    std::fs::write(&out, &text)?;
    println!("wrote {} ({} lines)", out.display(), text.lines().count());
    Ok(())
}

struct Define<'a> {
    name: &'a str,
    value: u32,
    /// The code point, when the header says the keysym is exactly that character.
    unicode: Option<u32>,
    deprecated: bool,
}

/// Reads `#define XK_name 0xVALUE /* U+XXXX NAME */` lines. Only a bare `U+XXXX` comment
/// is an exact mapping: `(U+XXXX)` and `<U+XXXX>` mark approximate ones.
fn parse_define(line: &str) -> Option<Define<'_>> {
    let rest = line.strip_prefix("#define XK_")?;
    let (name, rest) = rest.split_once(char::is_whitespace)?;
    let rest = rest.trim_start();
    let (value, comment) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let value = u32::from_str_radix(value.strip_prefix("0x")?, 16).ok()?;
    let comment = comment.trim();
    let body = comment
        .strip_prefix("/*")
        .map(|c| c.trim_end_matches("*/"))
        .or_else(|| comment.strip_prefix("//"))
        .unwrap_or("")
        .trim_start();
    let unicode = body
        .strip_prefix("U+")
        .and_then(|hex| u32::from_str_radix(hex.split(|c: char| !c.is_ascii_hexdigit()).next()?, 16).ok());
    Some(Define {
        name,
        value,
        unicode,
        deprecated: body.contains("deprecated") && !body.contains("non-deprecated"),
    })
}

fn keysyms(header: &str) -> Result<(String, usize), Error> {
    let defines: Vec<Define> = header.lines().filter_map(parse_define).collect();
    if defines.len() < 2000 {
        return Err(format!("only {} keysyms found; is this keysymdef.h?", defines.len()).into());
    }
    let mut by_name = BTreeMap::new();
    // The first name defined for a value is its canonical name, unless it is deprecated.
    let mut by_value: BTreeMap<u32, (&str, bool)> = BTreeMap::new();
    let mut to_unicode = BTreeMap::new();
    for d in &defines {
        if by_name.insert(d.name, d.value).is_some() {
            return Err(format!("keysym {} is defined twice", d.name).into());
        }
        // Keep the first name, unless it is deprecated and this one is not.
        let replace = by_value.get(&d.value).is_none_or(|&(_, deprecated)| deprecated && !d.deprecated);
        if replace {
            by_value.insert(d.value, (d.name, d.deprecated));
        }
        // Latin-1 and 0x01000000 + code point are mapped by formula; the table holds the rest.
        if let Some(cp) = d.unicode.filter(|_| d.value >= 0x100 && d.value < 0x0100_0000) {
            to_unicode.entry(d.value).or_insert(cp);
        }
    }

    let license = header
        .split("******************************************************************/")
        .next()
        .ok_or("keysymdef.h has no license block")?
        .trim_start_matches("/***********************************************************")
        .trim();
    let mut out = String::new();
    writeln!(
        out,
        "// @generated by `cargo xtask data` from xorgproto's include/X11/keysymdef.h. Do not edit."
    )?;
    writeln!(out, "//")?;
    for line in license.lines().map(str::trim_end) {
        if line.is_empty() {
            writeln!(out, "//")?;
        } else {
            writeln!(out, "// {line}")?;
        }
    }
    writeln!(out, "\n/// Every keysym name, sorted by name.")?;
    writeln!(out, "pub(crate) static BY_NAME: &[(&str, u32)] = &[")?;
    for (name, value) in &by_name {
        writeln!(out, "    ({name:?}, 0x{value:x}),")?;
    }
    writeln!(out, "];\n\n/// The canonical name of every keysym value, sorted by value.")?;
    writeln!(out, "pub(crate) static BY_VALUE: &[(u32, &str)] = &[")?;
    for (value, (name, _)) in &by_value {
        writeln!(out, "    (0x{value:x}, {name:?}),")?;
    }
    writeln!(
        out,
        "];\n\n/// Keysyms that are exactly one Unicode character, outside the ranges mapped by formula."
    )?;
    writeln!(out, "pub(crate) static TO_UNICODE: &[(u32, u32)] = &[")?;
    for (value, cp) in &to_unicode {
        writeln!(out, "    (0x{value:x}, 0x{cp:04x}),")?;
    }
    writeln!(out, "];")?;
    Ok((out, by_name.len()))
}

/// Does what libX11's build does to `Compose.pre`: `XCOMM` starts a `#` comment, and C
/// comments disappear.
fn preprocess_compose(pre: &str) -> Result<String, Error> {
    let mut out = String::with_capacity(pre.len());
    let mut rest = pre;
    // Strip /* ... */ first; they can span lines, and never occur inside rule strings here.
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        let end = rest[start..].find("*/").ok_or("unterminated /* comment in Compose.pre")?;
        rest = &rest[start + end + 2..];
    }
    out.push_str(rest);
    let mut text = String::with_capacity(out.len());
    for line in out.lines() {
        let line = match line.strip_prefix("XCOMM") {
            Some(comment) => format!("#{comment}"),
            None => line.to_owned(),
        };
        // Comments removed from the middle of a line leave trailing blanks behind.
        text.push_str(line.trim_end());
        text.push('\n');
    }
    // Collapse the blank runs left where multi-line comments were.
    while text.contains("\n\n\n") {
        text = text.replace("\n\n\n", "\n\n");
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_exact_and_approximate_mappings() {
        let d =
            parse_define("#define XK_Greek_alpha   0x07e1  /* U+03B1 GREEK SMALL LETTER ALPHA */").unwrap();
        assert_eq!((d.name, d.value, d.unicode), ("Greek_alpha", 0x7e1, Some(0x3b1)));
        let d = parse_define("#define XK_KP_0   0xffb0  /*<U+0030 DIGIT ZERO>*/").unwrap();
        assert_eq!(d.unicode, None);
        let d = parse_define("#define XK_leftanglebracket 0x0abc /*(U+2329 LEFT-POINTING ANGLE BRACKET)*/")
            .unwrap();
        assert_eq!(d.unicode, None);
        let d = parse_define("#define XK_dead_acute 0xfe51").unwrap();
        assert_eq!((d.value, d.unicode, d.deprecated), (0xfe51, None, false));
        assert!(parse_define("#define XK_quoteleft 0x0060  /* deprecated */").unwrap().deprecated);
        assert!(parse_define(" *     #define XK_space 0x0020").is_none());
    }

    #[test]
    fn preprocessing_turns_xcomm_into_comments_and_drops_c_comments() {
        let pre = "XCOMM title\n/* one\n * two */\n<a> : \"b\"\t/* note */\n";
        assert_eq!(preprocess_compose(pre).unwrap(), "# title\n\n<a> : \"b\"\n");
    }
}
