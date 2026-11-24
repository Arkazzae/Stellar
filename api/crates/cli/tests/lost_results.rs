//! A command that changed something and then could not write its answer must
//! not end as if all was well.

mod support;

use support::Instance;

#[test]
fn a_recovery_whose_credential_cannot_be_written_fails_and_says_it_was_made() {
    let instance = Instance::started();

    let run = instance.run_into_a_closed_pipe(&[
        "--json",
        "security",
        "recover",
        "jo",
        "--reason",
        "lost phone",
        "--yes",
    ]);

    assert_eq!(run.exit, 1);
    assert!(
        run.error.contains("The change was made") && run.error.contains("not delivered"),
        "{}",
        run.error
    );
    // The change stands: the credential exists, and nobody was told what it is.
    let issued: i64 = instance
        .db()
        .query_row("SELECT COUNT(*) FROM \"CliRecoveryCredentials\"", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(issued, 1);
}

#[test]
fn another_change_that_cannot_be_reported_also_fails_and_says_so() {
    let instance = Instance::started();
    let run =
        instance.run_into_a_closed_pipe(&["config", "set", "instance.name", "Harbour", "--yes"]);
    assert_eq!(run.exit, 1);
    assert!(run.error.contains("The change was made"), "{}", run.error);
    assert!(run.error.contains("config.set"));
}

#[test]
fn a_command_that_only_reads_keeps_its_status_when_its_answer_goes_nowhere() {
    let instance = Instance::started();
    let run = instance.run_into_a_closed_pipe(&["users", "list"]);
    assert_eq!(run.exit, 0, "{}", run.error);
    assert!(run.error.is_empty());
}

#[test]
fn a_change_that_made_nothing_is_not_reported_as_made() {
    let instance = Instance::started();
    // No override exists, so nothing is removed.
    let run = instance.run_into_a_closed_pipe(&["secrets", "clear", "mail.smtp-password", "--yes"]);
    assert_eq!(run.exit, 0, "{}", run.error);
}
