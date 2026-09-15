use crate::error::CliError;
use anyhow::Result;
use std::future::Future;
use std::time::Duration;

/// Register before starting the command so enable cannot race signal setup.
pub fn interruption() -> std::io::Result<impl Future<Output = Result<()>>> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut interrupt = signal(SignalKind::interrupt())?;
        let mut terminate = signal(SignalKind::terminate())?;
        Ok(async move {
            tokio::select! {
                _ = interrupt.recv() => Err(CliError::Interrupted.into()),
                _ = terminate.recv() => Err(CliError::Terminated.into()),
            }
        })
    }
    #[cfg(not(unix))]
    {
        Ok(async {
            tokio::signal::ctrl_c().await?;
            Err(CliError::Interrupted.into())
        })
    }
}

pub async fn run_command(
    body: impl Future<Output = Result<()>>,
    timeout_ms: Option<u64>,
    streaming: bool,
    interruption: impl Future<Output = Result<()>>,
) -> Result<()> {
    let command = async {
        if let Some(ms) = timeout_ms {
            match tokio::time::timeout(Duration::from_millis(ms), body).await {
                Ok(result) => result,
                Err(_) if streaming => Ok(()),
                Err(_) => Err(CliError::Timeout(format!(
                    "command did not complete within {} ms",
                    ms
                ))
                .into()),
            }
        } else {
            body.await
        }
    };
    tokio::select! {
        // Poll signal setup before the body on platforms using ctrl_c().
        biased;
        result = interruption => result,
        result = command => result,
    }
}

pub async fn cleanup(
    disable: impl Future<Output = Result<()>>,
    disconnect: impl Future<Output = ()>,
) {
    const SOURCE_DISABLE_TIMEOUT: Duration = Duration::from_secs(1);
    match tokio::time::timeout(SOURCE_DISABLE_TIMEOUT, disable).await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => eprintln!("Warning: could not disable console source: {}", err),
        Err(_) => eprintln!("Warning: timed out while disabling console source"),
    }
    disconnect.await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::future::pending;

    #[tokio::test]
    async fn interruption_during_enable_reaches_disable_then_disconnect() {
        let events = RefCell::new(Vec::new());
        let mut source_to_disable = None;
        let (started, ready) = tokio::sync::oneshot::channel();
        let session = async {
            let result = run_command(
                async {
                    source_to_disable = Some(1);
                    events.borrow_mut().push("enable");
                    started.send(()).unwrap();
                    pending().await
                },
                None,
                true,
                async {
                    ready.await.unwrap();
                    events.borrow_mut().push("interrupt");
                    Err(CliError::Interrupted.into())
                },
            )
            .await;
            cleanup(
                async {
                    assert_eq!(source_to_disable, Some(1));
                    events.borrow_mut().push("disable");
                    Ok(())
                },
                async {
                    events.borrow_mut().push("disconnect");
                },
            )
            .await;
            result
        };
        let error = tokio::time::timeout(Duration::from_secs(1), session)
            .await
            .expect("interruption must reach cleanup")
            .unwrap_err();
        assert_eq!(crate::error::classify_exit_code(&error), 130);
        assert_eq!(
            *events.borrow(),
            ["enable", "interrupt", "disable", "disconnect"]
        );
    }

    #[tokio::test]
    async fn timeout_keeps_streaming_and_bounded_command_exit_behavior() {
        assert!(run_command(pending(), Some(1), true, pending())
            .await
            .is_ok());
        let error = run_command(pending(), Some(1), false, pending())
            .await
            .unwrap_err();
        assert_eq!(crate::error::classify_exit_code(&error), 40);
    }

    #[tokio::test]
    async fn command_errors_are_preserved() {
        let error = run_command(
            async { Err(CliError::NotFound("source".into()).into()) },
            None,
            true,
            pending(),
        )
        .await
        .unwrap_err();
        assert_eq!(crate::error::classify_exit_code(&error), 20);
    }

    #[tokio::test]
    async fn failed_disable_still_disconnects() {
        let disconnected = Cell::new(false);
        cleanup(async { anyhow::bail!("link lost") }, async {
            disconnected.set(true);
        })
        .await;
        assert!(disconnected.get());
    }

    #[tokio::test]
    async fn stalled_disable_is_bounded_before_disconnect() {
        let disconnected = Cell::new(false);
        tokio::time::timeout(
            Duration::from_secs(2),
            cleanup(pending(), async {
                disconnected.set(true);
            }),
        )
        .await
        .expect("disable must be bounded");
        assert!(disconnected.get());
    }

    // Run actual process signals in a child: signal handlers are process-global.
    #[cfg(unix)]
    #[test]
    fn process_signals_reach_cleanup() {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};
        use std::sync::mpsc;

        for signal in ["INT", "TERM"] {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "lifecycle::tests::signal_child", "--nocapture"])
                .env("CFCLI_TEST_SIGNAL", signal)
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let stdout = child.stdout.take().unwrap();
            let (sender, receiver) = mpsc::channel();
            let reader = std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    if line.unwrap() == "source enabled" {
                        let _ = sender.send(());
                    }
                }
            });
            if receiver.recv_timeout(Duration::from_secs(5)).is_err() {
                let _ = child.kill();
                let _ = child.wait();
                panic!("child did not enable source");
            }
            let sent = Command::new("/bin/sh")
                .args([
                    "-c",
                    "kill -s \"$1\" \"$2\"",
                    "cfcli-test",
                    signal,
                    &child.id().to_string(),
                ])
                .status()
                .unwrap();
            assert!(sent.success());
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("{signal} did not finish cleanup");
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            reader.join().unwrap();
            assert!(status.success(), "{signal} child failed: {status}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn signal_child() {
        use std::io::Write;
        let Ok(signal) = std::env::var("CFCLI_TEST_SIGNAL") else {
            return;
        };
        let signal_future = interruption().unwrap();
        let events = RefCell::new(Vec::new());
        let error = run_command(
            async {
                events.borrow_mut().push("enable");
                println!("source enabled");
                std::io::stdout().flush().unwrap();
                pending().await
            },
            None,
            true,
            signal_future,
        )
        .await
        .unwrap_err();
        cleanup(
            async {
                events.borrow_mut().push("disable");
                Ok(())
            },
            async {
                events.borrow_mut().push("disconnect");
            },
        )
        .await;
        assert_eq!(*events.borrow(), ["enable", "disable", "disconnect"]);
        assert_eq!(
            crate::error::classify_exit_code(&error),
            if signal == "INT" { 130 } else { 143 }
        );
    }
}
