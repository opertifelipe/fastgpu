#[tokio::main]
async fn main() {
    let result = async {
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = fastgpu::execute() => result,
            _ = interrupt.recv() => anyhow::bail!("operation interrupted"),
            _ = terminate.recv() => anyhow::bail!("operation terminated"),
        }
    }
    .await;
    if let Err(error) = result {
        eprintln!("fastgpu: {error:#}");
        let code = error
            .downcast_ref::<fastgpu::CommandFailure>()
            .map_or(1, |e| e.code);
        std::process::exit(code);
    }
}
