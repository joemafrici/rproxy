use std::convert::Infallible;
use std::net::SocketAddr;

use log::info;

use http_body_util::{Full, Empty, combinators::BoxBody, BodyExt};
use hyper::body::{Body, Bytes};
use hyper::client;
use hyper::server::conn::http1;

use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use hyper::body::Frame;
use hyper::{Method, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use tokio::net::{TcpListener, TcpStream};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    env_logger::init();
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));

    // We create a TcpListener and bind it to 127.0.0.1:3000
    let listener = TcpListener::bind(addr).await?;
    info!("Listening on http://{}", addr);

    // We start a loop to continuously accept incoming connections
    loop {
        let (stream, _) = listener.accept().await?;

        // Use an adapter to access something implementing `tokio::io` traits as if they implement
        // `hyper::rt` IO traits.
        let io = TokioIo::new(stream);

        // Spawn a tokio task to serve multiple connections concurrently
        tokio::task::spawn(async move {
            // Finally, we bind the incoming connection to our `hello` service
            if let Err(err) = http1::Builder::new()
                // `service_fn` converts our function in a `Service`
                .serve_connection(io, service_fn(proxy_handler))
                .await
            {
                eprintln!("Error serving connection: {:?}", err);
            }
        });
    }
}

async fn proxy_handler(req: Request<hyper::body::Incoming>) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    info!("Incoming request from {:?}", req.uri());
    let client = Client::builder(hyper_util::rt::TokioExecutor::new()).build(HttpConnector::new());
    let path_and_query = req.uri().path_and_query().map(|pq| pq.as_str()).unwrap_or("");
    // TODO: decide port number based on request
    let uri = format!("http://localhost:3001{}", path_and_query);
    let mut proxy_req = Request::builder()
        .method(req.method())
        .uri(uri);

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

async fn hello(_: Request<hyper::body::Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    Ok(Response::new(Full::new(Bytes::from("Hello, World!"))))
}

async fn echo(req: Request<hyper::body::Incoming>) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/") => Ok(Response::new(full("Try POSTing data to /echo"))),
        (&Method::POST, "/echo") => Ok(Response::new(req.into_body().boxed())),
        (&Method::POST, "/echo/uppercase") => {
            let frame_stream = req.into_body().map_frame(|frame| {
              let frame = if let Ok(data) = frame.into_data() {
                  data.iter()
                      .map(|byte| byte.to_ascii_uppercase())
                      .collect::<Bytes>()
              } else {
                  Bytes::new()
              };

              Frame::data(frame)
            });
            Ok(Response::new(frame_stream.boxed()))
        },
        (&Method::POST, "/echo/reverse") => {
            let upper = req.body().size_hint().upper().unwrap_or(u64::MAX);
            if upper > 1024 * 64 {
                let mut resp = Response::new(full("Body too big"));
                *resp.status_mut() = StatusCode::PAYLOAD_TOO_LARGE;
                return Ok(resp);
            }
            let whole_body = req.collect().await?.to_bytes();

            let reversed_body = whole_body.iter()
                .rev()
                .cloned()
                .collect::<Vec<u8>>();
            Ok(Response::new(full(reversed_body)))
        },
        _ => {
            let mut not_found = Response::new(empty());
            *not_found.status_mut() = StatusCode::NOT_FOUND;
            Ok(not_found)
        }
    }
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