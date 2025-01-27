//! ## Type hierarchy
//!
//! When we receive the requirements from `pip sync`, we check which requirements already fulfilled
//! in the users environment ([`InstalledDist`]), whether the matching package is in our wheel cache
//! ([`CachedDist`]) or whether we need to download, (potentially build) and install it ([`Dist`]).
//! These three variants make up [`BuiltDist`].
//!
//! ## `Dist`
//! A [`Dist`] is either a built distribution (a wheel), or a source distribution that exists at
//! some location. We translate every PEP 508 requirement e.g. from `requirements.txt` or from
//! `pyproject.toml`'s `[project] dependencies` into a [`Dist`] by checking each index.
//! * [`BuiltDist`]: A wheel, with its three possible origins:
//!   * [`RegistryBuiltDist`]
//!   * [`DirectUrlBuiltDist`]
//!   * [`PathBuiltDist`]
//! * [`SourceDist`]: A source distribution, with its four possible origins:
//!   * [`RegistrySourceDist`]
//!   * [`DirectUrlSourceDist`]
//!   * [`GitSourceDist`]
//!   * [`PathSourceDist`]
//!
//! ## `CachedDist`
//! A [`CachedDist`] is a built distribution (wheel) that exists in the local cache, with the two
//! possible origins we currently track:
//! * [`CachedRegistryDist`]
//! * [`CachedDirectUrlDist`]
//!
//! ## `InstalledDist`
//! An [`InstalledDist`] is built distribution (wheel) that is installed in a virtual environment,
//! with the two possible origins we currently track:
//! * [`InstalledRegistryDist`]
//! * [`InstalledDirectUrlDist`]
//!
//! Since we read this information from [`direct_url.json`](https://packaging.python.org/en/latest/specifications/direct-url-data-structure/), it doesn't match the information [`Dist`] exactly.
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::Result;
use url::Url;

use distribution_filename::{SourceDistFilename, WheelFilename};
use pep440_rs::Version;
use pep508_rs::{Pep508Url, VerbatimUrl};
use uv_git::GitUrl;
use uv_normalize::PackageName;

pub use crate::annotation::*;
pub use crate::any::*;
pub use crate::buildable::*;
pub use crate::cached::*;
pub use crate::editable::*;
pub use crate::error::*;
pub use crate::file::*;
pub use crate::hash::*;
pub use crate::id::*;
pub use crate::index_url::*;
pub use crate::installed::*;
pub use crate::parsed_url::*;
pub use crate::prioritized_distribution::*;
pub use crate::requirement::*;
pub use crate::resolution::*;
pub use crate::resolved::*;
pub use crate::specified_requirement::*;
pub use crate::traits::*;

mod annotation;
mod any;
mod buildable;
mod cached;
mod editable;
mod error;
mod file;
mod hash;
mod id;
mod index_url;
mod installed;
mod parsed_url;
mod prioritized_distribution;
mod requirement;
mod resolution;
mod resolved;
mod specified_requirement;
mod traits;

#[derive(Debug, Clone)]
pub enum VersionOrUrlRef<'a, T: Pep508Url = VerbatimUrl> {
    /// A PEP 440 version specifier, used to identify a distribution in a registry.
    Version(&'a Version),
    /// A URL, used to identify a distribution at an arbitrary location.
    Url(&'a T),
}

impl Verbatim for VersionOrUrlRef<'_> {
    fn verbatim(&self) -> Cow<'_, str> {
        match self {
            VersionOrUrlRef::Version(version) => Cow::Owned(format!("=={version}")),
            VersionOrUrlRef::Url(url) => Cow::Owned(format!(" @ {}", url.verbatim())),
        }
    }
}

impl std::fmt::Display for VersionOrUrlRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VersionOrUrlRef::Version(version) => write!(f, "=={version}"),
            VersionOrUrlRef::Url(url) => write!(f, " @ {url}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum InstalledVersion<'a> {
    /// A PEP 440 version specifier, used to identify a distribution in a registry.
    Version(&'a Version),
    /// A URL, used to identify a distribution at an arbitrary location, along with the version
    /// specifier to which it resolved.
    Url(&'a Url, &'a Version),
}

impl std::fmt::Display for InstalledVersion<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstalledVersion::Version(version) => write!(f, "=={version}"),
            InstalledVersion::Url(url, version) => write!(f, "=={version} (from {url})"),
        }
    }
}

/// Either a built distribution, a wheel, or a source distribution that exists at some location.
///
/// The location can be an index, URL or path (wheel), or index, URL, path or Git repository (source distribution).
#[derive(Debug, Clone)]
pub enum Dist {
    Built(BuiltDist),
    Source(SourceDist),
}

/// A wheel, with its three possible origins (index, url, path)
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum BuiltDist {
    Registry(RegistryBuiltDist),
    DirectUrl(DirectUrlBuiltDist),
    Path(PathBuiltDist),
}

/// A source distribution, with its possible origins (index, url, path, git)
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum SourceDist {
    Registry(RegistrySourceDist),
    DirectUrl(DirectUrlSourceDist),
    Git(GitSourceDist),
    Path(PathSourceDist),
    Directory(DirectorySourceDist),
}

/// A built distribution (wheel) that exists in a registry, like `PyPI`.
#[derive(Debug, Clone)]
pub struct RegistryBuiltWheel {
    pub filename: WheelFilename,
    pub file: Box<File>,
    pub index: IndexUrl,
}

/// A built distribution (wheel) that exists in a registry, like `PyPI`.
#[derive(Debug, Clone)]
pub struct RegistryBuiltDist {
    /// All wheels associated with this distribution. It is guaranteed
    /// that there is at least one wheel.
    pub wheels: Vec<RegistryBuiltWheel>,
    /// The "best" wheel selected based on the current wheel tag
    /// environment.
    ///
    /// This is guaranteed to point into a valid entry in `wheels`.
    pub best_wheel_index: usize,
    /// A source distribution if one exists for this distribution.
    ///
    /// It is possible for this to be `None`. For example, when a distribution
    /// has no source distribution, or if it does have one but isn't compatible
    /// with the user configuration. (e.g., If `Requires-Python` isn't
    /// compatible with the installed/target Python versions, or if something
    /// like `--exclude-newer` was used.)
    pub sdist: Option<RegistrySourceDist>,
    // Ideally, this type would have an index URL on it, and the
    // `RegistryBuiltDist` and `RegistrySourceDist` types would *not* have an
    // index URL on them. Alas, the --find-links feature makes it technically
    // possible for the indexes to diverge across wheels/sdists in the same
    // distribution.
    //
    // Note though that at time of writing, when generating a universal lock
    // file, we require that all index URLs across wheels/sdists for a single
    // distribution are equivalent.
}

/// A built distribution (wheel) that exists at an arbitrary URL.
#[derive(Debug, Clone)]
pub struct DirectUrlBuiltDist {
    /// We require that wheel urls end in the full wheel filename, e.g.
    /// `https://example.org/packages/flask-3.0.0-py3-none-any.whl`
    pub filename: WheelFilename,
    /// The URL without the subdirectory fragment.
    pub location: Url,
    pub subdirectory: Option<PathBuf>,
    /// The URL with the subdirectory fragment.
    pub url: VerbatimUrl,
}

/// A built distribution (wheel) that exists in a local directory.
#[derive(Debug, Clone)]
pub struct PathBuiltDist {
    pub filename: WheelFilename,
    pub url: VerbatimUrl,
    pub path: PathBuf,
}

/// A source distribution that exists in a registry, like `PyPI`.
#[derive(Debug, Clone)]
pub struct RegistrySourceDist {
    pub filename: SourceDistFilename,
    pub file: Box<File>,
    pub index: IndexUrl,
    /// When an sdist is selected, it may be the case that there were
    /// available wheels too. There are many reasons why a wheel might not
    /// have been chosen (maybe none available are compatible with the
    /// current environment), but we still want to track that they exist. In
    /// particular, for generating a universal lock file, we do not want to
    /// skip emitting wheels to the lock file just because the host generating
    /// the lock file didn't have any compatible wheels available.
    pub wheels: Vec<RegistryBuiltWheel>,
}

/// A source distribution that exists at an arbitrary URL.
#[derive(Debug, Clone)]
pub struct DirectUrlSourceDist {
    /// Unlike [`DirectUrlBuiltDist`], we can't require a full filename with a version here, people
    /// like using e.g. `foo @ https://github.com/org/repo/archive/master.zip`
    pub name: PackageName,
    /// The URL without the subdirectory fragment.
    pub location: Url,
    pub subdirectory: Option<PathBuf>,
    /// The URL with the subdirectory fragment.
    pub url: VerbatimUrl,
}

/// A source distribution that exists in a Git repository.
#[derive(Debug, Clone)]
pub struct GitSourceDist {
    pub name: PackageName,
    /// The URL without the revision and subdirectory fragment.
    pub git: Box<GitUrl>,
    pub subdirectory: Option<PathBuf>,
    /// The URL with the revision and subdirectory fragment.
    pub url: VerbatimUrl,
}

/// A source distribution that exists in a local archive (e.g., a `.tar.gz` file).
#[derive(Debug, Clone)]
pub struct PathSourceDist {
    pub name: PackageName,
    pub url: VerbatimUrl,
    pub path: PathBuf,
}

/// A source distribution that exists in a local directory.
#[derive(Debug, Clone)]
pub struct DirectorySourceDist {
    pub name: PackageName,
    pub url: VerbatimUrl,
    pub path: PathBuf,
    pub editable: bool,
}

impl Dist {
    /// A remote built distribution (`.whl`) or source distribution from a `http://` or `https://`
    /// URL.
    pub fn from_http_url(
        name: PackageName,
        location: Url,
        subdirectory: Option<PathBuf>,
        url: VerbatimUrl,
    ) -> Result<Dist, Error> {
        if Path::new(url.path())
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("whl"))
        {
            // Validate that the name in the wheel matches that of the requirement.
            let filename = WheelFilename::from_str(&url.filename()?)?;
            if filename.name != name {
                return Err(Error::PackageNameMismatch(
                    name,
                    filename.name,
                    url.verbatim().to_string(),
                ));
            }

            Ok(Self::Built(BuiltDist::DirectUrl(DirectUrlBuiltDist {
                filename,
                location,
                subdirectory,
                url,
            })))
        } else {
            Ok(Self::Source(SourceDist::DirectUrl(DirectUrlSourceDist {
                name,
                location,
                subdirectory,
                url,
            })))
        }
    }

    /// A local built or source distribution from a `file://` URL.
    pub fn from_file_url(
        name: PackageName,
        path: &Path,
        editable: bool,
        url: VerbatimUrl,
    ) -> Result<Dist, Error> {
        // Store the canonicalized path, which also serves to validate that it exists.
        let path = match path.canonicalize() {
            Ok(path) => path,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::NotFound(url.to_url()));
            }
            Err(err) => return Err(err.into()),
        };

        // Determine whether the path represents an archive or a directory.
        if path.is_dir() {
            Ok(Self::Source(SourceDist::Directory(DirectorySourceDist {
                name,
                url,
                path,
                editable,
            })))
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("whl"))
        {
            // Validate that the name in the wheel matches that of the requirement.
            let filename = WheelFilename::from_str(&url.filename()?)?;
            if filename.name != name {
                return Err(Error::PackageNameMismatch(
                    name,
                    filename.name,
                    url.verbatim().to_string(),
                ));
            }

            if editable {
                return Err(Error::EditableFile(url));
            }

            Ok(Self::Built(BuiltDist::Path(PathBuiltDist {
                filename,
                url,
                path,
            })))
        } else {
            if editable {
                return Err(Error::EditableFile(url));
            }

            Ok(Self::Source(SourceDist::Path(PathSourceDist {
                name,
                url,
                path,
            })))
        }
    }

    /// A remote source distribution from a `git+https://` or `git+ssh://` url.
    pub fn from_git_url(
        name: PackageName,
        url: VerbatimUrl,
        git: GitUrl,
        subdirectory: Option<PathBuf>,
    ) -> Result<Dist, Error> {
        Ok(Self::Source(SourceDist::Git(GitSourceDist {
            name,
            git: Box::new(git),
            subdirectory,
            url,
        })))
    }

    /// Create a [`Dist`] for a URL-based distribution.
    pub fn from_url(name: PackageName, url: VerbatimParsedUrl) -> Result<Self, Error> {
        match url.parsed_url {
            ParsedUrl::Archive(archive) => {
                Self::from_http_url(name, archive.url, archive.subdirectory, url.verbatim)
            }
            ParsedUrl::Path(file) => Self::from_file_url(name, &file.path, false, url.verbatim),
            ParsedUrl::Git(git) => {
                Self::from_git_url(name, url.verbatim, git.url, git.subdirectory)
            }
        }
    }

    /// Create a [`Dist`] for a local editable distribution.
    pub fn from_editable(name: PackageName, editable: LocalEditable) -> Result<Self, Error> {
        let LocalEditable { url, path, .. } = editable;
        Ok(Self::Source(SourceDist::Directory(DirectorySourceDist {
            name,
            url,
            path,
            editable: true,
        })))
    }

    /// Return true if the distribution is editable.
    pub fn is_editable(&self) -> bool {
        match self {
            Self::Source(dist) => dist.is_editable(),
            Self::Built(_) => false,
        }
    }

    /// Returns the [`IndexUrl`], if the distribution is from a registry.
    pub fn index(&self) -> Option<&IndexUrl> {
        match self {
            Self::Built(dist) => dist.index(),
            Self::Source(dist) => dist.index(),
        }
    }

    /// Returns the [`File`] instance, if this dist is from a registry with simple json api support
    pub fn file(&self) -> Option<&File> {
        match self {
            Self::Built(built) => built.file(),
            Self::Source(source) => source.file(),
        }
    }

    pub fn version(&self) -> Option<&Version> {
        match self {
            Self::Built(wheel) => Some(wheel.version()),
            Self::Source(source_dist) => source_dist.version(),
        }
    }
}

impl BuiltDist {
    /// Returns the [`IndexUrl`], if the distribution is from a registry.
    pub fn index(&self) -> Option<&IndexUrl> {
        match self {
            Self::Registry(registry) => Some(&registry.best_wheel().index),
            Self::DirectUrl(_) => None,
            Self::Path(_) => None,
        }
    }

    /// Returns the [`File`] instance, if this distribution is from a registry.
    pub fn file(&self) -> Option<&File> {
        match self {
            Self::Registry(registry) => Some(&registry.best_wheel().file),
            Self::DirectUrl(_) | Self::Path(_) => None,
        }
    }

    pub fn version(&self) -> &Version {
        match self {
            Self::Registry(wheels) => &wheels.best_wheel().filename.version,
            Self::DirectUrl(wheel) => &wheel.filename.version,
            Self::Path(wheel) => &wheel.filename.version,
        }
    }
}

impl SourceDist {
    /// Returns the [`IndexUrl`], if the distribution is from a registry.
    pub fn index(&self) -> Option<&IndexUrl> {
        match self {
            Self::Registry(registry) => Some(&registry.index),
            Self::DirectUrl(_) | Self::Git(_) | Self::Path(_) | Self::Directory(_) => None,
        }
    }

    /// Returns the [`File`] instance, if this dist is from a registry with simple json api support
    pub fn file(&self) -> Option<&File> {
        match self {
            Self::Registry(registry) => Some(&registry.file),
            Self::DirectUrl(_) | Self::Git(_) | Self::Path(_) | Self::Directory(_) => None,
        }
    }

    pub fn version(&self) -> Option<&Version> {
        match self {
            Self::Registry(source_dist) => Some(&source_dist.filename.version),
            Self::DirectUrl(_) | Self::Git(_) | Self::Path(_) | Self::Directory(_) => None,
        }
    }

    /// Return true if the distribution is editable.
    pub fn is_editable(&self) -> bool {
        match self {
            Self::Directory(DirectorySourceDist { editable, .. }) => *editable,
            _ => false,
        }
    }

    /// Returns the path to the source distribution, if it's a local distribution.
    pub fn as_path(&self) -> Option<&Path> {
        match self {
            Self::Path(dist) => Some(&dist.path),
            Self::Directory(dist) => Some(&dist.path),
            _ => None,
        }
    }
}

impl RegistryBuiltDist {
    /// Returns the best or "most compatible" wheel in this distribution.
    pub fn best_wheel(&self) -> &RegistryBuiltWheel {
        &self.wheels[self.best_wheel_index]
    }
}

impl Name for RegistryBuiltWheel {
    fn name(&self) -> &PackageName {
        &self.filename.name
    }
}

impl Name for RegistryBuiltDist {
    fn name(&self) -> &PackageName {
        self.best_wheel().name()
    }
}

impl Name for DirectUrlBuiltDist {
    fn name(&self) -> &PackageName {
        &self.filename.name
    }
}

impl Name for PathBuiltDist {
    fn name(&self) -> &PackageName {
        &self.filename.name
    }
}

impl Name for RegistrySourceDist {
    fn name(&self) -> &PackageName {
        &self.filename.name
    }
}

impl Name for DirectUrlSourceDist {
    fn name(&self) -> &PackageName {
        &self.name
    }
}

impl Name for GitSourceDist {
    fn name(&self) -> &PackageName {
        &self.name
    }
}

impl Name for PathSourceDist {
    fn name(&self) -> &PackageName {
        &self.name
    }
}

impl Name for DirectorySourceDist {
    fn name(&self) -> &PackageName {
        &self.name
    }
}

impl Name for SourceDist {
    fn name(&self) -> &PackageName {
        match self {
            Self::Registry(dist) => dist.name(),
            Self::DirectUrl(dist) => dist.name(),
            Self::Git(dist) => dist.name(),
            Self::Path(dist) => dist.name(),
            Self::Directory(dist) => dist.name(),
        }
    }
}

impl Name for BuiltDist {
    fn name(&self) -> &PackageName {
        match self {
            Self::Registry(dist) => dist.name(),
            Self::DirectUrl(dist) => dist.name(),
            Self::Path(dist) => dist.name(),
        }
    }
}

impl Name for Dist {
    fn name(&self) -> &PackageName {
        match self {
            Self::Built(dist) => dist.name(),
            Self::Source(dist) => dist.name(),
        }
    }
}

impl DistributionMetadata for RegistryBuiltWheel {
    fn version_or_url(&self) -> VersionOrUrlRef {
        VersionOrUrlRef::Version(&self.filename.version)
    }
}

impl DistributionMetadata for RegistryBuiltDist {
    fn version_or_url(&self) -> VersionOrUrlRef {
        self.best_wheel().version_or_url()
    }
}

impl DistributionMetadata for DirectUrlBuiltDist {
    fn version_or_url(&self) -> VersionOrUrlRef {
        VersionOrUrlRef::Url(&self.url)
    }
}

impl DistributionMetadata for PathBuiltDist {
    fn version_or_url(&self) -> VersionOrUrlRef {
        VersionOrUrlRef::Url(&self.url)
    }
}

impl DistributionMetadata for RegistrySourceDist {
    fn version_or_url(&self) -> VersionOrUrlRef {
        VersionOrUrlRef::Version(&self.filename.version)
    }
}

impl DistributionMetadata for DirectUrlSourceDist {
    fn version_or_url(&self) -> VersionOrUrlRef {
        VersionOrUrlRef::Url(&self.url)
    }
}

impl DistributionMetadata for GitSourceDist {
    fn version_or_url(&self) -> VersionOrUrlRef {
        VersionOrUrlRef::Url(&self.url)
    }
}

impl DistributionMetadata for PathSourceDist {
    fn version_or_url(&self) -> VersionOrUrlRef {
        VersionOrUrlRef::Url(&self.url)
    }
}

impl DistributionMetadata for DirectorySourceDist {
    fn version_or_url(&self) -> VersionOrUrlRef {
        VersionOrUrlRef::Url(&self.url)
    }
}

impl DistributionMetadata for SourceDist {
    fn version_or_url(&self) -> VersionOrUrlRef {
        match self {
            Self::Registry(dist) => dist.version_or_url(),
            Self::DirectUrl(dist) => dist.version_or_url(),
            Self::Git(dist) => dist.version_or_url(),
            Self::Path(dist) => dist.version_or_url(),
            Self::Directory(dist) => dist.version_or_url(),
        }
    }
}

impl DistributionMetadata for BuiltDist {
    fn version_or_url(&self) -> VersionOrUrlRef {
        match self {
            Self::Registry(dist) => dist.version_or_url(),
            Self::DirectUrl(dist) => dist.version_or_url(),
            Self::Path(dist) => dist.version_or_url(),
        }
    }
}

impl DistributionMetadata for Dist {
    fn version_or_url(&self) -> VersionOrUrlRef {
        match self {
            Self::Built(dist) => dist.version_or_url(),
            Self::Source(dist) => dist.version_or_url(),
        }
    }
}

impl RemoteSource for File {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        Ok(Cow::Borrowed(&self.filename))
    }

    fn size(&self) -> Option<u64> {
        self.size
    }
}

impl RemoteSource for Url {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        // Identify the last segment of the URL as the filename.
        let filename = self
            .path_segments()
            .and_then(Iterator::last)
            .ok_or_else(|| Error::UrlFilename(self.clone()))?;

        // Decode the filename, which may be percent-encoded.
        let filename = urlencoding::decode(filename)?;

        Ok(filename)
    }

    fn size(&self) -> Option<u64> {
        None
    }
}

impl RemoteSource for RegistryBuiltWheel {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        self.file.filename()
    }

    fn size(&self) -> Option<u64> {
        self.file.size()
    }
}

impl RemoteSource for RegistryBuiltDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        self.best_wheel().filename()
    }

    fn size(&self) -> Option<u64> {
        self.best_wheel().size()
    }
}

impl RemoteSource for RegistrySourceDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        self.file.filename()
    }

    fn size(&self) -> Option<u64> {
        self.file.size()
    }
}

impl RemoteSource for DirectUrlBuiltDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        self.url.filename()
    }

    fn size(&self) -> Option<u64> {
        self.url.size()
    }
}

impl RemoteSource for DirectUrlSourceDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        self.url.filename()
    }

    fn size(&self) -> Option<u64> {
        self.url.size()
    }
}

impl RemoteSource for GitSourceDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        // The filename is the last segment of the URL, before any `@`.
        match self.url.filename()? {
            Cow::Borrowed(filename) => {
                if let Some((_, filename)) = filename.rsplit_once('@') {
                    Ok(Cow::Borrowed(filename))
                } else {
                    Ok(Cow::Borrowed(filename))
                }
            }
            Cow::Owned(filename) => {
                if let Some((_, filename)) = filename.rsplit_once('@') {
                    Ok(Cow::Owned(filename.to_owned()))
                } else {
                    Ok(Cow::Owned(filename))
                }
            }
        }
    }

    fn size(&self) -> Option<u64> {
        self.url.size()
    }
}

impl RemoteSource for PathBuiltDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        self.url.filename()
    }

    fn size(&self) -> Option<u64> {
        self.url.size()
    }
}

impl RemoteSource for PathSourceDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        self.url.filename()
    }

    fn size(&self) -> Option<u64> {
        self.url.size()
    }
}

impl RemoteSource for DirectorySourceDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        self.url.filename()
    }

    fn size(&self) -> Option<u64> {
        self.url.size()
    }
}

impl RemoteSource for SourceDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        match self {
            Self::Registry(dist) => dist.filename(),
            Self::DirectUrl(dist) => dist.filename(),
            Self::Git(dist) => dist.filename(),
            Self::Path(dist) => dist.filename(),
            Self::Directory(dist) => dist.filename(),
        }
    }

    fn size(&self) -> Option<u64> {
        match self {
            Self::Registry(dist) => dist.size(),
            Self::DirectUrl(dist) => dist.size(),
            Self::Git(dist) => dist.size(),
            Self::Path(dist) => dist.size(),
            Self::Directory(dist) => dist.size(),
        }
    }
}

impl RemoteSource for BuiltDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        match self {
            Self::Registry(dist) => dist.filename(),
            Self::DirectUrl(dist) => dist.filename(),
            Self::Path(dist) => dist.filename(),
        }
    }

    fn size(&self) -> Option<u64> {
        match self {
            Self::Registry(dist) => dist.size(),
            Self::DirectUrl(dist) => dist.size(),
            Self::Path(dist) => dist.size(),
        }
    }
}

impl RemoteSource for Dist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        match self {
            Self::Built(dist) => dist.filename(),
            Self::Source(dist) => dist.filename(),
        }
    }

    fn size(&self) -> Option<u64> {
        match self {
            Self::Built(dist) => dist.size(),
            Self::Source(dist) => dist.size(),
        }
    }
}

impl Identifier for Url {
    fn distribution_id(&self) -> DistributionId {
        DistributionId::Url(cache_key::CanonicalUrl::new(self))
    }

    fn resource_id(&self) -> ResourceId {
        ResourceId::Url(cache_key::RepositoryUrl::new(self))
    }
}

impl Identifier for File {
    fn distribution_id(&self) -> DistributionId {
        if let Some(hash) = self.hashes.first() {
            DistributionId::Digest(hash.clone())
        } else {
            self.url.distribution_id()
        }
    }

    fn resource_id(&self) -> ResourceId {
        if let Some(hash) = self.hashes.first() {
            ResourceId::Digest(hash.clone())
        } else {
            self.url.resource_id()
        }
    }
}

impl Identifier for Path {
    fn distribution_id(&self) -> DistributionId {
        DistributionId::PathBuf(self.to_path_buf())
    }

    fn resource_id(&self) -> ResourceId {
        ResourceId::PathBuf(self.to_path_buf())
    }
}

impl Identifier for FileLocation {
    fn distribution_id(&self) -> DistributionId {
        match self {
            Self::RelativeUrl(base, url) => {
                DistributionId::RelativeUrl(base.to_string(), url.to_string())
            }
            Self::AbsoluteUrl(url) => DistributionId::AbsoluteUrl(url.to_string()),
            Self::Path(path) => path.distribution_id(),
        }
    }

    fn resource_id(&self) -> ResourceId {
        match self {
            Self::RelativeUrl(base, url) => {
                ResourceId::RelativeUrl(base.to_string(), url.to_string())
            }
            Self::AbsoluteUrl(url) => ResourceId::AbsoluteUrl(url.to_string()),
            Self::Path(path) => path.resource_id(),
        }
    }
}

impl Identifier for RegistryBuiltWheel {
    fn distribution_id(&self) -> DistributionId {
        self.file.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.file.resource_id()
    }
}

impl Identifier for RegistryBuiltDist {
    fn distribution_id(&self) -> DistributionId {
        self.best_wheel().distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.best_wheel().resource_id()
    }
}

impl Identifier for RegistrySourceDist {
    fn distribution_id(&self) -> DistributionId {
        self.file.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.file.resource_id()
    }
}

impl Identifier for DirectUrlBuiltDist {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for DirectUrlSourceDist {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for PathBuiltDist {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for PathSourceDist {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for DirectorySourceDist {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for GitSourceDist {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for SourceDist {
    fn distribution_id(&self) -> DistributionId {
        match self {
            Self::Registry(dist) => dist.distribution_id(),
            Self::DirectUrl(dist) => dist.distribution_id(),
            Self::Git(dist) => dist.distribution_id(),
            Self::Path(dist) => dist.distribution_id(),
            Self::Directory(dist) => dist.distribution_id(),
        }
    }

    fn resource_id(&self) -> ResourceId {
        match self {
            Self::Registry(dist) => dist.resource_id(),
            Self::DirectUrl(dist) => dist.resource_id(),
            Self::Git(dist) => dist.resource_id(),
            Self::Path(dist) => dist.resource_id(),
            Self::Directory(dist) => dist.resource_id(),
        }
    }
}

impl Identifier for BuiltDist {
    fn distribution_id(&self) -> DistributionId {
        match self {
            Self::Registry(dist) => dist.distribution_id(),
            Self::DirectUrl(dist) => dist.distribution_id(),
            Self::Path(dist) => dist.distribution_id(),
        }
    }

    fn resource_id(&self) -> ResourceId {
        match self {
            Self::Registry(dist) => dist.resource_id(),
            Self::DirectUrl(dist) => dist.resource_id(),
            Self::Path(dist) => dist.resource_id(),
        }
    }
}

impl Identifier for InstalledDist {
    fn distribution_id(&self) -> DistributionId {
        self.path().distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.path().resource_id()
    }
}

impl Identifier for Dist {
    fn distribution_id(&self) -> DistributionId {
        match self {
            Self::Built(dist) => dist.distribution_id(),
            Self::Source(dist) => dist.distribution_id(),
        }
    }

    fn resource_id(&self) -> ResourceId {
        match self {
            Self::Built(dist) => dist.resource_id(),
            Self::Source(dist) => dist.resource_id(),
        }
    }
}

impl Identifier for DirectSourceUrl<'_> {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for GitSourceUrl<'_> {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for PathSourceUrl<'_> {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for DirectorySourceUrl<'_> {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for SourceUrl<'_> {
    fn distribution_id(&self) -> DistributionId {
        match self {
            Self::Direct(url) => url.distribution_id(),
            Self::Git(url) => url.distribution_id(),
            Self::Path(url) => url.distribution_id(),
            Self::Directory(url) => url.distribution_id(),
        }
    }

    fn resource_id(&self) -> ResourceId {
        match self {
            Self::Direct(url) => url.resource_id(),
            Self::Git(url) => url.resource_id(),
            Self::Path(url) => url.resource_id(),
            Self::Directory(url) => url.resource_id(),
        }
    }
}

impl Identifier for BuildableSource<'_> {
    fn distribution_id(&self) -> DistributionId {
        match self {
            BuildableSource::Dist(source) => source.distribution_id(),
            BuildableSource::Url(source) => source.distribution_id(),
        }
    }

    fn resource_id(&self) -> ResourceId {
        match self {
            BuildableSource::Dist(source) => source.resource_id(),
            BuildableSource::Url(source) => source.resource_id(),
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{BuiltDist, Dist, SourceDist};

    /// Ensure that we don't accidentally grow the `Dist` sizes.
    #[test]
    fn dist_size() {
        assert!(
            std::mem::size_of::<Dist>() <= 336,
            "{}",
            std::mem::size_of::<Dist>()
        );
        assert!(
            std::mem::size_of::<BuiltDist>() <= 336,
            "{}",
            std::mem::size_of::<BuiltDist>()
        );
        assert!(
            std::mem::size_of::<SourceDist>() <= 256,
            "{}",
            std::mem::size_of::<SourceDist>()
        );
    }
}
