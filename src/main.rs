// Copyright 2020 Cognite AS
use std::convert::Infallible;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use enum_map::Enum;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server};
use serde::{Deserialize, Serialize};
use unleash_api_client::client;
use unleash_api_client::config::EnvironmentConfig;
use unleash_api_client::Context;

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

async fn toggles<C>(
    client: Arc<client::Client<C, UserFeatures>>,
    req: Request<Body>,
) -> Result<Response<Body>, Infallible>
where
    C: http_client::HttpClient + Default,
{
    let cache = client.cached_state();
    let toggles = match cache.as_ref() {
        // Make an empty API doc with nothing in it
        None => Toggles::default(),
        Some(cache) => {
            let mut toggles = Toggles::default();
            let mut context: Context = Default::default();
            let fake_root = url::Url::parse("http://fakeroot.example.com/").unwrap();
            // unwrap should be safe because to get here the uri must have been valid already
            // but perhaps we should handle it
            let url = fake_root.join(&req.uri().to_string()).expect("valid uri");
            for (k, v) in url.query_pairs() {
                match k.as_ref() {
                    "environment" => context.environment = v.to_string(),
                    "appName" => context.app_name = v.to_string(),
                    "userId" => context.user_id = Some(v.to_string()),
                    "sessionId" => context.session_id = Some(v.to_string()),
                    "remoteAddress" => {
                        let ip_parsed = ipaddress::IPAddress::parse(v.to_string());
                        // should we report errors on bad IP address formats?
                        context.remote_address = ip_parsed.ok();
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
            cache.str_features(|str_features| {
                for (name, feature) in str_features {
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
            });
            toggles
        }
    };
    Ok(Response::new(serde_json::to_vec(&toggles).unwrap().into()))
}

#[tokio::main]
async fn main() -> Result<()> {
    // We'll bind to 127.0.0.1:3000
    let addr = ([127, 0, 0, 1], 3000).into();

    let config = EnvironmentConfig::from_env().map_err(|e| anyhow!(e))?;
    let client = Arc::new(
        client::ClientBuilder::default()
            .enable_string_features()
            .interval(500)
            .into_client::<http_client::native::NativeClient, UserFeatures>(
                &config.api_url,
                &config.app_name,
                &config.instance_id,
                config.secret,
            )
            .map_err(|e| anyhow!(e))?,
    );
    client.register().await.map_err(|e| anyhow!(e))?;

    let make_svc = make_service_fn(|_conn| {
        let conn_client = client.clone();
        async move {
            Ok::<_, Infallible>(service_fn(move |req: Request<Body>| {
                let req_client = conn_client.clone();
                async move { toggles(req_client, req).await }
            }))
        }
    });

    let server = Server::bind(&addr).serve(make_svc);

    if let Err(e) =
        futures::future::try_join(async { Ok(client.poll_for_updates().await) }, server).await
    {
        eprintln!("server error: {}", e);
    }
    Ok(())
}
