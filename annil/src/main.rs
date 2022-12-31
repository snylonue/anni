use annil::config::{Config, MetadataConfig, ProviderItem};
use annil::provider::AnnilProvider;
use annil::utils::compute_etag;

use anni_provider::cache::{Cache, CachePool};
use anni_provider::fs::LocalFileSystemProvider;
use anni_provider::providers::drive::DriveProviderSettings;
use anni_provider::providers::{CommonConventionProvider, CommonStrictProvider, DriveProvider};
use anni_provider::{AnniProvider, RepoDatabaseRead};
use anni_repo::{setup_git2, RepositoryManager};
use annil::extractor::token::Keys;
use annil::route::admin;
use annil::route::user;
use annil::AppState;
use axum::routing::{get, post};
use axum::{Extension, Router, Server};
use jwt_simple::prelude::HS256Key;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

struct LazyDb {
    metadata: MetadataConfig,
    db_path: Option<PathBuf>,
}

impl LazyDb {
    pub fn new(metadata: &MetadataConfig) -> Self {
        Self {
            metadata: metadata.clone(),
            db_path: None,
        }
    }

    pub fn open(&mut self) -> anyhow::Result<RepoDatabaseRead> {
        let db = match self.db_path {
            Some(ref p) => p,
            None => {
                let p = init_metadata(&self.metadata)?;
                self.db_path.insert(p)
            }
        };
        Ok(RepoDatabaseRead::new(db)?)
    }
}

fn init_metadata(metadata: &MetadataConfig) -> anyhow::Result<PathBuf> {
    log::info!("Fetching metadata repository...");
    let repo_root = metadata.base.join("repo");
    let repo = if !repo_root.exists() {
        log::debug!("Cloning metadata repository from {}", metadata.repo);
        RepositoryManager::clone(&metadata.repo, repo_root)?
    } else if metadata.pull {
        log::debug!(
            "Updating metadata repository at branch: {}",
            metadata.branch
        );
        RepositoryManager::pull(repo_root, &metadata.branch)?
    } else {
        log::debug!("Loading metadata repository at {}", repo_root.display());
        RepositoryManager::new(repo_root)?
    };
    log::debug!("Generating metadata database...");
    let repo = repo.into_owned_manager()?;
    let database_path = metadata.base.join("repo.db");
    repo.to_database(&database_path)?;
    log::info!("Metadata repository fetched.");

    Ok(database_path)
}

async fn init_state(config: Config) -> anyhow::Result<(Arc<AppState>, Keys)> {
    // proxy settings
    let mut db = if let Some(metadata) = &config.metadata {
        if let Some(proxy) = &metadata.proxy {
            // if metadata.proxy is an empty string, do not use proxy
            if proxy.is_empty() {
                setup_git2(None);
            } else {
                // otherwise, set proxy in config file
                setup_git2(Some(proxy.clone()));
            }
            // if no proxy was provided, use default behavior (http_proxy)
        }

        // init metadata
        Some(LazyDb::new(metadata))
    } else {
        None
    };

    log::info!("Start initializing providers...");
    let now = SystemTime::now();
    let mut providers = Vec::with_capacity(config.providers.len());
    let mut caches = HashMap::new();

    for (provider_name, provider_config) in config.providers.iter().filter(|(_, cfg)| cfg.enable) {
        log::debug!("Initializing provider: {}", provider_name);
        let mut provider: Box<dyn AnniProvider + Send + Sync> =
            match (&provider_config.item, &mut db) {
                (
                    ProviderItem::File {
                        root,
                        strict: false,
                        ..
                    },
                    Some(db),
                ) => Box::new(
                    CommonConventionProvider::new(
                        PathBuf::from(root),
                        db.open()?,
                        Box::new(LocalFileSystemProvider),
                    )
                    .await?,
                ),
                (
                    ProviderItem::File {
                        root,
                        strict: true,
                        layer,
                    },
                    _,
                ) => Box::new(
                    CommonStrictProvider::new(
                        PathBuf::from(root),
                        *layer,
                        Box::new(LocalFileSystemProvider),
                    )
                    .await?,
                ),
                (
                    ProviderItem::Drive {
                        drive_id,
                        corpora,
                        initial_token_path,
                        token_path,
                        strict: false,
                    },
                    Some(db),
                ) => {
                    if let Some(initial_token_path) = initial_token_path {
                        if initial_token_path.exists() && !token_path.exists() {
                            let _ = std::fs::copy(initial_token_path, token_path.clone());
                        }
                    }
                    Box::new(
                        DriveProvider::new(
                            Default::default(),
                            DriveProviderSettings {
                                corpora: corpora.to_string(),
                                drive_id: drive_id.clone(),
                            },
                            Some(db.open()?),
                            token_path.clone(),
                        )
                        .await?,
                    )
                }
                (
                    ProviderItem::Drive {
                        drive_id,
                        corpora,
                        initial_token_path,
                        token_path,
                        strict: true,
                    },
                    _,
                ) => {
                    if let Some(initial_token_path) = initial_token_path {
                        if initial_token_path.exists() && !token_path.exists() {
                            let _ = std::fs::copy(initial_token_path, token_path.clone());
                        }
                    }
                    Box::new(
                        DriveProvider::new(
                            Default::default(),
                            DriveProviderSettings {
                                corpora: corpora.to_string(),
                                drive_id: drive_id.clone(),
                            },
                            None,
                            token_path.clone(),
                        )
                        .await?,
                    )
                }
                (_, None) => {
                    log::error!(
                        "Metadata is not configured, but provider {} requires it.",
                        provider_name
                    );
                    continue;
                }
            };
        if let Some(cache) = provider_config.cache() {
            log::debug!(
                "Cache configuration detected: root = {}, max-size = {}",
                cache.root(),
                cache.max_size
            );
            if !caches.contains_key(cache.root()) {
                // new cache pool
                let pool = CachePool::new(cache.root(), cache.max_size);
                caches.insert(cache.root().to_string(), Arc::new(pool));
            }
            provider = Box::new(Cache::new(provider, caches[cache.root()].clone()));
        }
        let provider =
            AnnilProvider::new(provider_name.to_string(), provider, provider_config.enable).await?;
        providers.push(provider);
    }
    log::info!(
        "Provider initialization finished, used {:?}",
        now.elapsed().unwrap()
    );

    // etag
    let etag = compute_etag(&providers).await;

    // key
    let sign_key = HS256Key::from_bytes(config.server.key().as_ref());
    let share_key = HS256Key::from_bytes(config.server.share_key().as_ref())
        .with_key_id(config.server.share_key_id());
    let version = format!("Annil v{}", env!("CARGO_PKG_VERSION"));
    let last_update = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    Ok((
        Arc::new(AppState {
            providers: RwLock::new(providers),
            version,
            metadata: config.metadata,
            last_update: RwLock::new(last_update),
            etag: RwLock::new(Some(etag)),
        }),
        Keys {
            sign_key,
            share_key,
            admin_token: config.server.admin_token().to_string(),
        },
    ))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_env("ANNI_LOG")
        .filter_module("sqlx::query", log::LevelFilter::Warn)
        .init();
    let config = Config::from_file(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "config.toml".to_owned()),
    )?;
    let listen: SocketAddr = config.server.listen().parse()?;
    let (state, keys) = init_state(config).await?;

    let app = Router::new()
        .route("/info", get(user::info))
        .route(
            "/:album_id/:disc_id/:track_id",
            get(user::audio).head(user::audio_head),
        )
        .route("/cover/:album_id", get(user::cover))
        .route("/cover/:album_id/:disc_id", get(user::cover))
        .route("/admin/sign", post(admin::sign))
        .route("/admin/reload", post(admin::reload))
        .layer(Extension(state))
        .with_state(Arc::new(keys));

    Server::bind(&listen)
        .serve(app.into_make_service())
        .await
        .unwrap();

    // HttpServer::new(move || {
    //     App::new()
    //         .app_data(state.clone())
    //         .wrap(AuthExtractor)
    //         .wrap(
    //             Cors::default()
    //                 .allow_any_origin()
    //                 .allowed_methods(vec!["GET"])
    //                 .allow_any_header()
    //                 .send_wildcard(),
    //         )
    //         .wrap(Logger::default().exclude("/info"))
    //         .service(info)
    //         .service(admin::reload)
    //         .service(admin::sign)
    //         .service(cover)
    //         .service(
    //             web::resource("/{album_id}/{disc_id}/{track_id}")
    //                 .route(web::get().to(audio))
    //                 .route(web::head().to(audio_head)),
    //         )
    //         .service(albums)
    // })
    // .bind(listen)?
    // .run()
    // .await?;
    Ok(())
}
