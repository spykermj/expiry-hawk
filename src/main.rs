use prometheus::{Counter, Encoder, Gauge, HistogramVec, TextEncoder, IntGaugeVec};
use prometheus::{labels, opts, register_counter, register_gauge, register_histogram_vec, register_int_gauge_vec};

use lazy_static::lazy_static;

use hyper::{
    header::CONTENT_TYPE,
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server,
};

use chrono::DateTime;

use futures::{try_join, Stream, StreamExt, TryStreamExt};
use kube::{
    api::{Api, ApiResource, DynamicObject, GroupVersionKind, Resource, ResourceExt},
    runtime::{metadata_watcher, watcher, watcher::Event, WatchStreamExt},
};

use tracing::*;

lazy_static! {
    static ref SECRET_ROTATION_TIME: IntGaugeVec =
        register_int_gauge_vec!("expiry_hawk_secret_rotation_time", "The time this secret can be rotated", &["namespace", "name", "kind"]).unwrap();


    static ref SECRET_EXPIRY_TIME: IntGaugeVec =
        register_int_gauge_vec!("expiry_hawk_secret_expiry_time", "The time this secret expires", &["namespace", "name", "kind"]).unwrap();

    static ref HTTP_COUNTER: Counter = register_counter!(opts!(
        "expiry_hawk_http_requests_total",
        "Number of HTTP requests made.",
        labels! {"handler" => "metrics",}
    ))
    .unwrap();
    static ref HTTP_BODY_GAUGE: Gauge = register_gauge!(opts!(
        "expiry_hawk_http_response_size_bytes",
        "The HTTP response sizes in bytes.",
        labels! {"handler" => "metrics",}
    ))
    .unwrap();

    static ref HTTP_REQ_HISTOGRAM: HistogramVec = register_histogram_vec!(
        "expiry_hawk_http_request_duration_seconds",
        "The HTTP request latencies in seconds.",
        &["handler"]
    )
    .unwrap();
}

async fn serve_req(_req: Request<Body>) -> Result<Response<Body>, hyper::Error> {
    let encoder = TextEncoder::new();

    HTTP_COUNTER.inc();
    let timer = HTTP_REQ_HISTOGRAM.with_label_values(&["metrics"]).start_timer();

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

enum Kind {
   Deployment,
   StatefulSet,
   DaemonSet,
}

fn get_rotation_annotation() -> String {
    "spykerman.co.uk/secret-rotation-time".to_string()
}

fn get_expiry_annotation() -> String {
    "spykerman.co.uk/secret-expiry-time".to_string()
}

async fn serve_metrics() -> anyhow::Result<()>{
    let addr = ([0, 0, 0, 0], 9898).into();
    info!("Metrics server listening on http://{}", addr);

    let serve_future = Server::bind(&addr).serve(make_service_fn(|_| async {
        Ok::<_, hyper::Error>(service_fn(serve_req))
    }));

    if let Err(err) = serve_future.await {
        eprintln!("server error: {}", err);
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    init_tracing();
    if let Err(err) = scan_and_serve().await {
        error!("error running expiry-hawk: {}", err)
    }
}

fn init_tracing() {
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
    .with_max_level(Level::INFO)
    .finish();
    tracing::subscriber::set_global_default(subscriber)
    .expect("setting default subscriber failed");
}

async fn scan_and_serve() -> anyhow::Result<((),(),(),())> {
    let serve_fut = serve_metrics();
    let watch_statefulset = watch_metadata(Kind::StatefulSet);
    let watch_deployment = watch_metadata(Kind::Deployment);
    let watch_daemonset = watch_metadata(Kind::DaemonSet);
    try_join!(serve_fut, watch_deployment, watch_statefulset, watch_daemonset)
}

async fn watch_metadata(resource: Kind) -> anyhow::Result<()> {
    let kind = match resource {
        Kind::DaemonSet => "DaemonSet",
        Kind::Deployment => "Deployment",
        Kind::StatefulSet => "StatefulSet",
    };
    let group = "apps";
    let version = "v1";
    let client = kube::Client::try_default().await?;

    // Turn them into a GVK
    let gvk = GroupVersionKind::gvk(group, version, kind);
    // Use API discovery to identify more information about the type (like its plural)
    let (ar, _caps) = kube::discovery::pinned_kind(&client, &gvk).await?;

    // Use the full resource info to create an Api with the ApiResource as its DynamicType
    let api = Api::<DynamicObject>::all_with(client, &ar);
    let wc = watcher::Config::default();

    // Start a metadata or a full resource watch
    handle_events(metadata_watcher(api, wc), &ar).await
}

async fn handle_events<
    K: Resource<DynamicType = ApiResource> + Clone + Send + 'static,
>(
    stream: impl Stream<Item = watcher::Result<Event<K>>> + Send + 'static,
    ar: &ApiResource,
) -> anyhow::Result<()> {
    let mut items = stream.applied_objects().boxed();
    while let Some(resource) = items.try_next().await? {
        if let Some(ns) = resource.namespace() {
            if resource.annotations().contains_key(&get_rotation_annotation()) {
                info!("parsing rotation time for {} {} in {ns}", K::kind(ar), resource.name_any());
                let rfc3339 = resource.annotations().get(&get_rotation_annotation()).unwrap();
                if let Ok(timestamp) = DateTime::parse_from_rfc3339(rfc3339) {
                    SECRET_ROTATION_TIME.with_label_values(&[&ns, &resource.name_any(), &K::kind(ar)]).set(timestamp.timestamp_millis())
                } else {
                  error!("{} {} in {ns} failed to parse as rfc3339: {}", K::kind(ar), &resource.name_any(), rfc3339)
                }
            }
            if resource.annotations().contains_key(&get_expiry_annotation()) {
                info!("parsing expiry time for {} {} in {ns}", K::kind(ar), resource.name_any());
                let rfc3339 = resource.annotations().get(&get_expiry_annotation()).unwrap();
                if let Ok(timestamp) = DateTime::parse_from_rfc3339(rfc3339) {
                    SECRET_EXPIRY_TIME.with_label_values(&[&ns, &resource.name_any(), &K::kind(ar)]).set(timestamp.timestamp_millis())
                } else {
                  error!("{} {} in {ns} failed to parse as rfc3339: {}", K::kind(ar), &resource.name_any(), rfc3339)
                }
            }
        }
    }
    Ok(())
}
