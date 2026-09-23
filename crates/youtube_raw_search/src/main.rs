use std::{env, net::SocketAddr, path::PathBuf, process::ExitCode, sync::Arc};
use youtube_raw_search::{
    AdminState, MutationStore, Publisher, PublisherPaths, RawSearchEngine, ReloadableEngine,
    admin_router, build_index, slice_corpus,
};
use youtube_search_service::{SearchBackend, ServerConfig};

const USAGE: &str = "usage: youtube-raw-search build <chunks.jsonl> <channels.jsonl> <index-dir> | measure-broad-single <index-dir> <query>... | serve [index-dir] | serve-mutable | slice <input.jsonl> <output.jsonl> <max-videos>";
const DEFAULT_BIND: &str = "[::]:8108";

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "youtube_raw_search=info".into()),
        )
        .init();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "raw search experiment failed");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("--help" | "-h") => println!("{USAGE}"),
        Some("build") => {
            let chunks = required_path(args.next(), "chunks JSONL path")?;
            let channels = required_path(args.next(), "channels JSONL path")?;
            let index = required_path(args.next(), "index directory")?;
            let report = build_index(&chunks, &channels, &index)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Some("serve") => {
            let index_path = args
                .next()
                .map(PathBuf::from)
                .or_else(|| env::var_os("RAW_SEARCH_INDEX_PATH").map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from("/data/index"));
            let bind = env::var("RAW_SEARCH_BIND")
                .unwrap_or_else(|_| DEFAULT_BIND.to_string())
                .parse::<SocketAddr>()?;
            let api_key = env::var("RAW_SEARCH_API_KEY").ok();
            let engine = RawSearchEngine::open(&index_path)?;
            youtube_search_service::serve(
                ServerConfig {
                    bind,
                    api_key,
                    raw_route: true,
                },
                Arc::new(engine),
            )
            .await?;
        }
        Some("measure-broad-single") => {
            let index = required_path(args.next(), "index directory")?;
            let queries = args.collect::<Vec<_>>();
            let engine = RawSearchEngine::open(&index)?;
            let report = engine.measure_broad_single(&queries)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Some("serve-mutable") => {
            serve_mutable().await?;
        }
        Some("slice") => {
            let input = required_path(args.next(), "input JSONL path")?;
            let output = required_path(args.next(), "output JSONL path")?;
            let max_videos = args
                .next()
                .ok_or("missing maximum video count")?
                .parse::<usize>()?;
            let report = slice_corpus(&input, &output, max_videos)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        _ => {
            return Err(USAGE.into());
        }
    }
    Ok(())
}

async fn serve_mutable() -> Result<(), Box<dyn std::error::Error>> {
    let bind = env::var("RAW_SEARCH_BIND")
        .unwrap_or_else(|_| DEFAULT_BIND.to_string())
        .parse::<SocketAddr>()?;
    let base_index = env_path("RAW_SEARCH_INDEX_PATH", "/data/index");
    let state_directory = env_path("RAW_SEARCH_STATE_DIRECTORY", "/data/mutable");
    let publisher = Arc::new(Publisher::new(PublisherPaths {
        base_chunks_gzip: required_env_path("RAW_SEARCH_BASE_CHUNKS_GZIP")?,
        base_channels_gzip: required_env_path("RAW_SEARCH_BASE_CHANNELS_GZIP")?,
        base_index,
        state_directory: state_directory.clone(),
    }));
    let store = Arc::new(MutationStore::open(
        &env::var_os("RAW_SEARCH_MUTATION_DB")
            .map(PathBuf::from)
            .unwrap_or_else(|| state_directory.join("mutations.sqlite3")),
    )?);
    let (active_engine, active_sequence) = publisher.open_active_generation(&store)?;
    let engine = Arc::new(ReloadableEngine::new(active_engine, active_sequence));
    let admin = AdminState::new(
        Arc::clone(&engine),
        store,
        publisher,
        env::var("RAW_SEARCH_ADMIN_API_KEY")
            .map_err(|_| "RAW_SEARCH_ADMIN_API_KEY is required for serve-mutable")?,
    )?;
    let search_backend: Arc<dyn SearchBackend> = engine;
    let search_config = ServerConfig {
        bind,
        api_key: env::var("RAW_SEARCH_API_KEY").ok(),
        raw_route: true,
    };
    let app = youtube_search_service::router(&search_config, search_backend)
        .merge(admin_router(admin.clone()));
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, active_sequence, "mutable raw search ready");
    youtube_search_service::serve_router(listener, app).await?;
    Ok(())
}

fn env_path(name: &str, default: &str) -> PathBuf {
    env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

fn required_env_path(name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{name} is required for serve-mutable").into())
}

fn required_path(value: Option<String>, name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    value
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing {name}").into())
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_BIND;
    use std::net::{IpAddr, SocketAddr};

    #[test]
    fn default_bind_accepts_fly_private_ipv6() {
        let bind: SocketAddr = DEFAULT_BIND.parse().unwrap();
        assert_eq!(bind.ip(), IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED));
        assert_eq!(bind.port(), 8108);
    }
}
