//! Property tests over randomly generated stacks.

mod common;

use std::collections::BTreeMap;

use common::*;
use proptest::prelude::*;
use stack_testkit::Fixture;

/// One generated branch: its parent (`None`: the trunk), how many commits it has, and how its last commit is
/// rewritten after its children were built on it, if at all.
#[derive(Debug, Clone)]
struct Spec {
    parent: Option<usize>,
    commits: usize,
    rewrite: Rewrite,
}

#[derive(Debug, Clone, Copy)]
enum Rewrite {
    None,
    Reword,
    Amend,
}

/// Up to five branches, each built on the trunk or an earlier branch; then 0–2 trunk commits.
fn stacks() -> impl Strategy<Value = (Vec<Spec>, usize)> {
    let count = 1..=5usize;
    count.prop_flat_map(|count| {
        let specs: Vec<_> = (0..count)
            .map(|index| {
                let parent = if index == 0 {
                    Just(None).boxed()
                } else {
                    prop::option::of(0..index).boxed()
                };
                let rewrite = prop_oneof![
                    Just(Rewrite::None),
                    Just(Rewrite::Reword),
                    Just(Rewrite::Amend)
                ];
                (parent, 1..=2usize, rewrite).prop_map(|(parent, commits, rewrite)| Spec {
                    parent,
                    commits,
                    rewrite,
                })
            })
            .collect();
        (specs, 0..=2usize)
    })
}

fn name(index: usize) -> String {
    format!("b{index}")
}

/// Builds the stacks. Every commit touches its own file, so nothing conflicts. Returns each branch's expected
/// subjects, newest first.
fn build(specs: &[Spec], trunk_commits: usize) -> (Fixture, BTreeMap<String, Vec<String>>) {
    let fixture = repo();
    let mut subjects = BTreeMap::new();
    for (index, spec) in specs.iter().enumerate() {
        let parent = spec.parent.map_or_else(|| "develop".to_owned(), name);
        fixture.git(&["switch", "--quiet", "--create", &name(index), &parent]);
        let mut own = Vec::new();
        for commit in 0..spec.commits {
            let subject = format!("feat: {} {commit}", name(index));
            fixture.commit(&format!("{}-{commit}.txt", name(index)), "x", &subject);
            own.insert(0, subject);
        }
        subjects.insert(name(index), own);
    }
    // Rewrite parents after their children exist, as happens mid-review.
    for (index, spec) in specs.iter().enumerate() {
        fixture.git(&["switch", "--quiet", &name(index)]);
        let last = spec.commits - 1;
        match spec.rewrite {
            Rewrite::None => {}
            Rewrite::Reword => {
                let subject = format!("feat: {} {last}, reworded", name(index));
                fixture.git(&["commit", "--quiet", "--amend", "--message", &subject]);
                subjects.get_mut(&name(index)).unwrap()[0] = subject;
            }
            Rewrite::Amend => {
                fixture.write(&format!("{}-{last}.txt", name(index)), "amended");
                fixture.git(&["commit", "--quiet", "--all", "--amend", "--no-edit"]);
            }
        }
    }
    fixture.git(&["switch", "--quiet", "develop"]);
    for commit in 0..trunk_commits {
        fixture.commit(
            &format!("trunk-{commit}.txt"),
            "x",
            &format!("feat: trunk {commit}"),
        );
    }
    (fixture, subjects)
}

fn branch_tips(fixture: &Fixture, specs: &[Spec]) -> Vec<String> {
    (0..specs.len())
        .map(|index| tip(fixture, &name(index)))
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    #[test]
    fn restack_recovers_parents_predicts_and_preserves_commits_and_undoes((specs, trunk_commits) in stacks()) {
        let (fixture, subjects) = build(&specs, trunk_commits);

        // Auto-tracking recovers exactly the generated parents, rewrites and all.
        let tree = tree(&fixture);
        for (index, spec) in specs.iter().enumerate() {
            let expected = spec.parent.map_or_else(|| "develop".to_owned(), name);
            prop_assert_eq!(parent(&tree, &name(index)).map(|parent| parent.name), Some(expected), "b{}", index);
        }

        // check predicts exactly what restack moves.
        let preview = workspace(&fixture).check(Some("develop")).unwrap();
        prop_assert!(preview.conflicts.is_empty() && preview.blocked.is_empty());
        let before_tips = branch_tips(&fixture, &specs);
        let before = fixture.snapshot();
        let restacked = workspace(&fixture).restack("develop").unwrap();
        let predicted: Vec<&str> = preview.clean.iter().map(|moved| moved.name.as_str()).collect();
        let moved: Vec<&str> = restacked.moved.iter().map(|moved| moved.name.as_str()).collect();
        prop_assert_eq!(predicted, moved);

        // Every branch sits on its parent's tip, with exactly its own commits, in order.
        for (index, spec) in specs.iter().enumerate() {
            let parent = spec.parent.map_or_else(|| "develop".to_owned(), name);
            let log = fixture.git(&["log", "--format=%s", &format!("{parent}..{}", name(index))]);
            let log: Vec<&str> = log.lines().collect();
            prop_assert_eq!(&log, &subjects[&name(index)], "b{}", index);
            prop_assert_eq!(fixture.git(&["merge-base", &parent, &name(index)]), tip(&fixture, &parent));
        }

        // Restacking again changes nothing.
        let settled = fixture.snapshot();
        let again = workspace(&fixture).restack("develop").unwrap();
        prop_assert!(again.moved.is_empty());
        prop_assert!(settled.diff(&fixture.snapshot()).is_empty());

        // Undo puts every branch back.
        if !restacked.moved.is_empty() {
            workspace(&fixture).undo().unwrap();
        }
        prop_assert_eq!(branch_tips(&fixture, &specs), before_tips);
        let diff = before.diff(&fixture.snapshot());
        prop_assert!(diff.worktree.is_empty() && diff.index.is_empty() && diff.head.is_none(), "{}", diff);
    }
}
