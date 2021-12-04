mod backend;
mod config;
mod auth;
mod share;
mod error;

use actix_web::{HttpServer, App, web, Responder, get, HttpResponse, ResponseError};
use std::sync::Arc;
use anni_backend::backends::{FileBackend, DriveBackend};
use std::path::PathBuf;
use crate::backend::AnnilBackend;
use tokio_util::io::ReaderStream;
use crate::config::{Config, BackendItem};
use actix_web::middleware::Logger;
use jwt_simple::prelude::HS256Key;
use crate::auth::{AnnilAuth, AnnilClaims};
use anni_backend::AnniBackend;
use std::collections::{HashSet, HashMap};
use anni_backend::cache::{CachePool, Cache};
use anni_backend::backends::drive::DriveBackendSettings;
use actix_cors::Cors;
use crate::error::AnnilError;
use actix_web::web::Query;
use serde::Deserialize;
use std::process::Stdio;

struct AppState {
    backends: Vec<AnnilBackend>,
    key: HS256Key,
}

/// Get available albums of current annil server
#[get("/albums")]
async fn albums(claims: AnnilClaims, data: web::Data<AppState>) -> impl Responder {
    match claims {
        AnnilClaims::User(_) => {
            let mut albums: HashMap<&str, HashSet<&str>> = HashMap::new();

            // users can get real album list
            for backend in data.backends.iter() {
                let backend_albums = backend.albums();
                albums.extend(backend_albums.into_iter());
            }
            HttpResponse::Ok().json(albums)
        }
        AnnilClaims::Share(share) => {
            // guests can only get album list defined in jwt
            HttpResponse::Ok().json(share.audios.keys().collect::<Vec<_>>())
        }
    }
}

#[derive(Deserialize)]
struct AudioQuery {
    prefer_bitrate: Option<String>,
}

/// Get audio in an album with {catalog} and {track_id}
#[get("/{catalog}/{track_id}")]
async fn audio(claim: AnnilClaims, path: web::Path<(String, u8)>, data: web::Data<AppState>, query: Query<AudioQuery>) -> impl Responder {
    let (catalog, track_id) = path.into_inner();
    if !claim.can_fetch(&catalog, Some(track_id)) {
        return AnnilError::Unauthorized.error_response();
    }

    for backend in data.backends.iter() {
        if backend.enabled() && backend.has_album(&catalog) {
            let audio = backend.get_audio(&catalog, track_id).await.map_err(|_| AnnilError::NotFound);
            if let Err(e) = audio {
                return e.error_response();
            }

            let mut audio = audio.unwrap();
            let prefer_bitrate = if claim.is_guest() { "low" } else { query.prefer_bitrate.as_deref().unwrap_or("medium") };
            let bitrate = match prefer_bitrate {
                "low" => Some("128k"),
                "medium" => Some("192k"),
                "high" => Some("320k"),
                "lossless" => None,
                _ => Some("128k"),
            };

            let mut resp = HttpResponse::Ok();
            resp.append_header(("X-Origin-Type", format!("audio/{}", audio.extension)))
                .append_header(("X-Origin-Size", audio.size))
                .append_header(("X-Duration-Seconds", audio.duration))
                .content_type(match bitrate {
                    Some(_) => "audio/aac".to_string(),
                    None => format!("audio/{}", audio.extension)
                });

            return match bitrate {
                Some(bitrate) => {
                    let mut process = tokio::process::Command::new("ffmpeg")
                        .args(&[
                            "-i", "pipe:0",
                            "-map", "0:0",
                            "-b:a", bitrate,
                            "-f", "adts",
                            "-",
                        ])
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::null())
                        .spawn()
                        .unwrap();
                    let stdout = process.stdout.take().unwrap();
                    tokio::spawn(async move {
                        let mut stdin = process.stdin.as_mut().unwrap();
                        let _ = tokio::io::copy(&mut audio.reader, &mut stdin).await;
                    });
                    resp.streaming(ReaderStream::new(stdout))
                }
                None => {
                    resp.streaming(ReaderStream::new(audio.reader))
                }
            };
        }
    }
    HttpResponse::NotFound().finish()
}

/// Get audio cover of an album with {catalog}
#[get("/{catalog}/cover")]
async fn cover(claims: AnnilClaims, path: web::Path<String>, data: web::Data<AppState>) -> impl Responder {
    let catalog = path.into_inner();
    if !claims.can_fetch(&catalog, None) {
        return HttpResponse::Forbidden().finish();
    }

    for backend in data.backends.iter() {
        if backend.enabled() && backend.has_album_wide(&catalog) {
            return match backend.get_cover(&catalog).await {
                Ok(cover) => {
                    HttpResponse::Ok()
                        .content_type("image/jpeg")
                        .streaming(ReaderStream::new(cover))
                }
                Err(_) => {
                    HttpResponse::NotFound().finish()
                }
            };
        }
    }
    HttpResponse::NotFound().finish()
}

async fn init_state(config: &Config) -> anyhow::Result<web::Data<AppState>> {
    log::info!("Start initializing backends...");
    let now = std::time::SystemTime::now();
    let mut backends = Vec::with_capacity(config.backends.len());
    let mut caches = HashMap::new();
    for (backend_name, backend_config) in config.backends.iter() {
        log::debug!("Initializing backend: {}", backend_name);
        let mut backend = match &backend_config.item {
            BackendItem::File { root, .. } =>
                AnniBackend::File(FileBackend::new(PathBuf::from(root))),
            BackendItem::Drive { drive_id, corpora, token_path } =>
                AnniBackend::Drive(DriveBackend::new(Default::default(), DriveBackendSettings {
                    corpora: corpora.to_string(),
                    drive_id: drive_id.clone(),
                    token_path: token_path.as_deref().unwrap_or("annil.token").to_string(),
                }).await?),
        };
        if let Some(cache) = backend_config.cache() {
            log::debug!("Cache configuration detected: root = {}, max-size = {}", cache.root(), cache.max_size);
            if !caches.contains_key(cache.root()) {
                // new cache pool
                let pool = CachePool::new(cache.root(), cache.max_size);
                caches.insert(cache.root().to_string(), Arc::new(pool));
            }
            backend = AnniBackend::Cache(Cache::new(backend.into_box(), caches[cache.root()].clone()));
        }
        let mut backend = AnnilBackend::new(backend_name.to_string(), backend).await?;
        backend.set_enable(backend_config.enable);
        backends.push(backend);
    }
    log::info!("Backend initialization finished, used {:?}", now.elapsed().unwrap());

    // key
    let key = HS256Key::from_bytes(config.server.key().as_ref());
    Ok(web::Data::new(AppState { backends, key }))
}

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();
    let config = Config::from_file(std::env::args().nth(1).unwrap_or_else(|| "config.toml".to_owned()))?;
    let state = init_state(&config).await?;

    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .wrap(Cors::default()
                .allow_any_origin()
                .allowed_methods(vec!["GET", "OPTIONS"])
                .allowed_header(actix_web::http::header::AUTHORIZATION)
            )
            .wrap(AnnilAuth)
            .wrap(Logger::default())
            .service(cover)
            .service(audio)
            .service(albums)
            .service(share::share)
    })
        .bind(config.server.listen("localhost:3614"))?
        .run()
        .await?;
    Ok(())
}
