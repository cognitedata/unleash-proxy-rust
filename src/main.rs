// Copyright 2020 Cognite AS
#![warn(clippy::all)]

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Context as AnyhowContext, Result};
use chrono::Utc;
use enum_map::Enum;
use futures_timer::Delay;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server};
use hyper::{Method, StatusCode};
use log::{debug, warn};
use serde::{Deserialize, Serialize};
use unleash_api_client::api::{Metrics, MetricsBucket};
use unleash_api_client::client;
use unleash_api_client::config::EnvironmentConfig;
use unleash_api_client::context::{Context, IPAddress};

const ALLOWED_HEADERS: &str = "authorization,content-type,if-none-match";

#[allow(non_camel_case_types)]
#[derive(Debug, Deserialize, Serialize, Enum, Clone)]
enum UserFeatures {}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct Payload {
    #[serde(rename = "type")]
    _type: String,
    value: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct Variant {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<Payload>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct Toggle {
    name: String,
    enabled: bool,
    variant: Variant,
}

#[derive(Default, Deserialize, Serialize, Debug, Clone)]
struct Toggles {
    toggles: Vec<Toggle>,
}

async fn toggles(
    client: Arc<client::Client<UserFeatures>>,
    req: Request<Body>,
) -> Result<Response<Body>> {
    let cache = client.cached_state();
    let toggles = match cache.as_ref() {
        // Make an empty API doc with nothing in it
        None => Toggles::default(),
        Some(cache) => {
            let mut toggles = Toggles::default();
            let mut context: Context = Default::default();
            let fake_root = url::Url::parse("http://fakeroot.example.com/")?;
            // unwrap should be safe because to get here the uri must have been valid already
            // but perhaps we should handle it
            let url = fake_root
                .join(&req.uri().to_string())
                .context("bad uri in request")?;
            for (k, v) in url.query_pairs() {
                match k.as_ref() {
                    "environment" => context.environment = v.to_string(),
                    "appName" => context.app_name = v.to_string(),
                    "userId" => context.user_id = Some(v.to_string()),
                    "sessionId" => context.session_id = Some(v.to_string()),
                    "remoteAddress" => {
                        let ip_parsed = ipaddress::IPAddress::parse(v.to_string());
                        // should we report errors on bad IP address formats?
                        context.remote_address = ip_parsed.ok().map(IPAddress);
                    }
                    // TODO: how are properties.k=v handled? what separator?
                    // This seems to be unspecified in the js client.
                    k if k.starts_with("properties.") => {
                        context
                            .properties
                            .insert(k.split_at("properties".len()).1.to_owned(), v.to_string());
                    }
                    _ => {}
                }
            }
            for (name, feature) in cache.str_features() {
                let mut enabled = false;
                for memo in feature.strategies.iter() {
                    if memo(&context) {
                        enabled = true;
                        break;
                    }
                }
                let toggle = Toggle {
                    name: name.to_string(),
                    enabled,
                    variant: Variant {
                        // TODO: support variants in the underlying client
                        name: "default".into(),
                        payload: None,
                    },
                };
                toggles.toggles.push(toggle);
            }
            toggles
        }
    };

    Ok(Response::builder()
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .header(hyper::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(hyper::header::ACCESS_CONTROL_ALLOW_HEADERS, ALLOWED_HEADERS)
        .status(StatusCode::OK)
        .body(serde_json::to_vec(&toggles)?.into())?)
}

async fn metrics(
    metrics: Arc<Mutex<HashMap<String, Metrics>>>,
    req: Request<Body>,
) -> Result<Response<Body>> {
    // TODO: fixup the result types.
    let whole_body = hyper::body::to_bytes(req.into_body())
        .await
        .expect("failed to get body");
    let req_metrics: Metrics = serde_json::from_slice(&whole_body).expect("valid metrics");
    // We could could be super clever here and only merge buckets that are broadly compatible by time, but honestly,
    // supporting time skewed clients doesn't make sense. The use case for metrics is really just to know whether a
    // thing is or isn't being used and by which application.

    // Secondly, most folk running this are going to have just a few web apps, so being super scalable in the app-name
    // dimension isn't very useful: we actually need to be scalable in accepting the updates, and this is a write-heavy
    // workload, so arc_swap isn't useful. If this becomes a hot spot, we need to look at a journalling mechanism. For
    // now, we lock around updates, making this a serialisation point but hopefully fast.
    {
        let mut metrics = metrics.lock().unwrap();
        let entry = metrics
            .entry(req_metrics.app_name.clone())
            .or_insert_with(|| Metrics {
                app_name: req_metrics.app_name.clone(),
                instance_id: "proxy".into(),
                bucket: MetricsBucket {
                    // Save on computing times here: we will calculate appropriate buckets when we submit to the API
                    // server.
                    start: req_metrics.bucket.start,
                    stop: req_metrics.bucket.stop,
                    toggles: HashMap::new(),
                },
            });
        for (toggle, info) in req_metrics.bucket.toggles {
            for (state, count) in info {
                let toggle_map = entry.bucket.toggles.entry(toggle.clone());
                let counter = toggle_map
                    .or_insert_with(HashMap::new)
                    .entry(state)
                    .or_insert(0);
                *counter += count;
            }
        }
    }
    Ok(Response::builder()
        .header(hyper::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(hyper::header::ACCESS_CONTROL_ALLOW_HEADERS, ALLOWED_HEADERS)
        .status(StatusCode::OK)
        .body(Body::empty())?)
}

async fn send_metrics(
    url: &str,
    client: Arc<client::Client<UserFeatures>>,
    metrics: Arc<Mutex<HashMap<String, Metrics>>>,
    interval: Duration,
) {
    let metrics_endpoint = Metrics::endpoint(url);
    loop {
        let start = Utc::now();
        debug!("send_metrics: waiting {:?}", interval);
        Delay::new(interval).await;
        let mut batch = HashMap::new();
        {
            let mut locked = metrics.lock().unwrap();
            std::mem::swap(&mut batch, &mut locked);
        }
        debug!("sending metrics");
        let stop = Utc::now();
        // TODO: very large numbers of discrete apps will cause this loop to
        // start exceeding 15 seconds and require assembling a concurrent
        // approach here as well, but this is probably a very very long way off.
        for (app_name, mut metrics) in batch {
            let mut metrics_uploaded = false;
            metrics.bucket.start = start;
            metrics.bucket.stop = stop;
            let req = client.http.post(&metrics_endpoint);
            if let Ok(body) = http_types::Body::from_json(&metrics) {
                let res = req.body(body).await;
                if let Ok(res) = res {
                    if res.status().is_success() {
                        metrics_uploaded = true;
                        debug!("poll: uploaded feature metrics `{}`", app_name);
                    }
                }
            }
            if !metrics_uploaded {
                warn!("poll: error uploading feature metrics `{}`", app_name);
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    // We'll bind to 127.0.0.1:3000
    debug!("serving on 127.0.0.1:3000");
    let addr = ([127, 0, 0, 1], 3000).into();

    let config = EnvironmentConfig::from_env().map_err(|e| anyhow!(e))?;
    let client = Arc::new(
        client::ClientBuilder::default()
            .disable_metric_submission()
            .enable_string_features()
            .into_client::<UserFeatures>(
                &config.api_url,
                &config.app_name,
                &config.instance_id,
                config.secret.clone(),
            )
            .map_err(|e| anyhow!(e))?,
    );
    client.register().await.map_err(|e| anyhow!(e))?;

    let client_metrics = Arc::new(Mutex::new(HashMap::new()));

    let make_svc = make_service_fn(|_conn| {
        let conn_client = client.clone();
        let conn_metrics = client_metrics.clone();
        async move {
            Ok::<_, Infallible>(service_fn(move |req: Request<Body>| {
                // Consider making a single struct to reduce Arc reference
                // taking overheads if this service gets busy.
                let req_client = conn_client.clone();
                let req_metrics = conn_metrics.clone();
                async move {
                    match (req.method(), req.uri().path()) {
                        (&Method::GET, "/") => toggles(req_client, req).await,
                        (&Method::OPTIONS, "/") => Ok(Response::builder()
                            .header(hyper::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                            .header(hyper::header::ACCESS_CONTROL_ALLOW_HEADERS, ALLOWED_HEADERS)
                            .status(StatusCode::OK)
                            .body(Body::empty())?),
                        (&Method::POST, "/client/metrics") => metrics(req_metrics, req).await,
                        (&Method::OPTIONS, "/client/metrics") => Ok(Response::builder()
                            .header(hyper::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                            .header(hyper::header::ACCESS_CONTROL_ALLOW_HEADERS, ALLOWED_HEADERS)
                            .status(StatusCode::OK)
                            .body(Body::empty())?),
                        _ => Ok(Response::builder()
                            .header(hyper::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                            .status(StatusCode::NOT_FOUND)
                            .body(Body::empty())?),
                    }
                }
            }))
        }
    });

    let server = Server::bind(&addr).serve(make_svc);
    if let Err(e) = futures::try_join!(
        async { Ok(client.poll_for_updates().await) },
        async {
            Ok(send_metrics(
                &config.api_url,
                client.clone(),
                client_metrics.clone(),
                // 30 seconds is the default interval for metrics in the browser client source
                Duration::from_secs(30),
            )
            .await)
        },
        server,
    ) {
        eprintln!("server error: {}", e);
    }
    Ok(())
}
