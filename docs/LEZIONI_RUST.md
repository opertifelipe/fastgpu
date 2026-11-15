# From zero to fastgpu: Rust, SSH, RunPod, and npm

This course takes you from a first terminal command to understanding how the Rust implementation of fastgpu is assembled. It assumes no previous experience with Rust, npm, or SSH. Read Part I before Part II; work through the exercises in a separate directory. Part II walks through the actual repository, file by file, and explains the order in which to build and test each component. The repository source remains the complete implementation: examples in this course are deliberately small teaching examples, not replacements for all production checks.

The [CLI manual](CLI.md) describes the available commands. The project contains infrastructure operations that can create billable resources and delete resources: use local fixtures while learning and understand a command before running it against a real account.

## Course map

| Part | What you will learn | Result |
| --- | --- | --- |
| I: Foundations | Terminal, Rust, errors, async, HTTP, SSH, storage, containers | Understand the vocabulary and the data flow |
| II: Build the project | Each source file, its functions, dependencies, and tests | Reconstruct and reason about the CLI |
| III: Distribution | Cargo releases, Node, npm tarballs, and Docker images | Package the executable locally and prepare a future release |
| IV: Practice | Exercises, debugging, and a final checklist | Validate your understanding without a real GPU |

# Part I — Foundations

## 1. What is fastgpu?

A **command-line interface**, or CLI, is a program you operate by typing commands such as `fastgpu status`. The local `fastgpu` process asks RunPod's HTTPS API about Pods, volumes, and GPU availability. A **Pod** is the remote compute environment. A **Network Volume** is persistent storage mounted into that environment; it can remain after the Pod is terminated.

The Pod runs a container image containing software and another copy of the same fastgpu executable. Its hidden `worker` commands prepare models and manage an inference service. The local client connects with OpenSSH, copies files with rsync, and can forward a local port to the remote inference service.

```text
Local computer                               RunPod
┌───────────────────┐   HTTPS API       ┌─────────────────────┐
│ fastgpu client    │ ─────────────────> │ catalog, Pods,      │
│ config and state  │                    │ Network Volumes     │
│ private SSH key   │   SSH / rsync      │                     │
│                   │ ─────────────────> │ Pod + fastgpu worker│
│ localhost:8080    │ ─ SSH tunnel ────> │ inference :8080     │
└───────────────────┘                    │ /workspace: volume  │
                                         └─────────────────────┘
```

The provider can bill a Pod while it runs. Storage may incur charges independently, including after the Pod is stopped or terminated. The CLI's `cost` command reports a compute rate, not a final bill. `up --dry-run` asks the API for a plan without creating a Pod; it can still read resources and may update local recovery state in certain existing-state situations. Never interpret a network error as proof that a billable resource was not created.

### Vocabulary

- **Client/server:** the client initiates a request; the server receives it and responds.
- **Local/remote:** local is your computer; remote is the Pod on RunPod.
- **API:** a documented interface through which programs exchange requests and responses.
- **GPU/VRAM:** a processor suited to parallel calculations and the memory attached to it. Model size, context, and inference settings affect VRAM needs.
- **Container/image:** an image packages software; a container is a running instance of that image. The volume is separate storage mounted into it.
- **Inference:** running an existing trained model to generate outputs. Training and converting models are outside this CLI's automation.

## 2. The terminal and the operating system

A terminal displays a shell such as Bash or Zsh. The shell finds a program and passes it arguments. Practice in a disposable directory:

```sh
mkdir -p /tmp/fastgpu-learning
cd /tmp/fastgpu-learning
pwd
printf '%s\n' 'hello world'
ls -la
```

`/` is the filesystem root, `.` is the current directory, `..` is its parent, and `~` normally expands to your home. `/tmp/example` is absolute; `example` is relative to the current directory. Paths containing spaces need quoting: `cd 'my project'` passes one argument. Single quotes protect most shell metacharacters; double quotes still expand variables such as `$HOME`.

A **process** is a running program. A process inherits a working directory and environment variables from its parent. `PATH` is a list of directories searched for executable names. `export NAME=value` makes a variable available to child processes. Do not put real API keys into checked-in files or examples.

Each process has standard input (stdin), standard output (stdout), and standard error (stderr). `program > output.txt` redirects stdout; `program 2> errors.txt` redirects stderr; `first | second` connects stdout to the next process's stdin. The exit status is conventionally `0` on success and nonzero on failure; in a shell, inspect it immediately with `echo $?`.

An interactive terminal can send signals: `Ctrl-C` usually sends SIGINT to the foreground process group. A remote command can itself start further processes, so signal handling and cleanup require care.

**Exercise:** create a filename with spaces, print it with `printf '%s\n'`, and compare the arguments received with and without quotes. Do not use shell interpolation to assemble commands containing untrusted input.

## 3. Install the development tools

For the current source build you need Linux, Rust 1.91 or later with Cargo, a C compiler/linker and make. SSH and rsync are needed for remote operations. Node.js and npm are needed for the npm package; the launcher currently requires Node 22.15 or later in the 22.x series, or Node 24+. Docker is needed to build the image, not to run a prebuilt local client.

Install Rust with the [official Rust instructions](https://www.rust-lang.org/tools/install), then inspect your environment:

```sh
rustc --version
cargo --version
rustup component add rustfmt clippy
ssh -V
rsync --version
node --version
npm --version
```

Do not include credentials in a repository, command examples, logs, or a Docker build context. An environment variable avoids storing a key in source, but shell history and process environments still require care. The [CLI manual](CLI.md) explains the interactive key-entry workflow.

## 4. Create and run your first Rust program

In a scratch directory:

```sh
cargo new hello-rust
cd hello-rust
cargo run
```

Cargo creates `Cargo.toml` and `src/main.rs`. Replace the source with:

```rust
fn main() {
    println!("Hello, Rust!");
}
```

`fn` declares a function; `main` is the executable's entry point. Braces form a block, while `println!` is a macro: the exclamation mark distinguishes its invocation from a function call. `cargo run` compiles and runs; `cargo check` checks types without producing a finished executable; `cargo build --release` builds an optimized binary under `target/release/`. `cargo fmt` formats source and `cargo clippy` reports potential mistakes.

`Cargo.toml` is the package manifest. A **crate** is a compilation unit; a package may provide both a library crate (`src/lib.rs`) and a binary crate (`src/main.rs`). Dependencies are crates from the registry or other sources. `Cargo.lock` records resolved dependency versions, and `--locked` fails if the lockfile needs updating. Rust editions select language conventions; this project uses edition 2024.

**Exercise:** change the greeting, run `cargo check`, then introduce a misspelled variable. Read the compiler diagnostic and fix it before proceeding.

## 5. Variables, types, and functions

```rust
fn hourly_total(rate: f64, hours: f64) -> f64 {
    rate * hours
}

fn main() {
    let gpu_name: &str = "example";
    let mut hours: f64 = 2.0;
    hours += 1.0;
    let total = hourly_total(0.5, hours);
    println!("{gpu_name}: {total}");
}
```

`let` creates a binding; it cannot be reassigned unless marked `mut`. `: f64` is a type annotation. Rust often infers types from context, but annotations clarify APIs. `-> f64` gives a return type. The final expression has no semicolon and becomes the function's return value; adding a semicolon turns it into a statement whose value is `()` (the unit type).

`i64` and `u64` are signed and unsigned 64-bit integers. `usize` is an integer suitable for collection sizes and indexes. `f64` is floating point: decimal values can be approximate, so boundary tests matter for price comparisons. `bool` is `true` or `false`. A `char` is a Unicode scalar value, not necessarily a visible grapheme. A tuple groups values, while an array such as `[1, 2, 3]` has fixed length. `Vec<T>` is a growable sequence.

`String` owns UTF-8 text; `&str` is a borrowed string slice. Indexing a string by byte position is not generally valid for arbitrary Unicode characters. Rust's `format!` creates a `String`; `println!` prints to stdout and `eprintln!` to stderr.

**Exercise:** write a function taking a price and a maximum. Return whether the price is positive and under the maximum. Try zero, a value at the limit, and one above it.

## 6. Branches, loops, and pattern matching

```rust
fn category(price: f64) -> &'static str {
    if price < 1.5 {
        "small"
    } else if price < 3.5 {
        "medium"
    } else {
        "higher"
    }
}
```

`if` is an expression, so all branches must return compatible types. A `match` checks patterns exhaustively. `for` iterates through a collection, `while` repeats while a condition is true, and `loop` repeats until `break` or return. `0..3` excludes 3; `0..=3` includes it.

```rust
let sizes = ["small", "medium", "large"];
for size in sizes {
    match size {
        "small" => println!("lower price range"),
        other => println!("selected: {other}"),
    }
}
```

In the real `size_offers` implementation, small is below $1.50, medium is $1.50 to below $3.50, large includes $3.50 through $5, and xlarge is strictly above $5. Medium sorts by distance from $3, not simply by lowest price. Exact boundaries and ties should be tested.

## 7. Ownership, moves, and borrowing

Rust tracks who owns each value and when a resource can be freed. A `String` owns allocated text. Assigning it to another variable normally **moves** ownership:

```rust
let first = String::from("workspace");
let second = first;
println!("{second}");
// first can no longer be used here.
```

Small values implementing `Copy`, including integers and booleans, are copied instead. `clone()` explicitly creates an independent copy when the type supports it; for large values this may allocate or do expensive work.

A reference borrows without taking ownership. `&T` permits shared access; `&mut T` permits exclusive mutable access. The compiler prevents a mutable borrow from overlapping incompatible active borrows. A reference must never outlive the value it points to.

```rust
fn print_label(label: &str) {
    println!("{label}");
}

let mut label = String::from("gpu");
print_label(&label);
label.push_str("-example");
print_label(&label);
```

Lifetimes describe how long references are valid; Rust can infer many of them. `&Path` borrows a path and `PathBuf` owns one. In methods, `&self` borrows the object for reading, `&mut self` permits mutation, and `self` takes ownership. `App::up(&mut self, ...)` needs mutable access because it changes local state; a read-only selector can often use `&self`.

**Exercise:** try passing a `String` by value and then using it again. Observe the error. Change the function to borrow `&str`; explain why the caller can now reuse the string.

## 8. Structs, enums, traits, and modules

A struct gives related values names; an `impl` block supplies associated functions and methods:

```rust
#[derive(Debug, Clone)]
struct Volume {
    id: String,
    size_gb: u64,
}

impl Volume {
    fn description(&self) -> String {
        format!("{}: {} GB", self.id, self.size_gb)
    }
}
```

An enum models one of several alternatives, and variants may carry different data:

```rust
enum Action {
    Start,
    Stop,
    Select(String),
}
```

`match action { Action::Start => ..., Action::Select(id) => ..., ... }` extracts the variant's data. This is how Clap's derived `Commands` enum models subcommands.

A **trait** describes behavior a type supports. `Display` formats user-facing text; `Clone` makes an explicit copy; `Serialize` and `Deserialize` support conversion to and from serialized representations. `#[derive(...)]` generates implementations for suitable types. Generics let a function work with several types: `Vec<T>` stores elements of type `T`; `T: Serialize` means `T` must implement the trait. `impl Serialize` in an argument is a concise way to express a similar constraint.

A module organizes names. `mod api;` in `src/lib.rs` loads `src/api.rs`; `use crate::api::Api;` imports its `Api` name. `pub` exposes an item outside its module, subject to its parent's visibility. Keeping modules focused makes unsafe operations, state changes, and API calls easier to review.

## 9. Handling absence and errors

Rust encodes a possibly absent value as `Option<T>`: either `Some(value)` or `None`. It encodes a fallible operation as `Result<T, E>`: either `Ok(value)` or `Err(error)`. Both can be inspected with `match`:

```rust
fn parse_size(input: &str) -> Result<u64, std::num::ParseIntError> {
    let value = input.parse::<u64>()?;
    Ok(value)
}

match parse_size("50") {
    Ok(size) => println!("{size} GB"),
    Err(error) => eprintln!("invalid size: {error}"),
}
```

`?` returns an error early to the caller, converting it when the return type supports the conversion. `anyhow::Result` is convenient at an application boundary where several error types may occur; `.context(...)` adds an explanation without discarding the cause. A typed `ApiError` still matters because the application must distinguish status 404 from network failure.

`unwrap()` panics on `None` or `Err`; it is useful in tests but should not be the default way to handle external input. `expect("explanation")` also panics. For a missing file, matching `ErrorKind::NotFound` can be appropriate; other I/O errors must not silently turn into default data.

**Exercise:** change the example to parse `"many"`. Make the error explain what input was invalid without exposing a token or credential.

## 10. Collections, iterators, and closures

```rust
let prices = vec![0.4, 2.0, 4.0];
let affordable: Vec<f64> = prices.iter()
    .copied()
    .filter(|price| *price < 3.0)
    .collect();
assert_eq!(affordable, vec![0.4, 2.0]);
```

An iterator yields values on demand. `iter()` borrows elements; `into_iter()` may consume the collection. `filter` retains matching values, `map` transforms them, `find` returns the first match, and `collect` builds a collection. The closure `|price| ...` is an anonymous function that can capture nearby variables. Sorting with `sort_by` requires a comparison; floating-point prices use `total_cmp` in this project to provide a total ordering.

In `api.rs`, filter GPUs by secure availability, NVIDIA manufacturer, minimum VRAM, architecture, data center, and optional maximum price before applying the chosen price band. Sorting only after filtering avoids recommending a cheap but incompatible machine.

## 11. Files, paths, locks, and permissions

`std::fs::read`, `write`, and `read_to_string` interact with files. `Path` is a borrowed path and `PathBuf` is an owned path. `join` combines components; it does not itself prove that the resulting location is safe. Reject absolute paths where only relative paths are allowed and prevent `..` from escaping the chosen root. A symbolic link can point outside a lexically valid directory: the model path checks canonicalize existing paths before accepting them.

Configuration updates use a temporary file in the target directory, write and sync it, then rename it over the destination. This makes readers less likely to see a half-written JSON document. An exclusive file lock serializes cooperating processes operating on the same profile. It does not coordinate independently configured profiles or multiple computers. Unix permissions such as `0700` for private directories and `0600` for private files restrict access according to the operating system's user model.

A `Drop` implementation runs when a value leaves scope, including ordinary error returns. Here `FileLock` explicitly unlocks before its file closes. Do not assume cleanup runs after power loss or forced process termination; design on-disk state for recovery too.

## 12. Async Rust and cancellation

A network call can spend time waiting for a response. `async fn` returns a future representing work that can be polled; `.await` suspends that task until progress is possible. Tokio provides the runtime and asynchronous networking, timers, process handling, and signals.

```rust
async fn wait_a_moment() -> anyhow::Result<()> {
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    Ok(())
}
```

`#[tokio::main]` starts the runtime. `tokio::time::timeout` bounds waiting; `tokio::select!` waits for whichever branch completes first. If a future is dropped, its remaining work is cancelled. The project sets `kill_on_drop(true)` for child processes and uses guards for terminal and lock cleanup. Cancellation does not undo external actions already accepted by RunPod.

Asynchronous code is not automatically parallel and does not make blocking filesystem or CPU-heavy hashing free. Large file hashing and synchronous operations need particular attention when scaling the design. In this CLI, understand where waiting can occur before changing timeouts or cancellation behavior.

## 13. HTTP, JSON, and the RunPod API

A URL identifies a server and path. An HTTPS request has a method, headers, optional body, and response status. GET reads; POST generally creates or starts an action; PATCH updates; DELETE removes. A 2xx status denotes success; 404 indicates a missing resource; 429 indicates rate limiting; 5xx indicates a server-side failure. These are HTTP conventions; endpoint-specific semantics are determined by the provider.

A Bearer API key in an authorization header identifies the caller. HTTPS encrypts transport, but the key must still not be printed or included in uploaded Pod environment variables. JSON expresses objects, arrays, strings, numbers, booleans, and null. Serde maps Rust structures to JSON and back:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExampleVolume {
    id: String,
    data_center: String,
}
```

`data_center` maps to `dataCenter`. `#[serde(default)]` supplies defaults where the structure permits it; `skip_serializing_if` omits selected empty fields. Deserialize untrusted responses carefully and cap their size. Reqwest handles HTTPS through rustls in this project. The API client disables redirects for authenticated requests, caps ordinary responses, and redacts the key in API error details.

A list response may include a pagination cursor. Keep requesting pages until `hasNextPage` is false; reject an empty or repeated cursor to avoid an infinite loop. Retry some transient **GET** failures. A timeout on a creation POST is ambiguous: the server may have created the Pod even though the response never reached you. Save an operation identity first and reconcile it with the account's Pods before any new creation attempt.

RunPod has separate APIs for GPU catalog, Pods, and Network Volumes. Consult [RunPod documentation](https://docs.runpod.io/) for the current API and pricing; provider behavior and available GPU types can change.

## 14. SSH from first principles

SSH is a protocol and a program for encrypted remote connections. Your local client connects to an SSH server at a host and port. In public-key authentication, a private key remains local and a corresponding public key is authorized on the server. This project creates a dedicated Ed25519 key pair with `ssh-keygen` if no key exists; it sends only the public key when creating a Pod.

A server also has a **host key**, which identifies the server to the client. The client records host keys in a dedicated `known_hosts` file. `StrictHostKeyChecking accept-new` accepts an unseen key on first use and rejects unexpected changes later; the first acceptance alone does not verify the server identity.

```text
Host gpu-POD_ID
  HostName EXAMPLE_HOST
  Port EXAMPLE_PORT
  User gpu
  IdentityFile /path/to/private_key
  IdentitiesOnly yes
  StrictHostKeyChecking accept-new
```

The generated file contains the actual endpoint, key path, and dedicated known-hosts file. `ssh -F FILE ALIAS` uses this configuration. `-tt` requests a terminal. Batch mode avoids interactive password prompts; encrypted private keys should be loaded into a local agent first. Agent forwarding is disabled in the generated configuration.

A local port forward such as `ssh -L 127.0.0.1:8080:127.0.0.1:8080 ...` listens on your local loopback interface and sends connections through SSH to the Pod's loopback port. It does not expose the service to the public internet by itself. Closing the tunnel does not shut down the inference service or terminate the Pod.

Rsync transfers files over SSH. A trailing slash on a source directory affects whether its contents or the directory itself are copied. The project excludes several common secret patterns and does not pass `--delete`; this does not detect all secrets, and existing destination files can still be updated.

### Quoting and command boundaries

Local `Command::args` passes separate arguments without invoking a shell. SSH remote commands, however, are interpreted remotely, so `ssh.rs` quotes each argument and wraps it carefully. A value such as `$(...)` must stay data, not become a shell command. Test spaces, empty strings, single quotes, semicolons, and dollar signs. Prefix validation also rejects traversal outside the selected lexical root; it does not turn transfer paths into a security sandbox against remote symlinks.

## 15. Containers, persistent data, and models

A Dockerfile builds an image in layers. fastgpu's multi-stage Dockerfile compiles Rust in a builder stage, then copies the executable into an NVIDIA CUDA runtime image alongside Python, model engines, SSH, and Supervisor. The final image is separate from the npm package.

When the Pod starts, `docker/entrypoint.sh` requires `/workspace` to be mounted and a public key to be supplied. It initializes `/workspace/.gpu`, establishes a persistent home, configures SSH, protects concurrent use of the same managed home with a lock, creates a readiness marker, and starts Supervisor. The configured inference service is not automatically started on first container startup.

The persistent layout includes `/workspace/.gpu/home`, `/workspace/.gpu/models`, logs, and Ollama's model store. `/home/gpu` points to the persistent home. Container disk contents outside the mounted volume can disappear when a Pod is recreated. Persist your work deliberately.

GGUF is a file format commonly used by llama.cpp. vLLM commonly uses Hugging Face checkpoints; Ollama manages its own model store and can import supported artifacts. The same model is not necessarily compatible with every engine. Context length, quantization, and model architecture affect memory requirements.

Preset downloads use a temporary `.partial` file, HTTP Range when resuming, an expected size, and SHA-256. A digest is useful only if the expected value comes from a trusted source. After verification, the file is renamed into place. Readiness checks ask whether the expected model alias is available; a running process alone is insufficient evidence that inference is ready.

# Part II — Build the CLI, file by file

## 16. Set up an isolated study project

Make a separate copy for experiments. A practical strategy is to create an empty `fastgpu-study` directory, add one module at a time, and use the real file in this repository as the detailed reference. In early steps use placeholders for modules that do not yet exist; the real files depend on each other, so copying just one source file will not necessarily compile by itself. Run `cargo check` after each coherent change. Add tests as soon as a behavior is implemented.

Suggested milestones:

1. Local parser, configuration, and tests without an API key.
2. Fake HTTP server and safe GPU selection.
3. Process and SSH behavior using fake executables.
4. Lifecycle and recovery using fake API responses.
5. Worker, model verification, and engine readiness using small fixtures.
6. Container build and npm archive; live RunPod validation only after local tests pass.

## 17. Project manifest: `Cargo.toml`, `Cargo.lock`, and ignore files

Write `[package]` with `name`, `version`, `edition`, `rust-version`, license, and repository. Under `[dependencies]`, select the features used by the implementation:

| Dependency | Role in this project |
| --- | --- |
| `clap` with `derive` | Define commands, flags, help, and argument validation |
| `serde`, `serde_json` | JSON state, API responses, manifests |
| `tokio` | Async runtime, signals, timers, processes |
| `reqwest` with rustls | HTTP client without an OpenSSL runtime dependency |
| `anyhow` | Error context and application-level propagation |
| `fs2`, `tempfile` | Locks, free-space checks, atomic replacement |
| `sha2`, `regex`, `walkdir` | Fingerprints, validation, directory traversal |
| `rand`, `toml`, `libc` | Operation names, runpodctl credentials, Unix terminal APIs |

Copy the actual feature declarations from the repository, then use Cargo to generate the lockfile. `cargo build --locked` should not rewrite it. `[profile.release]` sets optimization-related choices including stripping. `.gitignore` excludes generated artifacts and common credential files; `.dockerignore` prevents local build outputs and secrets from entering the Docker build context. An ignore rule is not a substitute for reviewing staged files.

## 18. Entrypoints: `src/lib.rs` and `src/main.rs`

In `src/lib.rs`, declare `api`, `cli`, `config`, `engines`, `info`, `lifecycle`, `models`, `process`, `ssh`, and `worker`; expose only the items needed by the binary. In `src/main.rs`, start Tokio and register Unix interrupt and termination signals. Use `tokio::select!` to wait for `fastgpu::execute()` or a signal. Print errors with the `fastgpu:` prefix and preserve a positive child exit status through `CommandFailure` when available.

Trace this path for `fastgpu --version`: the operating system starts `main`, Tokio starts the async body, `execute` asks Clap to parse arguments, and Clap's version response is printed without requiring an API key. Test help and version before adding cloud behavior.

## 19. Configuration: `src/config.rs`

Start with constants for the version, remote home, workspace, and default image. Implement `Config` as serializable fields: image, registry credential ID, selected volume, SSH key, minimum resources, disk, price limit, context, and local port. `State` tracks the selected Pod ID, pending operation name, volume ID, managed flag, and tunnel port. Neither structure should contain the RunPod API key.

Implement `Default` for configuration. `App::new` resolves `GPU_CONFIG_DIR`, otherwise `XDG_CONFIG_HOME/gpu`, otherwise the user's `~/.config/gpu`; convert the result into an absolute path. `App::load` overlays an existing configuration JSON onto the defaults, so older profiles with missing fields still load. A malformed file should return an error rather than silently overwrite the user's configuration.

Build `atomic_file` with a temporary file in the destination directory, write bytes, sync, and persist/rename; `atomic_json` serializes formatted JSON first. Make the profile directory private and use an exclusive nonblocking lock for mutations. The `FileLock` wrapper unlocks in `Drop`. `api_key` reads a nonempty `RUNPOD_API_KEY` first and otherwise parses `apikey` from the user's runpodctl TOML configuration. Never include the key in config output.

**Checkpoint:** test default values, loading old JSON, malformed JSON, file permissions, atomic output, and a second operation failing to acquire an existing lock.

## 20. API layer: `src/api.rs`

Define `Api` with a base URL, key, and reusable HTTP client. `ApiError` holds HTTP status and a redacted detail; implement `Display` and `Error`. Escape path components before inserting resource IDs into URLs. `request` sends JSON, sets authorization, bounds response data, and retries transient GET responses only. It does not blindly retry POST, PATCH, or DELETE.

Define serde data structures for volumes, mounts, endpoints, Pods, and GPUs. Provider JSON uses names such as `dataCenter` and `volumeId`; serde renaming maps them to Rust naming conventions. `list` reads all pages and records seen cursors. `pods`, `volumes`, and `pod` are narrow wrappers. `logs` reads an event stream incrementally and prints decoded `data:` records.

Implement `resolve_volume` so an exact ID takes precedence and duplicate names are rejected. Implement `eligible` and `size_offers` separately: this permits independent tests for compatibility and price-band rules. The order of sorting affects selection, especially medium's distance from $3/hour.

**Checkpoint:** use a local HTTP server to assert the request method, path, response handling, redaction, repeated cursor rejection, and one POST on a 503. No real token or account is necessary.

## 21. Processes and terminals: `src/process.rs`

Define `CommandFailure { code, program }` and implement `Display` and `std::error::Error`. Its purpose is to preserve a child process's nonzero status through `anyhow` to the top-level exit handling. `run` accepts a Tokio `Command`, optional bytes for stdin, whether to capture stdout, and whether stdin should be inherited. Configure `kill_on_drop(true)`, spawn, write input asynchronously if supplied, wait for completion, and return an error on nonzero status.

`find_program` searches executable files in `PATH`. `Terminal::save` checks whether stdin is a terminal and saves its `termios`; `Drop` restores it. The `unsafe` calls into `libc` require valid pointers and initialized values. Read the safety comments and understand that such calls bypass some of Rust's usual compile-time guarantees.

**Checkpoint:** use `sh -c 'exit 37'` as a harmless fixture; assert the exit code survives. Test cancellation and terminal restoration separately. Do not write a test that kills a real user process.

## 22. SSH and information: `src/ssh.rs`, `src/info.rs`

In `ssh.rs`, first implement path and quoting helpers. `remote_path` maps `home:` to `/home/gpu` and `vol:` to `/workspace`, rejects control characters and absolute paths, normalizes components, and checks containment. `quote` produces a shell-safe single-quoted string; `shell_words` applies it to every argument. Verify round trips through a local Bash fixture, including single quotes and command-substitution syntax.

`ssh_config` validates host, Pod ID, and port before writing a dedicated configuration. `remote` constructs the SSH argument vector and a quoted remote command; interactive requests save terminal settings. `ensure_key` checks for both private and public key files or invokes `ssh-keygen`. `tunnel` checks the local port, uses an SSH control socket, and records the port in state; `close_tunnel` asks the control master to exit. `transfer` uses rsync exclusions and no deletion flag. `open_code` verifies the remote directory and adds an Include line to the user's SSH configuration if needed.

In `info.rs`, `public_pod_details` uses an allowlist rather than copying the entire provider response; environment and registry data may contain credentials. `connections` constructs printable SSH and tunnel examples. `info --live` additionally runs a remote Python probe.

**Checkpoint:** fake `ssh`, `rsync`, and `code` in a temporary `PATH`. Capture their argument lists and confirm paths with spaces remain one argument and output omits credential containers. Use a temporary HOME before testing modifications to SSH config.

## 23. Pod lifecycle: `src/lifecycle.rs`

`select_volume` chooses by explicit selector, saved selector, or interactive selection. A noninteractive ambiguous choice must fail rather than guess. `offers` requests the catalog for the selected volume's data center and applies API filtering. `await_ready` polls Pod status and checks the remote readiness file with a timeout.

The key invariant in `up` is **do not create a duplicate after an uncertain write**:

1. If a selected Pod exists, reuse it; if stopped, request start and wait for readiness. A different volume requires an explicit teardown first.
2. If a previous `pending_name` exists, list account Pods and reconcile by name. Refuse to guess when the result is ambiguous.
3. Before a new creation, list all account Pods and reject another active `gpu-*` Pod.
4. Select volume and offers, validate configuration, and show price information. A dry run ends before a new creation.
5. Ensure the SSH public key exists; generate and save a pending operation name and volume ID **before** sending POST.
6. Try a different offer only after an explicit capacity rejection. Preserve uncertain state on timeouts or ambiguous errors.
7. Save the returned Pod ID before waiting for startup. Check actual rate against the cap and, if needed, terminate the newly created Pod while retaining the volume.

`down` obtains the selected Pod, checks managed identity and its expected volume, closes the tunnel, requests Pod deletion, waits for termination, verifies the volume still exists, and only then clears local state. A local lock covers one profile, not concurrent creations from multiple machines or profiles.

**Checkpoint:** simulate a POST that creates a Pod but returns a 503. Restart the application with the saved state; assert reconciliation finds it and no second POST occurs. Simulate mismatched volume identity and verify `down` refuses deletion.

## 24. Presets and downloads: `src/models.rs`, `src/worker.rs`

`models.rs` defines `MODEL_ALIAS`, `MODEL_DIR`, `DEPLOY_FILE`, and `ModelPreset` values. Presets include a filename, pinned download URL, expected size, and expected SHA-256. `list_models` traverses a directory without following symlinks and prints aliases for managed preset paths or quoted absolute paths for custom files.

In `worker.rs`, define `Deployment` and `Verification` structures. `prepare_model` checks that the environment was initialized before downloading. `download_model` obtains a lock, checks free disk space, creates or opens a partial file, and resumes from its current length. A 206 response must report a matching Range; if a server replies 200, the download restarts from zero. A completed file is synced and checked for size and digest before it is renamed to its final name. A small verification metadata file supports reuse.

`supervisor` calls `supervisorctl` for the inference service. `worker_serve` loads and validates the deployment manifest, checks its fingerprint, and replaces itself with the chosen engine. `worker_check` runs diagnostic commands, including a CUDA computation.

**Checkpoint:** make a tiny local byte fixture, serve it from a test HTTP server, and test resume, server ignoring Range, wrong Range, bad digest, and reuse. Never use a fake digest to claim that a real model is verified.

## 25. Engines and readiness: `src/engines.rs`

Start with `default_model_name`, `model_path`, and `validate_deployment`. Validate the engine name, model alias, context range, Hugging Face ID, tokenizer, Modelfile, and extra arguments. Reserved options protect CLI-managed addresses, ports, model names, and other settings. Then implement `persistent_path`, resolving symlinks and requiring the resolved path to stay on the Network Volume.

`gguf_fingerprint` checks the GGUF magic, file size, and SHA-256, with a cache tied to file metadata. `deployment_fingerprint` handles GGUF files, checkpoint metadata, Hugging Face IDs, and Modelfiles as implemented in the source. The cache assumes trusted processes on the volume; it is not a defense against a malicious process deliberately preserving metadata.

`engine_command` returns a program name and an argument vector for llama.cpp, Ollama, or vLLM. For managed services the host is loopback and the port is 8080. `ready_model` checks health and the expected catalog alias; Ollama additionally checks its loaded models. `worker_deploy` serializes changes with a lock, resolves presets and fingerprints, reuses an identical ready deployment, otherwise stops the prior service, writes the manifest, starts the service, and handles Ollama import/load. `worker_ready` checks the current manifest.

**Checkpoint:** use a local fake HTTP server to return the wrong alias, then the right alias. A healthy HTTP server returning a different model must not count as ready. Real GPU generation remains a separate validation task.

## 26. Command parser and dispatch: `src/cli.rs`

Define `Cli` with Clap's `Parser` derive and an optional `Commands` enum. Each enum variant describes a command and its argument fields. Use nested enums for `ModelCommand`, `VolumeCommand`, and hidden `WorkerCommand`. `Up` groups mutually exclusive size flags; `Deploy` captures model, engine, context, tokenizer, and repeatable `--arg` values. `shell` uses trailing arguments so options after the remote program are passed through to that program.

`execute` handles parsing and delegates to `App::dispatch`; no subcommand defaults to `up`. `dispatch` acquires the profile lock for the operations identified as mutations and routes to the relevant module. Volume deletion requires the exact ID confirmation and checks configured, managed, and attached volumes. The `deploy` method validates first, starts or reuses a Pod, checks the remote CLI version, sends a JSON manifest over SSH stdin, waits for readiness, opens the tunnel, and checks the model through that tunnel.

**Checkpoint:** enumerate help pages, test mutual exclusion and invalid arguments, and verify `shell` does not consume flags intended for the remote command. Test `volume` as an alias for `volumes`. Read the [command manual](CLI.md) alongside the enum definitions.

## 27. Container files: `docker/`

Read `docker/Dockerfile` from top to bottom. The `cli` stage copies the Cargo manifest and Rust source and builds the release executable. The final CUDA stage installs Ubuntu packages and Python requirements, executes the numbered scripts in `docker/install.d/`, installs the CLI, creates the `gpu` user, copies configuration, and selects the entrypoint.

`docker/packages.apt` lists system packages; `docker/requirements.txt` lists Python requirements. The numbered installation scripts install build dependencies and tools such as GitHub CLI, Codex, llama.cpp, and Ollama. `entrypoint.sh` prepares the persistent workspace and SSH. `supervisord.conf` defines the SSH server and the disabled-by-default inference program with persistent logs. Inspect each file before changing it; external binaries and dependencies should be pinned and verified according to the repository's existing approach.

`make image` builds the complete image, which can be large. `docker build --target cli -f docker/Dockerfile -t fastgpu-cli-test .` checks just the builder stage. Neither build publishes an image. The 0.3.0 Rust worker must be present in an image used for deployment; changing the local executable alone does not replace a Pod's image.

## 28. Tests: `src/tests.rs` and `tests/cli.rs`

Unit tests can access internal modules through `#[cfg(test)]`; integration tests run the compiled executable as an external user would. The Rust tests use temporary directories, fake API servers, and fake programs rather than relying on real credentials or cloud resources. Examples include pagination loops, transient reads, uncertain creation, protected volumes, fingerprint changes, readiness aliases, SSH quoting, and process exit codes.

```sh
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
# Or run all three:
make test
```

A good test checks an observable contract: for example, one POST after an ambiguous failure, or no DELETE for a protected volume. A test that merely repeats an implementation's internal steps can pass while the user-facing behavior remains wrong. Local mocks do not validate real API compatibility, GPU availability, inference, or billing.

# Part III — Package and distribute

## 29. Node.js and npm basics

Node.js runs JavaScript outside a browser. npm manages packages and exposes installed commands. `package.json` describes a package; `name` and `version` identify it, `bin` maps a command to a script, `files` limits packaged files, and `scripts` defines commands and lifecycle hooks. `npm pack` creates a `.tgz` archive; installing a local archive does not require publishing it to a registry.

This package includes the compiled Rust executable and a small Node launcher. It currently has `private: true`, so npm will refuse normal registry publication. The archive is limited to Linux x64 with glibc. The build environment determines the minimum glibc version required by the executable; a binary built on a newer distribution may not run on an older one. Node itself is still needed to run the launcher; Cargo is not needed when installing an already-built archive.

## 30. npm files and lifecycle

In `package.json`, `bin.fastgpu` points to `npm/fastgpu.cjs`. The `prepack` hook runs `scripts/build-npm.cjs`; it checks Cargo metadata against the npm version, builds or selects a supplied binary, checks a Linux x64 ELF header and `--version`, copies the executable to `npm/bin/fastgpu`, and sets executable permissions. The `files` allowlist includes the launcher, executable, and documentation. There are no installation hooks that compile or download a binary.

The launcher checks the operating system, architecture, and availability of `process.execve`, then replaces itself with the Rust executable, passing through the argument list and environment. Replacing the process preserves the process identity and standard streams for interactive SSH and signal handling. Node documents `process.execve` as experimental for the relevant releases; check its current documentation before changing the supported Node versions.

```sh
make npm-package
npm install -g ./dist/fastgpu-0.3.0.tgz
fastgpu --version
npm test
```

The first command needs the Rust build tools and writes to `dist/`. The second installs the already-created archive under npm's global prefix; inspect that prefix with `npm prefix -g`. If another `fastgpu` binary exists earlier on `PATH`, `command -v fastgpu` shows which one the shell finds.

For a prebuilt binary, `FASTGPU_NPM_BINARY=/absolute/path/to/fastgpu make npm-package` skips rebuilding but still performs the package checks. The Docker CLI stage can produce a Debian Bookworm build; the [manual](CLI.md) contains the copy-and-package commands. The executable's imported glibc symbols, actual system compatibility, and supported CPU architecture must be validated for each release.

`tests/npm.test.cjs` covers arguments, stdin/stdout/stderr, process identity, signals, exit status, and errors. `tests/npm-package.test.cjs` builds an archive, verifies its file list and permissions, and installs offline in a temporary prefix. Run `npm test` before creating an archive for another machine.

## 31. Preparing a future public release

The current task and package are local. A future release requires deciding a registry name and ownership, validating the archive on supported systems, removing `private: true` deliberately, and configuring publication credentials or trusted publishing. Review the precise contents with `npm pack --dry-run` and inspect a real tarball. Keep Cargo, npm, and worker image versions synchronized. A published npm package does not publish the CUDA image; that is a separate artifact and deployment.

An npm package version is a release identifier, not a command to upgrade existing users automatically. Plan versioning, provenance, tests, and compatibility before publishing. See the [npm package.json reference](https://docs.npmjs.com/cli/v11/configuring-npm/package-json/) and [npm trusted publishing](https://docs.npmjs.com/trusted-publishers/) for current behavior.

# Part IV — Practice and debugging

## 32. Exercises in increasing difficulty

1. **Rust syntax:** implement `fn is_affordable(price: f64, cap: f64) -> bool`. Test zero, positive price, exact cap, and above cap. Explain how you treat a disabled cap.
2. **Ownership:** write a function accepting `&str` and returning a normalized `String`. Show that the caller can continue using the original input.
3. **Errors:** parse an integer from text into `Result`. Add context to an invalid value; avoid `unwrap` on user input.
4. **Selection:** create three in-memory GPU structs. Filter by VRAM and data center, then sort by price. Add tests around price-band boundaries.
5. **State:** write a small JSON object to a temporary directory and load it. Test missing and malformed files separately.
6. **HTTP:** write a local fake server that returns two pages. Add a test for a repeated cursor and assert that the client stops with an error.
7. **Shell quoting:** round-trip arguments containing spaces, quotes, and `$()` through a local test shell. Assert that no extra command executes.
8. **Recovery:** simulate a lost creation response and verify that a retry reconciles the operation name without creating another Pod.
9. **Models:** serve a small byte sequence locally, resume it with Range, and compare a SHA-256 digest before renaming the partial file.
10. **npm:** build a local tarball and install it into a temporary prefix. Check `--version`, `--help`, exit codes, and signals without changing the global installation.

## 33. Common diagnostics

| Symptom | What to inspect |
| --- | --- |
| `borrow of moved value` | Was an owned `String` or collection passed by value? Borrow it if ownership need not transfer. |
| `cannot borrow as mutable` | Does the operation actually modify the value? Declare the binding mutable and avoid overlapping borrows. |
| `?` cannot convert an error | Check the enclosing function's return type and required conversion; add context where useful. |
| `future cannot be sent` | Check values held across `.await` and the runtime requirements; avoid retaining unsuitable guards across suspension. |
| `fastgpu` command not found | Inspect `PATH`, `command -v fastgpu`, the installation prefix, and whether the shell has refreshed its environment. |
| API key missing | Check the environment or runpodctl configuration without printing the secret into logs. |
| SSH host key changed | Verify the remote server identity before changing the dedicated known-hosts file. |
| Creation outcome uncertain | Inspect `fastgpu recover` and account Pods; reconcile before clearing state or trying another creation. |
| Model not ready | Inspect `fastgpu logs`, engine compatibility, model paths, VRAM, and the requested alias; the Pod can remain running. |
| npm archive fails on another Linux | Inspect architecture, Node version, executable permission, and glibc compatibility. |

## 34. Final review checklist

- Explain what is local, what runs on the Pod, and what persists on the Network Volume.
- Explain why ownership, borrowing, `Option`, `Result`, and `Drop` matter for files and subprocesses.
- Trace a command from Clap parsing through `App::dispatch` to its module and tests.
- Explain why an interrupted POST cannot safely be retried as if it were a GET.
- Identify the controls preventing deletion of the configured, managed, or attached volume.
- Describe how SSH keys, host keys, command quoting, rsync, and local port forwarding differ.
- Describe the difference between model integrity, model readiness, and a successful real inference.
- Explain what goes into the npm archive, why it needs Node at runtime, and why the worker image is a separate artifact.
- Run local tests before any live experiment; check the actual provider state and storage after testing.

## Further reading

- [The Rust Programming Language](https://doc.rust-lang.org/book/) and [Rust by Example](https://doc.rust-lang.org/rust-by-example/)
- [The Cargo Book](https://doc.rust-lang.org/cargo/), [Tokio tutorial](https://tokio.rs/tokio/tutorial), and [Serde documentation](https://serde.rs/)
- [OpenSSH manuals](https://www.openssh.com/manual.html) and [rsync documentation](https://download.samba.org/pub/rsync/rsync.1)
- [RunPod documentation](https://docs.runpod.io/), [Docker documentation](https://docs.docker.com/), and [npm documentation](https://docs.npmjs.com/)
