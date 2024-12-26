use log::info;
use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use toml::Table;

use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use http_body_util::Empty;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::StatusCode;

use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

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

    // We start a loop to continuously accept incoming connections
    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();

        // Use an adapter to access something implementing `tokio::io` traits as if they implement
        // `hyper::rt` IO traits.
        let io = TokioIo::new(stream);

        // Spawn a tokio task to serve multiple connections concurrently
        tokio::task::spawn(async move {
            let service = service_fn(move |req| {
                let state = state.clone();
                proxy_handler(state, req)
            });
            // Finally, we bind the incoming connection to our `hello` service
            if let Err(err) = http1::Builder::new()
                // `service_fn` converts our function in a `Service`
                .serve_connection(io, service)
                .await
            {
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
// fn full<T: Into<Bytes>>(chunk: T) -> BoxBody<Bytes, hyper::Error> {
//     Full::new(chunk.into())
//         .map_err(|never| match never {})
//         .boxed()
// }
