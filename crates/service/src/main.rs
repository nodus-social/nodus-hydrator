//! The hydrator as a private HTTP service.
//!
//! The web host calls these routes; the browser never does. Bind it to loopback or a
//! private network and put the same-origin proxy in front of it.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, Request, State};
use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use nodus_hydrator::{Error, Hydrator};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::RwLock;

const TTL: Duration = Duration::from_secs(300);

#[derive(Clone)]
struct App {
    hydrator: Hydrator,
    cache: Arc<RwLock<HashMap<String, (Instant, Value)>>>,
    token: Option<String>,
}

impl App {
    async fn cached<F, T>(&self, key: String, read: F) -> Result<Json<Value>, Failure>
    where
        F: Future<Output = Result<T, Error>>,
        T: serde::Serialize,
    {
        if let Some((at, value)) = self.cache.read().await.get(&key)
            && at.elapsed() < TTL
        {
            return Ok(Json(value.clone()));
        }
        let value = serde_json::to_value(read.await?)
            .map_err(|error| Failure::internal(error.to_string()))?;
        self.cache
            .write()
            .await
            .insert(key, (Instant::now(), value.clone()));
        Ok(Json(value))
    }
}

#[derive(Debug)]
struct Failure {
    status: StatusCode,
    message: String,
}

impl Failure {
    fn internal(message: String) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message,
        }
    }
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

#[derive(Debug, Deserialize)]
struct Acct {
    acct: String,
    cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PageQuery {
    max_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(rename = "type", default = "search_type")]
    kind: String,
    offset: Option<String>,
}

fn search_type() -> String {
    "accounts".to_owned()
}

#[derive(Debug, Deserialize)]
struct GraphQuery {
    max_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UrlQuery {
    url: String,
}

async fn auth(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    if let Some(token) = &app.token
        && request
            .headers()
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            != Some(&format!("Bearer {token}"))
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    next.run(request).await
}

async fn healthz() -> Json<Value> {
    Json(json!({ "ok": true }))
}

async fn ap_account(
    State(app): State<Arc<App>>,
    Query(query): Query<Acct>,
) -> Result<Json<Value>, Failure> {
    app.cached(
        format!("acct:{}", query.acct),
        app.hydrator.account_acct(&query.acct),
    )
    .await
}

async fn account(
    State(app): State<Arc<App>>,
    Path((host, id)): Path<(String, String)>,
) -> Result<Json<Value>, Failure> {
    app.cached(
        format!("account:{host}:{id}"),
        app.hydrator.account(&host, &id),
    )
    .await
}

async fn posts(
    State(app): State<Arc<App>>,
    Path((host, id)): Path<(String, String)>,
    Query(query): Query<PageQuery>,
) -> Result<Json<Value>, Failure> {
    let key = format!("posts:{host}:{id}:{:?}", query.max_id);
    app.cached(key, app.hydrator.posts(&host, &id, query.max_id.as_deref()))
        .await
}

async fn ap_posts(
    State(app): State<Arc<App>>,
    Query(query): Query<Acct>,
) -> Result<Json<Value>, Failure> {
    let key = format!("ap-posts:{}:{:?}", query.acct, query.cursor);
    app.cached(
        key,
        app.hydrator
            .posts_acct(&query.acct, query.cursor.as_deref()),
    )
    .await
}

async fn graph(
    State(app): State<Arc<App>>,
    Path((host, id, kind)): Path<(String, String, String)>,
    Query(query): Query<GraphQuery>,
) -> Result<Json<Value>, Failure> {
    let following = match kind.as_str() {
        "followers" => false,
        "following" => true,
        _ => {
            return Err(Failure {
                status: StatusCode::NOT_FOUND,
                message: "graph kind is followers or following".to_owned(),
            });
        }
    };
    let key = format!("graph:{host}:{id}:{kind}:{:?}", query.max_id);
    app.cached(
        key,
        app.hydrator
            .graph(&host, &id, following, query.max_id.as_deref()),
    )
    .await
}

async fn instance(
    State(app): State<Arc<App>>,
    Path(host): Path<String>,
) -> Result<Json<Value>, Failure> {
    app.cached(format!("instance:{host}"), app.hydrator.instance(&host))
        .await
}

async fn search(
    State(app): State<Arc<App>>,
    Path(host): Path<String>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Value>, Failure> {
    let key = format!(
        "search:{host}:{}:{}:{:?}",
        query.q, query.kind, query.offset
    );
    app.cached(
        key,
        app.hydrator
            .search(&host, &query.q, &query.kind, query.offset.as_deref()),
    )
    .await
}

async fn status(
    State(app): State<Arc<App>>,
    Path((host, id)): Path<(String, String)>,
) -> Result<Json<Value>, Failure> {
    app.cached(
        format!("status:{host}:{id}"),
        app.hydrator.status(&host, &id),
    )
    .await
}

async fn thread(
    State(app): State<Arc<App>>,
    Path((host, id)): Path<(String, String)>,
) -> Result<Json<Value>, Failure> {
    app.cached(format!("thread:{host}:{id}"), async {
        app.hydrator
            .thread(&host, &id)
            .await
            .map(|items| json!({ "items": items }))
    })
    .await
}

async fn ap_replies(
    State(app): State<Arc<App>>,
    Query(query): Query<UrlQuery>,
) -> Result<Json<Value>, Failure> {
    app.cached(
        format!("replies:{}", query.url),
        app.hydrator.replies_acct(&query.url),
    )
    .await
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "nodus_hydrate=info".into()),
        )
        .init();
    let listen =
        std::env::var("NODUS_HYDRATE_LISTEN").unwrap_or_else(|_| "127.0.0.1:8091".to_owned());
    let app = Arc::new(App {
        hydrator: Hydrator::new(),
        cache: Arc::new(RwLock::new(HashMap::new())),
        token: std::env::var("NODUS_HYDRATE_TOKEN").ok(),
    });
    let router = Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/ap/account", get(ap_account))
        .route("/v1/ap/posts", get(ap_posts))
        .route("/v1/ap/replies", get(ap_replies))
        .route("/v1/account/{host}/{id}", get(account))
        .route("/v1/account/{host}/{id}/statuses", get(posts))
        .route("/v1/status/{host}/{id}", get(status))
        .route("/v1/instance/{host}", get(instance))
        .route("/v1/search/{host}", get(search))
        .route("/v1/account/{host}/{id}/{kind}", get(graph))
        .route("/v1/thread/{host}/{id}", get(thread))
        .layer(middleware::from_fn_with_state(app.clone(), auth))
        .with_state(app);
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    tracing::info!(%listen, "hydrator listening");
    axum::serve(listener, router).await?;
    Ok(())
}
