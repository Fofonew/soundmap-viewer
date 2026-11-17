//! Protocole `soundmap://` (Windows : `http://soundmap.localhost/`).
//!
//! Routes :
//! - `/wmts/{layer}/{z}/{x}/{y}`             proxy WMTS avec cache disque ;
//! - `/tms/{layer}/{z}/{x}/{y}.pbf`          proxy TMS (tuiles vectorielles) ;
//! - `/zone/{id}/terrain/{z}/{x}/{y}.png`    Terrarium depuis le pack ;
//! - `/zone/{id}/ortho/{z}/{x}/{y}.jpg`      ortho du pack ;
//! - `/zone/{id}/file/{chemin}`              fichier du pack (GeoJSON…) ;
//! - `/result/{id}/{variante}/{indicateur}.png` carte de niveaux calculée.

use crate::state::AppState;
use sm_data::{CancellationToken, tms, wmts};
use sm_geo::TileId;
use tauri::http::{Response, StatusCode, header};
use tauri::{Manager, Runtime, UriSchemeContext, UriSchemeResponder};

type Resp = Response<Vec<u8>>;

pub fn handle<R: Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: tauri::http::Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let app = ctx.app_handle().clone();
    let path = request.uri().path().to_owned();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let resp = match route(&state, &path).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(path, err = %e, "soundmap://");
                plain(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
            }
        };
        responder.respond(resp);
    });
}

fn build(status: StatusCode, mime: &str, body: Vec<u8>, cache: bool) -> Resp {
    let mut b = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, mime)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");
    if cache {
        b = b.header(header::CACHE_CONTROL, "max-age=86400");
    }
    b.body(body).unwrap_or_else(|_| Response::new(Vec::new()))
}

fn plain(status: StatusCode, msg: String) -> Resp {
    build(status, "text/plain; charset=utf-8", msg.into_bytes(), false)
}

fn not_found() -> Resp {
    build(StatusCode::NOT_FOUND, "text/plain", Vec::new(), false)
}

fn parse_zxy(z: &str, x: &str, y: &str) -> Option<TileId> {
    let y = y.split('.').next()?;
    Some(TileId::new(
        z.parse().ok()?,
        x.parse().ok()?,
        y.parse().ok()?,
    ))
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("json" | "geojson") => "application/json",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("pbf") => "application/x-protobuf",
        _ => "application/octet-stream",
    }
}

async fn route(state: &AppState, path: &str) -> anyhow::Result<Resp> {
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match parts.as_slice() {
        ["wmts", layer, z, x, y] => {
            let (Some(l), Some(t)) = (wmts::layer_by_id(layer), parse_zxy(z, x, y)) else {
                return Ok(not_found());
            };
            proxy(state, l.id, t, l.ext, l.format, |c, cancel| {
                Box::pin(async move { wmts::get_tile(c, &l, t, cancel).await })
            })
            .await
        }
        ["tms", layer, z, x, y] => {
            let (Some(l), Some(t)) = (tms::layer_by_id(layer), parse_zxy(z, x, y)) else {
                return Ok(not_found());
            };
            proxy(
                state,
                l.id,
                t,
                "pbf",
                "application/x-protobuf",
                |c, cancel| Box::pin(async move { tms::get_tile(c, &l, t, cancel).await }),
            )
            .await
        }
        ["zone", id, "terrain", z, x, y] => {
            let Some(t) = parse_zxy(z, x, y) else {
                return Ok(not_found());
            };
            let zone = state.zone(id)?;
            let png = tauri::async_runtime::spawn_blocking(move || zone.terrain_tile(t)).await??;
            Ok(match png {
                Some(b) => build(StatusCode::OK, "image/png", b, true),
                None => not_found(),
            })
        }
        ["zone", id, kind @ ("ortho" | "ocsge" | "cosia"), z, x, y] => {
            let Some(t) = parse_zxy(z, x, y) else {
                return Ok(not_found());
            };
            let ext = match *kind {
                "ortho" => "jpg",
                "ocsge" => "pbf",
                _ => "png",
            };
            let zone = state.zone(id)?;
            Ok(match zone.stored_tile(kind, t, ext) {
                Some(b) => build(StatusCode::OK, mime_for(ext), b, true),
                None => not_found(),
            })
        }
        ["result", id, variant, metric] => {
            let metric = metric.trim_end_matches(".png");
            Ok(match crate::sim::result_png(state, id, variant, metric) {
                Ok(png) => build(StatusCode::OK, "image/png", png, false),
                Err(e) => plain(StatusCode::NOT_FOUND, e.to_string()),
            })
        }
        ["zone", id, "file", rest @ ..] => {
            let rel = rest.join("/");
            let zone = state.zone(id)?;
            let Some(p) = zone.file(&rel) else {
                return Ok(not_found());
            };
            Ok(match tokio::fs::read(&p).await {
                Ok(b) => build(StatusCode::OK, mime_for(&rel), b, false),
                Err(_) => not_found(),
            })
        }
        _ => Ok(not_found()),
    }
}

type FetchFut<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = sm_data::Result<Option<sm_data::Bytes>>> + Send + 'a>,
>;

/// Cache disque d'abord ; sinon réseau (sauf hors ligne), puis mise en cache.
async fn proxy<F>(
    state: &AppState,
    layer: &str,
    t: TileId,
    ext: &str,
    mime: &str,
    fetch: F,
) -> anyhow::Result<Resp>
where
    F: for<'a> FnOnce(&'a sm_data::GeoClient, &'a CancellationToken) -> FetchFut<'a>,
{
    if let Some(hit) = state.cache.get(layer, t, ext).await {
        return Ok(match hit {
            Some(b) => build(StatusCode::OK, mime, b.to_vec(), true),
            None => not_found(),
        });
    }
    if state.offline {
        return Ok(not_found());
    }
    let cancel = CancellationToken::new();
    let data = fetch(&state.client, &cancel).await?;
    // Seules les tuiles présentes sont mises en cache : un 404 peut être
    // transitoire (observé sur le WMTS) et ne doit pas créer de trou durable.
    Ok(match data {
        Some(b) => {
            state.cache.put(layer, t, ext, Some(&b)).await?;
            build(StatusCode::OK, mime, b.to_vec(), true)
        }
        None => not_found(),
    })
}
