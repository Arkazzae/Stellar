//! Asking for a secret at a terminal: what the command does when the prompt
//! cannot hide the input, when Ctrl+C comes, and in what order the value is
//! judged.

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use stellarr_cli::terminal::PromptFailure;
use support::Instance;

const SET: [&str; 4] = ["--json", "secrets", "set", "mail.smtp-password"];

fn rows(instance: &Instance) -> (i64, i64) {
    let connection = instance.db();
    let count = |table: &str| -> i64 {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| {
                row.get(0)
            })
            .unwrap()
    };
    (count("InstanceSecrets"), count("OperatorAuditEntries"))
}

#[test]
fn a_prompt_that_cannot_hide_the_input_reads_nothing_and_says_why() {
    let instance = Instance::started();
    let mut args = SET.to_vec();
    args.push("--yes");
    let run = instance.run_prompted(
        &args,
        Box::new(|| Err(PromptFailure::EchoStays)),
        Arc::new(AtomicBool::new(false)),
    );
    assert_eq!(
        (run.exit, run.code().as_str()),
        (2, "secret_value_required")
    );
    let message = run.json()["error"]["message"].as_str().unwrap().to_string();
    assert!(
        message.contains("stty -echo") && message.contains("--stdin"),
        "{message}"
    );
    assert_eq!(rows(&instance), (0, 0));
}

#[test]
fn ctrl_c_at_the_prompt_is_a_cancellation_not_an_invalid_secret() {
    let instance = Instance::started();
    let mut args = SET.to_vec();
    args.push("--yes");

    let flag = Arc::new(AtomicBool::new(false));
    let seen = flag.clone();
    let interrupted = instance.run_prompted(
        &args,
        Box::new(move || {
            seen.store(true, Ordering::Relaxed);
            Err(PromptFailure::Interrupted)
        }),
        flag,
    );
    assert_eq!(
        (interrupted.exit, interrupted.code().as_str()),
        (2, "cancelled")
    );

    // The empty line that arrives with the interrupt is not judged either.
    let flag = Arc::new(AtomicBool::new(false));
    let seen = flag.clone();
    let with_a_line = instance.run_prompted(
        &args,
        Box::new(move || {
            seen.store(true, Ordering::Relaxed);
            Ok(String::new())
        }),
        flag,
    );
    assert_eq!(
        (with_a_line.exit, with_a_line.code().as_str()),
        (2, "cancelled")
    );
    assert_eq!(rows(&instance), (0, 0));
}

#[test]
fn without_an_interrupt_a_blank_value_is_judged_and_a_good_one_is_stored() {
    let instance = Instance::started();
    let mut args = SET.to_vec();
    args.push("--yes");
    let blank = instance.run_prompted(
        &args,
        Box::new(|| Ok("   ".to_string())),
        Arc::new(AtomicBool::new(false)),
    );
    assert_eq!((blank.exit, blank.code().as_str()), (2, "invalid_secret"));
    assert_eq!(rows(&instance), (0, 0));

    let stored = instance.run_prompted(
        &args,
        Box::new(|| Ok("hunter2".to_string())),
        Arc::new(AtomicBool::new(false)),
    );
    assert_eq!(stored.exit, 0, "{}", stored.error);
    assert!(!stored.output.contains("hunter2"));
    assert_eq!(rows(&instance), (1, 1));
}
