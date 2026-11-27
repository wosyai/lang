use super::git_wire::GitOid;
use super::{git_path_text, valid_package_name};
use crate::{DependencyDeclaration, DependencySource, GitSource, ResolvedPackage};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

pub(super) type Declarations = BTreeMap<String, DependencyDeclaration>;
pub(super) struct LoadedPackage {
    pub(super) root: PathBuf,
    pub(super) source: Option<ResolvedPackage>,
    pub(super) dependencies: Declarations,
    pub(super) revision: Option<GitOid>,
}

impl From<(ResolvedPackage, Declarations)> for LoadedPackage {
    fn from((source, dependencies): (ResolvedPackage, Declarations)) -> Self {
        Self { root: source.project_root.clone(), source: Some(source), dependencies, revision: None }
    }
}

#[derive(Clone, Debug)]
pub(super) struct GitRequirement {
    pub(super) source: GitSource,
    pub(super) revision: Option<GitOid>,
}

impl GitRequirement {
    pub(super) fn require_revision(&self, name: &str, actual: &GitOid) -> Result<(), String> {
        if let Some(expected) = &self.revision {
            if expected != actual {
                return Err(format!(
                    "repository-relative dependency {name} requires Git revision {}, not {}",
                    expected.as_str(), actual.as_str()
                ));
            }
        }
        Ok(())
    }
}

struct GitParent<'a> {
    source: &'a GitSource,
    revision: &'a GitOid,
}

/// One source binding per package name. A same-repository requirement can
/// additionally constrain the exact revision without creating another binding.
#[derive(Clone, Debug)]
pub(super) enum Binding {
    Local(PathBuf),
    Git(GitRequirement),
}

impl Binding {
    fn at(declaration: &DependencyDeclaration, parent: &Path, git: Option<GitParent<'_>>) -> Result<Self, String> {
        match &declaration.source {
            DependencySource::Local { path } => {
                if let Some(git) = git {
                    if !valid_package_name(&declaration.name) {
                        return Err(format!(
                            "Git dependency {} must use a snake_case package name",
                            declaration.name
                        ));
                    }
                    let package_path = sibling_path(&git.source.package_path, path).map_err(|_| {
                        format!(
                            "repository-relative dependency {} must remain inside its declaring Git repository",
                            declaration.name
                        )
                    })?;
                    return Ok(Self::Git(GitRequirement {
                        source: GitSource { origin: git.source.origin.clone(), package_path },
                        revision: Some(git.revision.clone()),
                    }));
                }
                let path = parent.join(path);
                let actual = path.canonicalize().map_err(|error| {
                    format!("cannot resolve local dependency {} at {}: {error}", declaration.name, path.display())
                })?;
                Ok(Self::Local(actual))
            }
            DependencySource::Git(source) => Ok(Self::Git(GitRequirement { source: source.clone(), revision: None })),
        }
    }

    fn same_source(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Local(first), Self::Local(second)) => first == second,
            (Self::Git(first), Self::Git(second)) => first.source == second.source,
            _ => false,
        }
    }
}

fn sibling_path(parent: &str, declaration: &Path) -> Result<String, String> {
    let mut path = PathBuf::new();
    for component in Path::new(parent).components().chain(declaration.components()) {
        match component {
            Component::Normal(component) => path.push(component),
            Component::CurDir => {}
            Component::ParentDir if path.pop() => {}
            _ => return Err("Git dependency path escapes its repository".to_owned()),
        }
    }
    if path.as_os_str().is_empty() {
        Ok(".".to_owned())
    } else {
        git_path_text(&path)
    }
}

struct ResolvedBinding {
    source: Binding,
    revision: Option<GitOid>,
    route: Vec<String>,
}

#[derive(Default)]
pub(super) struct Resolution {
    pub(super) packages: BTreeMap<String, ResolvedPackage>,
    pub(super) git_packages: BTreeSet<String>,
    bindings: BTreeMap<String, ResolvedBinding>,
}

impl Resolution {
    /// Extend a validated graph with one new top-level edge. Existing
    /// bindings are reused; only newly reached manifests are provided.
    pub(super) fn add(
        &mut self,
        root: &Path,
        package: &str,
        declaration: &DependencyDeclaration,
        provide: impl FnMut(&Binding, &Path, &DependencyDeclaration) -> Result<LoadedPackage, String>,
    ) -> Result<(), String> {
        let mut walk = Walker { active: vec![package.to_owned()], resolution: self, provide };
        walk.visit(root, declaration, None)
    }
}

pub(super) fn resolve(
    root: &Path,
    package: &str,
    dependencies: &Declarations,
    provide: impl FnMut(&Binding, &Path, &DependencyDeclaration) -> Result<LoadedPackage, String>,
) -> Result<Resolution, String> {
    let mut resolution = Resolution::default();
    let mut walk = Walker { active: vec![package.to_owned()], resolution: &mut resolution, provide };
    for declaration in dependencies.values() {
        walk.visit(root, declaration, None)?;
    }
    Ok(resolution)
}

struct Walker<'a, F> {
    active: Vec<String>,
    resolution: &'a mut Resolution,
    provide: F,
}

impl<F> Walker<'_, F>
where
    F: FnMut(&Binding, &Path, &DependencyDeclaration) -> Result<LoadedPackage, String>,
{
    fn visit(
        &mut self,
        parent: &Path,
        declaration: &DependencyDeclaration,
        parent_git: Option<GitParent<'_>>,
    ) -> Result<(), String> {
        let name = &declaration.name;
        let chain = || self.active.iter().chain(std::iter::once(name)).cloned().collect::<Vec<_>>();
        // The root is an active package but never a dependency binding.
        if self.active.first() == Some(name) {
            return Err(format!("package dependency cycle: {}", chain().join(" -> ")));
        }
        let binding = Binding::at(declaration, parent, parent_git)?;
        if let Some(existing) = self.resolution.bindings.get(name) {
            if !existing.source.same_source(&binding) {
                return Err(format!(
                    "conflicting package bindings for {name}: {} and {}",
                    existing.route.join(" -> "), chain().join(" -> ")
                ));
            }
            if let Binding::Git(requirement) = &binding {
                requirement.require_revision(name, existing.revision.as_ref().expect("resolved Git binding has a pin"))?;
            }
            if self.active.contains(name) {
                return Err(format!("package dependency cycle: {}", chain().join(" -> ")));
            }
            return Ok(());
        }

        let route = chain();
        let LoadedPackage { root, source, dependencies, revision } = (self.provide)(&binding, parent, declaration)
            .map_err(|error| format!("{}: {error}", route.join(" -> ")))?;
        let git = if let Binding::Git(requirement) = &binding {
            self.resolution.git_packages.insert(name.clone());
            Some((&requirement.source, revision.as_ref().expect("Git provider supplies its resolved pin")))
        } else {
            None
        };
        if let Some(mut package) = source {
            if let Binding::Local(path) = &binding {
                package.local_root_identity = Some(path.clone());
            }
            package.dependencies = dependencies.keys().cloned().collect();
            self.resolution.packages.insert(name.clone(), package);
        }
        self.resolution.bindings.insert(name.clone(), ResolvedBinding {
            source: binding.clone(), revision: revision.clone(), route,
        });
        self.active.push(name.clone());
        for child in dependencies.values() {
            self.visit(&root, child, git.map(|(source, revision)| GitParent { source, revision }))?;
        }
        self.active.pop();
        Ok(())
    }
}
