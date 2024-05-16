use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use glob::{glob, GlobError, PatternError};
use tracing::{debug, trace};

use uv_fs::{absolutize_path, Simplified};
use uv_normalize::PackageName;
use uv_warnings::warn_user;

use crate::pyproject::{PyProjectToml, Source, ToolUvWorkspace};
use crate::RequirementsSource;

#[derive(thiserror::Error, Debug)]
pub enum DiscoverError {
    #[error("No `pyproject.toml` found in current directory or any parent directory")]
    MissingPyprojectToml,

    #[error("Failed to find directories for glob: `{0}`")]
    Pattern(String, #[source] PatternError),

    #[error("Invalid glob: `{0}`")]
    Glob(String, #[source] GlobError),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("Failed to parse: `{}`", _0.user_display())]
    Toml(PathBuf, #[source] toml::de::Error),

    #[error("No `project` section found in: `{}`", _0.simplified_display())]
    MissingProject(PathBuf),

    #[error("Failed to normalize workspace member path")]
    Normalize(#[source] std::io::Error),
}

/// A package in a workspace.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
#[allow(dead_code)] // TODO(konsti): Resolve workspace package declarations.
pub struct WorkspaceMember {
    /// The path to the project root.
    root: PathBuf,
    pyproject_toml: PyProjectToml,
}

impl WorkspaceMember {
    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    pub fn pyproject_toml(&self) -> &PyProjectToml {
        &self.pyproject_toml
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct Workspace {
    /// The path to the workspace root, the directory containing the top level `pyproject.toml` with
    /// the `uv.tool.workspace`, or the `pyproject.toml` in an implicit single workspace project.
    root: PathBuf,
    /// The members of the workspace.
    packages: BTreeMap<PackageName, WorkspaceMember>,
    /// The source table for the workspace declaration.
    sources: BTreeMap<PackageName, Source>,
}

impl Workspace {
    /// There is no workspace, use this dummy when resolving.
    pub fn empty(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            packages: BTreeMap::default(),
            sources: BTreeMap::default(),
        }
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    pub fn packages(&self) -> &BTreeMap<PackageName, WorkspaceMember> {
        &self.packages
    }

    pub fn sources(&self) -> &BTreeMap<PackageName, Source> {
        &self.sources
    }
}

/// A package and the workspace it is part of.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct ProjectWorkspace {
    /// The path to the project root.
    project_root: PathBuf,
    /// The name of the package.
    project_name: PackageName,
    workspace: Workspace,
}

impl ProjectWorkspace {
    /// Find the current project and workspace.
    pub fn discover(path: impl AsRef<Path>) -> Result<Self, DiscoverError> {
        let Some(project_root) = path
            .as_ref()
            .ancestors()
            .find(|path| path.join("pyproject.toml").exists())
        else {
            return Err(DiscoverError::MissingPyprojectToml);
        };

        debug!("Project root: `{}`", project_root.simplified_display());

        Self::from_project_root(project_root)
    }

    pub fn from_project_root(project_root: &Path) -> Result<Self, DiscoverError> {
        // Read the `pyproject.toml`.
        let pyproject_path = project_root.join("pyproject.toml");
        let contents = fs_err::read_to_string(&pyproject_path)?;
        let pyproject_toml: PyProjectToml = toml::from_str(&contents)
            .map_err(|err| DiscoverError::Toml(pyproject_path.clone(), err))?;

        // Extract the `[project]` metadata.
        let Some(project) = pyproject_toml.project.clone() else {
            return Err(DiscoverError::MissingProject(pyproject_path.clone()));
        };

        Self::from_project(project_root, &pyproject_toml, project.name)
    }

    /// The directory containing the closest `pyproject.toml`, containing a `[project]` section.
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub fn project_name(&self) -> &PackageName {
        &self.project_name
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// Return the requirements for the project.
    pub fn requirements(&self) -> Vec<RequirementsSource> {
        vec![
            RequirementsSource::from_requirements_file(self.project_root.join("pyproject.toml")),
            RequirementsSource::Editable(self.project_root.to_string_lossy().to_string()),
        ]
    }

    fn from_project(
        project_path: &Path,
        project: &PyProjectToml,
        project_name: PackageName,
    ) -> Result<Self, DiscoverError> {
        let project_path = absolutize_path(project_path)
            .map_err(DiscoverError::Normalize)?
            .to_path_buf();

        let mut workspace = project
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.workspace.as_ref())
            .map(|workspace| (project_path.clone(), workspace.clone(), project.clone()));

        if workspace.is_none() {
            workspace = find_workspace(&project_path)?;
        }

        let mut workspace_members = BTreeMap::new();
        workspace_members.insert(
            project_name.clone(),
            WorkspaceMember {
                root: project_path.clone(),
                pyproject_toml: project.clone(),
            },
        );

        let Some((workspace_root, workspace_definition, project_in_workspace_root)) = workspace
        else {
            // The project and the workspace root are identical
            debug!("No explicit workspace root, using project root");
            return Ok(Self {
                project_root: project_path.clone(),
                project_name,
                workspace: Workspace {
                    root: project_path,
                    packages: workspace_members,
                    // There may be package sources, but we don't need to duplicate them into the
                    // workspace sources.
                    sources: BTreeMap::default(),
                },
            });
        };

        debug!("Workspace root: `{}`", workspace_root.simplified_display());
        if workspace_root != project_path {
            let contents = fs_err::read_to_string(workspace_root.join("pyproject.toml"))?;
            let pyproject_toml = toml::from_str(&contents)
                .map_err(|err| DiscoverError::Toml(workspace_root.join("pyproject.toml"), err))?;

            if let Some(project) = &project_in_workspace_root.project {
                workspace_members.insert(
                    project.name.clone(),
                    WorkspaceMember {
                        root: workspace_root.clone(),
                        pyproject_toml,
                    },
                );
            };
        }
        for member_glob in workspace_definition.members.unwrap_or_default() {
            let absolute_glob = workspace_root
                .join(member_glob.as_str())
                .to_string_lossy()
                .to_string();
            for member_root in glob(&absolute_glob)
                .map_err(|err| DiscoverError::Pattern(absolute_glob.to_string(), err))?
            {
                // TODO(konsti): Filter already seen.
                // TODO(konsti): Error context? There's no fs_err here.
                let member_root = member_root
                    .map_err(|err| DiscoverError::Glob(absolute_glob.to_string(), err))?;
                let member_root = absolutize_path(&member_root)
                    .map_err(DiscoverError::Normalize)?
                    .to_path_buf();

                trace!("Processing workspace member {}", member_root.user_display());

                // Read the `pyproject.toml`.
                let contents = fs_err::read_to_string(&member_root.join("pyproject.toml"))?;
                let pyproject_toml: PyProjectToml = toml::from_str(&contents)
                    .map_err(|err| DiscoverError::Toml(member_root.join("pyproject.toml"), err))?;

                // Extract the package name.
                let Some(project) = pyproject_toml.project.clone() else {
                    return Err(DiscoverError::MissingProject(member_root));
                };

                let contents = fs_err::read_to_string(member_root.join("pyproject.toml"))?;
                let pyproject_toml = toml::from_str(&contents)
                    .map_err(|err| DiscoverError::Toml(member_root.join("pyproject.toml"), err))?;
                let member = WorkspaceMember {
                    root: member_root.clone(),
                    pyproject_toml,
                };
                workspace_members.insert(project.name, member);
            }
        }
        let workspace_sources = project_in_workspace_root
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.sources.clone())
            .unwrap_or_default();

        check_nested_workspaces(&workspace_root);

        Ok(Self {
            project_root: project_path.clone(),
            project_name,
            workspace: Workspace {
                root: workspace_root,
                packages: workspace_members,
                sources: workspace_sources,
            },
        })
    }

    #[cfg(test)]
    pub(crate) fn dummy(root: &Path, project_name: &PackageName) -> Self {
        // This doesn't necessarily match the exact test case, but we don't use the other fields
        // for the test cases atm.
        let root_member = WorkspaceMember {
            root: root.to_path_buf(),
            pyproject_toml: PyProjectToml {
                project: Some(crate::pyproject::Project {
                    name: project_name.clone(),
                    dependencies: None,
                    optional_dependencies: None,
                    dynamic: None,
                }),
                tool: None,
            },
        };
        Self {
            project_root: root.to_path_buf(),
            project_name: project_name.clone(),
            workspace: Workspace {
                root: root.to_path_buf(),
                packages: [(project_name.clone(), root_member)].into_iter().collect(),
                sources: BTreeMap::default(),
            },
        }
    }
}

/// Find the workspace root above the current project, if any.
fn find_workspace(
    project_root: &Path,
) -> Result<Option<(PathBuf, ToolUvWorkspace, PyProjectToml)>, DiscoverError> {
    // Skip 1 to skip the project itself.
    for workspace_root in project_root.ancestors().skip(1) {
        let pyproject_path = workspace_root.join("pyproject.toml");
        if !pyproject_path.exists() {
            continue;
        }
        trace!(
            "Found pyproject.toml: {}",
            pyproject_path.simplified_display()
        );

        // Read the `pyproject.toml`.
        let contents = fs_err::read_to_string(&pyproject_path)?;
        let pyproject_toml: PyProjectToml = toml::from_str(&contents)
            .map_err(|err| DiscoverError::Toml(pyproject_path.clone(), err))?;

        return if let Some(workspace) = pyproject_toml
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.workspace.as_ref())
        {
            if is_excluded_from_workspace(workspace, workspace_root, project_root)? {
                debug!(
                    "Found workspace root `{}`, but project is excluded.",
                    workspace_root.simplified_display()
                );
                return Ok(None);
            }
            let workspace_root = absolutize_path(workspace_root)
                .map_err(DiscoverError::Normalize)?
                .to_path_buf();

            debug!(
                "Found workspace root: `{}`",
                workspace_root.simplified_display()
            );

            // We found a workspace root.
            Ok(Some((workspace_root, workspace.clone(), pyproject_toml)))
        } else if pyproject_toml.project.is_some() {
            // We're in a directory of another project, e.g. tests or examples.
            // Example:
            // ```
            // albatross
            // ├── examples
            // │   └── bird-feeder [CURRENT DIRECTORY]
            // │       ├── pyproject.toml
            // │       └── src
            // │           └── bird_feeder
            // │               └── __init__.py
            // ├── pyproject.toml
            // └── src
            //     └── albatross
            //         └── __init__.py
            // ```
            // The current project is the example (non-workspace) `bird-feeder` in `albatross`,
            // we ignore all `albatross` is doing and any potential workspace it might be
            // contained in.
            debug!(
                "Project is contained in non-workspace project: `{}`",
                workspace_root.simplified_display()
            );
            Ok(None)
        } else {
            // We require that a `project.toml` file either declares a workspace or a project.
            Err(DiscoverError::MissingProject(pyproject_path))
        };
    }

    Ok(None)
}

/// Warn when the valid workspace is included in another workspace.
fn check_nested_workspaces(inner_workspace_root: &Path) {
    for outer_workspace_root in inner_workspace_root
        .parent()
        .map(|path| path.ancestors())
        .into_iter()
        .flatten()
    {
        let pyproject_toml_path = outer_workspace_root.join("pyproject.toml");
        if !pyproject_toml_path.exists() {
            continue;
        }
        let contents = match fs_err::read_to_string(&pyproject_toml_path) {
            Ok(contents) => contents,
            Err(err) => {
                warn_user!(
                    "Unreadable pyproject.toml `{}`: {}",
                    pyproject_toml_path.user_display(),
                    err
                );
                return;
            }
        };
        let pyproject_toml: PyProjectToml = match toml::from_str(&contents) {
            Ok(contents) => contents,
            Err(err) => {
                warn_user!(
                    "Invalid pyproject.toml `{}`: {}",
                    pyproject_toml_path.user_display(),
                    err
                );
                return;
            }
        };

        if let Some(workspace) = pyproject_toml
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.workspace.as_ref())
        {
            let is_excluded = match is_excluded_from_workspace(
                workspace,
                outer_workspace_root,
                inner_workspace_root,
            ) {
                Ok(contents) => contents,
                Err(err) => {
                    warn_user!(
                        "Invalid pyproject.toml `{}`: {}",
                        pyproject_toml_path.user_display(),
                        err
                    );
                    return;
                }
            };
            if !is_excluded {
                warn_user!(
                    "Outer workspace including existing workspace, nested workspaces are not supported: `{}`",
                    pyproject_toml_path.user_display(),
                );
            }
        }

        // We're in the examples or tests of another project (not a workspace), this is fine.
        return;
    }
}

fn is_excluded_from_workspace(
    workspace: &ToolUvWorkspace,
    workspace_root: &Path,
    project_path: &Path,
) -> Result<bool, DiscoverError> {
    // Check if we're in the excludes of a workspace.
    for exclude_glob in workspace.exclude.iter().flatten() {
        let absolute_glob = workspace_root
            .join(exclude_glob.as_str())
            .to_string_lossy()
            .to_string();
        for excluded_root in glob(&absolute_glob)
            .map_err(|err| DiscoverError::Pattern(absolute_glob.to_string(), err))?
        {
            let excluded_root =
                excluded_root.map_err(|err| DiscoverError::Glob(absolute_glob.to_string(), err))?;
            if excluded_root == project_path {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::env;
    use std::path::Path;

    use insta::assert_json_snapshot;

    use crate::discovery::ProjectWorkspace;

    fn workspace_test(folder: impl AsRef<Path>) -> (ProjectWorkspace, String) {
        let root_dir = env::current_dir()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("scripts")
            .join("workspaces");
        let project = ProjectWorkspace::discover(root_dir.join(folder)).unwrap();
        let root_escaped = regex::escape(root_dir.to_string_lossy().as_ref());
        (project, root_escaped)
    }

    #[test]
    fn albatross_in_example() {
        let (project, root_escaped) = workspace_test("albatross-root-workspace");
        let filters = vec![(root_escaped.as_str(), "[ROOT]")];
        insta::with_settings!({filters => filters}, {
        assert_json_snapshot!(
            project,
            {
                ".workspace.packages.*.pyproject_toml" => "[PYPROJECT_TOML]"
            },
            @r###"
        {
          "project_root": "[ROOT]/albatross-root-workspace",
          "project_name": "albatross",
          "workspace": {
            "root": "[ROOT]/albatross-root-workspace",
            "packages": {
              "albatross": {
                "root": "[ROOT]/albatross-root-workspace",
                "pyproject_toml": "[PYPROJECT_TOML]"
              },
              "bird-feeder": {
                "root": "[ROOT]/albatross-root-workspace/packages/bird-feeder",
                "pyproject_toml": "[PYPROJECT_TOML]"
              },
              "seeds": {
                "root": "[ROOT]/albatross-root-workspace/packages/seeds",
                "pyproject_toml": "[PYPROJECT_TOML]"
              }
            },
            "sources": {
              "bird-feeder": {
                "workspace": true,
                "editable": null
              }
            }
          }
        }
        "###);
        });
    }

    #[test]
    fn albatross_project_in_excluded() {
        let (project, root_escaped) = workspace_test("albatross-project-in-excluded");
        let filters = vec![(root_escaped.as_str(), "[ROOT]")];
        insta::with_settings!({filters => filters}, {
            assert_json_snapshot!(
            project,
            {
                ".workspace.packages.*.pyproject_toml" => "[PYPROJECT_TOML]"
            },
            @r###"
            {
              "project_root": "[ROOT]/albatross-project-in-excluded",
              "project_name": "albatross",
              "workspace": {
                "root": "[ROOT]/albatross-project-in-excluded",
                "packages": {
                  "albatross": {
                    "root": "[ROOT]/albatross-project-in-excluded",
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  }
                },
                "sources": {}
              }
            }
            "###);
        });
    }

    #[test]
    fn albatross_root_workspace() {
        let (project, root_escaped) = workspace_test("albatross-root-workspace");
        let filters = vec![(root_escaped.as_str(), "[ROOT]")];
        insta::with_settings!({filters => filters}, {
            assert_json_snapshot!(
            project,
            {
                ".workspace.packages.*.pyproject_toml" => "[PYPROJECT_TOML]"
            },
            @r###"
            {
              "project_root": "[ROOT]/albatross-root-workspace",
              "project_name": "albatross",
              "workspace": {
                "root": "[ROOT]/albatross-root-workspace",
                "packages": {
                  "albatross": {
                    "root": "[ROOT]/albatross-root-workspace",
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  },
                  "bird-feeder": {
                    "root": "[ROOT]/albatross-root-workspace/packages/bird-feeder",
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  },
                  "seeds": {
                    "root": "[ROOT]/albatross-root-workspace/packages/seeds",
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  }
                },
                "sources": {
                  "bird-feeder": {
                    "workspace": true,
                    "editable": null
                  }
                }
              }
            }
            "###);
        });
    }

    #[test]
    fn albatross_virtual_workspace() {
        let (project, root_escaped) = workspace_test(
            Path::new("albatross-virtual-workspace")
                .join("packages")
                .join("albatross"),
        );
        let filters = vec![(root_escaped.as_str(), "[ROOT]")];
        insta::with_settings!({filters => filters}, {
            assert_json_snapshot!(
            project,
            {
                ".workspace.packages.*.pyproject_toml" => "[PYPROJECT_TOML]"
            },
            @r###"
            {
              "project_root": "[ROOT]/albatross-virtual-workspace/packages/albatross",
              "project_name": "albatross",
              "workspace": {
                "root": "[ROOT]/albatross-virtual-workspace",
                "packages": {
                  "albatross": {
                    "root": "[ROOT]/albatross-virtual-workspace/packages/albatross",
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  },
                  "bird-feeder": {
                    "root": "[ROOT]/albatross-virtual-workspace/packages/bird-feeder",
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  },
                  "seeds": {
                    "root": "[ROOT]/albatross-virtual-workspace/packages/seeds",
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  }
                },
                "sources": {}
              }
            }
            "###);
        });
    }

    #[test]
    fn albatross_just_project() {
        let (project, root_escaped) = workspace_test("albatross-just-project");
        let filters = vec![(root_escaped.as_str(), "[ROOT]")];
        insta::with_settings!({filters => filters}, {
            assert_json_snapshot!(
            project,
            {
                ".workspace.packages.*.pyproject_toml" => "[PYPROJECT_TOML]"
            },
            @r###"
            {
              "project_root": "[ROOT]/albatross-just-project",
              "project_name": "albatross",
              "workspace": {
                "root": "[ROOT]/albatross-just-project",
                "packages": {
                  "albatross": {
                    "root": "[ROOT]/albatross-just-project",
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  }
                },
                "sources": {}
              }
            }
            "###);
        });
    }
}
