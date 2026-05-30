# srvcs-isprime

The primality test of the srvcs.cloud distributed standard library.

Its single concern: **is the number prime?** It does no arithmetic of its own.
It runs a trial-division loop and delegates each "is `n` divisible by `d`?"
question to [`srvcs-isdivisibleby`](https://github.com/srvcs/isdivisibleby) over
HTTP.

Given `n`:

- if `n < 2`, the answer is `false` immediately (no dependency calls);
- otherwise, for each divisor `d` in `2..n`, ask `srvcs-isdivisibleby` whether
  `n` is divisible by `d`. The first divisor that divides `n` proves `n`
  composite and stops the loop (`false`).
- if no divisor divides `n`, it is prime (`true`).

## API

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/` | Service identity, concern, and dependency list |
| `POST` | `/` | Is `value` prime? |
| `GET` | `/healthz` `/readyz` `/metrics` `/openapi.json` | srvcs service standard surface |

```sh
curl -s -X POST localhost:8080/ -H 'content-type: application/json' -d '{"value": 7}'
# {"value":7,"result":true}
```

Responses:

- `200 {"value": n, "result": true | false}` — evaluated.
- `422` — invalid input, forwarded from the dependency.
- `500` — `value` is not an integer.
- `503` — the dependency is unavailable.

## Dependencies

- [`srvcs-isdivisibleby`](https://github.com/srvcs/isdivisibleby)

A single request fans out across the dependency graph once per candidate
divisor: `isprime → isdivisibleby` (up to `n - 2` times).

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `SRVCS_BIND_ADDR` | `0.0.0.0:8080` | Bind address |
| `SRVCS_ISDIVISIBLEBY_URL` | `http://127.0.0.1:8084` | Base URL of `srvcs-isdivisibleby` |
| `SRVCS_ENV` | `development` | Environment label for logs |
| `RUST_LOG` | `info,tower_http=info` | Tracing filter |

## Local checks

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Orchestration tests stand up a mock `srvcs-isdivisibleby` that genuinely
computes `a % b == 0`, so the trial-division loop is exercised end to end. See
[`srvcs/platform`](https://github.com/srvcs/platform) for the shared standard.

> Note: the `cargoHash` in `flake.nix` is inherited from the template and must be
> refreshed with a `nix build` before the Nix gates pass.
