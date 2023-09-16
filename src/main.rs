use futures::{pin_mut, TryStreamExt, try_join};
use k8s_openapi::api::apps::v1::Deployment;
use kube::{
    api::{Api, ResourceExt},
    runtime::{watcher, WatchStreamExt},
    Client,
};

use chrono::DateTime;

use prometheus::{Counter, Encoder, Gauge, HistogramVec, TextEncoder, IntGaugeVec};

use lazy_static::lazy_static;
use prometheus::{labels, opts, register_counter, register_gauge, register_histogram_vec, register_int_gauge_vec};

use hyper::{
    header::CONTENT_TYPE,
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server,
};

lazy_static! {
    static ref SECRET_ROTATION_TIME: IntGaugeVec =
        register_int_gauge_vec!("expiry_hawk_secret_rotation_time", "The time this secret can be rotated", &["namespace", "name", "kind"]).unwrap();


    static ref SECRET_EXPIRY_TIME: IntGaugeVec =
        register_int_gauge_vec!("expiry_hawk_secret_expiry_time", "The time this secret expires", &["namespace", "name", "kind"]).unwrap();

    static ref HTTP_COUNTER: Counter = register_counter!(opts!(
        "example_http_requests_total",
        "Number of HTTP requests made.",
        labels! {"handler" => "all",}
    ))
    .unwrap();
    static ref HTTP_BODY_GAUGE: Gauge = register_gauge!(opts!(
        "example_http_response_size_bytes",
        "The HTTP response sizes in bytes.",
        labels! {"handler" => "all",}
    ))
    .unwrap();

    static ref HTTP_REQ_HISTOGRAM: HistogramVec = register_histogram_vec!(
        "example_http_request_duration_seconds",
        "The HTTP request latencies in seconds.",
        &["handler"]
    )
    .unwrap();
}

async fn serve_req(_req: Request<Body>) -> Result<Response<Body>, hyper::Error> {
    let encoder = TextEncoder::new();

    HTTP_COUNTER.inc();
    let timer = HTTP_REQ_HISTOGRAM.with_label_values(&["all"]).start_timer();

    let metric_families = prometheus::gather();
    let mut buffer = vec![];
    encoder.encode(&metric_families, &mut buffer).unwrap();
    HTTP_BODY_GAUGE.set(buffer.len() as f64);

    let response = Response::builder()
        .status(200)
        .header(CONTENT_TYPE, encoder.format_type())
        .body(Body::from(buffer))
        .unwrap();

    timer.observe_duration();

    Ok(response)
}

async fn scan_deployments() -> anyhow::Result<()> {
    let client = Client::try_default().await?;
    let api = Api::<Deployment>::all(client);
    let stream = watcher(api, watcher::Config::default()).applied_objects();
    pin_mut!(stream);
    while let Some(event) = stream.try_next().await? {
        let rotation_time_annotation = "spykerman.co.uk/secret-rotation-time";
        let expiry_time_annotation = "spykerman.co.uk/secret-expiry-time";
        if event.annotations().contains_key(rotation_time_annotation) {
            let namespace = event.namespace().unwrap();
            let timestamp = DateTime::parse_from_rfc3339(event.annotations().get(rotation_time_annotation).unwrap()).unwrap();
            SECRET_ROTATION_TIME.with_label_values(&[&namespace, &event.name_any(), "deployment"]).set(timestamp.timestamp_millis())
        }
        if event.annotations().contains_key(expiry_time_annotation) {
            let namespace = event.namespace().unwrap();
            let timestamp = DateTime::parse_from_rfc3339(event.annotations().get(expiry_time_annotation).unwrap()).unwrap();
            SECRET_EXPIRY_TIME.with_label_values(&[&namespace, &event.name_any(), "deployment"]).set(timestamp.timestamp_millis())
        }
    }
    Ok(())
}

async fn serve_metrics() -> anyhow::Result<()>{
    let addr = ([0, 0, 0, 0], 9898).into();
    println!("Listening on http://{}", addr);

    let serve_future = Server::bind(&addr).serve(make_service_fn(|_| async {
        Ok::<_, hyper::Error>(service_fn(serve_req))
    }));

    if let Err(err) = serve_future.await {
        eprintln!("server error: {}", err);
    }
    Ok(())
}

async fn scan_and_serve() -> anyhow::Result<((),())> {
    let scan_fut = scan_deployments();
    let serve_fut = serve_metrics();
    try_join!(scan_fut, serve_fut)
}

#[tokio::main]
async fn main() {
    if let Err(err) = scan_and_serve().await {
        eprintln!("error running expiry-hawk: {}", err)
    }
}
