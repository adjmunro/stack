//! Stepping through stacks, switching, and creating branches.

use super::*;

impl Workspace {
    /// The branch one `step` away from `branch` in its stack. Nothing is checked out; see [`Self::switch`].
    ///
    /// # Errors
    /// - [`Error::UnknownBranch`].
    /// - [`Error::NoParent`] stepping down from a trunk or unattached branch; [`Error::IsTrunk`] for
    ///   [`Step::Bottom`] from a trunk.
    /// - [`Error::NoChildren`] or [`Error::SeveralChildren`] stepping up.
    pub fn step(&self, branch: &str, step: Step) -> Result<String, Error> {
        let (_, resolution) = self.resolve()?;
        let entry = resolution
            .branches
            .get(branch)
            .ok_or_else(|| Error::UnknownBranch {
                name: branch.into(),
            })?;
        let parent_of = |name: &str| {
            resolution.branches[name]
                .parent
                .as_ref()
                .map(|parent| parent.name.clone())
        };
        let only_child = |name: &str| -> Result<String, Error> {
            let mut children: Vec<String> = resolution
                .branches
                .iter()
                .filter(|(_, entry)| {
                    entry
                        .parent
                        .as_ref()
                        .is_some_and(|parent| parent.name == name)
                })
                .map(|(child, _)| child.clone())
                .collect();
            match children.len() {
                0 => Err(Error::NoChildren { name: name.into() }),
                1 => Ok(children.remove(0)),
                _ => Err(Error::SeveralChildren {
                    name: name.into(),
                    children,
                }),
            }
        };
        let mut current = branch.to_owned();
        match step {
            Step::Down(count) => {
                for _ in 0..count {
                    current = parent_of(&current).ok_or_else(|| Error::NoParent {
                        name: current.clone(),
                    })?;
                }
            }
            Step::Up(count) => {
                for _ in 0..count {
                    current = only_child(&current)?;
                }
            }
            Step::Top => loop {
                match only_child(&current) {
                    Ok(child) => current = child,
                    Err(Error::NoChildren { .. }) => break,
                    Err(error) => return Err(error),
                }
            },
            Step::Bottom => {
                if entry.role == Role::Trunk {
                    return Err(Error::IsTrunk {
                        name: branch.into(),
                    });
                }
                if entry.role == Role::Branch {
                    // A limb heads its own stack, so the walk ends on it; a trunk doesn't, so it ends above it.
                    while let Some(parent) = parent_of(&current) {
                        let role = resolution.branches[&parent].role;
                        if role != Role::Trunk {
                            current = parent;
                        }
                        if role != Role::Branch {
                            break;
                        }
                    }
                }
            }
        }
        Ok(current)
    }

    /// Checks out `branch` with `git switch` (so git's safety checks and hooks apply).
    ///
    /// # Errors
    /// [`Error::Git`] if git refuses, e.g. because local changes would be overwritten.
    pub fn switch(&self, branch: &str) -> Result<(), Error> {
        self.git.switch(branch, false)
    }

    /// Creates `name` on the current branch and checks it out, then commits if `message` is given (staging changes
    /// to tracked files first if `all`). A shortcut for `git switch --create` and `git commit`: the parent is worked
    /// out automatically, and undo doesn't cover it.
    ///
    /// # Errors
    /// [`Error::Git`] if git refuses (e.g. the branch exists, or there is nothing to commit).
    pub fn create(&self, name: &str, message: Option<&str>, all: bool) -> Result<(), Error> {
        self.git.switch(name, true)?;
        if let Some(message) = message {
            self.git.commit_index(message, all)?;
        }
        Ok(())
    }
}
