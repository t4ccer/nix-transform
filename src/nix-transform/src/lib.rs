use std::{
    backtrace::Backtrace,
    borrow::Cow,
    fmt::Display,
    io::{self, Write},
    process::Command,
};
use tree_sitter::{Language, Parser};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Fetcher {
    FetchFromGitHub,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FetchAction {
    pub fetcher: Fetcher,
    pub old_hash: String,
    pub new_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InsertInBetween {
    pub prefix_offset: usize,
    pub to_insert: String,
    pub suffix_offset: usize,
}

impl InsertInBetween {
    pub fn modify<W>(&self, original: &str, mut w: impl Write) -> io::Result<()> {
        write!(w, "{}", &original[..self.prefix_offset])?;
        write!(w, "{}", &self.to_insert)?;
        write!(w, "{}", &original[self.suffix_offset..])?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UpdateFetcher {
    pub modification: InsertInBetween,
    pub action: FetchAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct UpdateFetcherInput {
    old_hash_attr: Span,
    argument: Span,
    fetcher: Fetcher,
}

#[derive(Debug)]
pub enum UpdateFetcherError {
    InvalidAttrMissingChild { source: Backtrace, missing: String },
    InvalidAttrSetInvalidKind { source: Backtrace, actual: String },
    InvalidAttrSetNoParent { source: Backtrace },
    InvalidFetcherCall { source: Backtrace },
    InvalidFetcher { fetcher: String },
    InvalidCursor,
    ParseError,
    CouldNotFetchGitHubHash,
    MissingHashAttribute,
}

impl Display for UpdateFetcherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateFetcherError::InvalidAttrMissingChild { source, missing } => {
                write!(f, "{source}\nAttribute set is missing a child: `{missing}`")
            }
            UpdateFetcherError::InvalidAttrSetInvalidKind { source, actual } => {
                write!(f, "{source}\nAttribute set has invalid kind: `{actual}`")
            }
            UpdateFetcherError::InvalidAttrSetNoParent { source } => {
                write!(f, "{source}\nAttribute set has no parent")
            }
            UpdateFetcherError::InvalidFetcherCall { source } => {
                write!(f, "{source}\nInvalid call to fetcher")
            }
            UpdateFetcherError::InvalidFetcher { fetcher } => {
                write!(f, "Invalid fetcher: `{fetcher}`")
            }
            UpdateFetcherError::InvalidCursor => write!(f, "Invalid cursor position"),
            UpdateFetcherError::ParseError => write!(f, "Nix parse error"),
            UpdateFetcherError::CouldNotFetchGitHubHash => {
                write!(f, "Could not fetch hash from GitHub")
            }
            UpdateFetcherError::MissingHashAttribute => write!(f, "Missing `hash` attribute"),
        }
    }
}

impl UpdateFetcherError {
    #[track_caller]
    fn invalid_attr_missing_child(missing: String) -> UpdateFetcherError {
        UpdateFetcherError::InvalidAttrMissingChild {
            source: Backtrace::capture(),
            missing,
        }
    }

    #[track_caller]
    fn invalid_attr_set_invalid_kind(actual: String) -> UpdateFetcherError {
        UpdateFetcherError::InvalidAttrSetInvalidKind {
            source: Backtrace::capture(),
            actual,
        }
    }

    #[track_caller]
    fn invalid_attrset_no_parent() -> UpdateFetcherError {
        UpdateFetcherError::InvalidAttrSetNoParent {
            source: Backtrace::capture(),
        }
    }

    #[track_caller]
    fn invalid_fetcher_call() -> UpdateFetcherError {
        UpdateFetcherError::InvalidFetcherCall {
            source: Backtrace::capture(),
        }
    }
}

fn update_fetcher_prepare(
    source: &str,
    cursor_byte_offset: usize,
) -> Result<UpdateFetcherInput, UpdateFetcherError> {
    let mut parser = Parser::new();
    parser
        .set_language(&Language::from(tree_sitter_nix::LANGUAGE))
        .expect("unreachable: Could not load nix parser");

    let tree = parser
        .parse(source, None)
        .ok_or(UpdateFetcherError::ParseError)?;

    let mut cursor_node = tree
        .root_node()
        .descendant_for_byte_range(cursor_byte_offset, cursor_byte_offset)
        .ok_or(UpdateFetcherError::InvalidCursor)?;

    let attr_set = loop {
        match cursor_node.kind() {
            "attrset_expression" => {
                break cursor_node;
            }
            "binding_set" => {
                let p = cursor_node
                    .parent()
                    .ok_or_else(|| UpdateFetcherError::invalid_attrset_no_parent())?;
                match p.kind() {
                    "attrset_expression" => cursor_node = p,
                    kind => Err(UpdateFetcherError::invalid_attr_set_invalid_kind(
                        String::from(kind),
                    ))?,
                };
            }
            "identifier" => {
                let p = cursor_node
                    .parent()
                    .ok_or_else(|| UpdateFetcherError::invalid_attrset_no_parent())?;
                match p.kind() {
                    "attrpath" => cursor_node = p,
                    kind => Err(UpdateFetcherError::invalid_attr_set_invalid_kind(
                        String::from(kind),
                    ))?,
                };
            }
            "attrpath" => {
                let p = cursor_node
                    .parent()
                    .ok_or_else(|| UpdateFetcherError::invalid_attrset_no_parent())?;
                match p.kind() {
                    "binding" => cursor_node = p,
                    kind => Err(UpdateFetcherError::invalid_attr_set_invalid_kind(
                        String::from(kind),
                    ))?,
                };
            }
            "binding" => {
                let p = cursor_node
                    .parent()
                    .ok_or_else(|| UpdateFetcherError::invalid_attrset_no_parent())?;
                match p.kind() {
                    "binding_set" => cursor_node = p,
                    kind => Err(UpdateFetcherError::invalid_attr_set_invalid_kind(
                        String::from(kind),
                    ))?,
                };
            }
            "=" => {
                let p = cursor_node
                    .parent()
                    .ok_or_else(|| UpdateFetcherError::invalid_attrset_no_parent())?;
                match p.kind() {
                    "binding" => cursor_node = p,
                    kind => Err(UpdateFetcherError::invalid_attr_set_invalid_kind(
                        String::from(kind),
                    ))?,
                };
            }
            "\"" => {
                let p = cursor_node
                    .parent()
                    .ok_or_else(|| UpdateFetcherError::invalid_attrset_no_parent())?;
                match p.kind() {
                    "string_expression" => cursor_node = p,
                    kind => Err(UpdateFetcherError::invalid_attr_set_invalid_kind(
                        String::from(kind),
                    ))?,
                };
            }
            "string_expression" => {
                let p = cursor_node
                    .parent()
                    .ok_or_else(|| UpdateFetcherError::invalid_attrset_no_parent())?;
                match p.kind() {
                    "binding" => cursor_node = p,
                    kind => Err(UpdateFetcherError::invalid_attr_set_invalid_kind(
                        String::from(kind),
                    ))?,
                };
            }
            "string_fragment" => {
                let p = cursor_node
                    .parent()
                    .ok_or_else(|| UpdateFetcherError::invalid_attrset_no_parent())?;
                match p.kind() {
                    "string_expression" => cursor_node = p,
                    kind => Err(UpdateFetcherError::invalid_attr_set_invalid_kind(
                        String::from(kind),
                    ))?,
                };
            }
            ";" => {
                let p = cursor_node
                    .parent()
                    .ok_or_else(|| UpdateFetcherError::invalid_attrset_no_parent())?;
                match p.kind() {
                    "binding" => cursor_node = p,
                    kind => Err(UpdateFetcherError::invalid_attr_set_invalid_kind(
                        String::from(kind),
                    ))?,
                };
            }
            "{" => {
                let p = cursor_node
                    .parent()
                    .ok_or_else(|| UpdateFetcherError::invalid_attrset_no_parent())?;
                match p.kind() {
                    "attrset_expression" => cursor_node = p,
                    kind => Err(UpdateFetcherError::invalid_attr_set_invalid_kind(
                        String::from(kind),
                    ))?,
                };
            }
            "}" => {
                let p = cursor_node
                    .parent()
                    .ok_or_else(|| UpdateFetcherError::invalid_attrset_no_parent())?;
                match p.kind() {
                    "attrset_expression" => cursor_node = p,
                    kind => Err(UpdateFetcherError::invalid_attr_set_invalid_kind(
                        String::from(kind),
                    ))?,
                };
            }
            kind => Err(UpdateFetcherError::invalid_attr_set_invalid_kind(
                String::from(kind),
            ))?,
        }
    };

    let func_apply = attr_set
        .parent()
        .ok_or_else(|| UpdateFetcherError::invalid_attrset_no_parent())?;
    assert_eq!(func_apply.kind(), "apply_expression");

    let mut function = func_apply
        .child_by_field_name("function")
        .ok_or_else(|| UpdateFetcherError::invalid_fetcher_call())?;
    let function_name = loop {
        match function.kind() {
            "select_expression" => {
                function = function
                    .child_by_field_name("attrpath")
                    .ok_or_else(|| UpdateFetcherError::invalid_fetcher_call())?;
                function = function
                    .child_by_field_name("attr")
                    .ok_or_else(|| UpdateFetcherError::invalid_fetcher_call())?;
            }
            "variable_expression" => {
                function = function
                    .child_by_field_name("name")
                    .ok_or_else(|| UpdateFetcherError::invalid_fetcher_call())?;
            }
            "identifier" => {
                let start = function.start_byte();
                let end = function.end_byte();
                break &source[start..end];
            }
            unknown_kind => Err(UpdateFetcherError::invalid_attr_set_invalid_kind(
                String::from(unknown_kind),
            ))?,
        }
    };

    let fetcher = match function_name {
        "fetchFromGitHub" => Fetcher::FetchFromGitHub,
        unknown_fetcher => Err(UpdateFetcherError::InvalidFetcher {
            fetcher: String::from(unknown_fetcher),
        })?,
    };

    let argument = func_apply
        .child_by_field_name("argument")
        .ok_or_else(|| UpdateFetcherError::invalid_fetcher_call())?;
    let binding_set = argument
        .child(1)
        .ok_or_else(|| UpdateFetcherError::invalid_fetcher_call())?;
    let mut old_hash = None;
    for c in binding_set.children(&mut binding_set.walk()) {
        let attr = c.child_by_field_name("attrpath").ok_or_else(|| {
            UpdateFetcherError::invalid_attr_missing_child(String::from("attrpath"))
        })?;
        let expr = c.child_by_field_name("expression").ok_or_else(|| {
            UpdateFetcherError::invalid_attr_missing_child(String::from("expression"))
        })?;
        if &source[attr.start_byte()..attr.end_byte()] == "hash" {
            old_hash = Some(Span {
                start: expr.start_byte(),
                end: expr.end_byte(),
            });
        }
    }

    // TODO: Generate it ad-hoc
    let Some(old_hash_attr) = old_hash else {
        Err(UpdateFetcherError::MissingHashAttribute)?
    };

    let old_hash_raw = &source[old_hash_attr.start..old_hash_attr.end];
    assert!(old_hash_raw.len() >= 2, "string hash must have quotes");

    Ok(UpdateFetcherInput {
        old_hash_attr,
        argument: Span {
            start: argument.start_byte(),
            end: argument.end_byte(),
        },
        fetcher,
    })
}

pub fn update_fetcher(
    source: &str,
    cursor_byte_offset: usize,
) -> Result<UpdateFetcher, UpdateFetcherError> {
    let UpdateFetcherInput {
        old_hash_attr,
        argument,
        fetcher,
    } = update_fetcher_prepare(source, cursor_byte_offset)?;

    let old_hash = &source[old_hash_attr.start + 1..old_hash_attr.end - 1];

    let raw_attrset = match old_hash {
        "" => Cow::Borrowed(&source[argument.start..argument.end]),
        _ => {
            let mut raw_attrset = String::new();
            raw_attrset += &source[argument.start..old_hash_attr.start];
            raw_attrset += "\"\"";
            raw_attrset += &source[old_hash_attr.end..argument.end];
            Cow::Owned(raw_attrset)
        }
    };

    let new_hash = match fetcher {
        Fetcher::FetchFromGitHub => fetch_github_hash(&raw_attrset)?,
    };

    Ok(UpdateFetcher {
        modification: InsertInBetween {
            prefix_offset: old_hash_attr.start,
            to_insert: new_hash.clone(),
            suffix_offset: old_hash_attr.end,
        },
        action: FetchAction {
            fetcher,
            old_hash: String::from(old_hash),
            new_hash,
        },
    })
}

fn fetch_github_hash(attrset: &str) -> Result<String, UpdateFetcherError> {
    // TODO: Use nix interpreter directly
    let cmd = Command::new("nix")
        .args([
            "build",
            "--impure",
            "--expr",
            &format!("with import <nixpkgs> {{}}; fetchFromGitHub {}", attrset),
        ])
        .output()
        .map_err(|_| UpdateFetcherError::CouldNotFetchGitHubHash)?;

    let out = String::from_utf8(cmd.stderr)
        .ok()
        .ok_or(UpdateFetcherError::CouldNotFetchGitHubHash)?;
    let prefix = "got:    ";

    let start = out
        .find(prefix)
        .ok_or(UpdateFetcherError::CouldNotFetchGitHubHash)?;
    let out = &out[start..];
    let end = out
        .find("\n")
        .ok_or(UpdateFetcherError::CouldNotFetchGitHubHash)?;

    Ok(String::from(&out[prefix.len()..end]))
}

macro_rules! mk_test {
    ($name:ident, cursor_range: $cursor_range:expr, old_hash_attr: $old_hash_attr:expr, fetcher: $fetcher:expr) => {
        #[test]
        fn $name() {
            use std::fs;

            let source = fs::read_to_string(concat!("test/", stringify!($name), ".nix"))
                .expect("Could not read input file");
            for offset in $cursor_range.start..$cursor_range.end {
                assert_eq!(
                    update_fetcher_prepare(&source, offset).unwrap(),
                    UpdateFetcherInput {
                        old_hash_attr: $old_hash_attr,
                        argument: Span {
                            start: $cursor_range.start - 1,
                            end: $cursor_range.end,
                        },
                        fetcher: $fetcher,
                    }
                );
            }
        }
    };
}

mk_test! {
    github_plain_empty_hash,
    cursor_range: Span {
        start: 54,
        end: 129
    },
    old_hash_attr: Span {
        start: 124,
        end: 126
    },
    fetcher: Fetcher::FetchFromGitHub
}

mk_test! {
    github_plain_invalid_hash,
    cursor_range: Span {
        start: 54,
        end: 180,
    },
    old_hash_attr: Span {
        start: 124,
        end: 177,
    },
    fetcher: Fetcher::FetchFromGitHub
}

mk_test! {
    github_attr_empty_hash,
    cursor_range: Span {
        start: 59,
        end: 134,
    },
    old_hash_attr: Span {
        start: 129,
        end: 131,
    },
    fetcher: Fetcher::FetchFromGitHub
}

mk_test! {
    github_attr_invalid_hash,
    cursor_range: Span {
        start: 59,
        end: 185,
    },
    old_hash_attr: Span {
        start: 129,
        end: 182,
    },
    fetcher: Fetcher::FetchFromGitHub
}
