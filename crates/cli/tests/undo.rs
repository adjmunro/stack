mod common;

use common::{stack, stderr, stdout};
use stack_testkit::Fixture;

/// `develop` (a trunk) ← `feat/a`, HEAD on `feat/a`.
fn repo() -> Fixture {
    let fixture = Fixture::new();
    fixture.commit("base.txt", "base", "feat: base");
    fixture.git(&["switch", "--quiet", "--create", "feat/a"]);
    fixture.commit("a.txt", "a", "feat: a");
    stack(&fixture, &["trunk", "add", "develop"]);
    fixture
}

#[test]
fn undo_redo_and_oplog() {
    let fixture = repo();
    stack(&fixture, &["pin"]);

    assert_eq!(
        stdout(&stack(&fixture, &["undo"])),
        "Undid #2: pin feat/a on develop\n"
    );
    assert_eq!(
        stdout(&stack(&fixture, &["undo"])),
        "Undid #1: trunk add develop\n"
    );
    assert_eq!(
        stderr(&stack(&fixture, &["undo"])),
        "error: nothing to undo\n"
    );
    assert_eq!(
        stdout(&stack(&fixture, &["redo"])),
        "Redid #1: trunk add develop\n"
    );

    assert_eq!(
        stdout(&stack(&fixture, &["oplog"])),
        "#5 redo #1: trunk add develop\n\
         #4 undo #1: trunk add develop\n\
         #3 undo #2: pin feat/a on develop\n\
         #2 pin feat/a on develop (undone)\n\
         #1 trunk add develop\n"
    );
    assert_eq!(
        stdout(&stack(&fixture, &["oplog", "-n", "1"])),
        "#5 redo #1: trunk add develop\n"
    );
}

#[test]
fn oplog_is_empty_before_any_command() {
    let fixture = Fixture::new();
    fixture.commit("base.txt", "base", "feat: base");
    let before = fixture.snapshot();

    assert_eq!(stdout(&stack(&fixture, &["oplog"])), "No operations yet.\n");
    assert_eq!(stdout(&stack(&fixture, &["oplog", "--json"])), "[]\n");
    fixture.assert_unchanged(&before);
}

#[test]
fn interrupted_operation_is_recovered_and_reported() {
    let fixture = repo();
    let mark = fixture.git(&["rev-parse", "refs/stack/trunks/develop"]);
    // As if `stack trunk remove develop` crashed after recording its intent, before touching refs.
    let database = fixture.path().join(".git/stack/stack.db");
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch(&format!(
            "INSERT INTO operation (id, kind, description, state, started_at)
                 VALUES (9, 'command', 'trunk remove develop', 'pending', 0);
             INSERT INTO ref_update (operation, name, old, new)
                 VALUES (9, 'refs/stack/trunks/develop', '{mark}', NULL);"
        ))
        .unwrap();
    drop(connection);

    let output = stack(&fixture, &["status"]);

    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "note: operation #9 (trunk remove develop) was interrupted; nothing had changed, and it is now marked failed\n"
    );
    assert!(
        stdout(&stack(&fixture, &["oplog", "-n", "1"]))
            .starts_with("#9 trunk remove develop (failed)")
    );
    assert_eq!(
        String::from_utf8(stack(&fixture, &["status"]).stderr).unwrap(),
        ""
    );
}
