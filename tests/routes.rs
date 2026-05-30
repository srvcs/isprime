use axum::body::Body;
use axum::extract::Json as AxumJson;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router as AxumRouter};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use srvcs_isprime::{api::Deps, health, router, telemetry};
use tower::ServiceExt;

/// Mock `srvcs-isdivisibleby` that genuinely COMPUTES `a % b == 0` from the
/// request body. This is required to exercise the trial-division loop: a fixed
/// response could not distinguish prime from composite.
async fn spawn_computing_mock() -> String {
    let app = AxumRouter::new().route(
        "/",
        post(|AxumJson(body): AxumJson<Value>| async move {
            let a = body.get("a").and_then(Value::as_i64).unwrap_or(0);
            let b = body.get("b").and_then(Value::as_i64).unwrap_or(1);
            let result = b != 0 && a % b == 0;
            Json(json!({ "result": result }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// Mock answering with a fixed status + body (for error-path tests).
async fn spawn_mock(status: StatusCode, body: Value) -> String {
    let app = AxumRouter::new().route(
        "/",
        post(move || {
            let body = body.clone();
            async move { (status, Json(body)) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn app(isdivisibleby_url: &str) -> axum::Router {
    router(
        telemetry::metrics_handle_for_tests(),
        Deps {
            isdivisibleby_url: isdivisibleby_url.to_string(),
        },
    )
}

async fn eval(isdivisibleby_url: &str, value: Value) -> (StatusCode, Value) {
    let res = app(isdivisibleby_url)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "value": value }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

const DEAD_URL: &str = "http://127.0.0.1:1";

async fn status_of(uri: &str) -> StatusCode {
    app(DEAD_URL)
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn healthz_ok() {
    assert_eq!(status_of("/healthz").await, StatusCode::OK);
}

#[tokio::test]
async fn readyz_reflects_state() {
    health::set_ready(true);
    assert_eq!(status_of("/readyz").await, StatusCode::OK);
}

#[tokio::test]
async fn openapi_ok() {
    assert_eq!(status_of("/openapi.json").await, StatusCode::OK);
}

// --- Spec's asserted correctness cases, against the computing mock. ---

#[tokio::test]
async fn seven_is_prime() {
    let dep = spawn_computing_mock().await;
    let (status, body) = eval(&dep, json!(7)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], true);
    assert_eq!(body["value"], 7);
}

#[tokio::test]
async fn nine_is_not_prime() {
    let dep = spawn_computing_mock().await;
    let (status, body) = eval(&dep, json!(9)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], false);
}

#[tokio::test]
async fn two_is_prime() {
    let dep = spawn_computing_mock().await;
    let (status, body) = eval(&dep, json!(2)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], true);
}

#[tokio::test]
async fn one_is_not_prime() {
    let dep = spawn_computing_mock().await;
    let (status, body) = eval(&dep, json!(1)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], false);
}

#[tokio::test]
async fn zero_is_not_prime() {
    let dep = spawn_computing_mock().await;
    let (status, body) = eval(&dep, json!(0)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], false);
}

#[tokio::test]
async fn negative_is_not_prime() {
    let dep = spawn_computing_mock().await;
    let (status, body) = eval(&dep, json!(-5)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], false);
}

// --- Error / edge cases. ---

#[tokio::test]
async fn non_integer_value_is_500() {
    // A value below 2 short-circuits; use a fractional value > 2 to force the
    // integer-coercion failure rather than the n<2 short circuit.
    let dep = spawn_computing_mock().await;
    let (status, _) = eval(&dep, json!(7.5)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn forwards_422_from_dependency() {
    let dep = spawn_mock(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({ "error": "value is not an integer" }),
    )
    .await;
    // n >= 2 so the loop runs and hits the dependency.
    let (status, _) = eval(&dep, json!(7)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn degrades_when_dependency_unreachable() {
    // n >= 2 forces a dependency call; pointing at a dead port yields 503.
    let (status, body) = eval(DEAD_URL, json!(7)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["dependency"], "srvcs-isdivisibleby");
}

#[tokio::test]
async fn small_n_short_circuits_without_calls() {
    // n < 2 must answer false even with a dead dependency (no calls made).
    let (status, body) = eval(DEAD_URL, json!(1)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], false);
}
