//! The command line: a handful of global switches, and the rest left to the
//! command, so that `stellarr help` can explain a mistake without opening a
//! database.

use crate::errors::CliError;

const FLAGS: [&str; 8] = [
    "--json",
    "--yes",
    "-y",
    "--stdin",
    "--all",
    "--help",
    "-h",
    "--version",
];
const VALUED: [&str; 3] = ["--data-root", "--user", "--reason"];

#[derive(Debug, Clone, Default)]
pub struct Invocation {
    pub positionals: Vec<String>,
    pub data_root: Option<String>,
    pub user: Option<String>,
    pub reason: Option<String>,
    pub json: bool,
    pub yes: bool,
    pub stdin: bool,
    pub all: bool,
    pub help: bool,
    pub version: bool,
}

impl Invocation {
    pub fn parse(args: &[String]) -> Result<Invocation, CliError> {
        let mut invocation = Invocation::default();
        let mut options_ended = false;
        let mut index = 0;
        while index < args.len() {
            let argument = &args[index];
            if !options_ended && argument == "--" {
                options_ended = true;
                index += 1;
                continue;
            }
            if !options_ended && argument.starts_with('-') && argument != "-" {
                let (name, inline) = match argument.find('=') {
                    Some(at) => (&argument[..at], Some(argument[at + 1..].to_string())),
                    None => (argument.as_str(), None),
                };
                if VALUED.contains(&name) {
                    let value = match inline {
                        Some(value) => value,
                        None => next_value(args, &mut index, name)?,
                    };
                    match name {
                        "--data-root" => {
                            if is_blank(&value) {
                                return Err(CliError::usage("--data-root needs a non-empty path."));
                            }
                            invocation.data_root = Some(trim(&value));
                        }
                        "--reason" => {
                            if is_blank(&value) {
                                return Err(CliError::usage("--reason needs a non-empty value."));
                            }
                            invocation.reason = Some(trim(&value));
                        }
                        _ => invocation.user = Some(value),
                    }
                    index += 1;
                    continue;
                }
                if FLAGS.contains(&name) {
                    if inline.is_some() {
                        return Err(CliError::usage(format!("{name} does not take a value.")));
                    }
                    match name {
                        "--json" => invocation.json = true,
                        "--yes" | "-y" => invocation.yes = true,
                        "--stdin" => invocation.stdin = true,
                        "--all" => invocation.all = true,
                        "--help" | "-h" => invocation.help = true,
                        _ => invocation.version = true,
                    }
                    index += 1;
                    continue;
                }
                return Err(CliError::usage(format!("Unknown option '{name}'.")));
            }
            invocation.positionals.push(argument.clone());
            index += 1;
        }
        Ok(invocation)
    }

    /// Whether `--json` appeared, even when parsing later failed.
    pub fn json_requested(args: &[String]) -> bool {
        args.iter()
            .any(|argument| argument == "--json" || argument.starts_with("--json="))
    }
}

/// The value after the option at `index`, which is left at the value.
fn next_value(args: &[String], index: &mut usize, name: &str) -> Result<String, CliError> {
    let Some(next) = args.get(*index + 1) else {
        return Err(CliError::usage(format!("{name} needs a value.")));
    };
    if next.starts_with('-') && next != "-" {
        // Do not swallow the next option as a value. A deliberately
        // dash-prefixed value still has the explicit --name=value form.
        return Err(CliError::usage(format!(
            "{name} needs a value; '{next}' looks like another option."
        )));
    }
    *index += 1;
    Ok(next.clone())
}

/// What .NET's `string.IsNullOrWhiteSpace` and `Trim` call white space.
pub fn is_blank(text: &str) -> bool {
    text.chars().all(char::is_whitespace)
}

pub fn trim(text: &str) -> String {
    text.trim_matches(char::is_whitespace).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Invocation, CliError> {
        Invocation::parse(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn options_may_stand_anywhere_and_a_double_dash_ends_them() {
        let invocation = parse(&["config", "--json", "set", "k", "-y", "--", "--yes"]).unwrap();

        assert_eq!(invocation.positionals, ["config", "set", "k", "--yes"]);
        assert!(invocation.json && invocation.yes);
    }

    #[test]
    fn a_valued_option_takes_the_next_word_or_its_own_equals_form() {
        assert_eq!(
            parse(&["--data-root", "/x", "status"])
                .unwrap()
                .data_root
                .as_deref(),
            Some("/x")
        );
        assert_eq!(
            parse(&["--data-root=--virtual", "x"])
                .unwrap()
                .data_root
                .as_deref(),
            Some("--virtual")
        );
        assert_eq!(parse(&["--user=", "x"]).unwrap().user.as_deref(), Some(""));
    }

    #[test]
    fn every_mistake_is_a_usage_error_with_the_words_of_the_first_tool() {
        for (args, message) in [
            (vec!["--data-root"], "--data-root needs a value."),
            (
                vec!["--data-root", "--json"],
                "--data-root needs a value; '--json' looks like another option.",
            ),
            (
                vec!["--data-root", "  "],
                "--data-root needs a non-empty path.",
            ),
            (vec!["--data-root="], "--data-root needs a non-empty path."),
            (vec!["--reason", ""], "--reason needs a non-empty value."),
            (vec!["--json=1"], "--json does not take a value."),
            (vec!["--nope"], "Unknown option '--nope'."),
            (vec!["--nope=3"], "Unknown option '--nope'."),
        ] {
            let error = parse(&args).unwrap_err();
            assert_eq!(
                (error.code, error.message.as_str(), error.exit),
                ("usage", message, 2),
                "{args:?}"
            );
        }
    }
}
