use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::{OpenApi, ToSchema};

use crate::client::{self, DepError};

pub const SERVICE: &str = "srvcs-isprime";
pub const CONCERN: &str = "number theory: primality test";
pub const DEPENDS_ON: &[&str] = &["srvcs-isdivisibleby"];

/// Dependency endpoints, injected as router state so tests can point them at
/// mock services.
#[derive(Clone)]
pub struct Deps {
    pub isdivisibleby_url: String,
}

#[derive(Serialize, ToSchema)]
pub struct Info {
    pub service: &'static str,
    pub concern: &'static str,
    pub depends_on: Vec<&'static str>,
}

/// `GET /` — service identity (srvcs service standard).
#[utoipa::path(get, path = "/", responses((status = 200, body = Info)))]
pub async fn index() -> Json<Info> {
    Json(Info {
        service: SERVICE,
        concern: CONCERN,
        depends_on: DEPENDS_ON.to_vec(),
    })
}

#[derive(Deserialize, ToSchema)]
pub struct EvalRequest {
    #[schema(value_type = Object)]
    pub value: Value,
}

#[derive(Serialize, ToSchema)]
pub struct PrimeResponse {
    #[schema(value_type = Object)]
    pub value: Value,
    /// Whether `value` is prime.
    pub result: bool,
}

fn ok(value: Value, result: bool) -> Response {
    (
        StatusCode::OK,
        Json(json!({ "value": value, "result": result })),
    )
        .into_response()
}

fn degraded(dependency: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "dependency unavailable", "dependency": dependency })),
    )
        .into_response()
}

/// Forward a dependency's response verbatim (used to propagate `422` for invalid
/// input, so isprime reports the same rejection a leaf dependency did).
fn forward(status: u16, body: Value) -> Response {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    (code, Json(body)).into_response()
}

/// Ask `srvcs-isdivisibleby` whether `n` is divisible by `d`, mapping its
/// failures to the response this service should return.
async fn ask_divisible(url: &str, n: i64, d: i64) -> Result<bool, Response> {
    let payload = json!({ "a": n, "b": d });
    match client::call(url, &payload).await {
        Err(DepError::Unreachable) => Err(degraded("srvcs-isdivisibleby")),
        Ok((200, body)) => Ok(body.get("result").and_then(Value::as_bool).unwrap_or(false)),
        // Invalid input propagates from the leaf dependency; forward it.
        Ok((422, body)) => Err(forward(422, body)),
        Ok(_) => Err(degraded("srvcs-isdivisibleby")),
    }
}

/// `POST /` — is `value` prime?
///
/// This service does no arithmetic of its own. It runs a trial-division loop,
/// delegating each "is `n` divisible by `d`" question to `srvcs-isdivisibleby`
/// over HTTP. If `n < 2` it is not prime and we answer immediately without any
/// dependency calls. Otherwise we test divisors `d` in `2..n`: the first divisor
/// that divides `n` proves `n` composite and stops the loop. If none divides,
/// `n` is prime.
///
/// If the dependency is unreachable mid-loop, isprime reports itself degraded
/// rather than guessing.
#[utoipa::path(
    post,
    path = "/",
    request_body = EvalRequest,
    responses(
        (status = 200, body = PrimeResponse),
        (status = 422, description = "value is rejected by the dependency (forwarded)"),
        (status = 500, description = "value is not an integer"),
        (status = 503, description = "a dependency is unavailable")
    )
)]
pub async fn evaluate(State(deps): State<Deps>, Json(req): Json<EvalRequest>) -> Response {
    let n = match req.value.as_i64() {
        Some(n) => n,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "value is not an integer" })),
            )
                .into_response();
        }
    };

    // n < 2 is never prime; answer without any dependency calls.
    if n < 2 {
        return ok(req.value, false);
    }

    // Trial division: the first divisor in 2..n proves n composite.
    for d in 2..n {
        match ask_divisible(&deps.isdivisibleby_url, n, d).await {
            Ok(true) => return ok(req.value, false),
            Ok(false) => {}
            Err(resp) => return resp,
        }
    }

    ok(req.value, true)
}

#[derive(OpenApi)]
#[openapi(
    paths(index, evaluate),
    components(schemas(Info, EvalRequest, PrimeResponse))
)]
pub struct ApiDoc;

/// Serve OpenAPI document
pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_documents_routes() {
        let doc = ApiDoc::openapi();
        let root = doc.paths.paths.get("/").expect("path / present");
        assert!(root.get.is_some());
        assert!(root.post.is_some());
    }

    #[tokio::test]
    async fn index_reports_dependency() {
        let Json(info) = index().await;
        assert_eq!(info.service, "srvcs-isprime");
        assert_eq!(info.depends_on, vec!["srvcs-isdivisibleby"]);
    }
}
