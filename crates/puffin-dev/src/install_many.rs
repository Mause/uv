use std::iter::Iterator;
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Context, Result};
use clap::Parser;
use futures::StreamExt;
use itertools::{Either, Itertools};
use rustc_hash::FxHashMap;
use tracing::info;

use distribution_types::{
    CachedDist, Dist, DistributionMetadata, IndexUrls, Name, Resolution, VersionOrUrl,
};
use install_wheel_rs::linker::LinkMode;
use pep508_rs::Requirement;
use platform_host::Platform;
use platform_tags::Tags;
use puffin_cache::{Cache, CacheArgs};
use puffin_client::{RegistryClient, RegistryClientBuilder};
use puffin_dispatch::BuildDispatch;
use puffin_distribution::RegistryWheelIndex;
use puffin_installer::Downloader;
use puffin_interpreter::Virtualenv;
use puffin_normalize::PackageName;
use puffin_resolver::DistFinder;
use puffin_traits::{BuildContext, OnceMap};

#[derive(Parser)]
pub(crate) struct InstallManyArgs {
    /// Path to a file containing one requirement per line.
    requirements: PathBuf,
    #[clap(long)]
    limit: Option<usize>,
    /// Don't build source distributions. This means resolving will not run arbitrary code. The
    /// cached wheels of already built source distributions will be reused.
    #[clap(long)]
    no_build: bool,
    /// Run this many tasks in parallel
    #[clap(long, default_value = "50")]
    num_tasks: usize,
    #[command(flatten)]
    cache_args: CacheArgs,
}

pub(crate) async fn install_many(args: InstallManyArgs) -> Result<()> {
    let data = fs_err::read_to_string(&args.requirements)?;

    let lines = data.lines().map(Requirement::from_str);
    let requirements: Vec<Requirement> = if let Some(limit) = args.limit {
        lines.take(limit).collect::<Result<_, _>>()?
    } else {
        lines.collect::<Result<_, _>>()?
    };
    info!("Got {} requirements", requirements.len());

    let cache = Cache::try_from(args.cache_args)?;
    let platform = Platform::current()?;
    let venv = Virtualenv::from_env(platform, &cache)?;
    let client = RegistryClientBuilder::new(cache.clone()).build();
    let index_urls = IndexUrls::default();
    let tags = venv.interpreter().tags()?;
    let build_dispatch = BuildDispatch::new(
        &client,
        &cache,
        venv.interpreter(),
        &index_urls,
        venv.python_executable(),
        args.no_build,
    );

    for (idx, requirements) in requirements.chunks(100).enumerate() {
        info!("Chunk {idx}");
        install_chunk(
            requirements,
            &build_dispatch,
            tags,
            &client,
            &venv,
            &index_urls,
        )
        .await?;
    }

    Ok(())
}

async fn install_chunk(
    requirements: &[Requirement],
    build_dispatch: &BuildDispatch<'_>,
    tags: &Tags,
    client: &RegistryClient,
    venv: &Virtualenv,
    index_urls: &IndexUrls,
) -> Result<()> {
    let resolution: Vec<_> = DistFinder::new(tags, client, venv.interpreter())
        .resolve_stream(requirements)
        .collect()
        .await;
    let (resolution, failures): (FxHashMap<PackageName, Dist>, Vec<_>) =
        resolution.into_iter().partition_result();
    for failure in &failures {
        info!("Failed to find wheel: {failure}");
    }
    info!("Failed to find {} wheel(s)", failures.len());
    let wheels_and_source_dist = resolution.len();
    let resolution = if build_dispatch.no_build() {
        let only_wheels: FxHashMap<_, _> = resolution
            .into_iter()
            .filter(|(_, dist)| match dist {
                Dist::Built(_) => true,
                Dist::Source(_) => false,
            })
            .collect();
        info!(
            "Removed {} source dists",
            wheels_and_source_dist - only_wheels.len()
        );
        only_wheels
    } else {
        resolution
    };
    let dists = Resolution::new(resolution)
        .into_distributions()
        .collect::<Vec<_>>();

    let mut registry_index = RegistryWheelIndex::new(build_dispatch.cache(), tags, index_urls);
    let (cached, uncached): (Vec<_>, Vec<_>) = dists.into_iter().partition_map(|dist| {
        // We always want the wheel for the latest version not whatever matching is in cache
        let VersionOrUrl::Version(version) = dist.version_or_url() else {
            unreachable!();
        };

        if let Some(cached) = registry_index.get_version(dist.name(), version) {
            Either::Left(CachedDist::Registry(cached.clone()))
        } else {
            Either::Right(dist)
        }
    });
    info!("Cached: {}, Uncached {}", cached.len(), uncached.len());

    let downloader = Downloader::new(build_dispatch.cache(), tags, client, build_dispatch);
    let in_flight = OnceMap::default();
    let fetches: Vec<_> = futures::stream::iter(uncached)
        .map(|dist| downloader.get_wheel(dist, &in_flight))
        .buffer_unordered(50)
        .collect()
        .await;
    let (wheels, failures): (Vec<_>, Vec<_>) = fetches.into_iter().partition_result();
    for failure in &failures {
        info!("Failed to fetch wheel: {failure}");
    }
    info!("Failed to fetch {} wheel(s)", failures.len());

    let wheels: Vec<_> = wheels.into_iter().chain(cached).collect();
    puffin_installer::Installer::new(venv)
        .with_link_mode(LinkMode::default())
        .install(&wheels)
        .context("Failed to install")?;
    info!("Installed {} wheels", wheels.len());
    Ok(())
}