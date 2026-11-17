//! Client HTTP commun : UA générique, quotas par service, retries, annulation.

use crate::{DataError, Result};
use bytes::Bytes;
use governor::{DefaultDirectRateLimiter, Quota, RateLimiter};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

/// Services distants, chacun avec son quota.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Service {
    WmsR,
    Wmts,
    Tms,
    Wfs,
    Geocode,
    Altimetrie,
    Copc,
    OpenMeteo,
    AromeS3,
    Brgm,
}

impl Service {
    pub const ALL: [Service; 10] = [
        Service::WmsR,
        Service::Wmts,
        Service::Tms,
        Service::Wfs,
        Service::Geocode,
        Service::Altimetrie,
        Service::Copc,
        Service::OpenMeteo,
        Service::AromeS3,
        Service::Brgm,
    ];

    /// (requêtes/s, requêtes simultanées). Le WMS-R annonce ~40 req/s/IP :
    /// on reste à 30. Les autres valeurs sont prudentes.
    pub fn default_limits(self) -> (u32, usize) {
        match self {
            Service::WmsR => (30, 8),
            Service::Wmts | Service::Tms => (30, 12),
            Service::Wfs => (5, 4),
            Service::Geocode => (10, 4),
            Service::Altimetrie => (5, 2),
            Service::Copc => (10, 4),
            Service::OpenMeteo => (2, 2),
            Service::AromeS3 => (5, 2),
            Service::Brgm => (5, 2),
        }
    }
}

/// URL de base des services (surchargées dans les tests par un serveur mock).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Endpoints {
    pub geopf: String,
    pub open_meteo: String,
    pub arome_s3: String,
    pub brgm: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            geopf: "https://data.geopf.fr".into(),
            open_meteo: "https://api.open-meteo.com".into(),
            arome_s3: "https://meteofrance-pnt.s3.rbx.io.cloud.ovh.net".into(),
            brgm: "https://geoservices.brgm.fr".into(),
        }
    }
}

impl Endpoints {
    /// Tous les services pointent vers `base` (serveur de test).
    pub fn all_at(base: &str) -> Self {
        Self {
            geopf: base.into(),
            open_meteo: base.into(),
            arome_s3: base.into(),
            brgm: base.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub user_agent: String,
    /// Délai total maximal d'une requête.
    pub timeout: Duration,
    /// Délai maximal sans recevoir d'octet : une grosse réponse lente mais
    /// active n'est pas interrompue (débit partagé entre requêtes parallèles).
    pub read_timeout: Duration,
    pub max_retries: u32,
    pub base_backoff: Duration,
    pub limits: HashMap<Service, (u32, usize)>,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            user_agent: format!("soundmap-viewer/{}", env!("CARGO_PKG_VERSION")),
            timeout: Duration::from_secs(300),
            read_timeout: Duration::from_secs(30),
            max_retries: 5,
            base_backoff: Duration::from_millis(500),
            limits: Service::ALL
                .iter()
                .map(|s| (*s, s.default_limits()))
                .collect(),
        }
    }
}

/// Réponse HTTP réussie (2xx).
#[derive(Debug, Clone)]
pub struct HttpBody {
    pub status: u16,
    pub content_type: Option<String>,
    /// Taille totale annoncée par `Content-Range` (requêtes Range).
    pub total_len: Option<u64>,
    pub accept_ranges: bool,
    pub bytes: Bytes,
}

struct Gate {
    limiter: DefaultDirectRateLimiter,
    sem: Semaphore,
}

struct Inner {
    http: reqwest::Client,
    endpoints: Endpoints,
    cfg: HttpConfig,
    gates: HashMap<Service, Gate>,
}

/// Client partagé (clonable à coût nul).
#[derive(Clone)]
pub struct GeoClient(Arc<Inner>);

impl std::fmt::Debug for GeoClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeoClient")
            .field("endpoints", &self.0.endpoints)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Method {
    Get,
    Head,
}

impl GeoClient {
    pub fn new(endpoints: Endpoints, cfg: HttpConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(cfg.user_agent.clone())
            .timeout(cfg.timeout)
            .read_timeout(cfg.read_timeout)
            .connect_timeout(Duration::from_secs(15))
            .pool_max_idle_per_host(16)
            .build()?;
        let mut gates = HashMap::new();
        for s in Service::ALL {
            let (rps, conc) = cfg
                .limits
                .get(&s)
                .copied()
                .unwrap_or_else(|| s.default_limits());
            let rps = NonZeroU32::new(rps.max(1)).unwrap_or(NonZeroU32::MIN);
            gates.insert(
                s,
                Gate {
                    limiter: RateLimiter::direct(Quota::per_second(rps)),
                    sem: Semaphore::new(conc.max(1)),
                },
            );
        }
        Ok(Self(Arc::new(Inner {
            http,
            endpoints,
            cfg,
            gates,
        })))
    }

    /// Client par défaut vers les vrais services.
    pub fn public() -> Result<Self> {
        Self::new(Endpoints::default(), HttpConfig::default())
    }

    pub fn endpoints(&self) -> &Endpoints {
        &self.0.endpoints
    }

    /// GET avec retries ; renvoie une erreur `Status` pour toute réponse non 2xx.
    pub async fn get(
        &self,
        svc: Service,
        url: &str,
        cancel: &CancellationToken,
    ) -> Result<HttpBody> {
        self.request(svc, Method::Get, url, None, cancel).await
    }

    /// GET d'une plage d'octets `[start, end]` (inclusif).
    pub async fn get_range(
        &self,
        svc: Service,
        url: &str,
        start: u64,
        end: u64,
        cancel: &CancellationToken,
    ) -> Result<HttpBody> {
        self.request(svc, Method::Get, url, Some((start, end)), cancel)
            .await
    }

    /// HEAD (taille dans `total_len`).
    pub async fn head(
        &self,
        svc: Service,
        url: &str,
        cancel: &CancellationToken,
    ) -> Result<HttpBody> {
        self.request(svc, Method::Head, url, None, cancel).await
    }

    async fn request(
        &self,
        svc: Service,
        method: Method,
        url: &str,
        range: Option<(u64, u64)>,
        cancel: &CancellationToken,
    ) -> Result<HttpBody> {
        let gate = self
            .0
            .gates
            .get(&svc)
            .ok_or_else(|| DataError::Invalid(format!("service {svc:?} inconnu")))?;
        let mut attempt = 0u32;
        loop {
            if cancel.is_cancelled() {
                return Err(DataError::Cancelled);
            }
            let outcome = tokio::select! {
                _ = cancel.cancelled() => return Err(DataError::Cancelled),
                r = async {
                    let _permit = gate.sem.acquire().await.map_err(|_| DataError::Cancelled)?;
                    gate.limiter.until_ready().await;
                    self.once(method, url, range).await
                } => r,
            };
            let retryable = match &outcome {
                Ok(_) => return outcome,
                Err(DataError::Http(e)) => {
                    e.is_timeout() || e.is_connect() || e.is_request() || e.is_body()
                }
                Err(DataError::Status { status, body, .. }) => {
                    is_retryable_status(svc, *status, attempt) || body.contains("overloaded")
                }
                Err(_) => false,
            };
            if !retryable || attempt >= self.0.cfg.max_retries {
                return outcome;
            }
            let delay = backoff(self.0.cfg.base_backoff, attempt);
            match outcome.as_ref().err() {
                // 404 transitoire du WMTS : fréquent et sans gravité.
                Some(e) if e.is_not_found() => tracing::debug!(?svc, url, "404 transitoire, retry"),
                e => {
                    tracing::info!(?svc, attempt, ?delay, url, err = %e.map(ToString::to_string).unwrap_or_default(), "retry")
                }
            }
            attempt += 1;
            tokio::select! {
                _ = cancel.cancelled() => return Err(DataError::Cancelled),
                _ = tokio::time::sleep(delay) => {}
            }
        }
    }

    async fn once(&self, method: Method, url: &str, range: Option<(u64, u64)>) -> Result<HttpBody> {
        let mut req = match method {
            Method::Get => self.0.http.get(url),
            Method::Head => self.0.http.head(url),
        };
        if let Some((a, b)) = range {
            req = req.header(reqwest::header::RANGE, format!("bytes={a}-{b}"));
        }
        let resp = req.send().await?;
        let status = resp.status().as_u16();
        let headers = resp.headers().clone();
        let hdr = |k: reqwest::header::HeaderName| {
            headers
                .get(k)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let content_type = hdr(reqwest::header::CONTENT_TYPE);
        let accept_ranges = hdr(reqwest::header::ACCEPT_RANGES).is_some_and(|v| v == "bytes");
        let total_len = hdr(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.rsplit('/').next().and_then(|t| t.parse().ok()))
            .or_else(|| hdr(reqwest::header::CONTENT_LENGTH).and_then(|v| v.parse().ok()));
        let bytes = resp.bytes().await?;
        if !(200..300).contains(&status) {
            let body = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).into_owned();
            return Err(DataError::Status {
                status,
                url: url.to_owned(),
                body,
            });
        }
        Ok(HttpBody {
            status,
            content_type,
            total_len,
            accept_ranges,
            bytes,
        })
    }
}

fn is_retryable_status(svc: Service, status: u16, attempt: u32) -> bool {
    match status {
        408 | 425 | 429 | 500..=599 => true,
        // Le WMS-R renvoie parfois un 400 transitoire (observé le 17/11/2026).
        400 => svc == Service::WmsR,
        // Un 404 transitoire a été observé sur le WMTS sous charge : on
        // réessaie une fois avant de conclure que la tuile n'existe pas.
        404 => matches!(svc, Service::Wmts | Service::Tms) && attempt == 0,
        _ => false,
    }
}

/// Backoff exponentiel avec jitter ±50 %, plafonné à 20 s.
fn backoff(base: Duration, attempt: u32) -> Duration {
    let exp = base.saturating_mul(1u32 << attempt.min(6));
    let jitter = 0.5 + fastrand::f64();
    exp.mul_f64(jitter).min(Duration::from_secs(20))
}

/// Encode une valeur pour une query string (RFC 3986, caractères non réservés conservés).
pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn fast_client(base: &str) -> GeoClient {
        let cfg = HttpConfig {
            base_backoff: Duration::from_millis(5),
            max_retries: 3,
            ..HttpConfig::default()
        };
        GeoClient::new(Endpoints::all_at(base), cfg).unwrap()
    }

    #[tokio::test]
    async fn retries_on_503_then_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/x"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/x"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;
        let c = fast_client(&server.uri());
        let r = c
            .get(
                Service::Wmts,
                &format!("{}/x", server.uri()),
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(&r.bytes[..], b"ok");
    }

    #[tokio::test]
    async fn retries_on_overloaded_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_string(r#"{"error":true,"reason":"The service is overloaded"}"#),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;
        let c = fast_client(&server.uri());
        assert!(
            c.get(Service::OpenMeteo, &server.uri(), &CancellationToken::new())
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn no_retry_on_404() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;
        let c = fast_client(&server.uri());
        let e = c
            .get(Service::Wfs, &server.uri(), &CancellationToken::new())
            .await
            .unwrap_err();
        assert!(e.is_not_found());
    }

    #[tokio::test]
    async fn wmts_404_is_retried_once() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .expect(2)
            .mount(&server)
            .await;
        let c = fast_client(&server.uri());
        let e = c
            .get(Service::Wmts, &server.uri(), &CancellationToken::new())
            .await
            .unwrap_err();
        assert!(e.is_not_found());
    }

    #[tokio::test]
    async fn sends_generic_user_agent_and_range() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header("range", "bytes=0-3"))
            .and(header("user-agent", "soundmap-viewer/0.1.0"))
            .respond_with(
                ResponseTemplate::new(206)
                    .insert_header("content-range", "bytes 0-3/1000")
                    .set_body_bytes(b"LASF".to_vec()),
            )
            .mount(&server)
            .await;
        let c = fast_client(&server.uri());
        let r = c
            .get_range(
                Service::Copc,
                &server.uri(),
                0,
                3,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(r.status, 206);
        assert_eq!(r.total_len, Some(1000));
    }

    #[tokio::test]
    async fn cancellation_is_prompt() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(10)))
            .mount(&server)
            .await;
        let c = fast_client(&server.uri());
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            c2.cancel();
        });
        let t = std::time::Instant::now();
        let e = c
            .get(Service::Wmts, &server.uri(), &cancel)
            .await
            .unwrap_err();
        assert!(matches!(e, DataError::Cancelled));
        assert!(t.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn urlencode_reserved() {
        assert_eq!(urlencode("Millau 12100"), "Millau%2012100");
        assert_eq!(urlencode("a|b"), "a%7Cb");
    }
}
