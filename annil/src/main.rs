mod provider;
mod config;
mod auth;
mod share;
mod error;
mod services;

use actix_web::{HttpServer, App, web};
use std::sync::Arc;
use anni_provider::providers::{FileBackend, DriveBackend};
use std::path::PathBuf;
use crate::provider::AnnilProvider;
use crate::config::{Config, ProviderItem};
use actix_web::middleware::Logger;
use jwt_simple::prelude::HS256Key;
use crate::auth::{AnnilAuth, AnnilClaims};
use anni_provider::{AnniProvider, RepoDatabaseRead};
use std::collections::HashMap;
use anni_provider::cache::{CachePool, Cache};
use anni_provider::providers::drive::DriveProviderSettings;
use actix_cors::Cors;
use crate::error::AnnilError;
use std::time::{SystemTime, UNIX_EPOCH};
use jwt_simple::reexports::serde_json::json;
use tokio::sync::RwLock;
use anni_repo::RepositoryManager;
use crate::services::*;

pub struct AppState {
    providers: RwLock<Vec<AnnilProvider>>,
    key: HS256Key,
    reload_token: String,

    version: String,
    last_update: RwLock<u64>,
}

async fn init_state(config: &Config) -> anyhow::Result<web::Data<AppState>> {
    log::info!("Fetching metadata repository...");
    let repo = RepositoryManager::clone(&config.metadata.repo, config.metadata.base.join("repo"), &config.metadata.branch)?;
    let repo = repo.to_owned_manager()?;
    let database_path = config.metadata.base.join("repo.db");
    repo.to_database(&database_path).await?;
    log::info!("Metadata repository fetched.");

    log::info!("Start initializing providers...");
    let now = SystemTime::now();
    let mut providers = Vec::with_capacity(config.providers.len());
    let mut caches = HashMap::new();
    for (provider_name, provider_config) in config.providers.iter() {
        log::debug!("Initializing provider: {}", provider_name);
        let repo = RepoDatabaseRead::new(database_path.to_string_lossy().as_ref()).await?;
        let mut provider: Box<dyn AnniProvider + Send + Sync> = match &provider_config.item {
            ProviderItem::File { root } =>
                Box::new(FileBackend::new(PathBuf::from(root), repo).await?),
            ProviderItem::Drive { drive_id, corpora, token_path } =>
                Box::new(DriveBackend::new(Default::default(), DriveProviderSettings {
                    corpora: corpora.to_string(),
                    drive_id: drive_id.clone(),
                    token_path: token_path.as_deref().unwrap_or("annil.token").to_string(),
                }, repo).await?),
        };
        if let Some(cache) = provider_config.cache() {
            log::debug!("Cache configuration detected: root = {}, max-size = {}", cache.root(), cache.max_size);
            if !caches.contains_key(cache.root()) {
                // new cache pool
                let pool = CachePool::new(cache.root(), cache.max_size);
                caches.insert(cache.root().to_string(), Arc::new(pool));
            }
            provider = Box::new(Cache::new(provider, caches[cache.root()].clone()));
        }
        let provider = AnnilProvider::new(provider_name.to_string(), provider, provider_config.enable).await?;
        providers.push(provider);
    }
    log::info!("Provider initialization finished, used {:?}", now.elapsed().unwrap());

    // key
    let key = HS256Key::from_bytes(config.server.key().as_ref());
    let version = format!("Anniv v{}", env!("CARGO_PKG_VERSION"));
    let last_update = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    Ok(web::Data::new(AppState {
        providers: RwLock::new(providers),
        key,
        reload_token: config.server.reload_token().to_string(),
        version,
        last_update: RwLock::new(last_update),
    }))
}

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .filter_module("sqlx::query", log::LevelFilter::Warn)
        .init();
    let config = Config::from_file(std::env::args().nth(1).unwrap_or_else(|| "config.toml".to_owned()))?;
    let state = init_state(&config).await?;

    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .wrap(AnnilAuth)
            .wrap(Cors::default()
                .allow_any_origin()
                .allowed_methods(vec!["GET"])
                .allow_any_header()
                .send_wildcard()
            )
            .wrap(Logger::default())
            .service(info)
            .service(reload)
            .service(
                web::resource([
                    "/{album_id}/cover",
                    "/{album_id}/{disc_id}/cover",
                ])
                    .route(web::get().to(cover))
            )
            .service(web::resource("/{album_id}/{disc_id}/{track_id}")
                .route(web::get().to(audio))
                .route(web::head().to(audio_head))
            )
            .service(albums)
            .service(share::share)
    })
        .bind(config.server.listen("localhost:3614"))?
        .run()
        .await?;
    Ok(())
}
