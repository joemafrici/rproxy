use log::info;
use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use toml::Table;

use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use http_body_util::{Empty, Full};
use hyper::body::{Buf, Bytes};
use hyper::server::conn::http1;
use hyper::{Method, StatusCode};

use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

use serde::Deserialize;

#[derive(Deserialize)]
struct SwitchRequestBody {
    app: String,
    port: u32,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    env_logger::init();
    let config_str = fs::read_to_string("/Users/deepwater/code/rproxy/config.toml")
        .expect("Should have been able to read config file");
    let config_toml = config_str
        .parse::<Table>()
        .expect("Should have been able to parse config file");
    let state: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));
    let addr = SocketAddr::from(([127, 0, 0, 1], 3002));

    {
        let mut routes = state.lock().unwrap();
        for (service, port) in &config_toml {
            routes.insert(service.to_string(), port.to_string());
        }
    };

    let listener = TcpListener::bind(addr).await?;
    info!("Listening on http://{}", addr);

    // accept loop
    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();

        // use an adapter to access something implementing `tokio::io` traits as if they implement
        // `hyper::rt` IO traits.
        let io = TokioIo::new(stream);

        // spawn a tokio task to serve multiple connections concurrently
        tokio::task::spawn(async move {
            let service = service_fn(move |req| {
                let state = state.clone();
                proxy_handler(state, req)
            });
            if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                eprintln!("Error serving connection: {:?}", err);
            }
        });
    }
}

async fn proxy_handler(
    state: Arc<Mutex<HashMap<String, String>>>,
    req: Request<hyper::body::Incoming>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    info!("Incoming request from {:?}", req.uri());
    let result = match (req.method(), req.uri().path()) {
        (&Method::POST, "/api/deploy/update") => handle_deploy_update(state, req).await,
        (&Method::GET, "/api/port") => handle_get_port_number(state, req).await,
        (&Method::POST, "/api/switch") => handle_switch(state, req).await,

        _ => handle_forward_request(state, req).await,
    };
    return result;
}

async fn handle_switch(
    state: Arc<Mutex<HashMap<String, String>>>,
    req: Request<hyper::body::Incoming>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let body = req.into_body();
    let bytes = match body.collect().await {
        Ok(b) => b,
        Err(e) => panic!("Problem reading request body stream: {}", e),
    };

    let bytes = bytes.aggregate();
    let body: SwitchRequestBody = match serde_json::from_slice(bytes.chunk()) {
        Ok(b) => b,
        Err(e) => panic!("Problem deserializing request body: {}", e),
    };

    info!(
        "Received request to switch {} to port {}",
        body.app, body.port
    );

    {
        let mut routes = state.lock().unwrap();
        let old_port = routes.insert(body.app, body.port.to_string());
    };

    todo!()
}

// Returns a free port on the local system
async fn handle_get_port_number(
    _state: Arc<Mutex<HashMap<String, String>>>,
    _req: Request<hyper::body::Incoming>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let port = match port_check::free_local_port() {
        Some(p) => p,
        None => {
            info!("Error unable to get a free port");
            let mut internal_server_error = Response::new(empty());
            *internal_server_error.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            return Ok(internal_server_error);
        }
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(BoxBody::new(full(port.to_string())))
        .unwrap())
}
async fn handle_deploy_update(
    state: Arc<Mutex<HashMap<String, String>>>,
    req: Request<hyper::body::Incoming>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    todo!();
}
async fn handle_forward_request(
    state: Arc<Mutex<HashMap<String, String>>>,
    req: Request<hyper::body::Incoming>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let client = Client::builder(hyper_util::rt::TokioExecutor::new()).build(HttpConnector::new());
    let _path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("");

    let path = req.uri().path().strip_prefix("/").unwrap_or("");
    let service_name = match path {
        "static/styles.css" | "static/index.js" | "" => "webserver",
        _ => path,
    };

    info!("Request should be forwarded to {}", service_name);
    info!("Retrieving port number for {}", service_name);
    let port_number = {
        let routes = state.lock().unwrap();
        match routes.get(service_name) {
            Some(p) => p.to_string(),
            None => {
                info!("Unable to determine port number");
                let mut not_found = Response::new(empty());
                *not_found.status_mut() = StatusCode::NOT_FOUND;
                return Ok(not_found);
            }
        }
    };

    let uri = format!("http://localhost:{}/{}", port_number, path);
    info!("forwarding request to {}", uri);
    let mut proxy_req = Request::builder().method(req.method()).uri(uri);

    if let Some(headers) = proxy_req.headers_mut() {
        headers.extend(
            req.headers()
                .iter()
                .filter(|(h, _)| *h != "host")
                .map(|(h, v)| (h.clone(), v.clone())),
        );
    }

    let proxy_req = proxy_req.body(req.into_body()).unwrap();

    let response = client.request(proxy_req).await.unwrap();
    let (parts, body) = response.into_parts();
    let boxed_body = BoxBody::new(body);
    Ok(Response::from_parts(parts, boxed_body))
}

fn empty() -> BoxBody<Bytes, hyper::Error> {
    Empty::<Bytes>::new()
        .map_err(|never| match never {})
        .boxed()
}
fn full<T: Into<Bytes>>(chunk: T) -> BoxBody<Bytes, hyper::Error> {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}
