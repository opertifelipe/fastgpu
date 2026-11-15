use crate::{
    api::*,
    cli::{Cli, Commands},
    config::*,
    engines::*,
    info::public_pod_details,
    lifecycle::capacity_rejected,
    models::*,
    process,
    ssh::*,
    worker::*,
};
use anyhow::Result;
use clap::{CommandFactory, Parser};
use reqwest::{Client, Method};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

struct Request {
    method: String,
    path: String,
    headers: String,
    body: Value,
}
struct Response {
    status: u16,
    headers: String,
    body: Vec<u8>,
}
fn response(status: u16, value: Value) -> Response {
    Response {
        status,
        headers: String::new(),
        body: serde_json::to_vec(&value).unwrap(),
    }
}
fn ok(value: Value) -> Response {
    response(200, value)
}
struct Server {
    url: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn server(handler: impl FnMut(Request) -> Response + Send + 'static) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handler = Arc::new(Mutex::new(handler));
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let handler = handler.clone();
            tokio::spawn(async move {
                let mut data = Vec::new();
                let mut buf = [0; 4096];
                let end = loop {
                    let n = stream.read(&mut buf).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    data.extend_from_slice(&buf[..n]);
                    if let Some(n) = data.windows(4).position(|b| b == b"\r\n\r\n") {
                        break n + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&data[..end]).into_owned();
                let length = headers
                    .lines()
                    .filter_map(|l| l.split_once(':'))
                    .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                    .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                    .unwrap_or(0);
                while data.len() < end + length {
                    let n = stream.read(&mut buf).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buf[..n]);
                }
                let mut words = headers.split_whitespace();
                let request = Request {
                    method: words.next().unwrap().into(),
                    path: words.next().unwrap().into(),
                    headers,
                    body: serde_json::from_slice(&data[end..]).unwrap_or(Value::Null),
                };
                let response = handler.lock().unwrap()(request);
                let head = format!(
                    "HTTP/1.1 {} Test\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n",
                    response.status,
                    response.body.len(),
                    response.headers
                );
                stream.write_all(head.as_bytes()).await.unwrap();
                stream.write_all(&response.body).await.unwrap();
            });
        }
    });
    Server { url, task }
}
fn api(server: &Server) -> Api {
    Api {
        base: server.url.clone(),
        key: "test-secret".into(),
        http: Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
    }
}
fn app(dir: &Path, server: &Server) -> App {
    let mut app = App::load(dir).unwrap();
    app.api = Some(api(server));
    app
}
fn fixture_keys(app: &App) {
    fs::write(&app.config.ssh_key, "test-key").unwrap();
    fs::write(
        format!("{}.pub", app.config.ssh_key),
        "ssh-ed25519 test-public-key",
    )
    .unwrap();
}
fn gpu(id: &str, price: f64, memory: i64, dc: &str) -> Gpu {
    Gpu {
        id: id.into(),
        manufacturer: "NVIDIA".into(),
        secure: true,
        memory,
        price: Price { secure: price },
        data_centers: vec![DataCenter {
            id: dc.into(),
            availability: "LOW".into(),
        }],
        ..Gpu::default()
    }
}
fn catalog() -> Value {
    json!({"gpus":[gpu("small",0.4,24,"TEST"),gpu("medium",3.0,48,"TEST"),gpu("medium-fallback",2.5,48,"TEST"),gpu("large",4.0,96,"TEST")]})
}
fn volume_list() -> Value {
    json!({"networkVolumes":[{"id":"v","name":"volume-test","size":50,"dataCenter":"TEST"}]})
}
fn deployment() -> Deployment {
    Deployment {
        model: "my-model".into(),
        engine: "llama.cpp".into(),
        target: "/workspace/models/custom.gguf".into(),
        context: 4096,
        ..Deployment::default()
    }
}
async fn dispatch(app: &mut App, args: &[&str]) -> Result<()> {
    let cli = Cli::try_parse_from(std::iter::once("fastgpu").chain(args.iter().copied()))?;
    app.dispatch(cli.command.unwrap_or_else(|| {
        Cli::try_parse_from(["fastgpu", "up"])
            .unwrap()
            .command
            .unwrap()
    }))
    .await
}

#[test]
fn cli_definitions_and_help() {
    let mut root = Cli::command();
    root.clone().debug_assert();
    fn visit(cmd: &mut clap::Command, path: Vec<String>) {
        assert!(!cmd.render_help().to_string().is_empty());
        let mut args = path.clone();
        args.push("--help".into());
        let err = Cli::try_parse_from(&args).err().unwrap();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp, "{args:?}");
        for child in cmd.get_subcommands_mut().filter(|c| c.get_name() != "help") {
            let mut next = path.clone();
            next.push(child.get_name().into());
            visit(child, next);
        }
    }
    visit(&mut root, vec!["fastgpu".into()]);
    assert!(Cli::try_parse_from(["fastgpu", "volume", "list"]).is_ok());
    assert!(
        Cli::try_parse_from(["fastgpu", "help", "deploy"])
            .is_err_and(|e| e.kind() == clap::error::ErrorKind::DisplayHelp)
    );
    assert!(!root.render_help().to_string().contains("worker"));
}
#[test]
fn cli_rejects_extra_arguments_and_conflicting_sizes() {
    let sizes = ["--small", "--medium", "--large", "--xlarge"];
    for (i, a) in sizes.iter().enumerate() {
        for b in &sizes[i + 1..] {
            assert!(Cli::try_parse_from(["fastgpu", "up", a, b]).is_err());
        }
    }
    for args in [
        vec!["status", "extra"],
        vec!["code", "one", "two"],
        vec!["worker", "stop", "extra"],
        vec!["model", "pull", "one", "two"],
        vec!["up", "--bogus"],
    ] {
        assert!(Cli::try_parse_from(std::iter::once("fastgpu").chain(args)).is_err());
    }
}
#[test]
fn shell_keeps_remote_flags_and_deploy_accepts_repeated_args() {
    let cli = Cli::try_parse_from([
        "fastgpu",
        "shell",
        "--cwd",
        "vol:project",
        "python",
        "-c",
        "print(1)",
        "--help",
    ])
    .unwrap();
    match cli.command.unwrap() {
        Commands::Shell { cwd, command, .. } => {
            assert_eq!(cwd, "vol:project");
            assert_eq!(command, ["python", "-c", "print(1)", "--help"]);
        }
        _ => panic!(),
    }
    assert!(Cli::try_parse_from(["fastgpu", "deploy", "--arg=--threads", "--arg=4"]).is_ok());
}
#[test]
fn volume_identity() {
    let volumes: Vec<Volume> =
        serde_json::from_value(json!([{"id":"a","name":"same"},{"id":"b","name":"same"}])).unwrap();
    assert!(resolve_volume(&volumes, "same").is_err());
    assert_eq!(resolve_volume(&volumes, "b").unwrap().id, "b");
    assert!(resolve_volume(&volumes, "absent").is_err());
}
#[test]
fn cheapest_compatible_gpu_and_architecture() {
    let input = vec![
        gpu("cheap-small", 0.1, 16, "TEST"),
        gpu("wrong-dc", 0.2, 48, "OTHER"),
        gpu("expensive", 0.8, 24, "TEST"),
        gpu("winner", 0.4, 48, "TEST"),
        gpu("Tesla V100", 0.1, 32, "TEST"),
        gpu("RTX 6000", 0.2, 48, "TEST"),
    ];
    let offers = eligible(input, "TEST", 24, 0.6);
    assert_eq!(offers.len(), 1);
    assert_eq!(offers[0].id, "winner");
}
#[test]
fn gpu_size_boundaries_and_medium_order() {
    let prices = [
        6.0, 3.25, 0.4, 3.0, 5.0, 1.49, 4.0, 5.01, 3.5, 2.75, 1.5, 3.49,
    ];
    for (size, expected) in [
        ("small", vec![0.4, 1.49]),
        ("medium", vec![3.0, 2.75, 3.25, 3.49, 1.5]),
        ("large", vec![3.5, 4.0, 5.0]),
        ("xlarge", vec![5.01, 6.0]),
    ] {
        let input = prices
            .iter()
            .map(|p| gpu(&p.to_string(), *p, 24, "TEST"))
            .collect();
        let offers = size_offers(eligible(input, "TEST", 24, 0.0), size).unwrap();
        assert_eq!(
            offers.iter().map(|g| g.price.secure).collect::<Vec<_>>(),
            expected
        );
    }
    assert!(size_offers(vec![], "invalid").is_err());
}
#[tokio::test]
async fn pagination_and_no_retry_on_create() {
    let posts = Arc::new(Mutex::new(0));
    let count = posts.clone();
    let server = server(move |r| {
        assert!(r.headers.contains("Bearer test-secret"));
        if r.method == "POST" { *count.lock().unwrap() += 1; return response(503,json!({"detail":"test-secret unavailable"})); }
        if r.path.contains("cursor=next") { ok(json!({"networkVolumes":[{"id":"b"}]})) } else { ok(json!({"networkVolumes":[{"id":"a"}],"pagination":{"hasNextPage":true,"nextCursor":"next"}})) }
    }).await;
    let api = api(&server);
    assert_eq!(api.volumes().await.unwrap().len(), 2);
    let error = api
        .request(Method::POST, "/pods", Some(&json!({"name":"test"})))
        .await
        .unwrap_err();
    assert_eq!(*posts.lock().unwrap(), 1);
    assert!(!error.to_string().contains("test-secret"));
    assert!(is_status(&error, 503));
}
#[tokio::test]
async fn pagination_loop_is_rejected() {
    let server =
        server(|_| ok(json!({"pods":[],"pagination":{"hasNextPage":true,"nextCursor":"same"}})))
            .await;
    assert!(
        api(&server)
            .pods()
            .await
            .unwrap_err()
            .to_string()
            .contains("pagination")
    );
}
#[tokio::test]
async fn transient_get_retries_but_redirect_does_not() {
    let n = Arc::new(Mutex::new(0));
    let count = n.clone();
    let server = server(move |_| {
        let mut n = count.lock().unwrap();
        *n += 1;
        if *n < 3 {
            response(503, json!({}))
        } else {
            ok(json!({"pods":[]}))
        }
    })
    .await;
    assert!(api(&server).pods().await.is_ok());
    assert_eq!(*n.lock().unwrap(), 3);
    let redirect = super::tests::server(|_| Response {
        status: 302,
        headers: "Location: http://127.0.0.1:1/\r\n".into(),
        body: vec![],
    })
    .await;
    assert!(is_status(&api(&redirect).pods().await.unwrap_err(), 302));
}
#[tokio::test]
async fn sse_logs() {
    let server = server(|_| Response {
        status: 200,
        headers: "Content-Type: text/event-stream\r\n".into(),
        body: b"id: 1\ndata: {\"source\":\"container\",\"line\":\"ready\"}\n\n".to_vec(),
    })
    .await;
    let mut out = Vec::new();
    api(&server).logs("p", &mut out).await.unwrap();
    assert_eq!(out, b"[container] ready\n");
}
#[test]
fn profile_defaults_legacy_json_and_permissions() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"image":"previous-image","volume":"v","local_port":9090}"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("state.json"),
        r#"{"pod_id":"p","volume_id":"v","managed":true,"pending_name":"","tunnel_port":8080}"#,
    )
    .unwrap();
    let app = App::load(dir.path()).unwrap();
    assert_eq!(app.config.min_vram_gb, 24);
    assert_eq!(app.config.local_port, 9090);
    assert_eq!(app.state.pod_id, "p");
    assert_eq!(app.config.image, "previous-image");
    app.save_config().unwrap();
    app.save_state().unwrap();
    for name in ["config.json", "state.json"] {
        assert_eq!(
            fs::metadata(dir.path().join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let value: Value =
        serde_json::from_slice(&fs::read(dir.path().join("config.json")).unwrap()).unwrap();
    assert!(value.get("api_key").is_none());
}
#[test]
fn profile_lock_and_atomic_json() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = App::load(dir.path()).unwrap();
    let mut b = App::load(dir.path()).unwrap();
    a.acquire().unwrap();
    a.acquire().unwrap();
    assert!(b.acquire().is_err());
    drop(a);
    b.acquire().unwrap();
    b.save_state().unwrap();
    assert_eq!(
        fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".gpu-"))
            .count(),
        0
    );
}
#[tokio::test]
async fn uncertain_creation_is_reconciled_without_duplicate() {
    let posts = Arc::new(Mutex::new(0));
    let count = posts.clone();
    let mut name = String::new();
    let server = server(move |r| match (r.method.as_str(), r.path.as_str()) {
        ("POST", "/pods") => {
            *count.lock().unwrap() += 1;
            name = r.body["name"].as_str().unwrap().into();
            assert_eq!(r.body["cloud"], "SECURE");
            assert_eq!(r.body["gpu"]["count"], 1);
            assert_eq!(r.body["gpu"]["minRamPerGpu"], 32);
            assert!(r.body["env"].get("RUNPOD_API_KEY").is_none());
            response(503, json!({"detail":"upstream timed out"}))
        }
        (_, "/pods") => ok(if name.is_empty() {
            json!({"pods":[]})
        } else {
            json!({"pods":[{"id":"created","name":name,"status":"RUNNING"}]})
        }),
        (_, "/pods/created") => ok(json!({"id":"created","name":name,"status":"RUNNING"})),
        (_, "/network-volumes") => ok(volume_list()),
        _ if r.path.starts_with("/catalog/gpus?") => ok(catalog()),
        _ => panic!("unexpected {} {}", r.method, r.path),
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path(), &server);
    fixture_keys(&app);
    assert!(app.up("v", "small", false, true).await.is_err());
    assert!(!app.state.pending_name.is_empty());
    let mut app = super::tests::app(dir.path(), &server);
    app.up("", "small", true, true).await.unwrap();
    assert_eq!(app.state.pod_id, "created");
    assert_eq!(*posts.lock().unwrap(), 1);
}
#[tokio::test]
async fn startup_capacity_retries_stay_in_size() {
    for (args, expected) in [
        (vec![], vec!["small"]),
        (vec!["up", "--medium"], vec!["medium", "medium-fallback"]),
    ] {
        let requests = Arc::new(Mutex::new(vec![]));
        let seen = requests.clone();
        let server = server(move |r| match (r.method.as_str(), r.path.as_str()) {
            ("POST", "/pods") => {
                seen.lock()
                    .unwrap()
                    .push(r.body["gpu"]["id"].as_str().unwrap().to_string());
                response(400, json!({"detail":"capacity exhausted"}))
            }
            ("GET", "/pods") => ok(json!({"pods":[]})),
            ("GET", "/network-volumes") => ok(volume_list()),
            _ if r.path.starts_with("/catalog/gpus?") => ok(catalog()),
            _ => panic!("unexpected request"),
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), &server);
        fixture_keys(&app);
        assert!(capacity_rejected(
            &dispatch(&mut app, &args).await.unwrap_err()
        ));
        assert_eq!(*requests.lock().unwrap(), expected);
        assert!(app.state.pending_name.is_empty());
    }
}
#[tokio::test]
async fn startup_dry_run_is_read_only_and_empty_tier_fails() {
    let server = server(|r| {
        assert_eq!(r.method, "GET");
        match r.path.as_str() {
            "/pods" => ok(json!({"pods":[]})),
            "/network-volumes" => ok(volume_list()),
            _ => ok(catalog()),
        }
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path(), &server);
    dispatch(&mut app, &["up", "--dry-run", "--medium"])
        .await
        .unwrap();
    assert!(!Path::new(&app.config.ssh_key).exists());
    assert!(!dir.path().join("state.json").exists());
    assert!(
        dispatch(
            &mut app,
            &["up", "--dry-run", "--xlarge", "--max-price", "5"]
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("no compatible xlarge")
    );
}
#[tokio::test]
async fn startup_blocks_existing_unselected_pods_across_pages() {
    for status in ["RUNNING", "EXITED", "CREATED"] {
        let server = server(move |r| {
            assert_eq!(r.method, "GET");
            assert!(r.path.starts_with("/pods"));
            ok(if r.path.contains("cursor") {
                json!({"pods":[{"id":"p","name":"gpu-existing","status":status}]})
            } else {
                json!({"pods":[],"pagination":{"hasNextPage":true,"nextCursor":"next"}})
            })
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        assert!(
            app(dir.path(), &server)
                .up("", "large", false, true)
                .await
                .unwrap_err()
                .to_string()
                .contains("already exists")
        );
    }
}
#[tokio::test]
async fn startup_fails_closed_when_listing_fails() {
    let server = server(|r| {
        assert_eq!(r.path, "/pods");
        response(403, json!({}))
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    assert!(is_status(
        &app(dir.path(), &server)
            .up("", "small", false, true)
            .await
            .unwrap_err(),
        403
    ));
}
#[tokio::test]
async fn selected_pod_is_reused_on_dry_run_and_volume_change_is_refused() {
    let server = server(|r| {
        assert_eq!(r.method, "GET");
        match r.path.as_str() {
            "/pods/p" => {
                ok(json!({"id":"p","status":"EXITED","mounts":{"network":[{"volumeId":"old"}]}}))
            }
            "/network-volumes" => ok(volume_list()),
            _ => panic!("unexpected request"),
        }
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path(), &server);
    app.state.pod_id = "p".into();
    assert_eq!(app.up("", "xlarge", true, true).await.unwrap().id, "p");
    assert!(
        app.up("v", "small", true, true)
            .await
            .unwrap_err()
            .to_string()
            .contains("another volume")
    );
}
#[tokio::test]
async fn actual_price_cap_terminates_only_created_pod() {
    let deleted = Arc::new(Mutex::new(false));
    let seen = deleted.clone();
    let server = server(move |r| match (r.method.as_str(), r.path.as_str()) {
        ("GET", "/pods") => ok(json!({"pods":[]})),
        ("GET", "/network-volumes") => ok(volume_list()),
        ("POST", "/pods") => ok(json!({"id":"p","cost":2.0})),
        ("DELETE", "/pods/p") => {
            *seen.lock().unwrap() = true;
            response(204, Value::Null)
        }
        _ if r.path.starts_with("/catalog/gpus?") => ok(catalog()),
        _ => panic!("unexpected request"),
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path(), &server);
    fixture_keys(&app);
    app.config.max_price = 1.0;
    assert!(
        app.up("", "small", false, true)
            .await
            .unwrap_err()
            .to_string()
            .contains("volume retained")
    );
    assert!(*deleted.lock().unwrap());
    assert!(app.state.pod_id.is_empty());
}
#[tokio::test]
async fn down_only_deletes_managed_pod_and_verifies_volume() {
    let deleted = Arc::new(Mutex::new(false));
    let seen = deleted.clone();
    let checked = Arc::new(Mutex::new(false));
    let check = checked.clone();
    let server = server(move |r| match (r.method.as_str(), r.path.as_str()) {
        ("DELETE", "/pods/p") => {
            *seen.lock().unwrap() = true;
            response(204, Value::Null)
        }
        (_, "/pods/p") => {
            if *seen.lock().unwrap() {
                response(404, Value::Null)
            } else {
                ok(json!({"id":"p","mounts":{"network":[{"volumeId":"v","path":"/workspace"}]}}))
            }
        }
        (_, "/network-volumes/v") => {
            *check.lock().unwrap() = true;
            ok(json!({"id":"v","name":"volume-test","size":50}))
        }
        _ => panic!("unexpected deletion/request"),
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path(), &server);
    app.state = State {
        pod_id: "p".into(),
        volume_id: "v".into(),
        managed: true,
        ..State::default()
    };
    app.down().await.unwrap();
    assert!(*deleted.lock().unwrap());
    assert!(*checked.lock().unwrap());
    assert!(app.state.pod_id.is_empty());
}
#[tokio::test]
async fn down_rejects_identity_mismatch() {
    let server = server(|r| {
        assert_eq!(r.method, "GET");
        ok(json!({"id":"p","mounts":{"network":[{"volumeId":"other"}]}}))
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path(), &server);
    app.state = State {
        pod_id: "p".into(),
        volume_id: "v".into(),
        managed: true,
        ..State::default()
    };
    assert!(
        app.down()
            .await
            .unwrap_err()
            .to_string()
            .contains("termination refused")
    );
}
#[tokio::test]
async fn volume_deletion_protects_selection_attachment_and_confirmation() {
    for mode in ["profile", "state", "attached", "confirmation"] {
        let server = server(|r| {
            assert_eq!(r.method, "GET");
            if r.path == "/network-volumes" {
                ok(volume_list())
            } else {
                ok(json!({"pods":[{"id":"p","mounts":{"network":[{"volumeId":"v"}]}}]}))
            }
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), &server);
        if mode == "profile" {
            app.config.volume = "v".into();
        }
        if mode == "state" {
            app.state.volume_id = "v".into();
        }
        let confirm = if mode == "confirmation" { "wrong" } else { "v" };
        assert!(
            dispatch(&mut app, &["volumes", "delete", "v", "--confirm", confirm])
                .await
                .is_err()
        );
    }
}
#[tokio::test]
async fn volume_resize_cannot_shrink_and_explicit_create_works() {
    let server = server(|r| {
        if r.method == "GET" {
            return ok(volume_list());
        }
        assert_eq!(r.method, "POST");
        assert_eq!(r.path, "/network-volumes");
        assert_eq!(r.body["size"], 100);
        assert_eq!(r.body["dataCenter"], "TEST");
        ok(json!({"id":"created"}))
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path(), &server);
    assert!(
        dispatch(&mut app, &["volumes", "resize", "v", "49"])
            .await
            .is_err()
    );
    dispatch(
        &mut app,
        &[
            "volumes",
            "create",
            "test",
            "--size",
            "100",
            "--datacenter",
            "TEST",
        ],
    )
    .await
    .unwrap();
}
#[test]
fn capacity_rejection_does_not_retry_ambiguous_errors() {
    for detail in [
        "insufficient capacity",
        "There are no longer any instances available with the requested specifications.",
    ] {
        assert!(capacity_rejected(
            &ApiError {
                status: 400,
                detail: detail.into()
            }
            .into()
        ));
    }
    for (status, detail) in [(503, "capacity unknown"), (400, "invalid image")] {
        assert!(!capacity_rejected(
            &ApiError {
                status,
                detail: detail.into()
            }
            .into()
        ));
    }
    assert!(!capacity_rejected(&anyhow::anyhow!("timeout")));
}
#[test]
fn paths_and_shell_quoting_round_trip() {
    for bad in ["../../etc", "vol:../etc", "/root", "home:a\nb", "vol:\0"] {
        assert!(remote_path(bad).is_err());
    }
    assert_eq!(
        remote_path("home:project with spaces").unwrap(),
        "/home/gpu/project with spaces"
    );
    let args = [
        "spaces here",
        "quote'back",
        "$(printf injected)",
        "semi;colon",
        "",
    ];
    let out = std::process::Command::new("bash")
        .args(["-c", &format!("printf '%s\\n' {}", shell_words(&args))])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        args.join("\n") + "\n"
    );
}
#[test]
fn persistent_paths_reject_symlink_escape() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let file = root.path().join("my model.gguf");
    fs::write(&file, "GGUF").unwrap();
    assert_eq!(persistent_path(root.path(), &file).unwrap(), file);
    let link = root.path().join("escape");
    symlink(outside.path(), &link).unwrap();
    assert!(persistent_path(root.path(), &link).is_err());
    for bad in [
        "/tmp/model.gguf",
        "/workspace/../root/model.gguf",
        "vol:../bad",
        "/workspace-other/file",
    ] {
        assert!(model_path(bad).is_err());
    }
}
#[test]
fn gguf_fingerprint_tracks_changes_and_rejects_invalid_header() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("model.gguf");
    let cache = dir.path().join("checks");
    fs::write(&file, format!("GGUF{}", "a".repeat(64))).unwrap();
    let first = gguf_fingerprint(&file, &cache).unwrap();
    assert_eq!(gguf_fingerprint(&file, &cache).unwrap(), first);
    fs::write(&file, format!("GGUF{}", "b".repeat(65))).unwrap();
    assert_ne!(gguf_fingerprint(&file, &cache).unwrap(), first);
    fs::write(&file, "invalid".repeat(10)).unwrap();
    assert!(gguf_fingerprint(&file, &cache).is_err());
}
#[test]
fn deployment_validation_and_reserved_arguments() {
    let base = deployment();
    validate_deployment(&base).unwrap();
    for arg in [
        "--host=0.0.0.0",
        "--port",
        "--served-model-name=other",
        "--ho=0.0.0.0",
        "--config=other.yaml",
        "-m",
        "--ssl-keyfile=x",
    ] {
        let mut d = base.clone();
        d.args = vec![arg.into()];
        assert!(validate_deployment(&d).is_err(), "{arg}");
    }
    let mut d = base.clone();
    d.args = ["--threads", "4", "--n-gpu-layers", "-1"]
        .map(str::to_string)
        .into();
    validate_deployment(&d).unwrap();
    let mut d = base.clone();
    d.engine = "vllm".into();
    assert!(validate_deployment(&d).is_err());
    d.tokenizer = "org/tokenizer".into();
    validate_deployment(&d).unwrap();
    d.hf = true;
    d.target = "../..".into();
    assert!(validate_deployment(&d).is_err());
    d.target = "org/model".into();
    validate_deployment(&d).unwrap();
    let mut d = base;
    d.engine = "ollama".into();
    d.args = vec!["--threads".into()];
    assert!(validate_deployment(&d).is_err());
    assert!(serde_json::from_value::<Deployment>(json!({"unexpected":"field"})).is_err());
    assert!(serde_json::from_value::<Deployment>(json!({"args":null})).is_ok());
}
#[test]
fn preset_validation_and_model_names() {
    for alias in preset_aliases() {
        valid_model(alias).unwrap();
        let mut d = deployment();
        d.target = alias.into();
        d.model = alias.into();
        validate_deployment(&d).unwrap();
        d.engine = "vllm".into();
        assert!(validate_deployment(&d).is_err());
    }
    assert!(valid_model("unknown").is_err());
    assert_eq!(default_model_name("/workspace/My Model.gguf"), "my-model");
    assert_eq!(default_model_name("..."), "model");
}
#[test]
fn engine_commands_keep_target_alias_and_loopback() {
    for engine in ["llama.cpp", "ollama", "vllm"] {
        let mut d = deployment();
        d.engine = engine.into();
        d.target = "/workspace/file with spaces.gguf".into();
        d.tokenizer = "org/base".into();
        let (name, args) = engine_command(&d);
        if engine == "ollama" {
            assert_eq!(name, "ollama");
            assert_eq!(args, ["serve"]);
        } else {
            assert!(args.contains(&d.target));
            assert!(args.contains(&"127.0.0.1".into()));
            assert!(args.contains(&d.model));
        }
    }
}
#[tokio::test]
async fn readiness_requires_alias_and_ollama_loaded_model() {
    let alias = Arc::new(Mutex::new("wrong".to_string()));
    let value = alias.clone();
    let loaded = Arc::new(Mutex::new(false));
    let load = loaded.clone();
    let server = server(move |r| {
        if r.path == "/health" {
            return ok(json!({"status":"ok"}));
        }
        if r.path == "/api/ps" {
            return ok(if *load.lock().unwrap() {
                json!({"models":[{"model":*value.lock().unwrap()}]})
            } else {
                json!({"models":[]})
            });
        }
        ok(json!({"data":[{"id":*value.lock().unwrap()}]}))
    })
    .await;
    let mut d = deployment();
    assert!(ready_model(&server.url, &d).await.is_err());
    *alias.lock().unwrap() = d.model.clone();
    ready_model(&server.url, &d).await.unwrap();
    d.engine = "ollama".into();
    *alias.lock().unwrap() = format!("{}:latest", d.model);
    assert!(ready_model(&server.url, &d).await.is_err());
    *loaded.lock().unwrap() = true;
    ready_model(&server.url, &d).await.unwrap();
}
#[test]
fn info_omits_credential_containers_and_ssh_uses_gpu_identity() {
    let raw = json!({"id":"pod","gpu":{"memory":64},"env":{"TOKEN":"secret"},"args":"--token secret","registry":{"password":"secret"}});
    let value = public_pod_details(&raw).to_string();
    assert!(!value.contains("secret"));
    assert!(value.contains("memory"));
    let dir = tempfile::tempdir().unwrap();
    let app = App::load(&dir.path().join("profile with spaces")).unwrap();
    let mut pod:Pod=serde_json::from_value(json!({"id":"test","status":"RUNNING","ssh":{"direct":{"host":"203.0.113.10","port":22022,"username":"root"}}})).unwrap();
    let commands = app.connections(&pod, 8081, 8080).unwrap();
    assert_eq!(commands["tcp"], "203.0.113.10:22022");
    assert!(commands["ssh"].contains("gpu@203.0.113.10"));
    assert!(commands["tunnel_tcp"].contains("127.0.0.1:8081:127.0.0.1:8080"));
    assert!(
        fs::read_to_string(app.dir.join("ssh_config"))
            .unwrap()
            .contains("User gpu")
    );
    pod.status = "EXITED".into();
    assert!(
        !app.connections(&pod, 8081, 8080)
            .unwrap()
            .contains_key("ssh")
    );
    assert!(app.connections(&pod, 0, 8080).is_err());
}
#[tokio::test]
async fn preset_downloads_resume_restart_verify_and_reuse() {
    for alias in preset_aliases() {
        for mode in ["resume", "restart", "bad checksum", "bad range"] {
            let data = format!("GGUF{}", alias.repeat(8)).into_bytes();
            let served = data.clone();
            let n = Arc::new(Mutex::new(0));
            let requests = n.clone();
            let server = server(move |r| {
                *requests.lock().unwrap() += 1;
                assert!(r.headers.to_lowercase().contains("range: bytes=8-"));
                if mode == "restart" {
                    return Response {
                        status: 200,
                        headers: String::new(),
                        body: served.clone(),
                    };
                }
                Response {
                    status: 206,
                    headers: format!(
                        "Content-Range: bytes {}-{}/{}\r\n",
                        if mode == "bad range" { 0 } else { 8 },
                        served.len() - 1,
                        served.len()
                    ),
                    body: served[8..].to_vec(),
                }
            })
            .await;
            let mut model = preset(alias).unwrap();
            model.url = format!("{}/{}", server.url, model.file);
            model.size = data.len() as u64;
            model.sha256 = if mode == "bad checksum" {
                "0".repeat(64)
            } else {
                format!("{:x}", Sha256::digest(&data))
            };
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(&model.file);
            fs::write(format!("{}.partial", path.display()), &data[..8]).unwrap();
            let result = download_model(dir.path(), &model).await;
            if mode.starts_with("bad") {
                assert!(result.is_err());
                assert!(!path.exists());
                continue;
            }
            result.unwrap();
            assert_eq!(fs::read(&path).unwrap(), data);
            assert!(!Path::new(&format!("{}.partial", path.display())).exists());
            download_model(dir.path(), &model).await.unwrap();
            assert_eq!(*n.lock().unwrap(), 1);
            model.sha256 = "0".repeat(64);
            assert!(verify_model(&path, &model).is_err());
        }
    }
}
#[test]
fn model_list_targets_exclude_partial_cache_and_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let managed = root.join(".gpu/models");
    let custom = root.join("models/my model's weights.gguf");
    let copy = root.join("models").join(preset(MODEL_ALIAS).unwrap().file);
    let mut files = vec![
        custom.clone(),
        copy.clone(),
        managed.join("incomplete.gguf.partial"),
        managed.join("cached.gguf.verified.json"),
    ];
    for alias in preset_aliases() {
        files.push(managed.join(preset(alias).unwrap().file));
    }
    for file in files {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, "GGUF").unwrap();
    }
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), root.join("external")).unwrap();
    symlink(&custom, managed.join("linked.gguf")).unwrap();
    let mut out = Vec::new();
    list_models(&mut out, root, root).unwrap();
    let text = String::from_utf8(out).unwrap();
    for target in [
        quote(&custom.to_string_lossy()),
        quote(&copy.to_string_lossy()),
    ]
    .into_iter()
    .chain(preset_aliases().map(str::to_string))
    {
        assert!(text.contains(&target));
    }
    for hidden in [".partial", ".verified.json", "linked.gguf"] {
        assert!(!text.contains(hidden));
    }
    let mut out = Vec::new();
    list_models(&mut out, root, &managed).unwrap();
    assert!(
        !String::from_utf8(out)
            .unwrap()
            .contains(&quote(&custom.to_string_lossy()))
    );
    assert!(list_models(&mut Vec::new(), root, &root.join("external")).is_err());
    let mut out = Vec::new();
    list_models(&mut out, outside.path(), outside.path()).unwrap();
    assert!(
        String::from_utf8(out)
            .unwrap()
            .contains("No GGUF models found")
    );
}
#[tokio::test]
async fn subprocess_exit_status_is_preserved() {
    let error = process::run(
        tokio::process::Command::new("sh").args(["-c", "exit 37"]),
        None,
        false,
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<process::CommandFailure>()
            .unwrap()
            .code,
        37
    );
}

// Run command fixtures in child test processes so PATH/HOME never change in
// concurrently executing tests. Only this test helper accepts a mock API URL.
#[tokio::test]
async fn subprocess_fixture() {
    let Ok(arguments) = std::env::var("FASTGPU_TEST_ARGUMENTS") else {
        return;
    };
    let dir = std::env::var("GPU_CONFIG_DIR").unwrap();
    let mut app = App::load(Path::new(&dir)).unwrap();
    app.api = Some(Api {
        base: std::env::var("FASTGPU_TEST_API").unwrap(),
        key: "test-secret".into(),
        http: Client::new(),
    });
    let args: Vec<String> = serde_json::from_str(&arguments).unwrap();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    if let Err(error) = dispatch(&mut app, &args).await {
        eprintln!("fastgpu: {error:#}");
        std::process::exit(
            error
                .downcast_ref::<process::CommandFailure>()
                .map_or(1, |e| e.code),
        );
    }
}
fn stub_program(dir: &Path, name: &str, script: &str) {
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
async fn fixture_cli(dir: &Path, server: &Server, args: &[&str]) -> std::process::Output {
    tokio::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::subprocess_fixture", "--nocapture"])
        .env(
            "FASTGPU_TEST_ARGUMENTS",
            serde_json::to_string(args).unwrap(),
        )
        .env("FASTGPU_TEST_API", &server.url)
        .env("GPU_CONFIG_DIR", dir.join("profile"))
        .env("HOME", dir)
        .env(
            "PATH",
            format!(
                "{}:{}",
                dir.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("FASTGPU_TEST_CAPTURE", dir.join("arguments"))
        .kill_on_drop(true)
        .output()
        .await
        .unwrap()
}
fn selected_fixture(dir: &Path) {
    let mut app = App::load(&dir.join("profile")).unwrap();
    app.state = State {
        pod_id: "p".into(),
        volume_id: "v".into(),
        managed: true,
        ..State::default()
    };
    app.save_state().unwrap();
}
fn running_pod() -> Value {
    json!({"id":"p","status":"RUNNING","ssh":{"direct":{"host":"203.0.113.10","port":22022}},"mounts":{"network":[{"volumeId":"v"}]}})
}
#[tokio::test]
async fn startup_resumes_and_reuses_selected_pod_with_ssh() {
    for initial in ["RUNNING", "EXITED"] {
        let mut status = initial;
        let starts = Arc::new(Mutex::new(0));
        let count = starts.clone();
        let server = server(move |r| {
            if r.method == "POST" {
                assert_eq!(r.path, "/pods/p/action");
                assert_eq!(r.body["action"], "start");
                *count.lock().unwrap() += 1;
                status = "RUNNING";
                return response(204, Value::Null);
            }
            assert_eq!(r.path, "/pods/p");
            let mut pod = running_pod();
            pod["status"] = json!(status);
            ok(pod)
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        selected_fixture(dir.path());
        stub_program(dir.path(), "ssh", "exit 0");
        for args in [vec![], vec!["up", "--xlarge", "--skip-model"]] {
            let output = fixture_cli(dir.path(), &server, &args).await;
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8_lossy(&output.stderr).contains("already exists"));
        }
        assert_eq!(
            *starts.lock().unwrap(),
            if initial == "EXITED" { 1 } else { 0 }
        );
    }
}
#[tokio::test]
async fn code_opens_home_or_project_preserving_ssh_config() {
    let server = server(|r| {
        assert_eq!(r.path, "/pods/p");
        ok(running_pod())
    })
    .await;
    for (folder, expected) in [
        ("", "/home/gpu"),
        ("projects/demo", "/home/gpu/projects/demo"),
        ("home:project with spaces", "/home/gpu/project with spaces"),
        ("vol:projects/demo", "/workspace/projects/demo"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        selected_fixture(dir.path());
        stub_program(dir.path(), "ssh", "exit 0");
        stub_program(
            dir.path(),
            "code",
            "printf '%s\\n' \"$@\" > \"$FASTGPU_TEST_CAPTURE\"",
        );
        fs::create_dir_all(dir.path().join(".ssh")).unwrap();
        fs::write(
            dir.path().join(".ssh/config"),
            "Host existing\n  HostName example.invalid\n",
        )
        .unwrap();
        let args = if folder.is_empty() {
            vec!["code"]
        } else {
            vec!["code", folder]
        };
        let output = fixture_cli(dir.path(), &server, &args).await;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("arguments")).unwrap(),
            format!("--new-window\n--remote\nssh-remote+gpu-p\n{expected}\n")
        );
        let config = fs::read_to_string(dir.path().join(".ssh/config")).unwrap();
        assert!(config.starts_with("Include "));
        assert!(config.contains("Host existing"));
    }
}
#[tokio::test]
async fn shell_propagates_status_and_transfer_preserves_arguments() {
    let server = server(|r| {
        assert_eq!(r.path, "/pods/p");
        ok(running_pod())
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    selected_fixture(dir.path());
    stub_program(
        dir.path(),
        "ssh",
        "printf '%s\\n' \"$@\" > \"$FASTGPU_TEST_CAPTURE\"\nexit 37",
    );
    let output = fixture_cli(
        dir.path(),
        &server,
        &[
            "shell",
            "--cwd",
            "home:project with spaces",
            "python",
            "-c",
            "print('hello')",
        ],
    )
    .await;
    assert_eq!(output.status.code(), Some(37));
    let captured = fs::read_to_string(dir.path().join("arguments")).unwrap();
    assert!(captured.contains("BatchMode=yes"));
    assert!(captured.contains("project with spaces"));
    stub_program(dir.path(), "ssh", "exit 0");
    stub_program(
        dir.path(),
        "rsync",
        "printf '%s\\n' \"$@\" > \"$FASTGPU_TEST_CAPTURE\"",
    );
    let local = dir.path().join("project with spaces");
    fs::create_dir(&local).unwrap();
    let output = fixture_cli(
        dir.path(),
        &server,
        &[
            "push",
            &format!("{}/", local.display()),
            "home:project with spaces",
        ],
    )
    .await;
    assert!(output.status.success());
    let args = fs::read_to_string(dir.path().join("arguments")).unwrap();
    assert!(args.contains("--protect-args\n"));
    assert!(!args.contains("--delete"));
    assert!(args.contains("--exclude\n.env\n"));
    assert!(args.contains(&format!("{}/\n", local.display())));
    assert!(args.ends_with("gpu-p:/home/gpu/project with spaces/\n"));
}
#[tokio::test]
async fn cancelled_subprocess_is_killed() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    let mut command = tokio::process::Command::new("sh");
    command
        .args(["-c", "echo $$ > \"$1\"; exec sleep 30", "fixture"])
        .arg(&pid_file);
    let task = tokio::spawn(async move { process::run(&mut command, None, true, false).await });
    tokio::time::timeout(Duration::from_secs(3), async {
        while !pid_file.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let pid: i32 = fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    task.abort();
    let _ = task.await;
    tokio::time::timeout(Duration::from_secs(3), async {
        // Tokio reaps dropped child processes; wait until the PID disappears.
        while Path::new(&format!("/proc/{pid}")).exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
